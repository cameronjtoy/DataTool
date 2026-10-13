# DataTool Roadmap

## Goal

A tool that lets someone open a dataset and understand it quickly, without writing
code first. Load a file, get a plain-language overview of what's in it and what's
wrong with it, clean and reshape it with spreadsheet-style formulas, chart it, and
run common analyses, with every step recorded so the result is reproducible.

**Who it's for:** analysts, students, and domain experts who know their data but
don't want to set up a Python notebook to answer "what's in this file and is it any
good?" A secondary audience is data scientists who want a fast first look before
moving to code.

**Principles**
- **Understanding first.** Every view answers "what does this tell me?", not just
  "here are the numbers."
- **Zero setup.** One binary, opens CSV/Excel/Parquet directly, sensible defaults.
- **Fast on real data.** Interactive at 1M+ rows; profiles a 1 GB CSV in seconds.
- **Reproducible.** Every edit is a step in a recipe that can be replayed, shared,
  or exported as code.
- **Non-destructive.** The source file is never modified; results are exported.

## Where things stand

**Milestone M1 ("First look") is implemented.** `datatool profile data.csv`
prints a summary, ranked data quality issues and per-column statistics, and can
write a standalone HTML report or JSON. On a 1M-row × 30-column CSV (198 MB) a
release build profiles everything in about 4 seconds on 4 cores. The workspace is
split into the `datatool-core` library and the `datatool` CLI, with CI running
fmt, clippy and tests.

The cell-based formula engine supports arithmetic, comparisons, text, logic
(`IF/AND/OR/NOT`) and aggregates over ranges. It has dependency tracking,
recalculation and cycle detection, but isn't yet connected to datasets (Phase 3).

## Key architecture decision: columns, not cells

The current engine stores every cell in a `HashMap<(row, col), Cell>`. That's right
for a small scratch sheet but won't scale to datasets (1M rows × 20 columns = 20M
heap-allocated cells, and a range like `A1:A1000000` expands to a million
dependency edges).

**Recommendation:** build the dataset layer on a columnar engine,
[Polars](https://pola.rs) (Rust-native, Arrow memory format, lazy queries, reads
CSV/Parquet/JSON/IPC). Keep the formula engine, but point it at columns:

| Layer | Responsibility | Built on |
|---|---|---|
| **Dataset** | Loading, typed columns, filtering, grouping, joins | Polars `DataFrame` / `LazyFrame` |
| **Formula** | User-facing expressions: `=price * qty`, `=IF(age > 65, "senior", "adult")` | Existing parser, compiled to Polars expressions for column formulas; current cell evaluator kept for scratch cells |
| **Profile** | Per-column and dataset-wide statistics, quality checks, narrative summary | Polars aggregations |
| **Recipe** | Ordered, serializable list of steps (load → clean → derive → filter …) | `serde` (JSON/YAML) |
| **Interface** | CLI first, then an interactive UI | `clap`; later web UI (see Phase 6) |

Restructure the crate into a library plus thin binaries so every interface shares
one core:

```
datatool/
  crates/
    core/      # dataset, formula, profile, recipe (library, no I/O to terminal)
    cli/       # `datatool` command
    app/       # later: UI backend
```

## Phases

Each phase ends with something a user can actually do. Estimates assume one
developer working part-time and are rough.

### Phase 0: Foundations (≈1 week)
- [x] Fix formula evaluation, dependency tracking, and cycle detection; remove dead
      duplicate modules; add tests.
- [x] Split into a `datatool-core` library and a `datatool` CLI (Cargo workspace).
- [x] CI on GitHub Actions running `cargo fmt --check`, `clippy -D warnings`, and `cargo test`.
- [x] README with what the tool does, how to install it, and a quickstart.
- [x] Formula engine: comparison operators, `IF`, string literals and `&`
      concatenation, `AND/OR/NOT`, and a typed `Value` (number, text, boolean).
- [ ] Formula engine: absolute refs (`$A$1`), and error values stored per cell
      (`#DIV/0!`, `#REF!`) instead of only being returned.

**Done when** CI is green and `cargo install --path crates/cli` gives a working `datatool` binary.

