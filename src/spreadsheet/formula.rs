// Formula parsing and evaluation.
//
// Supported syntax (an optional leading '=' is ignored, names are case-insensitive):
//   numbers        10, 2.5, .5, 1e3
//   cell refs      A1, b2, AA10
//   ranges         A1:B3 (only as function arguments)
//   operators      + - * / ^ and unary -, with parentheses for grouping
//   functions      SUM, AVERAGE (AVG), MIN, MAX, COUNT, ABS, SQRT, ROUND

use crate::spreadsheet::error::SpreadsheetError;

pub type Coord = (usize, usize);

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    CellRef(Coord),
    Range(Coord, Coord),
    Neg(Box<Expr>),
    Binary(Box<Expr>, BinOp, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

// What a referenced cell holds, as seen by the evaluator
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    Number(f64),
    Text,
    Empty,
}

// Convert an A1-style reference into zero-based (row, col) coordinates
pub fn parse_cell_ref(reference: &str) -> Result<Coord, SpreadsheetError> {
    let invalid = || SpreadsheetError::InvalidReference(reference.to_string());
    let split = reference
        .find(|c: char| c.is_ascii_digit())
        .ok_or_else(invalid)?;
    let (letters, digits) = reference.split_at(split);
    if letters.is_empty() || !letters.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(invalid());
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(invalid());
    }

    let mut col: usize = 0;
    for c in letters.chars() {
        let digit = (c.to_ascii_uppercase() as u8 - b'A') as usize + 1;
        col = col
            .checked_mul(26)
            .and_then(|v| v.checked_add(digit))
            .ok_or_else(invalid)?;
    }
    let row: usize = digits.parse().map_err(|_| invalid())?;
    if row == 0 {
        return Err(invalid());
    }
    Ok((row - 1, col - 1))
}

