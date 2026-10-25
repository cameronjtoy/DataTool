pub mod cell;
pub mod error;
pub mod formula;

use crate::spreadsheet::cell::{Cell, CellValue};
use crate::spreadsheet::error::SpreadsheetError;
use crate::spreadsheet::formula::{Coord, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
pub struct Spreadsheet {
    cells: HashMap<Coord, Cell>, // Stores each cell by its zero-based (row, col) coordinates
}

impl Spreadsheet {
    pub fn new() -> Self {
        Self::default()
    }

    // Set a cell's value, rewire the dependency graph, and invalidate cached
    // results of every cell that (transitively) reads from it
    pub fn set_cell(&mut self, row: usize, col: usize, value: CellValue) {
        let coord = (row, col);

        // Unlink the old formula's dependencies
        let old_dependencies = self
            .cells
            .get_mut(&coord)
            .map(|cell| {
                let deps = cell.dependencies.clone();
                cell.clear_dependencies();
                deps
            })
            .unwrap_or_default();
        for dep in old_dependencies {
            if let Some(dep_cell) = self.cells.get_mut(&dep) {
                dep_cell.remove_dependent(coord);
            }
        }

        let dependencies = match &value {
            CellValue::Formula(f) => formula::parse(f)
                .map(|expr| formula::references(&expr))
                .unwrap_or_default(), // An unparsable formula reports its error on evaluation
            _ => Vec::new(),
        };

        self.cells
            .entry(coord)
            .or_insert_with(|| Cell::new(CellValue::Empty))
            .set_value(value);
        self.update_dependents(coord, dependencies);
        self.invalidate(coord);
    }

    // Set a cell using an A1-style reference, e.g. "B3"
    pub fn set(&mut self, reference: &str, value: CellValue) -> Result<(), SpreadsheetError> {
        let (row, col) = formula::parse_cell_ref(reference)?;
        self.set_cell(row, col, value);
        Ok(())
    }

    // Public method to get the raw value (or formula text) of a cell by coordinates
    pub fn get_cell_value(&self, row: usize, col: usize) -> Option<&CellValue> {
        self.cells.get(&(row, col)).map(|cell| &cell.value)
    }

    fn update_dependents(&mut self, cell: Coord, dependencies: Vec<Coord>) {
        if let Some(cell_data) = self.cells.get_mut(&cell) {
            for &dep in &dependencies {
                cell_data.add_dependency(dep);
            }
        }

        for dep in dependencies {
            self.cells
                .entry(dep)
                .or_insert_with(|| Cell::new(CellValue::Empty))
                .add_dependent(cell);
        }
    }

    // Drop cached results for a cell and everything downstream of it
    fn invalidate(&mut self, start: Coord) {
        let mut stack = vec![start];
        let mut seen = HashSet::new();
        while let Some(coord) = stack.pop() {
            if !seen.insert(coord) {
                continue;
            }
            if let Some(cell) = self.cells.get_mut(&coord) {
                cell.cached = None;
                stack.extend(cell.dependents.iter().copied());
            }
        }
    }

    // Evaluate a cell, computing formulas (and the formulas they reference)
    // as needed. Results are cached until an upstream cell changes.
    // An empty cell evaluates to 0.
    pub fn evaluate_cell(&mut self, row: usize, col: usize) -> Result<Value, SpreadsheetError> {
        if !self.cells.contains_key(&(row, col)) {
            return Err(SpreadsheetError::CellNotFound);
        }
        Ok(self
            .evaluate_inner((row, col), &mut HashSet::new())?
            .unwrap_or(Value::Number(0.0)))
    }

    // Evaluate a cell using an A1-style reference, e.g. "B3"
    pub fn evaluate(&mut self, reference: &str) -> Result<Value, SpreadsheetError> {
        let (row, col) = formula::parse_cell_ref(reference)?;
        self.evaluate_cell(row, col)
    }

    // A cell's value (None if empty), evaluating it first if it is a formula
    fn evaluate_inner(
        &mut self,
        coord: Coord,
        visiting: &mut HashSet<Coord>,
    ) -> Result<Option<Value>, SpreadsheetError> {
        let source = match self.cells.get(&coord) {
            None => return Ok(None),
            Some(cell) => match &cell.value {
                CellValue::Empty => return Ok(None),
                CellValue::Number(n) => return Ok(Some(Value::Number(*n))),
                CellValue::Text(s) => return Ok(Some(Value::Text(s.clone()))),
                CellValue::Formula(f) => match &cell.cached {
                    Some(v) => return Ok(Some(v.clone())),
                    None => f.clone(),
                },
            },
        };

        if !visiting.insert(coord) {
            return Err(SpreadsheetError::CircularDependency);
        }
        let result = formula::parse(&source)
            .and_then(|expr| formula::evaluate(&expr, &mut |c| self.evaluate_inner(c, visiting)));
        visiting.remove(&coord);

        if let (Ok(v), Some(cell)) = (&result, self.cells.get_mut(&coord)) {
            cell.cached = Some(v.clone());
        }
        result.map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(n: f64) -> CellValue {
        CellValue::Number(n)
    }

    fn formula(f: &str) -> CellValue {
        CellValue::Formula(f.into())
    }

    #[test]
    fn evaluates_simple_formula() {
        let mut sheet = Spreadsheet::new();
        sheet.set_cell(0, 0, num(10.0));
        sheet.set_cell(0, 1, num(5.0));
        sheet.set_cell(1, 0, formula("A1 + B1"));
        assert_eq!(sheet.evaluate_cell(1, 0), Ok(Value::Number(15.0)));
        // Evaluating must not overwrite the formula itself
        assert_eq!(sheet.get_cell_value(1, 0), Some(&formula("A1 + B1")));
    }

    #[test]
    fn recalculates_after_upstream_change() {
        let mut sheet = Spreadsheet::new();
        sheet.set("A1", num(2.0)).unwrap();
        sheet.set("A2", formula("A1 * 10")).unwrap();
        sheet.set("A3", formula("A2 + 1")).unwrap();
        assert_eq!(sheet.evaluate("A3"), Ok(Value::Number(21.0)));

        sheet.set("A1", num(3.0)).unwrap();
        assert_eq!(sheet.evaluate("A3"), Ok(Value::Number(31.0)));

        // Replacing a formula removes its old dependency links
        sheet.set("A2", num(100.0)).unwrap();
        assert_eq!(sheet.evaluate("A3"), Ok(Value::Number(101.0)));
        sheet.set("A1", num(0.0)).unwrap();
        assert_eq!(sheet.evaluate("A3"), Ok(Value::Number(101.0)));
    }

    #[test]
    fn aggregate_functions_over_ranges() {
        let mut sheet = Spreadsheet::new();
        sheet.set("A1", num(1.0)).unwrap();
        sheet.set("A2", num(2.0)).unwrap();
        sheet.set("A3", CellValue::Text("n/a".into())).unwrap();
        sheet.set("A4", formula("A1 + A2")).unwrap();
        // A5 left empty
        sheet.set("B1", formula("SUM(A1:A5)")).unwrap();
        sheet.set("B2", formula("AVERAGE(A1:A5)")).unwrap();
        sheet.set("B3", formula("MIN(A1:A5)")).unwrap();
        sheet.set("B4", formula("MAX(A1:A5, 10)")).unwrap();
        sheet.set("B5", formula("COUNT(A1:A5)")).unwrap();

        assert_eq!(sheet.evaluate("B1"), Ok(Value::Number(6.0)));
        assert_eq!(sheet.evaluate("B2"), Ok(Value::Number(2.0)));
        assert_eq!(sheet.evaluate("B3"), Ok(Value::Number(1.0)));
        assert_eq!(sheet.evaluate("B4"), Ok(Value::Number(10.0)));
        assert_eq!(sheet.evaluate("B5"), Ok(Value::Number(3.0)));

        sheet.set("A5", num(9.0)).unwrap();
        assert_eq!(sheet.evaluate("B1"), Ok(Value::Number(15.0)));
    }

    #[test]
    fn empty_and_text_cells() {
        let mut sheet = Spreadsheet::new();
        sheet.set("A1", formula("Z99 + 1")).unwrap();
        assert_eq!(sheet.evaluate("A1"), Ok(Value::Number(1.0)));

        sheet.set("B1", CellValue::Text("hello".into())).unwrap();
        sheet.set("B2", formula("B1 * 2")).unwrap();
        assert!(matches!(
            sheet.evaluate("B2"),
            Err(SpreadsheetError::TypeMismatch(_))
        ));
        assert_eq!(sheet.evaluate("C9"), Err(SpreadsheetError::CellNotFound));
    }

    #[test]
    fn detects_cycles() {
        let mut sheet = Spreadsheet::new();
        sheet.set("A1", formula("B1 + 1")).unwrap();
        sheet.set("B1", formula("A1 + 1")).unwrap();
        assert_eq!(
            sheet.evaluate("A1"),
            Err(SpreadsheetError::CircularDependency)
        );
        assert_eq!(
            sheet.evaluate("B1"),
            Err(SpreadsheetError::CircularDependency)
        );

        sheet.set("C1", formula("SUM(C1:C3)")).unwrap();
        assert_eq!(
            sheet.evaluate("C1"),
            Err(SpreadsheetError::CircularDependency)
        );

        // Breaking the cycle makes both cells evaluate again
        sheet.set("B1", num(5.0)).unwrap();
        assert_eq!(sheet.evaluate("A1"), Ok(Value::Number(6.0)));
    }

    #[test]
    fn text_and_logic_across_cells() {
        let mut sheet = Spreadsheet::new();
        sheet.set("A1", CellValue::Text("Ada".into())).unwrap();
        sheet.set("B1", num(72.0)).unwrap();
        sheet
            .set("C1", formula("IF(B1 >= 65, \"senior\", \"adult\")"))
            .unwrap();
        sheet.set("D1", formula("A1 & \" is \" & C1")).unwrap();
        sheet
            .set("E1", formula("AND(B1 > 18, C1 = \"senior\")"))
            .unwrap();
        assert_eq!(
            sheet.evaluate("D1"),
            Ok(Value::Text("Ada is senior".into()))
        );
        assert_eq!(sheet.evaluate("E1"), Ok(Value::Bool(true)));
        assert_eq!(sheet.evaluate("A1"), Ok(Value::Text("Ada".into())));

        sheet.set("B1", num(30.0)).unwrap();
        assert_eq!(sheet.evaluate("D1"), Ok(Value::Text("Ada is adult".into())));
        assert_eq!(sheet.evaluate("E1"), Ok(Value::Bool(false)));
    }

    #[test]
    fn invalid_formula_reports_error() {
        let mut sheet = Spreadsheet::new();
        sheet.set("A1", formula("1 +* 2")).unwrap();
        assert!(matches!(
            sheet.evaluate("A1"),
            Err(SpreadsheetError::InvalidFormula(_))
        ));
        assert!(sheet.set("1A", num(1.0)).is_err());
    }
}
