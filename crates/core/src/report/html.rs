// Standalone HTML profile report: one file, no external resources, charts as
// inline SVG so it works offline and can be emailed or attached.

use crate::profile::narrative::{fmt_num, pct, plural};
use crate::profile::quality::Severity;
use crate::profile::stats::Bin;
use crate::profile::{ColumnProfile, Correlations, DatasetProfile};
use std::fmt::Write;

const STYLE: &str = r#"
:root {
  color-scheme: light;
  --page: #f9f9f7; --surface: #fcfcfb; --ink: #0b0b0b; --ink-2: #52514e; --muted: #898781;
  --grid: #e1e0d9; --axis: #c3c2b7; --border: rgba(11,11,11,0.10);
  --series-1: #2a78d6; --div-neg: #e34948; --div-mid: #f0efec; --div-pos: #2a78d6;
  --high: #d03b3b; --medium: #ec835a; --low: #898781;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    color-scheme: dark;
    --page: #0d0d0d; --surface: #1a1a19; --ink: #f0efec; --ink-2: #c3c2b7; --muted: #898781;
    --grid: #2c2c2a; --axis: #383835; --border: rgba(255,255,255,0.10);
    --series-1: #3987e5; --div-neg: #e66767; --div-mid: #383835; --div-pos: #3987e5;
  }
}
:root[data-theme="dark"] {
  color-scheme: dark;
  --page: #0d0d0d; --surface: #1a1a19; --ink: #f0efec; --ink-2: #c3c2b7; --muted: #898781;
  --grid: #2c2c2a; --axis: #383835; --border: rgba(255,255,255,0.10);
  --series-1: #3987e5; --div-neg: #e66767; --div-mid: #383835; --div-pos: #3987e5;
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--page); color: var(--ink);
  font: 15px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }
main { max-width: 1080px; margin: 0 auto; padding: 32px 16px 64px; }
h1 { font-size: 26px; margin: 0 0 4px; overflow-wrap: anywhere; }
h2 { font-size: 18px; margin: 40px 0 12px; }
h3 { font-size: 16px; margin: 0; overflow-wrap: anywhere; }
.sub { color: var(--ink-2); margin: 0 0 24px; }
.tiles { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 12px; }
.tile { background: var(--surface); border: 1px solid var(--border); border-radius: 10px; padding: 14px 16px; }
.tile .v { font-size: 24px; font-weight: 600; }
.tile .k { color: var(--ink-2); font-size: 13px; }
.card { background: var(--surface); border: 1px solid var(--border); border-radius: 10px; padding: 16px 20px; }
.summary p { margin: 0 0 8px; }
.summary p:last-child { margin: 0; }
code { font: 0.92em ui-monospace, SFMono-Regular, Menlo, monospace; }
.issues { list-style: none; margin: 0; padding: 0; display: grid; gap: 8px; }
.issue { background: var(--surface); border: 1px solid var(--border); border-left-width: 4px;
  border-radius: 8px; padding: 10px 14px; }
.issue.high { border-left-color: var(--high); }
.issue.medium { border-left-color: var(--medium); }
.issue.low { border-left-color: var(--low); }
.issue .t { font-weight: 600; }
.issue .d { color: var(--ink-2); }
.issue .s { color: var(--ink-2); font-size: 14px; }
.badge { display: inline-block; font-size: 12px; font-weight: 600; border-radius: 999px;
  padding: 1px 8px; margin-right: 8px; border: 1px solid currentColor; }
.badge.high { color: var(--high); } .badge.medium { color: var(--medium); } .badge.low { color: var(--muted); }
.filters { display: flex; gap: 8px; flex-wrap: wrap; margin-bottom: 12px; }
.filters button { font: inherit; font-size: 14px; color: var(--ink); background: var(--surface);
  border: 1px solid var(--border); border-radius: 999px; padding: 4px 12px; cursor: pointer; }
