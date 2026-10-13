//! Core library for DataTool.
//!
//! - [`dataset`]: load CSV, Excel, Parquet and JSON files with automatic type inference
//! - [`spreadsheet`]: a cell grid with A1-style formulas, dependency tracking and recalculation

pub mod dataset;
pub mod error;
pub mod profile;
pub mod report;
pub mod spreadsheet;

pub use dataset::{load, Dataset};
pub use error::{DataToolError, Result};
pub use profile::{profile, DatasetProfile};
