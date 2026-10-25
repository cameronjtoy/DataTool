// Type inference for columns that arrive as text (CSV, Excel, string columns
// in JSON/Parquet). Each column is classified from its values, converted to a
// typed Polars series, and everything that had to be cleaned along the way is
// recorded so the profile can report it.

use chrono::{NaiveDate, NaiveDateTime};
use polars::prelude::*;
use serde::Serialize;
use std::collections::BTreeMap;

// Share of non-missing values that must parse for a type to be chosen. Values
// that don't parse are reported and treated as missing.
const PARSE_THRESHOLD: f64 = 0.95;
// Values checked before committing to a full pass for a candidate type
const SAMPLE_SIZE: usize = 1000;
const MAX_EXAMPLES: usize = 5;

// Values treated as missing (compared case-insensitively after trimming)
const PLACEHOLDERS: &[&str] = &[
    "",
    "na",
    "n/a",
    "nan",
    "null",
    "none",
    "nil",
    "-",
    "--",
    "?",
    "missing",
    "unknown",
    "#n/a",
    "undefined",
];

const BOOL_TRUE: &[&str] = &["true", "yes", "y", "t"];
const BOOL_FALSE: &[&str] = &["false", "no", "n", "f"];

// Tried in order, so month-first wins for ambiguous dates like 03/04/2024
const DATE_FORMATS: &[&str] = &[
    "%Y-%m-%d",
    "%Y/%m/%d",
    "%m/%d/%Y",
    "%d/%m/%Y",
    "%m-%d-%Y",
    "%d-%m-%Y",
    "%d.%m.%Y",
    "%b %d %Y",
    "%b %d, %Y",
    "%d %b %Y",
    "%B %d %Y",
    "%B %d, %Y",
    "%d %B %Y",
];
const DATETIME_FORMATS: &[&str] = &[
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M",
    "%Y-%m-%d %H:%M",
    "%m/%d/%Y %H:%M:%S",
    "%m/%d/%Y %H:%M",
    "%d/%m/%Y %H:%M:%S",
    "%d/%m/%Y %H:%M",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InferredType {
    Integer,
    Float,
    Boolean,
    Date,
    Datetime,
    Text,
    // No non-missing values at all
    Empty,
    // A type from a typed source (Parquet/JSON) that isn't profiled in detail
    Other,
}

impl InferredType {
    pub fn is_numeric(self) -> bool {
        matches!(self, InferredType::Integer | InferredType::Float)
    }

    pub fn is_temporal(self) -> bool {
        matches!(self, InferredType::Date | InferredType::Datetime)
    }

    pub fn label(self) -> &'static str {
        match self {
            InferredType::Integer => "integer",
            InferredType::Float => "decimal",
            InferredType::Boolean => "true/false",
            InferredType::Date => "date",
            InferredType::Datetime => "date & time",
            InferredType::Text => "text",
            InferredType::Empty => "empty",
            InferredType::Other => "other",
        }
    }

    pub fn from_dtype(dtype: &DataType) -> Self {
        match dtype {
            DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64 => InferredType::Integer,
            DataType::Float32 | DataType::Float64 => InferredType::Float,
            DataType::Boolean => InferredType::Boolean,
            DataType::Date => InferredType::Date,
            DataType::Datetime(..) => InferredType::Datetime,
            DataType::String => InferredType::Text,
            DataType::Null => InferredType::Empty,
            _ => InferredType::Other,
        }
    }
}

// What inference found and fixed in one column
#[derive(Debug, Clone, Serialize)]
pub struct ColumnInference {
    pub inferred: InferredType,
    // Values like "N/A" or "-" that were treated as missing
    pub placeholder_nulls: usize,
    pub placeholder_examples: Vec<String>,
    // Values that didn't match the column's type and were treated as missing
    pub unparsed: usize,
    pub unparsed_examples: Vec<String>,
    // Values that had leading or trailing whitespace (trimmed)
    pub whitespace_padded: usize,
    // Numbers written with thousands separators or currency symbols
    pub formatted_numbers: usize,
    // Date formats seen, with how many values used each
    pub date_formats: BTreeMap<String, usize>,
}

impl ColumnInference {
    pub fn typed(inferred: InferredType) -> Self {
        Self {
            inferred,
            placeholder_nulls: 0,
            placeholder_examples: Vec::new(),
            unparsed: 0,
            unparsed_examples: Vec::new(),
            whitespace_padded: 0,
            formatted_numbers: 0,
            date_formats: BTreeMap::new(),
        }
    }
}