.filters button[aria-pressed="true"] { border-color: var(--ink); font-weight: 600; }
.columns { display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 12px; }
.col-head { display: flex; justify-content: space-between; gap: 8px; align-items: baseline; flex-wrap: wrap; }
.chips { display: flex; gap: 6px; }
.chip { font-size: 12px; color: var(--ink-2); border: 1px solid var(--border); border-radius: 999px; padding: 0 8px; }
.col-sum { color: var(--ink-2); font-size: 14px; margin: 8px 0; }
dl.stats { display: grid; grid-template-columns: repeat(3, 1fr); gap: 6px 12px; margin: 8px 0 12px; }
dl.stats div { min-width: 0; }
dl.stats dt { color: var(--muted); font-size: 12px; }
dl.stats dd { margin: 0; font-size: 14px; font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
svg { display: block; width: 100%; height: auto; overflow: visible; }
svg text { fill: var(--muted); font-size: 11px; font-family: inherit; }
svg .bar { fill: var(--series-1); }
svg .bar:hover { opacity: 0.75; }
svg .base { stroke: var(--axis); stroke-width: 1; }
.toplist { display: grid; gap: 4px; font-size: 13px; }
.toprow { display: grid; grid-template-columns: minmax(0, 40%) 1fr auto; gap: 8px; align-items: center; }
.toprow .lab { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.toprow .track { height: 10px; }
.toprow .fill { height: 10px; background: var(--series-1); border-radius: 0 4px 4px 0; min-width: 2px; }
.toprow .n { color: var(--ink-2); font-variant-numeric: tabular-nums; }
.scroll { overflow-x: auto; }
table.corr { border-collapse: separate; border-spacing: 2px; font-size: 12px; font-variant-numeric: tabular-nums; }
table.corr th { color: var(--ink-2); font-weight: 500; padding: 2px 6px; text-align: right; white-space: nowrap; }
table.corr thead th { writing-mode: vertical-rl; transform: rotate(180deg); text-align: left; }
table.corr td { width: 44px; height: 32px; text-align: center; border-radius: 4px; }
.legend { display: flex; align-items: center; gap: 8px; color: var(--ink-2); font-size: 12px; margin-top: 8px; }
.legend .ramp { width: 160px; height: 10px; border-radius: 4px;
  background: linear-gradient(90deg, var(--div-neg), var(--div-mid), var(--div-pos)); }
.pairs { margin: 12px 0 0; padding-left: 20px; color: var(--ink-2); font-size: 14px; }
footer { color: var(--muted); font-size: 13px; margin-top: 48px; }
"#;

const SCRIPT: &str = r#"
document.querySelectorAll('.filters button').forEach(function (b) {
  b.addEventListener('click', function () {
    document.querySelectorAll('.filters button').forEach(function (o) { o.setAttribute('aria-pressed', o === b); });
    var sev = b.dataset.sev;
    document.querySelectorAll('.issue').forEach(function (i) {
      i.hidden = sev !== 'all' && !i.classList.contains(sev);
    });
  });
});
"#;

pub fn render(p: &DatasetProfile) -> String {
    let mut h = String::with_capacity(64 * 1024);
    let _ = write!(
        h,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{} profile</title><style>{}</style></head><body><main>",
        esc(&p.name),
        STYLE
    );
    let _ = write!(
        h,
        "<h1>{}</h1><p class=\"sub\">{} · {} in memory</p>",
        esc(&p.name),
        esc(&p.format),
        fmt_bytes(p.memory_bytes)
    );

    h.push_str("<section class=\"tiles\">");
    for (value, label) in [
        (fmt_num(p.rows as f64), "rows"),
        (fmt_num(p.cols as f64), "columns"),
        (pct(p.completeness), "complete"),
        (
            fmt_num(p.duplicate_rows as f64),
            plural(p.duplicate_rows, "duplicate row", "duplicate rows"),
        ),
        (
            format!("{}", p.warnings.len()),
            plural(p.warnings.len(), "issue", "issues"),
        ),
    ] {
        let _ = write!(
            h,
            "<div class=\"tile\"><div class=\"v\">{}</div><div class=\"k\">{}</div></div>",
            esc(&value),
            label
        );
    }
    h.push_str("</section>");

    h.push_str("<h2>Summary</h2><section class=\"card summary\">");
    for line in &p.summary {
        let _ = write!(h, "<p>{}</p>", inline_code(line));
    }
    h.push_str("</section>");

    let _ = write!(h, "<h2>Issues ({})</h2>", p.warnings.len());
    if p.warnings.is_empty() {
        h.push_str("<p class=\"card\">No data quality issues found.</p>");
    } else {
        let count = |s: Severity| p.warnings.iter().filter(|w| w.severity == s).count();
        let _ = write!(
            h,
            "<div class=\"filters\" role=\"group\" aria-label=\"Filter issues by priority\">\
             <button data-sev=\"all\" aria-pressed=\"true\">All {}</button>\
             <button data-sev=\"high\" aria-pressed=\"false\">High {}</button>\
             <button data-sev=\"medium\" aria-pressed=\"false\">Medium {}</button>\
             <button data-sev=\"low\" aria-pressed=\"false\">Low {}</button></div>",
            p.warnings.len(),
            count(Severity::High),
            count(Severity::Medium),
            count(Severity::Low)
        );
        h.push_str("<ul class=\"issues\">");
        for w in &p.warnings {
            let sev = w.severity.label();
            let icon = match w.severity {
                Severity::High => "▲",
                Severity::Medium => "●",
                Severity::Low => "○",
            };
            let _ = write!(
                h,
                "<li class=\"issue {sev}\"><div class=\"t\"><span class=\"badge {sev}\">{icon} {sev}</span>{}</div>\
                 <div class=\"d\">{}</div><div class=\"s\">Suggested: {}</div></li>",
                inline_code(&w.title),
                inline_code(&w.detail),
                inline_code(&w.suggestion),
            );
        }
        h.push_str("</ul>");
    }

    if p.correlations.columns.len() >= 2 {
        h.push_str("<h2>Relationships between numeric columns</h2><section class=\"card\">");
        correlation_table(&mut h, &p.correlations);
        h.push_str("</section>");
    }

    h.push_str("<h2>Columns</h2><section class=\"columns\">");
    for c in &p.columns {
        column_card(&mut h, c);
    }
    h.push_str("</section>");

    let _ = write!(
        h,
        "<footer>Generated by DataTool {}. Values were cleaned on load as described in the issues above; the source file was not changed.</footer>",
        env!("CARGO_PKG_VERSION")
    );
    let _ = write!(h, "</main><script>{}</script></body></html>", SCRIPT);
    h
}

fn column_card(h: &mut String, c: &ColumnProfile) {
    let _ = write!(
        h,
        "<article class=\"card\"><div class=\"col-head\"><h3>{}</h3>\
         <div class=\"chips\"><span class=\"chip\">{}</span><span class=\"chip\">{}</span></div></div>\
         <p class=\"col-sum\">{}</p>",
        esc(&c.name),
        c.inferred.label(),
        c.role.label(),
        inline_code(&c.summary)
    );

    let mut stats: Vec<(&str, String)> = vec![
        ("present", fmt_num(c.count as f64)),
        ("missing", pct(c.missing_fraction())),
        ("distinct", fmt_num(c.distinct as f64)),
    ];
    // Summary statistics and charts of identifier values aren't meaningful
    let is_id = c.role == crate::profile::Role::Identifier;
    let numeric = c.numeric.as_ref().filter(|_| !is_id);
    let categorical = c.categorical.as_ref().filter(|_| !is_id);
    if let Some(n) = numeric {
        stats.extend([
            ("min", fmt_num(n.min)),
            ("median", fmt_num(n.median)),
            ("max", fmt_num(n.max)),
            ("mean", fmt_num(n.mean)),
            ("std dev", fmt_num(n.std)),
            ("outliers", fmt_num(n.outliers as f64)),
        ]);
    }
    if let Some(t) = &c.temporal {
        stats.extend([("earliest", t.min.clone()), ("latest", t.max.clone())]);
    }
    if let Some(s) = categorical {
        stats.extend([
            ("avg length", format!("{:.0}", s.mean_length)),
            ("once only", fmt_num(s.singletons as f64)),
        ]);
    }
    h.push_str("<dl class=\"stats\">");
    for (k, v) in stats {
        let _ = write!(h, "<div><dt>{}</dt><dd>{}</dd></div>", k, esc(&v));
    }
    h.push_str("</dl>");

    if let Some(n) = numeric {
        if n.histogram.len() > 1 {
            let labels: Vec<String> = n
                .histogram
                .iter()
                .map(|b| {
                    if b.start == b.end {
                        fmt_num(b.start)
                    } else {
                        format!("{} to {}", fmt_num(b.start), fmt_num(b.end))
                    }
                })
                .collect();
            bar_chart(
                h,
                &c.name,
                &n.histogram,
                &labels,
                &fmt_num(n.min),
                &fmt_num(n.max),
            );
        }
    } else if let Some(t) = &c.temporal {
        if t.periods.len() > 1 {
            let bins: Vec<Bin> = t
                .periods
                .iter()
                .enumerate()
                .map(|(i, (_, count))| Bin {
                    start: i as f64,
                    end: i as f64 + 1.0,
                    count: *count,
                })
                .collect();
            let labels: Vec<String> = t.periods.iter().map(|(k, _)| k.clone()).collect();
            bar_chart(
                h,
                &c.name,
                &bins,
                &labels,
                &labels[0],
                &labels[labels.len() - 1],
            );
        }
    } else if let Some(s) = categorical {
        if c.role != crate::profile::Role::FreeText || s.top.first().is_some_and(|(_, n)| *n > 1) {
            let rows: Vec<(String, usize)> = s.top.iter().take(8).cloned().collect();
            top_list(h, &rows, c.count);
        }
    } else if let Some(b) = &c.boolean {
        top_list(
            h,
            &[
                ("true".to_string(), b.true_count),
                ("false".to_string(), b.false_count),
            ],
            c.count,
        );
    }
    h.push_str("</article>");
}

// Vertical bars with a hover tooltip per bar (SVG <title>)
fn bar_chart(h: &mut String, name: &str, bins: &[Bin], labels: &[String], left: &str, right: &str) {
    let (w, ht, pad_bottom) = (300.0, 80.0, 16.0);
    let max = bins.iter().map(|b| b.count).max().unwrap_or(1).max(1) as f64;
    let slot = w / bins.len() as f64;
    let gap = if slot > 6.0 { 2.0 } else { 0.5 };
    let _ = write!(
        h,
        "<svg viewBox=\"0 0 {w} {}\" role=\"img\" aria-label=\"Distribution of {}\">",
        ht + pad_bottom,
        esc(name)
    );
    for (i, (b, label)) in bins.iter().zip(labels).enumerate() {
        let bh = if b.count == 0 {
            0.0
        } else {
            (b.count as f64 / max * ht).max(1.5)
        };
        let x = i as f64 * slot + gap / 2.0;
        let r = (slot - gap).min(8.0) / 2.0;
        let _ = write!(
            h,
            "<g><rect class=\"bar\" x=\"{x:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{bh:.2}\" rx=\"{:.1}\"/>\
             <rect x=\"{:.2}\" y=\"0\" width=\"{slot:.2}\" height=\"{ht}\" fill=\"transparent\"><title>{}: {}</title></rect></g>",
            ht - bh,
            slot - gap,
            r.min(4.0).min(bh / 2.0),
            i as f64 * slot,
            esc(label),
            fmt_num(b.count as f64)
        );
    }
    let _ = write!(
        h,
        "<line class=\"base\" x1=\"0\" x2=\"{w}\" y1=\"{ht}\" y2=\"{ht}\"/>\
         <text x=\"0\" y=\"{}\">{}</text><text x=\"{w}\" y=\"{}\" text-anchor=\"end\">{}</text></svg>",
        ht + 13.0,
        esc(left),
        ht + 13.0,
        esc(right)
    );
}

// Horizontal bars for the most common values, labelled with counts
fn top_list(h: &mut String, rows: &[(String, usize)], total: usize) {
    let max = rows.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1) as f64;
    h.push_str("<div class=\"toplist\">");
    for (label, n) in rows {
        let _ = write!(
            h,
            "<div class=\"toprow\" title=\"{}: {} ({})\"><span class=\"lab\">{}</span>\
             <span class=\"track\"><span class=\"fill\" style=\"display:block;width:{:.1}%\"></span></span>\
             <span class=\"n\">{}</span></div>",
            esc(label),
            fmt_num(*n as f64),
            pct(*n as f64 / total.max(1) as f64),
            esc(label),
            *n as f64 / max * 100.0,
            fmt_num(*n as f64)
        );
    }
    h.push_str("</div>");
}

fn correlation_table(h: &mut String, c: &Correlations) {
    h.push_str("<div class=\"scroll\"><table class=\"corr\"><thead><tr><th></th>");
    for name in &c.columns {
        let _ = write!(h, "<th scope=\"col\">{}</th>", esc(name));
    }
    h.push_str("</tr></thead><tbody>");
    for (i, row) in c.matrix.iter().enumerate() {
        let _ = write!(h, "<tr><th scope=\"row\">{}</th>", esc(&c.columns[i]));
        for (j, r) in row.iter().enumerate() {
            match r {
                Some(r) => {
                    let (pole, text) = if *r >= 0.0 {
                        ("--div-pos", if *r > 0.55 { "#fff" } else { "var(--ink)" })
                    } else {
                        ("--div-neg", if *r < -0.55 { "#fff" } else { "var(--ink)" })
                    };
                    let mix = (r.abs() * 100.0).round();
                    let _ = write!(
                        h,
                        "<td style=\"background:color-mix(in oklab, var({pole}) {mix}%, var(--div-mid));color:{text}\" \
                         title=\"{} vs {}: r = {}\">{}</td>",
                        esc(&c.columns[i]),
                        esc(&c.columns[j]),
                        fmt_corr(*r),
                        if i == j { "1".into() } else { fmt_corr(*r) }
                    );
                }
                None => h.push_str("<td title=\"not enough paired values\">–</td>"),
            }
        }
        h.push_str("</tr>");
    }
    h.push_str(
        "</tbody></table></div><div class=\"legend\"><span>−1</span><span class=\"ramp\"></span><span>+1</span>\
         <span>Pearson correlation (blue: rise together, red: one rises as the other falls)</span></div>",
    );
    let strong: Vec<_> = c
        .top_pairs
        .iter()
        .filter(|p| p.pearson.abs() >= 0.3)
        .take(5)
        .collect();
    if !strong.is_empty() {
        h.push_str("<ul class=\"pairs\">");
        for p in strong {
            let _ = write!(
                h,
                "<li><code>{}</code> &amp; <code>{}</code>: r = {:.2}{}</li>",
                esc(&p.a),
                esc(&p.b),
                p.pearson,
                p.spearman
                    .map(|s| format!(", rank correlation {:.2}", s))
                    .unwrap_or_default()
            );
        }
        h.push_str("</ul>");
    }
}

// Two decimals, without a "-0.00" for tiny negative values
fn fmt_corr(r: f64) -> String {
    let s = format!("{:.2}", r);
    if s == "-0.00" {
        "0.00".into()
    } else {
        s
    }
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

// Escape text and turn `backticks` into <code> spans
fn inline_code(s: &str) -> String {
    esc(s)
        .split('`')
        .enumerate()
        .map(|(i, part)| {
            if i % 2 == 1 {
                format!("<code>{}</code>", part)
            } else {
                part.to_string()
            }
        })
        .collect()
}

fn fmt_bytes(n: usize) -> String {
    let n = n as f64;
    if n >= 1e9 {
        format!("{:.1} GB", n / 1e9)
    } else if n >= 1e6 {
        format!("{:.1} MB", n / 1e6)
    } else if n >= 1e3 {
        format!("{:.0} KB", n / 1e3)
    } else {
        format!("{} B", n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_formats_code() {
        assert_eq!(
            esc("<a href=\"x\">&</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;"
        );
        assert_eq!(inline_code("`a<b` is x"), "<code>a&lt;b</code> is x");
    }

    #[test]
    fn renders_fixture_report() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/messy.csv");
        let p = crate::profile(&crate::load(path).unwrap()).unwrap();
        let html = render(&p);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<h1>messy.csv</h1>"));
        assert!(html.contains("class=\"issue high\""));
        assert!(html.contains("<svg"));
        assert!(html.ends_with("</html>"));
    }
}