### Phase 1: Load any dataset (≈2 weeks)
- Readers for CSV/TSV (delimiter, quoting and encoding sniffing), Excel
  (`calamine`), JSON/NDJSON, and Parquet.
- Type inference for integer, float, boolean, date/datetime (multiple formats),
  categorical (low-cardinality strings), and free text. Users can override any
  column's type.
- Header detection, trimming, and safe column names; flag duplicate names.
- Lazy loading with sampled previews, so large files open instantly and full
  passes run in the background.
- `datatool head data.csv` and `datatool schema data.csv`.

**Done when** a messy real-world CSV (mixed date formats, `"N/A"` placeholders,
thousands separators) loads with correct types and no manual config.

**Status: done, except**
- [ ] Per-column type overrides.
- [ ] Lazy loading and sampled previews. Loading is eager today, which is fast
      enough at 1M rows (about 1.5 s) but holds the whole file in memory.
- [ ] Choosing an Excel sheet (only the first sheet is read).
- [ ] Encoding detection beyond UTF-8. Invalid bytes are replaced rather than
      decoded as Latin-1 and similar encodings.
- [ ] Day-first vs month-first preference. Ambiguous dates are read month-first
      and flagged as a high-priority issue.

### Phase 2: Comprehend the dataset (≈3 weeks). This is the core value.
Run `datatool profile data.csv` to get a terminal summary plus a standalone HTML
report.

**Dataset overview**
- Rows, columns, memory, column type breakdown, duplicate rows, overall
  completeness score.

**Per-column profile**
- Numeric columns get count, missing %, mean, median, std, min/max, quartiles,
  skew, zeros, negatives, histogram, and outliers (IQR and z-score).
- Categorical and text columns get distinct count, top-N values with
  frequencies, rare categories, string length stats, and case and whitespace
  inconsistencies (`"NY"` vs `"ny "`).
- Date columns get range, gaps, a distribution by day, month, and year, and
  future or implausible dates.
- Every column gets a likely role (ID, constant, near-constant, target
  candidate, free text).

