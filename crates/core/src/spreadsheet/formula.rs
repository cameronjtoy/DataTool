// Formula parsing and evaluation.
//
// Supported syntax (an optional leading '=' is ignored, names are case-insensitive):
//   numbers        10, 2.5, .5, 1e3
//   text           "hello", with "" for a literal quote
//   booleans       TRUE, FALSE
//   cell refs      A1, b2, AA10
//   ranges         A1:B3 (only as function arguments)
//   operators      + - * / ^ and unary -, & (text concatenation),
//                  = <> < > <= >= (comparison), with parentheses for grouping
//   functions      SUM, AVERAGE (AVG), MIN, MAX, COUNT, ABS, SQRT, ROUND,
//                  IF, AND, OR, NOT
//
// Precedence, lowest to highest: comparison, &, + -, * /, unary -, ^

use crate::spreadsheet::error::SpreadsheetError;
use std::cmp::Ordering;
use std::fmt;

pub type Coord = (usize, usize);

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    Bool(bool),
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
    Concat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

// The result of evaluating a formula, or the contents of a non-empty cell
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(f64),
    Text(String),
    Bool(bool),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Number(n) => write!(f, "{}", n),
            Value::Text(s) => write!(f, "{}", s),
            Value::Bool(b) => write!(f, "{}", if *b { "TRUE" } else { "FALSE" }),
        }
    }
}

