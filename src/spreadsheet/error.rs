#[derive(Debug, Clone, PartialEq)]
pub enum SpreadsheetError {
    CellNotFound,
    CircularDependency,
    InvalidFormula(String),
    InvalidReference(String),
    DivisionByZero,
    TypeMismatch(String),
}

impl std::fmt::Display for SpreadsheetError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            SpreadsheetError::CellNotFound => write!(f, "Cell not found"),
            SpreadsheetError::CircularDependency => write!(f, "Circular dependency detected"),
            SpreadsheetError::InvalidFormula(e) => write!(f, "Invalid formula: {}", e),
            SpreadsheetError::InvalidReference(r) => write!(f, "Invalid cell reference: {}", r),
            SpreadsheetError::DivisionByZero => write!(f, "Division by zero"),
            SpreadsheetError::TypeMismatch(e) => write!(f, "Type mismatch: {}", e),
        }
    }
}

impl std::error::Error for SpreadsheetError {}
