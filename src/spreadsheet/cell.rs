#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Number(f64),     // Represents a numeric value in the cell
    Text(String),    // Represents text in the cell
    Formula(String), // Represents a formula in the cell as a string, e.g. "A1 + B1" or "=SUM(A1:A3)"
    Empty,           // Represents an empty cell
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub value: CellValue,                  // The value or formula the user entered
    pub dependencies: Vec<(usize, usize)>, // Cells this cell's formula reads from
    pub dependents: Vec<(usize, usize)>,   // Cells whose formulas read from this cell
    pub cached: Option<f64>, // Last evaluated result; None means it must be recomputed
}

impl Cell {
    pub fn new(value: CellValue) -> Self {
        Self {
            value,
            dependencies: Vec::new(),
            dependents: Vec::new(),
            cached: None,
        }
    }

    // Set a new value and drop any cached result
    pub fn set_value(&mut self, value: CellValue) {
        self.value = value;
        self.cached = None;
    }

    pub fn add_dependency(&mut self, dep: (usize, usize)) {
        if !self.dependencies.contains(&dep) {
            self.dependencies.push(dep);
        }
    }

    pub fn add_dependent(&mut self, dep: (usize, usize)) {
        if !self.dependents.contains(&dep) {
            self.dependents.push(dep);
        }
    }

    pub fn remove_dependent(&mut self, dep: (usize, usize)) {
        self.dependents.retain(|d| *d != dep);
    }

    // Clear all dependencies (used when a formula is replaced)
    pub fn clear_dependencies(&mut self) {
        self.dependencies.clear();
    }
}
