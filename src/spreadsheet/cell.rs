#[derive(Debug, Clone)]

pub enum CellValue {
    Number(f64),         // Represents a numeric value in the cell
    Text(String),        // Represents text in the cell
    Formula(String),     // Represents a formula in the cell as a string
    Empty,               // Represents an empty cell
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub value: CellValue,                  // The actual value or formula of the cell
    pub dependencies: Vec<(usize, usize)>, // Optional list of cells this cell depends on
    pub dependents: Vec<(usize, usize)>,   // Cells that depend on this cell
    pub evaluated: bool,                   // Tracks if the cell has been evaluated (useful for caching)
}

impl Cell {
    pub fn new(value: CellValue) -> Self {
        Self {
            value,
            dependencies: Vec::new(),
            dependents: Vec::new(),
            evaluated: false,
        }
    }

    // Method to set a new value
    pub fn set_value(&mut self, value: CellValue) {
        self.value = value;
        self.evaluated = false; // Mark as not evaluated if the value changes
    }

    // Method to add a dependency
    pub fn add_dependency(&mut self, dep: (usize, usize)) {
        if !self.dependencies.contains(&dep) {
            self.dependencies.push(dep);
        }
    }

    // Method to add a dependent
    pub fn add_dependent(&mut self, dep: (usize, usize)) {
        if !self.dependents.contains(&dep) {
            self.dependents.push(dep);
        }
    }

    // Clear all dependencies (useful when updating a formula)
    pub fn clear_dependencies(&mut self) {
        self.dependencies.clear();
    }

    // Clear all dependents (if the cell value changes and no longer affects others)
    pub fn clear_dependents(&mut self) {
        self.dependents.clear();
    }
}
