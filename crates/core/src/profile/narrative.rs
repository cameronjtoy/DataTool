// Plain-language descriptions generated from profile statistics. Template
// based, so output is deterministic and needs no external service.

use super::quality::Severity;
use super::{ColumnProfile, DatasetProfile, Role};

pub fn describe_dataset(p: &DatasetProfile) -> Vec<String> {
    let mut out = Vec::new();

    let types: Vec<String> = p
        .type_counts
        .iter()
        .map(|(t, n)| format!("{} {}", n, t))
        .collect();
    out.push(format!(
        "{} has {} {} and {} {} ({}).",
        p.name,
        fmt_num(p.rows as f64),
        plural(p.rows, "row", "rows"),
        p.cols,
        plural(p.cols, "column", "columns"),
        types.join(", ")
    ));

    if p.missing_cells == 0 {
        out.push("No values are missing.".into());
    } else if let Some(worst) = p
        .columns
        .iter()
        .filter(|c| c.missing > 0)
        .max_by_key(|c| c.missing)
    {
        out.push(format!(
            "It is {} complete; the most gaps are in `{}` ({} missing).",
            pct(p.completeness),
            worst.name,
            pct(worst.missing_fraction())
        ));
    }
    if p.duplicate_rows > 0 {
        out.push(format!(
            "{} {} exact {}.",
            fmt_num(p.duplicate_rows as f64),
            plural(p.duplicate_rows, "row is an", "rows are"),
            plural(p.duplicate_rows, "duplicate", "duplicates")
        ));
    }

    let names = |role: Role| -> Vec<String> {
        p.columns
            .iter()
            .filter(|c| c.role == role)
            .map(|c| format!("`{}`", c.name))
            .collect()
    };
    let mut groups = Vec::new();
    for (role, label) in [
        (Role::Identifier, "identifiers"),
        (Role::Measure, "measures"),
        (Role::Category, "categories"),
        (Role::Timestamp, "dates"),
        (Role::Flag, "yes/no flags"),
        (Role::FreeText, "free text"),
    ] {
        let cols = names(role);
        if !cols.is_empty() {
            groups.push(format!("{}: {}", label, cols.join(", ")));
        }
    }
    if !groups.is_empty() {
        out.push(format!("Columns by role — {}.", groups.join("; ")));
    }

    if let Some(pair) = p.correlations.top_pairs.first() {
        if pair.pearson.abs() >= 0.5 {
            out.push(format!(
                "The strongest relationship is between `{}` and `{}` (r = {:.2}): when one is higher, the other tends to be {}.",
                pair.a,
                pair.b,
                pair.pearson,
                if pair.pearson > 0.0 { "higher too" } else { "lower" }
            ));
        } else {
            out.push(
                "No strong linear relationships between numeric columns (all |r| < 0.5).".into(),
            );
        }
    }

    if p.warnings.is_empty() {
        out.push("No data quality issues were found.".into());
    } else {
        let count = |s: Severity| p.warnings.iter().filter(|w| w.severity == s).count();
        let first: Vec<String> = p
            .warnings
            .iter()
            .filter(|w| w.severity != Severity::Low)
            .take(3)
            .map(|w| w.title.clone())
            .collect();
        let mut sentence = format!(
            "Found {} {} ({} high, {} medium, {} low priority).",
            p.warnings.len(),
            plural(p.warnings.len(), "issue", "issues"),
            count(Severity::High),
            count(Severity::Medium),
            count(Severity::Low)
        );
        if !first.is_empty() {
            sentence.push_str(&format!(" Start with: {}.", first.join("; ")));
        }
        out.push(sentence);
    }
    out
}