// Infer a column's type from its text values and return the typed series
pub fn infer_column<'a, I>(name: &str, values: I) -> PolarsResult<(Series, ColumnInference)>
where
    I: IntoIterator<Item = Option<&'a str>>,
{
    let mut info = ColumnInference::typed(InferredType::Text);
    let mut cleaned: Vec<Option<&str>> = Vec::new();
    for value in values {
        let Some(raw) = value else {
            cleaned.push(None);
            continue;
        };
        let trimmed = raw.trim();
        if is_placeholder(trimmed) {
            info.placeholder_nulls += 1;
            push_example(&mut info.placeholder_examples, raw);
            cleaned.push(None);
            continue;
        }
        if trimmed.len() != raw.len() {
            info.whitespace_padded += 1;
        }
        cleaned.push(Some(trimmed));
    }

    let present: Vec<&str> = cleaned.iter().flatten().copied().collect();
    let name = PlSmallStr::from(name);
    if present.is_empty() {
        info.inferred = InferredType::Empty;
        return Ok((Series::new_null(name, cleaned.len()), info));
    }

    let sample = &present[..present.len().min(SAMPLE_SIZE)];
    let passes = |parse: &dyn Fn(&str) -> bool, values: &[&str]| {
        values.iter().filter(|v| parse(v)).count() as f64 / values.len() as f64 >= PARSE_THRESHOLD
    };

    let series = if passes(&|v| parse_bool(v).is_some(), sample)
        && passes(&|v| parse_bool(v).is_some(), &present)
    {
        info.inferred = InferredType::Boolean;
        let typed = convert(&cleaned, &mut info, parse_bool);
        Series::new(name, typed)
    } else if passes(&|v| parse_number(v).is_some(), sample)
        && passes(&|v| parse_number(v).is_some(), &present)
    {
        let mut all_integers = true;
        let typed = convert(&cleaned, &mut info, |v| {
            let (n, integral, formatted) = parse_number(v)?;
            all_integers &= integral;
            Some((n, formatted))
        });
        info.formatted_numbers = typed.iter().flatten().filter(|(_, f)| *f).count();
        let numbers: Vec<Option<f64>> = typed.into_iter().map(|v| v.map(|(n, _)| n)).collect();
        if all_integers {
            info.inferred = InferredType::Integer;
            Series::new(
                name,
                numbers
                    .into_iter()
                    .map(|v| v.map(|n| n as i64))
                    .collect::<Vec<_>>(),
            )
        } else {
            info.inferred = InferredType::Float;
            Series::new(name, numbers)
        }
    } else if passes(&|v| parse_temporal(v).is_some(), sample)
        && passes(&|v| parse_temporal(v).is_some(), &present)
    {
        let typed = convert(&cleaned, &mut info, parse_temporal);
        let mut has_time = false;
        for (_, format, with_time) in typed.iter().flatten() {
            *info.date_formats.entry(format.to_string()).or_default() += 1;
            has_time |= *with_time;
        }
        let millis: Vec<Option<i64>> = typed
            .iter()
            .map(|v| v.map(|(dt, _, _)| dt.and_utc().timestamp_millis()))
            .collect();
        let series =
            Series::new(name, millis).cast(&DataType::Datetime(TimeUnit::Milliseconds, None))?;
        if has_time {
            info.inferred = InferredType::Datetime;
            series
        } else {
            info.inferred = InferredType::Date;
            series.cast(&DataType::Date)?
        }
    } else {
        Series::new(name, cleaned)
    };
    Ok((series, info))
}

// Apply `parse` to every present value; failures become missing and are recorded
fn convert<T>(
    cleaned: &[Option<&str>],
    info: &mut ColumnInference,
    mut parse: impl FnMut(&str) -> Option<T>,
) -> Vec<Option<T>> {
    cleaned
        .iter()
        .map(|v| {
            let v = (*v)?;
            let parsed = parse(v);
            if parsed.is_none() {
                info.unparsed += 1;
                push_example(&mut info.unparsed_examples, v);
            }
            parsed
        })
        .collect()
}

fn push_example(examples: &mut Vec<String>, value: &str) {
    if examples.len() < MAX_EXAMPLES && !examples.iter().any(|e| e == value) {
        examples.push(value.to_string());
    }
}

pub fn is_placeholder(trimmed: &str) -> bool {
    trimmed.len() <= 9 && PLACEHOLDERS.iter().any(|p| p.eq_ignore_ascii_case(trimmed))
}

fn parse_bool(v: &str) -> Option<bool> {
    if BOOL_TRUE.iter().any(|t| t.eq_ignore_ascii_case(v)) {
        Some(true)
    } else if BOOL_FALSE.iter().any(|f| f.eq_ignore_ascii_case(v)) {
        Some(false)
    } else {
        None
    }
}

// Parse a number, accepting a leading currency symbol and thousands separators.
// Returns (value, is_integral, had_formatting).
fn parse_number(v: &str) -> Option<(f64, bool, bool)> {
    let (sign, rest) = match v.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", v.strip_prefix('+').unwrap_or(v)),
    };
    let unprefixed = rest.trim_start_matches(['$', '€', '£']);
    let had_currency = unprefixed.len() != rest.len();

    let digits: String;
    let body = if unprefixed.contains(',') {
        if !valid_thousands(unprefixed) {
            return None;
        }
        digits = unprefixed.replace(',', "");
        digits.as_str()
    } else {
        unprefixed
    };
    // Reject things Rust parses but people don't mean as numbers ("inf", "NaN")
    if !body.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let n: f64 = format!("{}{}", sign, body).parse().ok()?;
    if !n.is_finite() {
        return None;
    }
    let integral = !body.contains(['.', 'e', 'E']) && n.abs() < 9.0e15;
    Some((n, integral, had_currency || body.len() != unprefixed.len()))
}

