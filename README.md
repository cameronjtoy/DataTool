# DataTool

Understand a dataset in seconds. Point DataTool at a CSV, Excel, Parquet or JSON
file to see what's in it, what's wrong with it, and what stands out, all in
plain language and without writing code.

```
$ datatool profile customers.csv --html report.html

customers.csv
CSV (separator ',') · 21 rows × 9 columns · 87.3% complete · 1 duplicate row
────────────────────────────────────────────────────────────────────────

SUMMARY
  customers.csv has 21 rows and 9 columns (1 date, 3 integer, 4 text, 1 true/false).
  It is 87.3% complete; the most gaps are in `notes` (90.5% missing).
  1 row is an exact duplicate.
  Found 15 issues (3 high, 6 medium, 6 low priority). Start with: `customer_id`
  looks like an ID but has repeats; `signup_date` mixes 3 date formats; ...

ISSUES
  [HIGH] `signup_date` mixes 3 date formats
         Both month-first (01/02/2023) and day-first (15/02/2023) dates appear ...
         → Standardise dates to one format (ideally YYYY-MM-DD) at the source.
  [MED]  `age` contains -999
         1 row has -999, a common code for "missing" that will distort averages.
         → If it means missing, replace it with an empty value.
  ...
```

## Install

DataTool is written in Rust. With a [Rust toolchain](https://rustup.rs) installed, run:

```sh
cargo install --path crates/cli
```

This installs the `datatool` command.

## Commands

| Command | What it does |
|---|---|
| `datatool profile <file>` | Full profile: summary, ranked data quality issues, and per-column statistics |
| `datatool profile <file> --html report.html` | Also writes a standalone HTML report with charts (works offline, light and dark mode) |
| `datatool profile <file> --json` | Prints the complete profile as JSON for use in other tools |
| `datatool schema <file>` | Each column's detected type and what was cleaned while loading |
| `datatool head <file> -n 20` | The first rows, after type detection |

Supported files are `.csv`, `.tsv` and `.txt` (the separator is detected
automatically), `.xlsx`, `.xls`, `.xlsm`, `.xlsb` and `.ods` (first sheet),
`.parquet`, `.json`, and `.ndjson`/`.jsonl`.

## What the profile tells you

- **Types, detected from the values themselves.** This covers integers,
  decimals, true/false values, dates and date-times (in many formats, including
  mixed ones) and text. Numbers like `$1,250` and placeholders like `N/A`,
  `-`, `unknown` and `null` are handled.
- **What each column is for.** Each column gets a role: identifier, measure,
  category, flag, timestamp, free text, constant or empty.
- **Per-column statistics.**
  - Numeric columns get range, median, mean, spread, skew, outliers and a
    histogram.
  - Category columns get their top values.
  - Date columns get their range and a count per month or year.
- **Relationships.** A correlation matrix covers the numeric columns, and the
  strongest pairs are listed.
- **Data quality issues, ranked by priority, each with a suggested fix.** It
  checks for:
  - duplicate rows, and IDs that repeat
  - heavy missingness
  - values that don't match the column's type
  - mixed or ambiguous date formats
  - inconsistent spellings (`Basic` vs `basic`)
  - missing-value codes like `-999`
  - unexpected negatives and extreme outliers
  - future dates
  - padded or duplicate headers
- **A plain-language summary** of all of the above.

The source file is never modified.

## Formula engine

The `datatool-core` library also contains a spreadsheet formula engine with
A1 references, ranges, dependency tracking, automatic recalculation and
circular-reference detection:

```rust
use datatool_core::spreadsheet::{cell::CellValue, Spreadsheet};

let mut sheet = Spreadsheet::new();
sheet.set("A1", CellValue::Number(72.0))?;
sheet.set("B1", CellValue::Formula(r#"IF(A1 >= 65, "senior", "adult")"#.into()))?;
sheet.evaluate("B1")?; // Value::Text("senior")
```

It supports `+ - * / ^`, `&` (concatenation) and `= <> < > <= >=`. The
functions are `SUM`, `AVERAGE`, `MIN`, `MAX`, `COUNT`, `ABS`, `SQRT`,
`ROUND`, `IF`, `AND`, `OR` and `NOT`.

## Project layout

```
crates/
  core/   datatool-core library: dataset loading, profiling, reports, formulas
  cli/    the `datatool` command
```

Run the checks the same way CI does:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

See [ROADMAP.md](ROADMAP.md) for what's planned next.
