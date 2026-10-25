// Plain-text profile summary for the terminal

use crate::profile::narrative::{fmt_num, pct, plural};
use crate::profile::quality::Severity;
use crate::profile::DatasetProfile;
use std::fmt::Write;

pub fn render(p: &DatasetProfile) -> String {
    let mut out = String::new();
    let rule = "─".repeat(72);
    let _ = writeln!(out, "{}", p.name);
    let _ = writeln!(
        out,
        "{} · {} rows × {} columns · {} complete · {} duplicate {}",
        p.format,
        fmt_num(p.rows as f64),
        p.cols,
        pct(p.completeness),
        fmt_num(p.duplicate_rows as f64),
        plural(p.duplicate_rows, "row", "rows")
    );
    let _ = writeln!(out, "{}", rule);

    let _ = writeln!(out, "\nSUMMARY");
    for line in &p.summary {
        let _ = writeln!(out, "  {}", line);
    }

    let _ = writeln!(out, "\nISSUES");
    if p.warnings.is_empty() {
        let _ = writeln!(out, "  None found.");
    }
    for w in &p.warnings {
        let marker = match w.severity {
            Severity::High => "[HIGH]",
            Severity::Medium => "[MED] ",
            Severity::Low => "[LOW] ",
        };
        let _ = writeln!(out, "  {} {}", marker, w.title);
        let _ = writeln!(out, "         {}", w.detail);
        let _ = writeln!(out, "         → {}", w.suggestion);
    }

    let _ = writeln!(out, "\nCOLUMNS");
    let width = p
        .columns
        .iter()
        .map(|c| c.name.chars().count())
        .max()
        .unwrap_or(6)
        .clamp(6, 28);
    let _ = writeln!(
        out,
        "  {:<width$}  {:<11}  {:<10}  {:>8}  {:>9}",
        "column", "type", "role", "missing", "distinct"
    );
    for c in &p.columns {
        let name: String = if c.name.chars().count() > width {
            c.name.chars().take(width - 1).collect::<String>() + "…"
        } else {
            c.name.clone()
        };
        let _ = writeln!(
            out,
            "  {:<width$}  {:<11}  {:<10}  {:>8}  {:>9}",
            name,
            c.inferred.label(),
            c.role.label(),
            pct(c.missing_fraction()),
            fmt_num(c.distinct as f64)
        );
    }
    let _ = writeln!(out);
    for c in &p.columns {
        let _ = writeln!(out, "  • {}", c.summary);
    }
    out
}