impl Value {
    // Numeric view used by arithmetic: booleans count as 1/0, text is an error
    pub fn as_number(&self) -> Result<f64, SpreadsheetError> {
        match self {
            Value::Number(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Text(s) => Err(SpreadsheetError::TypeMismatch(format!(
                "expected a number, found text \"{}\"",
                s
            ))),
        }
    }

    // Truth value used by IF/AND/OR/NOT: non-zero numbers are true, text is an error
    pub fn as_bool(&self) -> Result<bool, SpreadsheetError> {
        match self {
            Value::Bool(b) => Ok(*b),
            Value::Number(n) => Ok(*n != 0.0),
            Value::Text(s) => Err(SpreadsheetError::TypeMismatch(format!(
                "expected TRUE/FALSE, found text \"{}\"",
                s
            ))),
        }
    }
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
    let expr = parser.comparison()?;
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
        Expr::Number(_) | Expr::Text(_) | Expr::Bool(_) => {}
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

// Evaluate an expression, resolving cell references through `lookup`,
// which returns None for an empty cell
pub fn evaluate<F>(expr: &Expr, lookup: &mut F) -> Result<Value, SpreadsheetError>
where
    F: FnMut(Coord) -> Result<Option<Value>, SpreadsheetError>,
{
    match expr {
        Expr::Number(n) => Ok(Value::Number(*n)),
        Expr::Text(s) => Ok(Value::Text(s.clone())),
        Expr::Bool(b) => Ok(Value::Bool(*b)),
        // An empty cell reads as 0 in a scalar context
        Expr::CellRef(c) => Ok(lookup(*c)?.unwrap_or(Value::Number(0.0))),
        Expr::Range(..) => Err(SpreadsheetError::InvalidFormula(
            "a range can only be used as a function argument".into(),
        )),
        Expr::Neg(e) => Ok(Value::Number(-evaluate(e, lookup)?.as_number()?)),
        Expr::Binary(l, op, r) => {
            let l = evaluate(l, lookup)?;
            let r = evaluate(r, lookup)?;
            binary(*op, l, r)
        }
        Expr::Call(name, args) => call(name, args, lookup),
    }
}

fn binary(op: BinOp, l: Value, r: Value) -> Result<Value, SpreadsheetError> {
    let compare = |ordering: fn(Ordering) -> bool| -> Result<Value, SpreadsheetError> {
        Ok(Value::Bool(ordering(compare_values(&l, &r)?)))
    };
    match op {
        BinOp::Concat => Ok(Value::Text(format!("{}{}", l, r))),
        BinOp::Eq => Ok(Value::Bool(compare_values(&l, &r)? == Ordering::Equal)),
        BinOp::Ne => Ok(Value::Bool(compare_values(&l, &r)? != Ordering::Equal)),
        BinOp::Lt => compare(|o| o == Ordering::Less),
        BinOp::Gt => compare(|o| o == Ordering::Greater),
        BinOp::Le => compare(|o| o != Ordering::Greater),
        BinOp::Ge => compare(|o| o != Ordering::Less),
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => {
            let (l, r) = (l.as_number()?, r.as_number()?);
            let n = match op {
                BinOp::Add => l + r,
                BinOp::Sub => l - r,
                BinOp::Mul => l * r,
                BinOp::Div if r == 0.0 => return Err(SpreadsheetError::DivisionByZero),
                BinOp::Div => l / r,
                _ => l.powf(r),
            };
            Ok(Value::Number(n))
        }
    }
}

// Numbers compare numerically, text case-insensitively, booleans FALSE < TRUE.
// Comparing different types is an error.
fn compare_values(l: &Value, r: &Value) -> Result<Ordering, SpreadsheetError> {
    match (l, r) {
        (Value::Number(a), Value::Number(b)) => a
            .partial_cmp(b)
            .ok_or_else(|| SpreadsheetError::TypeMismatch("cannot compare NaN".into())),
        (Value::Text(a), Value::Text(b)) => Ok(a.to_lowercase().cmp(&b.to_lowercase())),
        (Value::Bool(a), Value::Bool(b)) => Ok(a.cmp(b)),
        _ => Err(SpreadsheetError::TypeMismatch(format!(
            "cannot compare {} with {}",
            type_name(l),
            type_name(r)
        ))),
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Number(_) => "a number",
        Value::Text(_) => "text",
        Value::Bool(_) => "a boolean",
    }
}

fn call<F>(name: &str, args: &[Expr], lookup: &mut F) -> Result<Value, SpreadsheetError>
where
    F: FnMut(Coord) -> Result<Option<Value>, SpreadsheetError>,
{
    let arity = |min: usize, max: usize| {
        if (min..=max).contains(&args.len()) {
            Ok(())
        } else if min == max {
            Err(SpreadsheetError::InvalidFormula(format!(
                "{} expects {} argument(s), got {}",
                name,
                min,
                args.len()
            )))
        } else {
            Err(SpreadsheetError::InvalidFormula(format!(
                "{} expects {} to {} arguments, got {}",
                name,
                min,
                max,
                args.len()
            )))
        }
    };
    let number = |n: f64| Ok(Value::Number(n));

    match name {
        "SUM" => number(numeric_args(args, lookup)?.iter().sum()),
        "AVERAGE" | "AVG" => {
            let values = numeric_args(args, lookup)?;
            if values.is_empty() {
                return Err(SpreadsheetError::DivisionByZero);
            }
            number(values.iter().sum::<f64>() / values.len() as f64)
        }
        "MIN" => number(
            numeric_args(args, lookup)?
                .into_iter()
                .reduce(f64::min)
                .unwrap_or(0.0),
        ),
        "MAX" => number(
            numeric_args(args, lookup)?
                .into_iter()
                .reduce(f64::max)
                .unwrap_or(0.0),
        ),
        "COUNT" => number(numeric_args(args, lookup)?.len() as f64),
        "ABS" => {
            arity(1, 1)?;
            number(evaluate(&args[0], lookup)?.as_number()?.abs())
        }
        "SQRT" => {
            arity(1, 1)?;
            let v = evaluate(&args[0], lookup)?.as_number()?;
            if v < 0.0 {
                return Err(SpreadsheetError::InvalidFormula(
                    "SQRT of a negative number".into(),
                ));
            }
            number(v.sqrt())
        }
        "ROUND" => {
            arity(1, 2)?;
            let v = evaluate(&args[0], lookup)?.as_number()?;
            let digits = match args.get(1) {
                Some(d) => evaluate(d, lookup)?.as_number()?.trunc() as i32,
                None => 0,
            };
            let factor = 10f64.powi(digits);
            number((v * factor).round() / factor)
        }
        "IF" => {
            arity(2, 3)?;
            // Only the chosen branch is evaluated
            if evaluate(&args[0], lookup)?.as_bool()? {
                evaluate(&args[1], lookup)
            } else {
                match args.get(2) {
                    Some(e) => evaluate(e, lookup),
                    None => Ok(Value::Bool(false)),
                }
            }
        }
        "AND" | "OR" => {
            if args.is_empty() {
                return Err(SpreadsheetError::InvalidFormula(format!(
                    "{} expects at least 1 argument",
                    name
                )));
            }
            let values = logical_args(args, lookup)?;
            Ok(Value::Bool(if name == "AND" {
                values.iter().all(|b| *b)
            } else {
                values.iter().any(|b| *b)
            }))
        }
        "NOT" => {
            arity(1, 1)?;
            Ok(Value::Bool(!evaluate(&args[0], lookup)?.as_bool()?))
        }
        _ => Err(SpreadsheetError::InvalidFormula(format!(
            "unknown function {}",
            name
        ))),
    }
}

// Values an aggregate argument contributes. For references and ranges this is
// the non-empty cells' values; any other expression contributes its own value.
fn arg_values<F>(arg: &Expr, lookup: &mut F) -> Result<ArgValues, SpreadsheetError>
where
    F: FnMut(Coord) -> Result<Option<Value>, SpreadsheetError>,
{
    match arg {
        Expr::CellRef(c) => Ok(ArgValues::Referenced(lookup(*c)?.into_iter().collect())),
        Expr::Range(a, b) => {
            let mut values = Vec::new();
            for c in range_cells(*a, *b) {
                values.extend(lookup(c)?);
            }
            Ok(ArgValues::Referenced(values))
        }
        other => Ok(ArgValues::Direct(evaluate(other, lookup)?)),
    }
}

enum ArgValues {
    Referenced(Vec<Value>),
    Direct(Value),
}

// Aggregate arguments: referenced cells contribute only numbers (text and
// booleans are skipped); direct values must be convertible to numbers.
fn numeric_args<F>(args: &[Expr], lookup: &mut F) -> Result<Vec<f64>, SpreadsheetError>
where
    F: FnMut(Coord) -> Result<Option<Value>, SpreadsheetError>,
{
    let mut out = Vec::new();
    for arg in args {
        match arg_values(arg, lookup)? {
            ArgValues::Referenced(values) => out.extend(values.iter().filter_map(|v| match v {
                Value::Number(n) => Some(*n),
                _ => None,
            })),
            ArgValues::Direct(v) => out.push(v.as_number()?),
        }
    }
    Ok(out)
}

// Logical arguments: referenced cells contribute booleans and numbers (text is
// skipped); direct values must be convertible to booleans.
fn logical_args<F>(args: &[Expr], lookup: &mut F) -> Result<Vec<bool>, SpreadsheetError>
where
    F: FnMut(Coord) -> Result<Option<Value>, SpreadsheetError>,
{
    let mut out = Vec::new();
    for arg in args {
        match arg_values(arg, lookup)? {
            ArgValues::Referenced(values) => {
                for v in values {
                    if !matches!(v, Value::Text(_)) {
                        out.push(v.as_bool()?);
                    }
                }
            }
            ArgValues::Direct(v) => out.push(v.as_bool()?),
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Text(String),
    Ident(String),
    Op(BinOp),
    Minus,
    Plus,
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
        let next = chars.get(i + 1).copied();
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
            '"' => {
                // String literal; "" inside the literal is an escaped quote
                let mut text = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => {
                            return Err(SpreadsheetError::InvalidFormula(
                                "unterminated text literal".into(),
                            ))
                        }
                        Some('"') if chars.get(i + 1) == Some(&'"') => {
                            text.push('"');
                            i += 2;
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(ch) => {
                            text.push(*ch);
                            i += 1;
                        }
                    }
                }
                tokens.push(Token::Text(text));
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                tokens.push(Token::Ident(ident.to_ascii_uppercase()));
            }
            '<' if next == Some('>') => {
                tokens.push(Token::Op(BinOp::Ne));
                i += 2;
            }
            '<' if next == Some('=') => {
                tokens.push(Token::Op(BinOp::Le));
                i += 2;
            }
            '>' if next == Some('=') => {
                tokens.push(Token::Op(BinOp::Ge));
                i += 2;
            }
            _ => {
                let token = match c {
                    '+' => Token::Plus,
                    '-' => Token::Minus,
                    '*' => Token::Op(BinOp::Mul),
                    '/' => Token::Op(BinOp::Div),
                    '^' => Token::Op(BinOp::Pow),
                    '&' => Token::Op(BinOp::Concat),
                    '=' => Token::Op(BinOp::Eq),
                    '<' => Token::Op(BinOp::Lt),
                    '>' => Token::Op(BinOp::Gt),
                    '(' => Token::LParen,
                    ')' => Token::RParen,
                    ',' => Token::Comma,
                    ':' => Token::Colon,
                    _ => {
                        return Err(SpreadsheetError::InvalidFormula(format!(
                            "unexpected character '{}'",
                            c
                        )))
                    }
                };
                tokens.push(token);
                i += 1;
            }
        }
    }
    Ok(tokens)
}

// Recursive-descent parser, one method per precedence level
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

