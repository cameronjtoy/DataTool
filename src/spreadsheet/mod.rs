pub mod cell;

use crate::spreadsheet::cell::{Cell, CellValue};
use std::collections::HashMap;

#[derive(Debug)]
pub struct Spreadsheet {
    cells: HashMap<(usize, usize), Cell>, // Stores each cell by its coordinates
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
        self.cells
            .entry(coord)
            .or_insert_with(|| Cell::new(CellValue::Empty));

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

    // Public method to get the value of a cell by coordinates
    pub fn get_cell_value(&self, row: usize, col: usize) -> Option<&CellValue> {
        self.cells.get(&(row, col)).map(|cell| &cell.value)
    }

    fn update_dependents(&mut self, cell: (usize, usize), dependencies: Vec<(usize, usize)>) {
        // Temporary list of dependencies to update after releasing the first borrow
        let mut new_dependencies = vec![];

        // Clear current dependencies in the cell to avoid duplicates
        if let Some(cell_data) = self.cells.get_mut(&cell) {
            cell_data.clear_dependencies();

            // Collect dependencies for later update
            for dep in dependencies {
                cell_data.add_dependency(dep);
                new_dependencies.push(dep);
            }
        }

        // Update dependents after releasing the previous borrow
        for dep in new_dependencies {
            self.cells
                .entry(dep)
                .or_insert_with(|| Cell::new(CellValue::Empty))
                .add_dependent(cell);
        }
    }

    fn recalculate_dependents(&mut self, cell: (usize, usize)) {
        // Collect dependents to update after releasing the borrow on `cell_data`
        let dependents = if let Some(cell_data) = self.cells.get(&cell) {
            cell_data.dependents.clone()
        } else {
            vec![]
        };

        // Iterate over collected dependents and evaluate them
        for dependent in dependents {
            let _ = self.evaluate_cell(dependent.0, dependent.1); // Recalculate each dependent
        }
    }

    pub fn evaluate_cell(&mut self, row: usize, col: usize) -> Result<f64, String> {
        let coord = (row, col);
        if let Some(cell) = self.cells.get_mut(&coord) {
            if let CellValue::Formula(_) = cell.value {
                // Evaluate the formula directly here instead of calling another method
                // Replace this with the actual formula evaluation logic
                let value = 0.0; // Placeholder for the calculated value

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
