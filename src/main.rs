mod spreadsheet;



use spreadsheet::cell::CellValue;
use spreadsheet::Spreadsheet;

fn main() {
    let mut sheet = Spreadsheet::new();

    // Set some cell values
    sheet.set_cell(0, 0, CellValue::Number(10.0));           // Cell A1
    sheet.set_cell(0, 1, CellValue::Number(5.0));            // Cell B1
    sheet.set_cell(1, 0, CellValue::Formula("A1 + B1".into())); // Cell A2 with formula "A1 + B1"

    // Attempt to evaluate the cell with a formula
    match sheet.evaluate_cell(1, 0) { // Evaluate Cell A2
        Ok(value) => println!("The value at cell A2 is: {}", value),
        Err(e) => println!("Error evaluating cell A2: {}", e),
    }

    // Access and print values using the public getter method
    if let Some(value) = sheet.get_cell_value(0, 0) {
        println!("The value at cell A1 is: {:?}", value);
    }
    if let Some(value) = sheet.get_cell_value(0, 1) {
        println!("The value at cell B1 is: {:?}", value);
    }
    if let Some(value) = sheet.get_cell_value(1, 0) {
        println!("The value at cell A2 is: {:?}", value);
    }
}
