mod spreadsheet;

use spreadsheet::cell::CellValue;
use spreadsheet::error::SpreadsheetError;
use spreadsheet::Spreadsheet;

fn main() -> Result<(), SpreadsheetError> {
    let mut sheet = Spreadsheet::new();

    // Set some cell values
    sheet.set_cell(0, 0, CellValue::Number(10.0)); // Cell A1
    sheet.set_cell(0, 1, CellValue::Number(5.0)); // Cell B1
    sheet.set_cell(1, 0, CellValue::Formula("A1 + B1".into())); // Cell A2 with formula "A1 + B1"

    // Cells can also be addressed with A1-style references
    sheet.set("C1", CellValue::Text("label".into()))?;
    sheet.set("A3", CellValue::Formula("=AVERAGE(A1:B1) * 2".into()))?;
    sheet.set("A4", CellValue::Formula("SUM(A1:A3)".into()))?;

    for reference in ["A2", "A3", "A4"] {
        match sheet.evaluate(reference) {
            Ok(value) => println!("The value at cell {} is: {}", reference, value),
            Err(e) => println!("Error evaluating cell {}: {}", reference, e),
        }
    }

    // Changing an input recalculates everything downstream of it
    sheet.set("A1", CellValue::Number(20.0))?;
    println!("After setting A1 to 20, A4 is: {}", sheet.evaluate("A4")?);

    // Access raw values (formulas are kept as entered) using the public getter method
    if let Some(value) = sheet.get_cell_value(1, 0) {
        println!("The raw value at cell A2 is: {:?}", value);
    }
    if let Some(value) = sheet.get_cell_value(0, 2) {
        println!("The raw value at cell C1 is: {:?}", value);
    }
    Ok(())
}
