// Data quality rules. Each rule looks at the finished profile and reports a
// problem in plain language with a suggested fix.

use super::narrative::{fmt_num, pct, plural, quote_list};
use super::{ColumnProfile, DatasetProfile, Role};
use crate::dataset::infer::InferredType;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    // Likely to give wrong answers if ignored
    High,
    // Worth fixing before analysis
    Medium,
    // Good to know; fixed automatically or minor
    Low,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Warning {
    pub severity: Severity,
    pub column: Option<String>,
    pub title: String,
    pub detail: String,
    pub suggestion: String,
}

pub fn check(p: &DatasetProfile) -> Vec<Warning> {
    let mut warnings = Vec::new();
    let mut warn =
        |severity, column: Option<&str>, title: String, detail: String, suggestion: &str| {
            warnings.push(Warning {
                severity,
                column: column.map(String::from),
                title,
                detail,
                suggestion: suggestion.to_string(),
            })
        };

    if p.duplicate_rows > 0 {
        let share = p.duplicate_rows as f64 / p.rows as f64;
        warn(
            if share > 0.05 {
                Severity::High
            } else {
                Severity::Medium
            },
            None,
            format!(
                "{} duplicate {}",
                fmt_num(p.duplicate_rows as f64),
                plural(p.duplicate_rows, "row", "rows")
            ),
            format!(
                "{} of rows are exact copies of another row, which inflates counts and averages.",
                pct(share)
            ),
            "Remove duplicate rows unless repeated records are expected.",
        );
    }

    for c in &p.columns {
        let name = Some(c.name.as_str());
        let col = format!("`{}`", c.name);
        let missing = c.missing_fraction();

        match c.role {
            Role::Empty => {
                warn(
                    Severity::Medium,
                    name,
                    format!("{} is empty", col),
                    "Every value is missing.".into(),
                    "Drop the column.",
                );
                continue;
            }
            Role::Constant => warn(
                Severity::Low,
                name,
                format!("{} has a single value", col),
                format!(
                    "Every non-missing value is {}, so it carries no information.",
                    constant_value(c)
                ),
                "Drop the column unless the value matters for context.",
            ),
            _ => {}
        }

        if missing > 0.05 {
            let severity = if missing > 0.5 {
                Severity::High
            } else if missing > 0.2 {
                Severity::Medium
            } else {
                Severity::Low
            };
            warn(
                severity,
                name,
                format!("{} is {} missing", col, pct(missing)),
                format!(
                    "{} of {} rows have no value.",
                    fmt_num(c.missing as f64),
                    fmt_num((c.count + c.missing) as f64)
                ),
                if missing > 0.5 {
                    "Consider dropping the column, or check whether missing means something (e.g. \"none\")."
                } else {
                    "Check why values are missing; fill them (e.g. with the median) or exclude those rows."
                },
            );
        }

        let info = &c.cleaning;
        if info.unparsed > 0 {
            warn(
                Severity::Medium,
                name,
                {
                    let (one, many) = unreadable_noun(c.inferred);
                    format!("{} has {}", col, plural_count(info.unparsed, one, many))
                },
                format!(
                    "{} treated as missing: {}.",
                    if info.unparsed == 1 {
                        "It was"
                    } else {
                        "They were"
                    },
                    quote_list(&info.unparsed_examples)
                ),
                "Correct these entries at the source, or convert them by hand.",
            );
        }
        if info.placeholder_nulls > 0 {
            warn(
                Severity::Low,
                name,
                format!("{} uses placeholders for missing values", col),
                format!(
                    "{} like {} {} read as missing.",
                    plural_count(info.placeholder_nulls, "value", "values"),
                    quote_list(&info.placeholder_examples),
                    if info.placeholder_nulls == 1 {
                        "was"
                    } else {
                        "were"
                    }
                ),
                "No action needed; check that these really mean \"missing\".",
            );
        }
        if info.date_formats.len() > 1 {
            let formats: Vec<String> = info.date_formats.keys().cloned().collect();
            let day_month_mix = formats.iter().any(|f| f.starts_with("%m/"))
                && formats.iter().any(|f| f.starts_with("%d/"));
            warn(
                if day_month_mix {
                    Severity::High
                } else {
                    Severity::Medium
                },
                name,
                format!("{} mixes {} date formats", col, info.date_formats.len()),
                if day_month_mix {
                    "Both month-first (01/02/2023) and day-first (15/02/2023) dates appear, so dates like 03/04 are ambiguous; they were read as month-first.".into()
                } else {
                    format!("Formats seen: {}.", formats.join(", "))
                },
                "Standardise dates to one format (ideally YYYY-MM-DD) at the source.",
            );
        }
        if info.whitespace_padded > 0 {
            warn(
                Severity::Low,
                name,
                format!("{} has extra spaces", col),
                format!(
                    "{} had leading or trailing spaces, which were removed.",
                    plural_count(info.whitespace_padded, "value", "values")
                ),
                "No action needed; trim values at the source to avoid mismatches elsewhere.",
            );
        }
        if let Some(original) = &c.original_name {
            warn(
                Severity::Low,
                name,
                format!("Column header {:?} was renamed to {}", original, col),
                "The header was blank, padded with spaces, or duplicated another column.".into(),
                "Give every column a unique, trimmed name.",
            );
        }

        if c.role == Role::Identifier && c.distinct < c.count {
            let repeats = c.count - c.distinct;
            warn(
                Severity::High,
                name,
                format!("{} looks like an ID but has repeats", col),
                format!(
                    "{} repeated, so it can't uniquely identify rows.",
                    plural_count(repeats, "value is", "values are")
                ),
                "Remove duplicate records or check whether the ID is reused.",
            );
        }

        if let Some(s) = &c.categorical {
            if !s.inconsistent_variants.is_empty() && c.role != Role::FreeText {
                let examples: Vec<String> = s
                    .inconsistent_variants
                    .iter()
                    .take(3)
                    .map(|group| quote_list(group).replace(", ", " / "))
                    .collect();
                warn(
                    Severity::Medium,
                    name,
                    format!("{} has inconsistent spellings", col),
                    format!(
                        "Some values differ only by capitalization or spacing: {}.",
                        examples.join("; ")
                    ),
                    "Standardise the spelling so these are counted as one category.",
                );
            }
        }

        // Value-level checks only make sense for quantities, not IDs or codes
        if let Some(s) = c.numeric.as_ref().filter(|_| c.role == Role::Measure) {
            for (value, count) in &s.sentinels {
                warn(
                    Severity::Medium,
                    name,
                    format!("{} contains {}", col, fmt_num(*value)),
                    format!(
                        "{} {} {}, a common code for \"missing\" that will distort averages.",
                        fmt_num(*count as f64),
                        plural(*count, "row has", "rows have"),
                        fmt_num(*value)
                    ),
                    "If it means missing, replace it with an empty value.",
                );
            }
            let sentinel_negatives: usize = s
                .sentinels
                .iter()
                .filter(|(v, _)| *v < 0.0)
                .map(|(_, n)| n)
                .sum();
            let other_negatives = s.negatives - sentinel_negatives;
            if other_negatives > 0 && (s.negatives as f64) <= c.count as f64 * 0.05 {
                warn(
                    Severity::Medium,
                    name,
                    format!(
                        "{} has {} in otherwise positive data",
                        col,
                        plural_count(other_negatives, "negative value", "negative values")
                    ),
                    format!(
                        "For example {}. These may be errors or special codes.",
                        s.negative_examples
                            .iter()
                            .filter(|v| !s.sentinels.iter().any(|(sv, _)| sv == *v))
                            .map(|v| fmt_num(*v))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    "Check whether negative values are valid here.",
                );
            }
            let iqr = s.q3 - s.q1;
            let non_sentinel_outliers: Vec<f64> = s
                .outlier_examples
                .iter()
                .copied()
                .filter(|v| *v < s.q1 - 3.0 * iqr || *v > s.q3 + 3.0 * iqr)
                .filter(|v| !s.sentinels.iter().any(|(sv, _)| sv == v))
                .collect();
            if s.extreme_outliers > 0 && !non_sentinel_outliers.is_empty() {
                warn(
                    if (s.extreme_outliers as f64) > c.count as f64 * 0.01 {
                        Severity::Medium
                    } else {
                        Severity::Low
                    },
                    name,
                    format!(
                        "{} has {}",
                        col,
                        plural_count(s.extreme_outliers, "extreme outlier", "extreme outliers")
                    ),
                    format!(
                        "Values far outside the typical range ({} to {}), e.g. {}.",
                        fmt_num(s.q1),
                        fmt_num(s.q3),
                        non_sentinel_outliers
                            .iter()
                            .take(3)
                            .map(|v| fmt_num(*v))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    "Confirm these are real; consider the median rather than the mean for this column.",
                );
            }
        }

        if let Some(t) = &c.temporal {
            if t.future > 0 {
                warn(
                    Severity::Medium,
                    name,
                    format!(
                        "{} has {} in the future",
                        col,
                        plural_count(t.future, "date", "dates")
                    ),
                    format!("The latest is {}.", t.max),
                    "Check for typos in the year or a wrong date format.",
                );
            }
            if t.before_1900 > 0 {
                warn(
                    Severity::Medium,
                    name,
                    format!(
                        "{} has {} before 1900",
                        col,
                        plural_count(t.before_1900, "date", "dates")
                    ),
                    format!("The earliest is {}.", t.min),
                    "These are often placeholders (e.g. 1900-01-01) or typos.",
                );
            }
        }
    }

    for pair in &p.correlations.top_pairs {
        if pair.pearson.abs() >= 0.95 {
            warn(
                Severity::Low,
                Some(&pair.a),
                format!("`{}` and `{}` are nearly identical", pair.a, pair.b),
                format!(
                    "Their correlation is {:.2}, so one is almost a copy of the other.",
                    pair.pearson
                ),
                "Keep only one of them in models to avoid double-counting.",
            );
        }
    }

    // Stable sort keeps column order within each severity
    warnings.sort_by_key(|w| w.severity);
    warnings
}

fn plural_count(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", fmt_num(n as f64), plural(n, one, many))
}

// What to call values that didn't match the column's type
fn unreadable_noun(t: InferredType) -> (&'static str, &'static str) {
    match t {
        InferredType::Integer | InferredType::Float => ("non-numeric value", "non-numeric values"),
        InferredType::Boolean => (
            "value that isn't true/false",
            "values that aren't true/false",
        ),
        InferredType::Date | InferredType::Datetime => ("unreadable date", "unreadable dates"),
        _ => ("unreadable value", "unreadable values"),
    }
}

fn constant_value(c: &ColumnProfile) -> String {
    if let Some(s) = &c.numeric {
        fmt_num(s.min)
    } else if let Some(s) = &c.categorical {
        s.top
            .first()
            .map(|(v, _)| format!("{:?}", v))
            .unwrap_or_default()
    } else if let Some(t) = &c.temporal {
        t.min.clone()
    } else if let Some(b) = &c.boolean {
        (b.true_count > 0).to_string()
    } else {
        "the same".into()
    }
}