// Convert zero-based (row, col) coordinates into an A1-style reference
pub fn to_cell_ref((row, col): Coord) -> String {
    let mut letters = Vec::new();
    let mut n = col + 1;
    while n > 0 {
        let rem = (n - 1) % 26;
        letters.push((b'A' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    letters.iter().rev().collect::<String>() + &(row + 1).to_string()
}

pub fn parse(formula: &str) -> Result<Expr, SpreadsheetError> {
    let source = formula.trim();
    let source = source.strip_prefix('=').unwrap_or(source);
    let tokens = tokenize(source)?;
    let mut parser = Parser { tokens, pos: 0 };
    let expr = parser.expression()?;
    match parser.peek() {
        None => Ok(expr),
        Some(t) => Err(SpreadsheetError::InvalidFormula(format!(
            "unexpected {:?} in '{}'",
            t, formula
        ))),
    }
}

// Every cell an expression reads from, with ranges expanded
pub fn references(expr: &Expr) -> Vec<Coord> {
    let mut out = Vec::new();
    collect_references(expr, &mut out);
    out.sort();
    out.dedup();
    out
}

fn collect_references(expr: &Expr, out: &mut Vec<Coord>) {
    match expr {
        Expr::Number(_) => {}
        Expr::CellRef(c) => out.push(*c),
        Expr::Range(a, b) => out.extend(range_cells(*a, *b)),
        Expr::Neg(e) => collect_references(e, out),
        Expr::Binary(l, _, r) => {
            collect_references(l, out);
            collect_references(r, out);
        }
        Expr::Call(_, args) => args.iter().for_each(|a| collect_references(a, out)),
    }
}

fn range_cells(a: Coord, b: Coord) -> impl Iterator<Item = Coord> {
    let (r0, r1) = (a.0.min(b.0), a.0.max(b.0));
    let (c0, c1) = (a.1.min(b.1), a.1.max(b.1));
    (r0..=r1).flat_map(move |r| (c0..=c1).map(move |c| (r, c)))
}

// Evaluate an expression, resolving cell references through `lookup`
pub fn evaluate<F>(expr: &Expr, lookup: &mut F) -> Result<f64, SpreadsheetError>
where
    F: FnMut(Coord) -> Result<Resolved, SpreadsheetError>,
{
    match expr {
        Expr::Number(n) => Ok(*n),
        Expr::CellRef(c) => match lookup(*c)? {
            Resolved::Number(n) => Ok(n),
            Resolved::Empty => Ok(0.0),
            Resolved::Text => Err(SpreadsheetError::TypeMismatch(format!(
                "{} contains text",
                to_cell_ref(*c)
            ))),
        },
        Expr::Range(..) => Err(SpreadsheetError::InvalidFormula(
            "a range can only be used as a function argument".into(),
        )),
        Expr::Neg(e) => Ok(-evaluate(e, lookup)?),
        Expr::Binary(l, op, r) => {
            let l = evaluate(l, lookup)?;
            let r = evaluate(r, lookup)?;
            match op {
                BinOp::Add => Ok(l + r),
                BinOp::Sub => Ok(l - r),
                BinOp::Mul => Ok(l * r),
                BinOp::Div if r == 0.0 => Err(SpreadsheetError::DivisionByZero),
                BinOp::Div => Ok(l / r),
                BinOp::Pow => Ok(l.powf(r)),
            }
        }
        Expr::Call(name, args) => call(name, args, lookup),
    }
}

fn call<F>(name: &str, args: &[Expr], lookup: &mut F) -> Result<f64, SpreadsheetError>
where
    F: FnMut(Coord) -> Result<Resolved, SpreadsheetError>,
{
    let arity = |n: usize| {
        if args.len() == n {
            Ok(())
        } else {
            Err(SpreadsheetError::InvalidFormula(format!(
                "{} expects {} argument(s), got {}",
                name,
                n,
                args.len()
            )))
        }
    };

    match name {
        "SUM" => Ok(numeric_args(args, lookup)?.iter().sum()),
        "AVERAGE" | "AVG" => {
            let values = numeric_args(args, lookup)?;
            if values.is_empty() {
                return Err(SpreadsheetError::DivisionByZero);
            }
            Ok(values.iter().sum::<f64>() / values.len() as f64)
        }
        "MIN" => Ok(numeric_args(args, lookup)?
            .into_iter()
            .reduce(f64::min)
            .unwrap_or(0.0)),
        "MAX" => Ok(numeric_args(args, lookup)?
            .into_iter()
            .reduce(f64::max)
            .unwrap_or(0.0)),
        "COUNT" => Ok(numeric_args(args, lookup)?.len() as f64),
        "ABS" => {
            arity(1)?;
            Ok(evaluate(&args[0], lookup)?.abs())
        }
        "SQRT" => {
            arity(1)?;
            let v = evaluate(&args[0], lookup)?;
            if v < 0.0 {
                return Err(SpreadsheetError::InvalidFormula(
                    "SQRT of a negative number".into(),
                ));
            }
            Ok(v.sqrt())
        }
        "ROUND" => {
            if args.len() != 1 {
                arity(2)?;
            }
            let v = evaluate(&args[0], lookup)?;
            let digits = match args.get(1) {
                Some(d) => evaluate(d, lookup)?.trunc() as i32,
                None => 0,
            };
            let factor = 10f64.powi(digits);
            Ok((v * factor).round() / factor)
        }
        _ => Err(SpreadsheetError::InvalidFormula(format!(
            "unknown function {}",
            name
        ))),
    }
}

// Aggregate arguments: references and ranges contribute only their numeric cells
// (text and empty cells are skipped); any other expression is evaluated directly.
fn numeric_args<F>(args: &[Expr], lookup: &mut F) -> Result<Vec<f64>, SpreadsheetError>
where
    F: FnMut(Coord) -> Result<Resolved, SpreadsheetError>,
{
    let mut values = Vec::new();
    for arg in args {
        match arg {
            Expr::CellRef(c) => {
                if let Resolved::Number(n) = lookup(*c)? {
                    values.push(n);
                }
            }
            Expr::Range(a, b) => {
                for c in range_cells(*a, *b) {
                    if let Resolved::Number(n) = lookup(c)? {
                        values.push(n);
                    }
                }
            }
            other => values.push(evaluate(other, lookup)?),
        }
    }
    Ok(values)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Op(char),
    LParen,
    RParen,
    Comma,
    Colon,
}

fn tokenize(source: &str) -> Result<Vec<Token>, SpreadsheetError> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                // Optional exponent, e.g. 1e3 or 2.5E-4
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    let mut j = i + 1;
                    if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j].is_ascii_digit() {
                        i = j;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let text: String = chars[start..i].iter().collect();
                let n = text.parse().map_err(|_| {
                    SpreadsheetError::InvalidFormula(format!("bad number '{}'", text))
                })?;
                tokens.push(Token::Number(n));
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                tokens.push(Token::Ident(ident.to_ascii_uppercase()));
            }
            '+' | '-' | '*' | '/' | '^' => {
                tokens.push(Token::Op(c));
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            ':' => {
                tokens.push(Token::Colon);
                i += 1;
            }
            _ => {
                return Err(SpreadsheetError::InvalidFormula(format!(
                    "unexpected character '{}'",
                    c
                )))
            }
        }
    }
    Ok(tokens)
}