**Relationships**
- Correlation matrix (Pearson, Spearman, and Cramér's V for categoricals),
  highly correlated pairs, and candidate keys and functional dependencies.

**Data quality warnings, ranked by severity, each with a suggested fix:**
- High missingness, constant columns, duplicate rows, and mixed types in one column.
- Suspicious placeholders (`-999`, `"unknown"`, `""`), outliers, and leading or
  trailing whitespace.
- Probable ID columns, and columns that leak a likely target.

**Plain-language summary** generated from the stats (template-based, no LLM
required), e.g. *"`income` is right-skewed (median $52k, mean $71k) with 4% missing
values; 12 rows have negative income, which is likely an error."*

**Done when** the report on a 1M-row, 30-column CSV renders in under 10 s and a
non-programmer can name the dataset's three biggest problems after reading it.

**Status: done, except**
- [ ] Cramér's V for categorical pairs, candidate keys and functional dependencies.
- [ ] Target-leakage checks. These need a chosen target column, which fits
      better with Phase 5's "What drives X?".
- [ ] z-score outliers. Only IQR fences are used, since they hold up better
      on skewed data.
- [ ] Day-of-week distributions for dates.

Charts are inline SVG instead of Vega-Lite, so the report has no external
dependencies and works offline. Profile tests assert on structure and
specific findings rather than comparing whole golden files, because some
values depend on the current date (future-date checks) and on memory
estimates.

### Phase 3: Clean and transform (≈3 weeks)
- Operations: select, rename, reorder, filter (formula-based), sort, deduplicate,
  fill or drop missing values (constant, mean, median, mode, forward-fill), cast
  types, split and merge columns, find and replace, trim and normalise case, and
  bin numeric columns.
- **Computed columns** using the formula engine with column names, e.g.
  `=revenue - cost`, `=ROUND(price * 1.08, 2)`, `=IF(score >= 50, "pass", "fail")`.
  These compile to vectorised Polars expressions rather than per-cell evaluation.
- Group-by and aggregate, pivot and unpivot, and joins between two datasets with a
  match-rate report.
- **Recipes:** every operation is appended to a recipe file. Users can undo, redo,
  replay a recipe on a new file (`datatool run recipe.yaml new_data.csv`), and
  export it as equivalent Python/pandas or SQL so work carries over into code.
- One-click fixes for Phase 2 warnings, where each warning links to the operation
  that resolves it.
- Export to CSV, Parquet, or Excel.

**Done when** a typical cleaning workflow (fix types, fill missing, dedupe, add two
derived columns, filter) can be done without code and replayed on next month's file.

### Phase 4: Visualize (≈2 weeks)
- Automatic chart per column (histogram, bar of top values, or time series) in
  the profile.
- Chart builder covering scatter, line, bar, box, and heatmap, with colour or
  facet by a categorical column.
- "Suggested charts" based on column types and the strongest relationships found
  in Phase 2.
- Render via Vega-Lite specs embedded in HTML, plus PNG/SVG export.

### Phase 5: Analyze (≈3–4 weeks)
Each analysis comes with a written interpretation and its assumptions checked
and stated.
- Compare groups using a t-test or Mann-Whitney U, ANOVA or Kruskal-Wallis, and
  chi-square, with effect sizes, not just p-values.
- Fit linear and logistic regression, reporting coefficients, confidence
  intervals, residual diagnostics, and a plain-language reading.
- **"What drives X?"**: rank features by association with a chosen target
  column using mutual information and a quick tree-based model.
- Clustering (k-means with an elbow or silhouette chart) with a description of
  each cluster.
- Time series covering trend and seasonality decomposition and simple forecasts.
- Guardrails that warn on small samples, multiple comparisons, and
  correlation-vs-causation.

### Phase 6: Interactive interface (≈4–6 weeks)
- **Recommended:** a local web app. The `datatool open data.csv` command starts
  a local server and opens the browser. The backend is `axum` serving the core
  library, and the frontend is a virtualised data grid, profile panels, chart
  builder, and recipe sidebar.
  - It reuses the HTML report work from Phases 2 and 4, works the same on every
    OS, and can later be packaged as a desktop app with Tauri.
  - Alternatives considered: a terminal UI (`ratatui`), which is fast to build
    but can't show charts well, and a browser-only WASM build, which avoids a
    local install but has memory limits on large files. A WASM build can come
    later for a hosted demo.
- The grid supports sort, filter, and edit-by-formula on column headers, and
  shows the profile for whichever column is selected.
- Scratch sheets use the existing cell-based engine for quick side calculations
  that can reference dataset aggregates.

### Phase 7: Assistant (optional, ≈2 weeks)
- Users can ask natural-language questions ("which region had the biggest drop in
  Q3?"), which are translated into recipe steps the user can see, edit, and
  undo. The assistant never acts as a black box.
- "Explain this column / chart / result" builds on the Phase 2 summaries.
- Opt-in and provider-configurable. Only schema and aggregate stats are sent by
  default, never raw rows, unless the user allows it.

## Cross-cutting work
- **Performance:** benchmarks on 10k, 1M, and 10M-row datasets tracked in CI. Use
  lazy evaluation and streaming for files larger than memory.
- **Testing:** unit tests per operation, golden-file tests for profile output,
  property tests for the formula parser, and a corpus of deliberately messy
  datasets.
- **Docs:** a quickstart, a formula function reference, and a gallery of example
  datasets with their reports.
- **Distribution:** prebuilt binaries for macOS, Linux, and Windows via GitHub
  Releases, plus `cargo install` and Homebrew.
- **Privacy:** everything runs locally with no telemetry by default.

## Suggested order and milestones

| Milestone | Phases | What a user can do |
|---|---|---|
| **M1: "First look"** | 0–2 | `datatool profile data.csv` gives a full HTML report with quality warnings and a narrative summary |
| **M2: "Clean it"** | 3–4 | Fix problems, add computed columns, chart, export, replay recipes |
| **M3: "Answer questions"** | 5 | Group comparisons, regression, and drivers analysis with interpretations |
| **M4: "No terminal needed"** | 6 | Everything above in a point-and-click local app |
| **M5: "Ask in English"** | 7 | Natural-language questions turned into editable steps |

M1 is the most valuable standalone release. It's useful on its own, and Phases 3–7
build on its column profiling.
