use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataToolError {
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("unsupported file type '{0}' (expected csv, tsv, txt, xlsx, xls, xlsm, ods, parquet, json or ndjson)")]
    UnsupportedFormat(String),
    #[error("{0} contains no data")]
    Empty(String),
    #[error("could not read spreadsheet: {0}")]
    Excel(String),
    #[error(transparent)]
    Polars(#[from] polars::error::PolarsError),
}

pub type Result<T> = std::result::Result<T, DataToolError>;
