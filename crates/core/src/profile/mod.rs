//! Profiling: per-column statistics, relationships between columns, data
//! quality warnings and a plain-language summary of a [`Dataset`].

pub mod narrative;
pub mod quality;
pub mod stats;

use crate::dataset::infer::{ColumnInference, InferredType};
use crate::dataset::Dataset;
use crate::error::Result;
use chrono::{DateTime, Datelike, NaiveDateTime, Utc};
use polars::prelude::*;
use quality::Warning;
use serde::Serialize;
use stats::{CategoricalStats, NumericStats};
use std::collections::BTreeMap;

// Correlations are computed on at most this many rows (evenly spaced)
const CORRELATION_SAMPLE: usize = 200_000;
// Columns beyond this are left out of the correlation matrix
const MAX_CORRELATION_COLUMNS: usize = 30;
// A text column with at most this many distinct values is a category
const MAX_CATEGORIES: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    // Unique per row, e.g. customer_id
    Identifier,
    // A quantity to summarise, e.g. income
    Measure,
    // A small set of labels, e.g. plan
    Category,
    // Yes/no
    Flag,
    // Dates and times
    Timestamp,
    // Mostly-unique text, e.g. names or comments
    FreeText,
    // A single value in every row
    Constant,
    // No values at all
    Empty,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Identifier => "identifier",
            Role::Measure => "measure",
            Role::Category => "category",
            Role::Flag => "flag",
            Role::Timestamp => "timestamp",
            Role::FreeText => "free text",
            Role::Constant => "constant",
            Role::Empty => "empty",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DatasetProfile {
    pub name: String,
    pub format: String,
    pub rows: usize,
    pub cols: usize,
    pub memory_bytes: usize,
    // Extra copies of rows that appear more than once
    pub duplicate_rows: usize,
    pub missing_cells: usize,
    // Share of cells that are present, 0..=1
    pub completeness: f64,
    pub type_counts: BTreeMap<String, usize>,
    pub columns: Vec<ColumnProfile>,
    pub correlations: Correlations,
    pub warnings: Vec<Warning>,
    pub summary: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ColumnProfile {
    pub name: String,
    pub original_name: Option<String>,
    pub inferred: InferredType,
    pub role: Role,
    pub count: usize,
    pub missing: usize,
    pub distinct: usize,
    pub numeric: Option<NumericStats>,
    pub categorical: Option<CategoricalStats>,
    pub temporal: Option<TemporalStats>,
    pub boolean: Option<BooleanStats>,
    pub cleaning: ColumnInference,
    pub summary: String,
}

impl ColumnProfile {
    pub fn missing_fraction(&self) -> f64 {
        let total = self.count + self.missing;
        if total == 0 {
            0.0
        } else {
            self.missing as f64 / total as f64
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TemporalStats {
    pub min: String,
    pub max: String,
    pub span_days: f64,
    pub has_time: bool,
    // Counts per period ("2023-04" or "2023"), oldest first
    pub periods: Vec<(String, usize)>,
    pub future: usize,
    pub before_1900: usize,
    // Longest stretch with no values, in days
    pub largest_gap_days: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BooleanStats {
    pub true_count: usize,
    pub false_count: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Correlations {
    pub columns: Vec<String>,
    // Pearson correlation matrix aligned with `columns` (None where undefined)
    pub matrix: Vec<Vec<Option<f64>>>,
    // Strongest pairs first
    pub top_pairs: Vec<CorrelationPair>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CorrelationPair {
    pub a: String,
    pub b: String,
    pub pearson: f64,
    pub spearman: Option<f64>,
}

pub fn profile(ds: &Dataset) -> Result<DatasetProfile> {
    let rows = ds.rows();
    let mut columns = Vec::with_capacity(ds.cols());
    for (meta, column) in ds.columns.iter().zip(ds.df.columns()) {
        columns.push(profile_column(
            &meta.name,
            meta.original_name.clone(),
            &meta.inference,
            column.as_materialized_series(),
            rows,
        )?);
    }

    let duplicate_rows = if rows > 1 {
        rows - ds
            .df
            .unique_stable(None, UniqueKeepStrategy::Any, None)?
            .height()
    } else {
        0
    };
    let missing_cells: usize = columns.iter().map(|c| c.missing).sum();
    let total_cells = rows * ds.cols();
    let mut type_counts = BTreeMap::new();
    for c in &columns {
        *type_counts
            .entry(c.inferred.label().to_string())
            .or_default() += 1;
    }
    let correlations = correlations(ds, &columns)?;

    let mut profile = DatasetProfile {
        name: ds.name.clone(),
        format: ds.format.to_string(),
        rows,
        cols: ds.cols(),
        memory_bytes: ds.df.estimated_size(),
        duplicate_rows,
        missing_cells,
        completeness: if total_cells == 0 {
            1.0
        } else {
            1.0 - missing_cells as f64 / total_cells as f64
        },
        type_counts,
        columns,
        correlations,
        warnings: Vec::new(),
        summary: Vec::new(),
    };
    profile.warnings = quality::check(&profile);
    for column in &mut profile.columns {
        column.summary = narrative::describe_column(column);
    }
    profile.summary = narrative::describe_dataset(&profile);
    Ok(profile)
}

fn profile_column(
    name: &str,
    original_name: Option<String>,
    inference: &ColumnInference,
    series: &Series,
    rows: usize,
) -> Result<ColumnProfile> {
    let missing = series.null_count();
    let count = rows - missing;
    let inferred = inference.inferred;
    let mut profile = ColumnProfile {
        name: name.to_string(),
        original_name,
        inferred,
        role: Role::Empty,
        count,
        missing,
        distinct: 0,
        numeric: None,
        categorical: None,
        temporal: None,
        boolean: None,
        cleaning: inference.clone(),
        summary: String::new(),
    };
    if count == 0 {
        return Ok(profile);
    }

    match inferred {
        InferredType::Integer | InferredType::Float => {
            let mut values = f64_values(series)?;
            values.retain(|v| !v.is_nan());
            profile.numeric = stats::numeric_stats(&mut values);
            // numeric_stats leaves `values` sorted
            profile.distinct = count_distinct_sorted(&values);
        }
        InferredType::Date | InferredType::Datetime => {
            let mut millis = millis_values(series)?;
            millis.sort_unstable();
            profile.distinct = {
                let mut d = millis.clone();
                d.dedup();
                d.len()
            };
            profile.temporal = temporal_stats(&millis, inferred == InferredType::Datetime);
        }
        InferredType::Boolean => {
            let values = series.bool()?;
            let true_count = values.sum().unwrap_or(0) as usize;
            profile.boolean = Some(BooleanStats {
                true_count,
                false_count: count - true_count,
            });
            profile.distinct = (true_count > 0) as usize + (true_count < count) as usize;
        }
        InferredType::Text => {
            profile.categorical = stats::categorical_stats(series.str()?.iter().flatten());
            profile.distinct = profile.categorical.as_ref().map_or(0, |c| c.distinct);
        }
        InferredType::Empty | InferredType::Other => {
            profile.distinct = series.n_unique()?;
        }
    }
    profile.role = role(&profile);
    Ok(profile)
}

fn role(c: &ColumnProfile) -> Role {
    if c.count == 0 {
        return Role::Empty;
    }
    if c.distinct == 1 {
        return Role::Constant;
    }
    let unique_ratio = c.distinct as f64 / c.count as f64;
    let is_id_name = looks_like_id(&c.name);
    match c.inferred {
        InferredType::Integer | InferredType::Text if is_id_name && unique_ratio >= 0.9 => {
            Role::Identifier
        }
        InferredType::Text
            if unique_ratio == 1.0
                && c.categorical
                    .as_ref()
                    .is_some_and(|s| s.min_length == s.max_length && s.max_length >= 8) =>
        {
            // Fixed-length unique codes such as UUIDs or order numbers
            Role::Identifier
        }
        InferredType::Integer | InferredType::Float | InferredType::Other => Role::Measure,
        InferredType::Boolean => Role::Flag,
        InferredType::Date | InferredType::Datetime => Role::Timestamp,
        InferredType::Text => {
            if c.distinct <= MAX_CATEGORIES && (c.distinct as f64) <= (c.count as f64 * 0.5) {
                Role::Category
            } else {
                Role::FreeText
            }
        }
        InferredType::Empty => Role::Empty,
    }
}

// "id", "customer_id", "CustomerID", "order-id", "uuid", "user key"
pub fn looks_like_id(name: &str) -> bool {
    let lower = name.to_lowercase();
    if lower.contains("uuid") || lower.contains("guid") {
        return true;
    }
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    tokens.iter().any(|t| *t == "id" || *t == "key")
        || name.ends_with("Id")
        || (name.ends_with("ID") && name.len() > 2)
}

fn f64_values(series: &Series) -> Result<Vec<f64>> {
    Ok(series
        .cast(&DataType::Float64)?
        .f64()?
        .iter()
        .flatten()
        .collect())
}

fn f64_options(series: &Series) -> Result<Vec<Option<f64>>> {
    Ok(series.cast(&DataType::Float64)?.f64()?.iter().collect())
}

fn millis_values(series: &Series) -> Result<Vec<i64>> {
    let as_datetime = series.cast(&DataType::Datetime(TimeUnit::Milliseconds, None))?;
    Ok(as_datetime
        .cast(&DataType::Int64)?
        .i64()?
        .iter()
        .flatten()
        .collect())
}

fn count_distinct_sorted(sorted: &[f64]) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    1 + sorted.windows(2).filter(|w| w[0] != w[1]).count()
}

fn temporal_stats(sorted_millis: &[i64], has_time: bool) -> Option<TemporalStats> {
    let (first, last) = (*sorted_millis.first()?, *sorted_millis.last()?);
    let to_dt = |ms: i64| DateTime::<Utc>::from_timestamp_millis(ms).map(|d| d.naive_utc());
    let format = |dt: NaiveDateTime| {
        if has_time {
            dt.format("%Y-%m-%d %H:%M").to_string()
        } else {
            dt.format("%Y-%m-%d").to_string()
        }
    };
    const DAY_MS: f64 = 86_400_000.0;
    let span_days = (last - first) as f64 / DAY_MS;

    // Monthly buckets for spans up to ~5 years, yearly beyond that
    let monthly = span_days <= 5.0 * 366.0;
    let mut periods: BTreeMap<String, usize> = BTreeMap::new();
    let mut before_1900 = 0;
    for ms in sorted_millis {
        if let Some(dt) = to_dt(*ms) {
            let key = if monthly {
                format!("{:04}-{:02}", dt.year(), dt.month())
            } else {
                format!("{:04}", dt.year())
            };
            *periods.entry(key).or_default() += 1;
            if dt.year() < 1900 {
                before_1900 += 1;
            }
        }
    }
    let now = Utc::now().timestamp_millis();
    let largest_gap_days = sorted_millis
        .windows(2)
        .map(|w| (w[1] - w[0]) as f64 / DAY_MS)
        .fold(0.0, f64::max);

    Some(TemporalStats {
        min: format(to_dt(first)?),
        max: format(to_dt(last)?),
        span_days,
        has_time,
        periods: periods.into_iter().collect(),
        future: sorted_millis.iter().filter(|ms| **ms > now).count(),
        before_1900,
        largest_gap_days,
    })
}

fn correlations(ds: &Dataset, columns: &[ColumnProfile]) -> Result<Correlations> {
    let candidates: Vec<&ColumnProfile> = columns
        .iter()
        .filter(|c| c.role == Role::Measure && c.count >= 3)
        .take(MAX_CORRELATION_COLUMNS)
        .collect();
    if candidates.len() < 2 {
        return Ok(Correlations::default());
    }

    let rows = ds.rows();
    let step = rows.div_ceil(CORRELATION_SAMPLE).max(1);
    let mut values = Vec::with_capacity(candidates.len());
    for c in &candidates {
        let series = ds.df.column(&c.name)?.as_materialized_series();
        let all = f64_options(series)?;
        let sampled: Vec<Option<f64>> = all
            .into_iter()
            .step_by(step)
            .map(|v| v.filter(|x| x.is_finite()))
            .collect();
        values.push(sampled);
    }
    let ranks: Vec<Vec<Option<f64>>> = values.iter().map(|v| stats::ranks(v)).collect();

    let n = candidates.len();
    let mut matrix = vec![vec![None; n]; n];
    let mut top_pairs = Vec::new();
    for i in 0..n {
        matrix[i][i] = Some(1.0);
        for j in (i + 1)..n {
            let (xs, ys) = paired(&values[i], &values[j]);
            let r = stats::pearson(&xs, &ys);
            matrix[i][j] = r;
            matrix[j][i] = r;
            if let Some(pearson) = r {
                let (rx, ry) = paired(&ranks[i], &ranks[j]);
                top_pairs.push(CorrelationPair {
                    a: candidates[i].name.clone(),
                    b: candidates[j].name.clone(),
                    pearson,
                    spearman: stats::pearson(&rx, &ry),
                });
            }
        }
    }
    top_pairs.sort_by(|a, b| b.pearson.abs().total_cmp(&a.pearson.abs()));
    top_pairs.truncate(10);

    Ok(Correlations {
        columns: candidates.iter().map(|c| c.name.clone()).collect(),
        matrix,
        top_pairs,
    })
}

// Rows where both values are present
fn paired(a: &[Option<f64>], b: &[Option<f64>]) -> (Vec<f64>, Vec<f64>) {
    a.iter()
        .zip(b)
        .filter_map(|(x, y)| Some(((*x)?, (*y)?)))
        .unzip()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_names() {
        for name in [
            "id",
            "customer_id",
            "CustomerID",
            "orderId",
            "user key",
            "row-uuid",
        ] {
            assert!(looks_like_id(name), "{}", name);
        }
        for name in ["idea", "paid", "valid", "Width", "Kidney"] {
            assert!(!looks_like_id(name), "{}", name);
        }
    }

    #[test]
    fn profiles_messy_fixture() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/messy.csv");
        let ds = crate::load(path).unwrap();
        let p = profile(&ds).unwrap();
        assert_eq!((p.rows, p.cols), (21, 9));
        assert_eq!(p.duplicate_rows, 1);

        let col = |name: &str| p.columns.iter().find(|c| c.name == name).unwrap();
        assert_eq!(col("customer_id").role, Role::Identifier);
        assert_eq!(col("customer_id").distinct, 20);
        assert_eq!(col("income").role, Role::Measure);
        assert_eq!(col("plan").role, Role::Category);
        assert_eq!(col("active").role, Role::Flag);
        assert_eq!(col("signup_date").role, Role::Timestamp);
        assert_eq!(col("notes").role, Role::FreeText);
        assert_eq!(col("Name").role, Role::FreeText);

        let plan = col("plan").categorical.as_ref().unwrap();
        assert_eq!(plan.inconsistent_variants.len(), 1);
        let age = col("age").numeric.as_ref().unwrap();
        assert_eq!(age.sentinels, vec![(-999.0, 1)]);
        let dates = col("signup_date").temporal.as_ref().unwrap();
        assert_eq!(dates.min, "2023-01-15");
        assert_eq!(dates.max, "2023-07-21");

        assert!(!p.warnings.is_empty());
        assert!(!p.summary.is_empty());
        assert!(p.columns.iter().all(|c| !c.summary.is_empty()));
    }
}
