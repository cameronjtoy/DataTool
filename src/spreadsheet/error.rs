// src/utils/error.rs
#[derive(Debug)]
pub enum SpreadsheetError {
    CellNotFound,
    CircularDependency,
    InvalidFormula(String),
}

impl std::fmt::Display for SpreadsheetError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            SpreadsheetError::CellNotFound => write!(f, "Cell not found"),
            SpreadsheetError::CircularDependency => write!(f, "Circular dependency detected"),
            SpreadsheetError::InvalidFormula(e) => write!(f, "Invalid formula: {}", e),
        }
    }
}