// Recursive-descent parser. Precedence, lowest to highest:
//   + -    (left-associative)
//   * /    (left-associative)
//   unary -
//   ^      (right-associative)
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn expect(&mut self, expected: Token) -> Result<(), SpreadsheetError> {
        match self.next() {
            Some(t) if t == expected => Ok(()),
            other => Err(SpreadsheetError::InvalidFormula(format!(
                "expected {:?}, found {:?}",
                expected, other
            ))),
        }
    }

    fn expression(&mut self) -> Result<Expr, SpreadsheetError> {
        let mut left = self.term()?;
        while let Some(Token::Op(op @ ('+' | '-'))) = self.peek() {
            let op = if *op == '+' { BinOp::Add } else { BinOp::Sub };
            self.pos += 1;
            let right = self.term()?;
            left = Expr::Binary(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<Expr, SpreadsheetError> {
        let mut left = self.unary()?;
        while let Some(Token::Op(op @ ('*' | '/'))) = self.peek() {
            let op = if *op == '*' { BinOp::Mul } else { BinOp::Div };
            self.pos += 1;
            let right = self.unary()?;
            left = Expr::Binary(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, SpreadsheetError> {
        match self.peek() {
            Some(Token::Op('-')) => {
                self.pos += 1;
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            Some(Token::Op('+')) => {
                self.pos += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Expr, SpreadsheetError> {
        let base = self.primary()?;
        if let Some(Token::Op('^')) = self.peek() {
            self.pos += 1;
            let exponent = self.unary()?;
            return Ok(Expr::Binary(Box::new(base), BinOp::Pow, Box::new(exponent)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Expr, SpreadsheetError> {
        match self.next() {
            Some(Token::Number(n)) => Ok(Expr::Number(n)),
            Some(Token::LParen) => {
                let e = self.expression()?;
                self.expect(Token::RParen)?;
                Ok(e)
            }
            Some(Token::Ident(name)) => {
                if let Some(Token::LParen) = self.peek() {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if let Some(Token::RParen) = self.peek() {
                        self.pos += 1;
                    } else {
                        loop {
                            args.push(self.expression()?);
                            match self.next() {
                                Some(Token::Comma) => continue,
                                Some(Token::RParen) => break,
                                other => {
                                    return Err(SpreadsheetError::InvalidFormula(format!(
                                        "expected ',' or ')' in call to {}, found {:?}",
                                        name, other
                                    )))
                                }
                            }
                        }
                    }
                    return Ok(Expr::Call(name, args));
                }

                let start = parse_cell_ref(&name)?;
                if let Some(Token::Colon) = self.peek() {
                    self.pos += 1;
                    match self.next() {
                        Some(Token::Ident(end)) => {
                            return Ok(Expr::Range(start, parse_cell_ref(&end)?))
                        }
                        other => {
                            return Err(SpreadsheetError::InvalidFormula(format!(
                                "expected cell reference after ':', found {:?}",
                                other
                            )))
                        }
                    }
                }
                Ok(Expr::CellRef(start))
            }
            other => Err(SpreadsheetError::InvalidFormula(format!(
                "unexpected {:?}",
                other
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_str(formula: &str) -> Result<f64, SpreadsheetError> {
        evaluate(&parse(formula)?, &mut |_| Ok(Resolved::Empty))
    }

    #[test]
    fn cell_refs_round_trip() {
        assert_eq!(parse_cell_ref("A1").unwrap(), (0, 0));
        assert_eq!(parse_cell_ref("b2").unwrap(), (1, 1));
        assert_eq!(parse_cell_ref("Z1").unwrap(), (0, 25));
        assert_eq!(parse_cell_ref("AA10").unwrap(), (9, 26));
        for coord in [(0, 0), (9, 26), (99, 701), (4, 702)] {
            assert_eq!(parse_cell_ref(&to_cell_ref(coord)).unwrap(), coord);
        }
        assert!(parse_cell_ref("A0").is_err());
        assert!(parse_cell_ref("1A").is_err());
        assert!(parse_cell_ref("A").is_err());
        assert!(parse_cell_ref("A1B").is_err());
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(eval_str("1 + 2 * 3").unwrap(), 7.0);
        assert_eq!(eval_str("(1 + 2) * 3").unwrap(), 9.0);
        assert_eq!(eval_str("=10 / 4 - 1").unwrap(), 1.5);
        assert_eq!(eval_str("2 ^ 3 ^ 2").unwrap(), 512.0);
        assert_eq!(eval_str("-2 ^ 2").unwrap(), -4.0);
        assert_eq!(eval_str("--3").unwrap(), 3.0);
        assert_eq!(eval_str("1e3 + .5").unwrap(), 1000.5);
    }

    #[test]
    fn errors() {
        assert_eq!(eval_str("1 / 0"), Err(SpreadsheetError::DivisionByZero));
        assert!(matches!(
            eval_str("1 +"),
            Err(SpreadsheetError::InvalidFormula(_))
        ));
        assert!(matches!(
            eval_str("(1"),
            Err(SpreadsheetError::InvalidFormula(_))
        ));
        assert!(matches!(
            eval_str("1 2"),
            Err(SpreadsheetError::InvalidFormula(_))
        ));
        assert!(matches!(
            eval_str("FOO(1)"),
            Err(SpreadsheetError::InvalidFormula(_))
        ));
        assert!(matches!(
            eval_str("A1:B2"),
            Err(SpreadsheetError::InvalidFormula(_))
        ));
        assert!(matches!(
            eval_str("1 # 2"),
            Err(SpreadsheetError::InvalidFormula(_))
        ));
    }

    #[test]
    fn references_expand_ranges() {
        let expr = parse("SUM(B2:A1) + C3 + A1").unwrap();
        assert_eq!(
            references(&expr),
            vec![(0, 0), (0, 1), (1, 0), (1, 1), (2, 2)]
        );
    }

    #[test]
    fn scalar_functions() {
        assert_eq!(eval_str("ABS(-3)").unwrap(), 3.0);
        assert_eq!(eval_str("SQRT(16)").unwrap(), 4.0);
        assert_eq!(eval_str("ROUND(2.456, 2)").unwrap(), 2.46);
        assert_eq!(eval_str("round(2.5)").unwrap(), 3.0);
        assert!(eval_str("SQRT(-1)").is_err());
        assert!(eval_str("ABS(1, 2)").is_err());
    }
}