pub fn describe_column(c: &ColumnProfile) -> String {
    let name = format!("`{}`", c.name);
    let mut s = match c.role {
        Role::Empty => format!("{} has no values.", name),
        Role::Constant => format!("{} has the same value in every row.", name),
        Role::Identifier => format!(
            "{} looks like an identifier ({} unique values).",
            name,
            fmt_num(c.distinct as f64)
        ),
        Role::Measure => match &c.numeric {
            Some(n) => {
                let mut s = format!(
                    "{} ranges from {} to {}, with a median of {}.",
                    name,
                    fmt_num(n.min),
                    fmt_num(n.max),
                    fmt_num(n.median)
                );
                if n.skew > 1.0 {
                    s.push_str(&format!(
                        " It is right-skewed: a few large values pull the mean ({}) above the median.",
                        fmt_num(n.mean)
                    ));
                } else if n.skew < -1.0 {
                    s.push_str(&format!(
                        " It is left-skewed: a few small values pull the mean ({}) below the median.",
                        fmt_num(n.mean)
                    ));
                } else {
                    s.push_str(&format!(
                        " Values are fairly symmetric (mean {}).",
                        fmt_num(n.mean)
                    ));
                }
                if let Some((code, _)) = n.sentinels.first() {
                    s.push_str(&format!(
                        " These figures include {}, which is probably a code for \"missing\".",
                        fmt_num(*code)
                    ));
                }
                s
            }
            None => format!("{} is numeric.", name),
        },
        Role::Category => match &c.categorical {
            Some(cat) => {
                let (top, n) = &cat.top[0];
                format!(
                    "{} has {} categories; the most common is {:?} ({} of values).",
                    name,
                    cat.distinct,
                    top,
                    pct(*n as f64 / c.count as f64)
                )
            }
            None => format!("{} is a category.", name),
        },
        Role::Flag => match &c.boolean {
            Some(b) => format!(
                "{} is true for {} of rows.",
                name,
                pct(b.true_count as f64 / c.count as f64)
            ),
            None => format!("{} is a yes/no flag.", name),
        },
        Role::Timestamp => match &c.temporal {
            Some(t) => format!(
                "{} covers {} to {} ({}).",
                name,
                t.min,
                t.max,
                describe_span(t.span_days)
            ),
            None => format!("{} holds dates.", name),
        },
        Role::FreeText => match &c.categorical {
            Some(cat) => format!(
                "{} is mostly-unique text ({} distinct values, {:.0} characters on average).",
                name,
                fmt_num(cat.distinct as f64),
                cat.mean_length
            ),
            None => format!("{} is free text.", name),
        },
    };
    if c.missing > 0 && c.role != Role::Empty {
        s.push_str(&format!(
            " {} of values are missing.",
            pct(c.missing_fraction())
        ));
    }
    s
}

fn describe_span(days: f64) -> String {
    if days < 1.0 {
        format!("{:.0} hours", days * 24.0)
    } else if days < 60.0 {
        format!("{:.0} days", days)
    } else if days < 730.0 {
        format!("about {:.0} months", days / 30.44)
    } else {
        format!("about {:.1} years", days / 365.25)
    }
}

// Human-friendly number: thousands separators, at most 2 decimals, trailing zeros dropped
pub fn fmt_num(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    if x != 0.0 && x.abs() < 1e-6 {
        return format!("{:.2e}", x);
    }
    // Two significant digits for small values (0.0023), else 1–2 decimals
    let decimals = if x.abs() >= 1000.0 {
        1
    } else if x != 0.0 && x.abs() < 0.01 {
        (-x.abs().log10().floor()) as usize + 1
    } else {
        2
    };
    let rounded = format!("{:.*}", decimals, x.abs());
    let (int_part, frac_part) = rounded.split_once('.').unwrap_or((&rounded, ""));
    let mut grouped = String::new();
    for (i, ch) in int_part.chars().enumerate() {
        if i > 0 && (int_part.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let frac = frac_part.trim_end_matches('0');
    let sign = if x < 0.0 && (grouped != "0" || !frac.is_empty()) {
        "-"
    } else {
        ""
    };
    if frac.is_empty() {
        format!("{}{}", sign, grouped)
    } else {
        format!("{}{}.{}", sign, grouped, frac)
    }
}

// 0.123 -> "12.3%", with "<0.1%" / ">99.9%" near the ends
pub fn pct(fraction: f64) -> String {
    let p = fraction * 100.0;
    if p > 0.0 && p < 0.1 {
        "<0.1%".into()
    } else if fraction < 1.0 && p > 99.9 {
        // Never round an incomplete share up to 100%
        ">99.9%".into()
    } else if (p - p.round()).abs() < 0.05 {
        format!("{:.0}%", p)
    } else {
        format!("{:.1}%", p)
    }
}

pub fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 {
        one
    } else {
        many
    }
}

// ["a", "b"] -> "\"a\", \"b\""
pub fn quote_list(values: &[String]) -> String {
    values
        .iter()
        .map(|v| format!("{:?}", v))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formatting() {
        assert_eq!(fmt_num(1250000.0), "1,250,000");
        assert_eq!(fmt_num(-999.0), "-999");
        assert_eq!(fmt_num(105762.38), "105,762.4");
        assert_eq!(fmt_num(4.56789), "4.57");
        assert_eq!(fmt_num(2.5), "2.5");
        assert_eq!(fmt_num(0.0), "0");
        assert_eq!(fmt_num(0.002), "0.002");
        assert_eq!(fmt_num(-0.00234), "-0.0023");
        assert_eq!(fmt_num(1e-9), "1.00e-9");
        assert_eq!(fmt_num(-0.0001), "-0.0001");
    }

    #[test]
    fn percentages() {
        assert_eq!(pct(0.5), "50%");
        assert_eq!(pct(0.123), "12.3%");
        assert_eq!(pct(0.0004), "<0.1%");
        assert_eq!(pct(0.9996), ">99.9%");
        assert_eq!(pct(0.995), "99.5%");
        assert_eq!(pct(1.0), "100%");
    }
}