    // Parse a left-associative chain of `ops` over operands parsed by `operand`
    fn left_assoc(
        &mut self,
        ops: &[BinOp],
        operand: fn(&mut Self) -> Result<Expr, SpreadsheetError>,
    ) -> Result<Expr, SpreadsheetError> {
        let mut left = operand(self)?;
        loop {
            let op = match self.peek() {
                Some(Token::Op(op)) if ops.contains(op) => *op,
                Some(Token::Plus) if ops.contains(&BinOp::Add) => BinOp::Add,
                Some(Token::Minus) if ops.contains(&BinOp::Sub) => BinOp::Sub,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = operand(self)?;
            left = Expr::Binary(Box::new(left), op, Box::new(right));
        }
    }

    fn comparison(&mut self) -> Result<Expr, SpreadsheetError> {
        use BinOp::*;
        self.left_assoc(&[Eq, Ne, Lt, Gt, Le, Ge], Self::concat)
    }

    fn concat(&mut self) -> Result<Expr, SpreadsheetError> {
        self.left_assoc(&[BinOp::Concat], Self::additive)
    }

    fn additive(&mut self) -> Result<Expr, SpreadsheetError> {
        self.left_assoc(&[BinOp::Add, BinOp::Sub], Self::term)
    }

    fn term(&mut self) -> Result<Expr, SpreadsheetError> {
        self.left_assoc(&[BinOp::Mul, BinOp::Div], Self::unary)
    }

    fn unary(&mut self) -> Result<Expr, SpreadsheetError> {
        match self.peek() {
            Some(Token::Minus) => {
                self.pos += 1;
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            Some(Token::Plus) => {
                self.pos += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    // ^ is right-associative and binds tighter than unary minus: -2^2 = -4
    fn power(&mut self) -> Result<Expr, SpreadsheetError> {
        let base = self.primary()?;
        if let Some(Token::Op(BinOp::Pow)) = self.peek() {
            self.pos += 1;
            let exponent = self.unary()?;
            return Ok(Expr::Binary(Box::new(base), BinOp::Pow, Box::new(exponent)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Expr, SpreadsheetError> {
        match self.next() {
            Some(Token::Number(n)) => Ok(Expr::Number(n)),
            Some(Token::Text(s)) => Ok(Expr::Text(s)),
            Some(Token::LParen) => {
                let e = self.comparison()?;
                self.expect(Token::RParen)?;
                Ok(e)
            }
            Some(Token::Ident(name)) => {
                if let Some(Token::LParen) = self.peek() {
                    self.pos += 1;
                    return Ok(Expr::Call(name.clone(), self.call_args(&name)?));
                }
                match name.as_str() {
                    "TRUE" => return Ok(Expr::Bool(true)),
                    "FALSE" => return Ok(Expr::Bool(false)),
                    _ => {}
                }

                let start = parse_cell_ref(&name)?;
                if let Some(Token::Colon) = self.peek() {
                    self.pos += 1;
                    return match self.next() {
                        Some(Token::Ident(end)) => Ok(Expr::Range(start, parse_cell_ref(&end)?)),
                        other => Err(SpreadsheetError::InvalidFormula(format!(
                            "expected cell reference after ':', found {:?}",
                            other
                        ))),
                    };
                }
                Ok(Expr::CellRef(start))
            }
            other => Err(SpreadsheetError::InvalidFormula(format!(
                "unexpected {:?}",
                other
            ))),
        }
    }

    // Arguments of a function call, after the opening parenthesis
    fn call_args(&mut self, name: &str) -> Result<Vec<Expr>, SpreadsheetError> {
        let mut args = Vec::new();
        if let Some(Token::RParen) = self.peek() {
            self.pos += 1;
            return Ok(args);
        }
        loop {
            args.push(self.comparison()?);
            match self.next() {
                Some(Token::Comma) => continue,
                Some(Token::RParen) => return Ok(args),
                other => {
                    return Err(SpreadsheetError::InvalidFormula(format!(
                        "expected ',' or ')' in call to {}, found {:?}",
                        name, other
                    )))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_str(formula: &str) -> Result<Value, SpreadsheetError> {
        evaluate(&parse(formula)?, &mut |_| Ok(None))
    }

    fn num(formula: &str) -> f64 {
        match eval_str(formula) {
            Ok(Value::Number(n)) => n,
            other => panic!("{} evaluated to {:?}", formula, other),
        }
    }

    fn val(formula: &str) -> Value {
        eval_str(formula).unwrap_or_else(|e| panic!("{} failed: {}", formula, e))
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
        assert_eq!(num("1 + 2 * 3"), 7.0);
        assert_eq!(num("(1 + 2) * 3"), 9.0);
        assert_eq!(num("=10 / 4 - 1"), 1.5);
        assert_eq!(num("10 - 2 - 3"), 5.0);
        assert_eq!(num("2 ^ 3 ^ 2"), 512.0);
        assert_eq!(num("-2 ^ 2"), -4.0);
        assert_eq!(num("--3"), 3.0);
        assert_eq!(num("1e3 + .5"), 1000.5);
        assert_eq!(num("TRUE + 1"), 2.0);
    }

    #[test]
    fn text_and_comparison() {
        assert_eq!(val("\"a\" & \"b\" & 1"), Value::Text("ab1".into()));
        assert_eq!(val("\"say \"\"hi\"\"\""), Value::Text("say \"hi\"".into()));
        assert_eq!(val("1 + 1 = 2"), Value::Bool(true));
        assert_eq!(val("3 <> 3"), Value::Bool(false));
        assert_eq!(val("2 < 10"), Value::Bool(true));
        assert_eq!(val("2 >= 2"), Value::Bool(true));
        assert_eq!(val("\"Apple\" = \"apple\""), Value::Bool(true));
        assert_eq!(val("\"a\" < \"B\""), Value::Bool(true));
        assert_eq!(val("1 & 2 = \"12\""), Value::Bool(true));
        assert!(matches!(
            eval_str("1 < \"a\""),
            Err(SpreadsheetError::TypeMismatch(_))
        ));
        assert!(matches!(
            eval_str("\"a\" * 2"),
            Err(SpreadsheetError::TypeMismatch(_))
        ));
    }

    #[test]
    fn logical_functions() {
        assert_eq!(val("IF(1 > 0, \"yes\", \"no\")"), Value::Text("yes".into()));
        assert_eq!(val("IF(FALSE, 1)"), Value::Bool(false));
        // The branch not taken is never evaluated
        assert_eq!(num("IF(TRUE, 1, 1 / 0)"), 1.0);
        assert_eq!(val("AND(TRUE, 1, 2 > 1)"), Value::Bool(true));
        assert_eq!(val("OR(FALSE, 0)"), Value::Bool(false));
        assert_eq!(val("NOT(0)"), Value::Bool(true));
        assert!(eval_str("AND()").is_err());
        assert!(eval_str("IF(\"x\", 1, 2)").is_err());
    }

    #[test]
    fn errors() {
        assert_eq!(eval_str("1 / 0"), Err(SpreadsheetError::DivisionByZero));
        for bad in [
            "1 +", "(1", "1 2", "FOO(1)", "A1:B2", "1 # 2", "\"open", "SUM(1",
        ] {
            assert!(
                matches!(eval_str(bad), Err(SpreadsheetError::InvalidFormula(_))),
                "{} should be an invalid formula",
                bad
            );
        }
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
        assert_eq!(num("ABS(-3)"), 3.0);
        assert_eq!(num("SQRT(16)"), 4.0);
        assert_eq!(num("ROUND(2.456, 2)"), 2.46);
        assert_eq!(num("round(2.5)"), 3.0);
        assert!(eval_str("SQRT(-1)").is_err());
        assert!(eval_str("ABS(1, 2)").is_err());
        assert!(eval_str("ROUND(1, 2, 3)").is_err());
    }
}
