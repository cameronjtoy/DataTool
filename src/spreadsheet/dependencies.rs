pub mod cell;


use std::collections::HashMap;
use crate::spreadsheet::cell::{Cell, CellValue};

#[derive(Debug)]
pub struct Spreadsheet {
    cells: HashMap<(usize, usize), Cell>,      // Stores each cell by its coordinates
}

impl Spreadsheet {
    pub fn new() -> Self {
        Self {
            cells: HashMap::new(),
        }
    }

    // Set a cell’s value and update dependencies if it’s a formula
    pub fn set_cell(&mut self, row: usize, col: usize, value: CellValue) {
        let coord = (row, col);
        
        // Add the cell if it doesn't already exist
        self.cells.entry(coord).or_insert_with(|| Cell::new(CellValue::Empty));

        // Update the cell value
        if let Some(cell) = self.cells.get_mut(&coord) {
            cell.set_value(value.clone());

            // If the value is a formula, parse dependencies and update dependents
            if let CellValue::Formula(_) = value {
                let dependencies = self.parse_dependencies(&value); // Parse dependencies from formula
                self.update_dependents(coord, dependencies);
            }
        }

        // Recalculate dependents of this cell after setting its value
        self.recalculate_dependents(coord);
    }

    // Parse dependencies from a formula (stub function for now)
    fn parse_dependencies(&self, value: &CellValue) -> Vec<(usize, usize)> {
        // Example: Extract dependencies from the formula here
        // This is just a placeholder and should parse actual cell references in real usage
        vec![]
    }

    // Update dependents in other cells based on a new formula
    fn update_dependents(&mut self, cell: (usize, usize), dependencies: Vec<(usize, usize)>) {
        // Clear current dependencies in the cell to avoid duplicates
        if let Some(cell_data) = self.cells.get_mut(&cell) {
            cell_data.dependencies.clear();

            // Add each new dependency and update the dependent cells
            for dep in dependencies {
                cell_data.add_dependency(dep);
                
                // Ensure each dependency cell exists and update its dependents
                self.cells
                    .entry(dep)
                    .or_insert_with(|| Cell::new(CellValue::Empty))
                    .add_dependent(cell);
            }
        }
    }

    // Recalculate all dependents of a changed cell
    fn recalculate_dependents(&mut self, cell: (usize, usize)) {
        if let Some(cell_data) = self.cells.get(&cell) {
            for &dependent in &cell_data.dependents {
                let _ = self.evaluate_cell(dependent.0, dependent.1); // Recalculate each dependent
            }
        }
    }

    // Evaluate a cell’s formula if it contains one
    pub fn evaluate_cell(&mut self, row: usize, col: usize) -> Result<f64, String> {
        let coord = (row, col);
        if let Some(cell) = self.cells.get_mut(&coord) {
            if let CellValue::Formula(_) = cell.value {
                let value = self.evaluate_formula(cell)?; // Custom function to evaluate the formula
                cell.set_value(CellValue::Number(value)); // Cache the result
                Ok(value)
            } else if let CellValue::Number(num) = cell.value {
                Ok(num)
            } else {
                Err("Invalid cell type for evaluation".to_string())
            }
        } else {
            Err("Cell not found".to_string())
        }
    }

    // Placeholder function to evaluate a formula in a cell
    fn evaluate_formula(&self, cell: &Cell) -> Result<f64, String> {
        // Example: Actual formula evaluation logic should go here
        Ok(0.0) // Placeholder value
    }
}
