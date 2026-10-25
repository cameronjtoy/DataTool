//! Loading datasets from files into typed, cleaned Polars data frames.

pub mod infer;

use crate::error::{DataToolError, Result};
use calamine::{open_workbook_auto, Data, DataType as _, Reader};
use infer::{infer_column, ColumnInference, InferredType};
use polars::prelude::*;
use serde::Serialize;
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Format {
    Csv { separator: char },
    Excel { sheet: String },
    Parquet,
    Json,
    NdJson,
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Format::Csv { separator: '\t' } => write!(f, "TSV"),
            Format::Csv { separator } => write!(f, "CSV (separator '{}')", separator),
            Format::Excel { sheet } => write!(f, "Excel (sheet '{}')", sheet),
            Format::Parquet => write!(f, "Parquet"),
            Format::Json => write!(f, "JSON"),
            Format::NdJson => write!(f, "NDJSON"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ColumnMeta {
    pub name: String,
    // The header as it appeared in the file, when it had to be changed
    pub original_name: Option<String>,
    pub inference: ColumnInference,
}

#[derive(Debug, Clone)]
pub struct Dataset {
    pub name: String,
    pub path: PathBuf,
    pub format: Format,
    pub df: DataFrame,
    pub columns: Vec<ColumnMeta>,
}

impl Dataset {
    pub fn rows(&self) -> usize {
        self.df.height()
    }

    pub fn cols(&self) -> usize {
        self.df.width()
    }
}

// Load a dataset, choosing the reader from the file extension
pub fn load(path: impl AsRef<Path>) -> Result<Dataset> {
    let path = path.as_ref();
    let display = path.display().to_string();
    let io_err = |source| DataToolError::Io {
        path: display.clone(),
        source,
    };
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let (format, df, columns) = match ext.as_str() {
        "csv" | "tsv" | "txt" | "tab" => {
            let separator = if ext == "tsv" || ext == "tab" {
                b'\t'
            } else {
                sniff_separator(path).map_err(io_err)?
            };
            let (df, columns) = read_csv(path, separator)?;
            (
                Format::Csv {
                    separator: separator as char,
                },
                df,
                columns,
            )
        }
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => {
            let (sheet, df, columns) = read_excel(path)?;
            (Format::Excel { sheet }, df, columns)
        }
        "parquet" | "pq" => {
            let file = File::open(path).map_err(io_err)?;
            let df = ParquetReader::new(file).finish()?;
            let (df, columns) = refine_typed(df)?;
            (Format::Parquet, df, columns)
        }
        "json" | "ndjson" | "jsonl" => {
            let (format, json_format) = if ext == "json" {
                (Format::Json, JsonFormat::Json)
            } else {
                (Format::NdJson, JsonFormat::JsonLines)
            };
            let file = File::open(path).map_err(io_err)?;
            let df = JsonReader::new(file)
                .with_json_format(json_format)
                .finish()?;
            let (df, columns) = refine_typed(df)?;
            (format, df, columns)
        }
        other => return Err(DataToolError::UnsupportedFormat(other.to_string())),
    };

    if columns.is_empty() {
        return Err(DataToolError::Empty(display));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(display);
    Ok(Dataset {
        name,
        path: path.to_path_buf(),
        format,
        df,
        columns,
    })
}

// Pick the separator that splits the first lines into the most consistent
// number of fields
fn sniff_separator(path: &Path) -> std::io::Result<u8> {
    let mut buf = vec![0; 64 * 1024];
    let n = File::open(path)?.read(&mut buf)?;
    let text = String::from_utf8_lossy(&buf[..n]);
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(20)
        .collect();
    if lines.is_empty() {
        return Ok(b',');
    }

    let mut best = (b',', 0usize, 0usize); // (separator, consistent lines, fields)
    for sep in *b",\t;|" {
        let counts: Vec<usize> = lines.iter().map(|l| count_outside_quotes(l, sep)).collect();
        let fields = counts[0];
        if fields == 0 {
            continue;
        }
        let consistent = counts.iter().filter(|c| **c == fields).count();
        if (consistent, fields) > (best.1, best.2) {
            best = (sep, consistent, fields);
        }
    }
    Ok(best.0)
}

fn count_outside_quotes(line: &str, sep: u8) -> usize {
    let mut in_quotes = false;
    let mut count = 0;
    for b in line.bytes() {
        if b == b'"' {
            in_quotes = !in_quotes;
        } else if b == sep && !in_quotes {
            count += 1;
        }
    }
    count
}

// Read every field as text (header included) so inference sees raw values
fn read_csv(path: &Path, separator: u8) -> Result<(DataFrame, Vec<ColumnMeta>)> {
    let raw = CsvReadOptions::default()
        .with_has_header(false)
        .with_infer_schema_length(Some(0))
        .map_parse_options(|o| {
            o.with_separator(separator)
                .with_encoding(CsvEncoding::LossyUtf8)
                .with_truncate_ragged_lines(true)
                .with_missing_is_null(true)
        })
        .try_into_reader_with_file_path(Some(path.to_path_buf()))?
        .finish()?;

    let mut text_columns = Vec::with_capacity(raw.width());
    for column in raw.columns() {
        let values = column.str()?;
        let header = values.get(0).map(str::to_string);
        let body: Vec<Option<&str>> = values.iter().skip(1).collect();
        text_columns.push((header, body));
    }
    build_from_text(text_columns)
}

fn read_excel(path: &Path) -> Result<(String, DataFrame, Vec<ColumnMeta>)> {
    let excel_err = |e: calamine::Error| DataToolError::Excel(e.to_string());
    let mut workbook = open_workbook_auto(path).map_err(excel_err)?;
    let sheet = workbook
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| DataToolError::Empty(path.display().to_string()))?;
    let range = workbook.worksheet_range(&sheet).map_err(excel_err)?;

    let width = range.width();
    let mut rows = range.rows();
    let header: Vec<Option<String>> = match rows.next() {
        Some(row) => row.iter().map(excel_text).collect(),
        None => return Err(DataToolError::Empty(path.display().to_string())),
    };
    let mut columns: Vec<Vec<Option<String>>> = vec![Vec::new(); width];
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(width) {
            columns[i].push(excel_text(cell));
        }
    }

    let text_columns = header
        .into_iter()
        .zip(columns.iter())
        .map(|(h, values)| (h, values.iter().map(|v| v.as_deref()).collect()))
        .collect();
    let (df, columns) = build_from_text(text_columns)?;
    Ok((sheet, df, columns))
}

fn excel_text(cell: &Data) -> Option<String> {
    match cell {
        Data::Empty | Data::Error(_) => None,
        Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => Some(s.clone()),
        Data::Int(i) => Some(i.to_string()),
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => Some(format!("{}", *f as i64)),
        Data::Float(f) => Some(f.to_string()),
        Data::Bool(b) => Some(b.to_string()),
        Data::DateTime(_) => cell.as_datetime().map(|dt| {
            if dt.time() == chrono::NaiveTime::MIN {
                dt.format("%Y-%m-%d").to_string()
            } else {
                dt.format("%Y-%m-%d %H:%M:%S").to_string()
            }
        }),
    }
}

// Clean headers and infer each column's type from its text values
fn build_from_text(
    text_columns: Vec<(Option<String>, Vec<Option<&str>>)>,
) -> Result<(DataFrame, Vec<ColumnMeta>)> {
    let headers: Vec<Option<String>> = text_columns.iter().map(|(h, _)| h.clone()).collect();
    let names = clean_names(&headers);
    let height = text_columns.first().map(|(_, v)| v.len()).unwrap_or(0);

    let mut columns = Vec::with_capacity(text_columns.len());
    let mut metas = Vec::with_capacity(text_columns.len());
    for ((name, original_name), (_, values)) in names.into_iter().zip(text_columns) {
        let (series, inference) = infer_column(&name, values)?;
        columns.push(series.into_column());
        metas.push(ColumnMeta {
            name,
            original_name,
            inference,
        });
    }
    Ok((DataFrame::new(height, columns)?, metas))
}

// For typed sources, keep typed columns and run inference only on text columns
fn refine_typed(df: DataFrame) -> Result<(DataFrame, Vec<ColumnMeta>)> {
    let height = df.height();
    let mut columns = Vec::with_capacity(df.width());
    let mut metas = Vec::with_capacity(df.width());
    for column in df.columns() {
        let name = column.name().to_string();
        let (column, inference) = if column.dtype() == &DataType::String {
            let (series, inference) = infer_column(&name, column.str()?.iter())?;
            (series.into_column(), inference)
        } else {
            let inferred = InferredType::from_dtype(column.dtype());
            (column.clone(), ColumnInference::typed(inferred))
        };
        columns.push(column);
        metas.push(ColumnMeta {
            name,
            original_name: None,
            inference,
        });
    }
    Ok((DataFrame::new(height, columns)?, metas))
}

// Trim header names, fill in blanks and make duplicates unique. Returns
// (name, original name if it was changed).
fn clean_names(headers: &[Option<String>]) -> Vec<(String, Option<String>)> {
    let mut seen = HashSet::new();
    headers
        .iter()
        .enumerate()
        .map(|(i, raw)| {
            let raw_str = raw.as_deref().unwrap_or("");
            let trimmed = raw_str.trim();
            let base = if trimmed.is_empty() {
                format!("column_{}", i + 1)
            } else {
                trimmed.to_string()
            };
            let mut name = base.clone();
            let mut n = 2;
            while !seen.insert(name.to_lowercase()) {
                name = format!("{}_{}", base, n);
                n += 1;
            }
            let original = (name != raw_str).then(|| raw_str.to_string());
            (name, original)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(name: &str, contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("datatool-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        File::create(&path)
            .unwrap()
            .write_all(contents.as_bytes())
            .unwrap();
        path
    }

    #[test]
    fn loads_csv_with_messy_headers_and_types() {
        let path = write_temp(
            "messy.csv",
            "id, Name ,,name,joined\n1,Ann,x,a,2024-01-02\n2,Bob,y,b,N/A\n3,\"Lee, Jr\",z,c,2024-03-04\n",
        );
        let ds = load(&path).unwrap();
        assert_eq!(ds.format, Format::Csv { separator: ',' });
        assert_eq!((ds.rows(), ds.cols()), (3, 5));
        let names: Vec<&str> = ds.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["id", "Name", "column_3", "name_2", "joined"]);
        assert_eq!(ds.columns[1].original_name.as_deref(), Some(" Name "));
        assert_eq!(ds.columns[0].inference.inferred, InferredType::Integer);
        assert_eq!(ds.columns[4].inference.inferred, InferredType::Date);
        assert_eq!(ds.columns[4].inference.placeholder_nulls, 1);
        assert_eq!(
            ds.df.column("Name").unwrap().str().unwrap().get(2),
            Some("Lee, Jr")
        );
    }

    #[test]
    fn sniffs_semicolons_and_tabs() {
        let path = write_temp("semi.csv", "a;b;c\n1,5;2;3\n4;5;6\n");
        let ds = load(&path).unwrap();
        assert_eq!(ds.format, Format::Csv { separator: ';' });
        assert_eq!(ds.cols(), 3);

        let path = write_temp("tabs.tsv", "a\tb\n1\t2\n");
        assert_eq!(load(&path).unwrap().format, Format::Csv { separator: '\t' });
    }

    #[test]
    fn loads_ndjson_and_refines_strings() {
        let path = write_temp(
            "rows.ndjson",
            "{\"n\": 1, \"when\": \"2024-01-01\"}\n{\"n\": 2, \"when\": \"2024-02-01\"}\n",
        );
        let ds = load(&path).unwrap();
        assert_eq!(ds.format, Format::NdJson);
        assert_eq!(ds.columns[0].inference.inferred, InferredType::Integer);
        assert_eq!(ds.columns[1].inference.inferred, InferredType::Date);
    }

    #[test]
    fn rejects_unknown_extensions() {
        let path = write_temp("data.xyz", "a");
        assert!(matches!(
            load(&path),
            Err(DataToolError::UnsupportedFormat(_))
        ));
    }
}