// "1,234" or "12,345,678.9": groups of exactly three digits after the first
fn valid_thousands(v: &str) -> bool {
    let int_part = v.split('.').next().unwrap_or("");
    let mut groups = int_part.split(',');
    let first = groups.next().unwrap_or("");
    !first.is_empty()
        && first.len() <= 3
        && first.chars().all(|c| c.is_ascii_digit())
        && groups.all(|g| g.len() == 3 && g.chars().all(|c| c.is_ascii_digit()))
}

// Returns (timestamp, format used, has a time component)
fn parse_temporal(v: &str) -> Option<(NaiveDateTime, &'static str, bool)> {
    // Quick reject: dates contain a digit and are short
    if v.len() > 32 || !v.contains(|c: char| c.is_ascii_digit()) {
        return None;
    }
    for format in DATETIME_FORMATS {
        if let Ok(dt) = NaiveDateTime::parse_from_str(v, format) {
            return Some((dt, format, true));
        }
    }
    for format in DATE_FORMATS {
        if let Ok(d) = NaiveDate::parse_from_str(v, format) {
            return Some((d.and_hms_opt(0, 0, 0)?, format, false));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn infer(values: &[&str]) -> (Series, ColumnInference) {
        infer_column("c", values.iter().map(|v| Some(*v))).unwrap()
    }

    #[test]
    fn integers_with_placeholders_and_formatting() {
        let (s, info) = infer(&["1", " 2 ", "N/A", "$1,000", "-5", ""]);
        assert_eq!(info.inferred, InferredType::Integer);
        assert_eq!(s.dtype(), &DataType::Int64);
        assert_eq!(info.placeholder_nulls, 2);
        assert_eq!(info.whitespace_padded, 1);
        assert_eq!(info.formatted_numbers, 1);
        let values: Vec<Option<i64>> = s.i64().unwrap().iter().collect();
        assert_eq!(
            values,
            vec![Some(1), Some(2), None, Some(1000), Some(-5), None]
        );
    }

    #[test]
    fn floats_with_stray_text() {
        let mut values = vec!["1.5"; 40];
        values.push("twelve");
        let (s, info) = infer(&values);
        assert_eq!(info.inferred, InferredType::Float);
        assert_eq!(info.unparsed, 1);
        assert_eq!(info.unparsed_examples, vec!["twelve".to_string()]);
        assert_eq!(s.null_count(), 1);
    }

    #[test]
    fn mostly_text_stays_text() {
        let (s, info) = infer(&["apple", "banana", "3", "cherry"]);
        assert_eq!(info.inferred, InferredType::Text);
        assert_eq!(s.dtype(), &DataType::String);
        assert_eq!(info.unparsed, 0);
    }

    #[test]
    fn booleans_but_not_zero_one() {
        let (_, info) = infer(&["Yes", "no", "TRUE", "f"]);
        assert_eq!(info.inferred, InferredType::Boolean);
        let (_, info) = infer(&["1", "0", "1"]);
        assert_eq!(info.inferred, InferredType::Integer);
    }

    #[test]
    fn dates_in_mixed_formats() {
        let (s, info) = infer(&["2024-01-15", "01/16/2024", "Jan 17 2024", "31/01/2024"]);
        assert_eq!(info.inferred, InferredType::Date);
        assert_eq!(s.dtype(), &DataType::Date);
        assert_eq!(info.date_formats.len(), 4);
        assert_eq!(info.unparsed, 0);

        let (s, info) = infer(&["2024-01-15 10:30:00", "2024-01-15T11:00"]);
        assert_eq!(info.inferred, InferredType::Datetime);
        assert!(matches!(s.dtype(), DataType::Datetime(..)));
    }

    #[test]
    fn number_parsing_edge_cases() {
        assert_eq!(parse_number("1,234.5"), Some((1234.5, false, true)));
        assert_eq!(parse_number("-£12"), Some((-12.0, true, true)));
        assert_eq!(parse_number("1e3"), Some((1000.0, false, false)));
        assert_eq!(parse_number("12,34"), None);
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number("NaN"), None);
        assert_eq!(parse_number("abc"), None);
        assert_eq!(parse_number("1.2.3"), None);
    }

    #[test]
    fn all_missing_is_empty() {
        let (s, info) = infer(&["", "NA", "null"]);
        assert_eq!(info.inferred, InferredType::Empty);
        assert_eq!(s.null_count(), 3);
    }
}
