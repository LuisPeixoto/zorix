//! `math.evaluate`: arithmetic with a small hand-written parser. There is no `eval`, no variables and no functions, so
//! there is nothing to escape from.

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{Tool, ToolAnnotations, ToolContext, ToolError, ToolSpec};

const MAX_LENGTH: usize = 200;
const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    text: &'a [u8],
    at: usize,
    depth: usize,
}

type Calc<T> = Result<T, String>;

impl Parser<'_> {
    fn skip_spaces(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_spaces();
        self.text.get(self.at).copied()
    }

    fn enter(&mut self) -> Calc<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err("The expression is nested too deeply.".into());
        }
        Ok(())
    }

    fn expression(&mut self) -> Calc<f64> {
        let mut value = self.term()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.at += 1;
            let right = self.term()?;
            value = if op == b'+' { value + right } else { value - right };
        }
        Ok(value)
    }

    fn term(&mut self) -> Calc<f64> {
        let mut value = self.unary()?;
        while let Some(op @ (b'*' | b'/' | b'%')) = self.peek() {
            self.at += 1;
            let right = self.unary()?;
            value = match op {
                b'*' => value * right,
                b'/' if right == 0.0 => return Err("Division by zero.".into()),
                b'/' => value / right,
                _ if right == 0.0 => return Err("Division by zero.".into()),
                _ => value % right,
            };
        }
        Ok(value)
    }

    fn unary(&mut self) -> Calc<f64> {
        self.enter()?;
        let result = match self.peek() {
            Some(b'-') => {
                self.at += 1;
                self.unary().map(|v| -v)
            }
            Some(b'+') => {
                self.at += 1;
                self.unary()
            }
            _ => self.power(),
        };
        self.depth -= 1;
        result
    }

    fn power(&mut self) -> Calc<f64> {
        let base = self.atom()?;
        if self.peek() == Some(b'^') {
            self.at += 1;
            let exponent = self.unary()?;
            return Ok(base.powf(exponent));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Calc<f64> {
        match self.peek() {
            Some(b'(') => {
                self.at += 1;
                self.enter()?;
                let value = self.expression()?;
                self.depth -= 1;
                if self.peek() != Some(b')') {
                    return Err("A closing parenthesis is missing.".into());
                }
                self.at += 1;
                Ok(value)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => self.number(),
            Some(c) => {
                Err(format!("Unexpected “{}”. Only numbers, + - * / % ^ and parentheses are allowed.", c as char))
            }
            None => Err("The expression ends too early.".into()),
        }
    }

    fn number(&mut self) -> Calc<f64> {
        let start = self.at;
        while self.text.get(self.at).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
            self.at += 1;
        }
        let slice = std::str::from_utf8(&self.text[start..self.at]).map_err(|_| "Not a number.".to_string())?;
        slice.parse::<f64>().map_err(|_| format!("“{slice}” is not a number."))
    }
}

/// Evaluate an arithmetic expression.
pub fn evaluate(expression: &str) -> Calc<f64> {
    if expression.trim().is_empty() {
        return Err("The expression is empty.".into());
    }
    if expression.len() > MAX_LENGTH {
        return Err(format!("The expression is longer than {MAX_LENGTH} characters."));
    }
    let mut parser = Parser { text: expression.as_bytes(), at: 0, depth: 0 };
    let value = parser.expression()?;
    if parser.peek().is_some() {
        return Err(format!("Unexpected “{}” after the end of the expression.", parser.text[parser.at] as char));
    }
    if !value.is_finite() {
        return Err("The result is too large to represent.".into());
    }
    Ok(value)
}

fn as_json(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9.0e15 { json!(value as i64) } else { json!(value) }
}

/// The `math.evaluate` tool.
pub struct MathTool {
    spec: ToolSpec,
}

impl MathTool {
    /// A new tool.
    pub fn new() -> Self {
        let schema = json!({
            "type": "object",
            "properties": {"expression": {"type": "string", "maxLength": MAX_LENGTH, "description": "For example 17*23 or (2+3)^2"}},
            "required": ["expression"],
            "additionalProperties": false
        });
        let mut spec = ToolSpec::new(
            "math.evaluate",
            "Calculator",
            "Evaluates an arithmetic expression with + - * / % ^ and parentheses, and returns the number.",
            schema,
        );
        spec.annotations = ToolAnnotations::safe_read();
        spec.timeout_ms = 1_000;
        Self { spec }
    }
}

impl Default for MathTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MathTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let expression = args["expression"].as_str().unwrap_or_default();
        let value = evaluate(expression).map_err(ToolError::Failed)?;
        Ok(json!({ "expression": expression.trim(), "result": as_json(value) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> f64 {
        evaluate(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(ok("17*23"), 391.0);
        assert_eq!(ok("2+3*4"), 14.0);
        assert_eq!(ok("(2+3)*4"), 20.0);
        assert_eq!(ok("10-4-3"), 3.0, "left associative");
        assert_eq!(ok("2^3^2"), 512.0, "power is right associative");
        assert_eq!(ok("-2^2"), -4.0);
        assert_eq!(ok("2^-1"), 0.5);
        assert_eq!(ok("10 % 4"), 2.0);
        assert_eq!(ok("--3"), 3.0);
        assert_eq!(ok(" 1.5 + .5 "), 2.0);
    }

    #[test]
    fn mistakes_are_explained() {
        for (text, wanted) in [
            ("", "empty"),
            ("1/0", "Division by zero"),
            ("5%0", "Division by zero"),
            ("2 +", "ends too early"),
            ("(1+2", "closing parenthesis"),
            ("1+2)", "after the end"),
            ("abc", "Unexpected"),
            ("1; rm -rf /", "after the end"),
            ("__import__('os')", "Unexpected"),
            ("1.2.3", "not a number"),
            ("9^9^9^9", "too large"),
        ] {
            let error = evaluate(text).unwrap_err();
            assert!(error.contains(wanted), "{text}: {error}");
        }
    }

    #[test]
    fn size_and_depth_are_bounded() {
        assert!(evaluate(&"1+".repeat(150)).unwrap_err().contains("longer than"));
        let deep = format!("{}1{}", "(".repeat(70), ")".repeat(70));
        assert!(evaluate(&deep).unwrap_err().contains("nested too deeply"));
        let minuses = format!("{}1", "-".repeat(150));
        assert!(evaluate(&minuses).is_err());
    }

    #[test]
    fn results_are_integers_when_they_can_be() {
        assert_eq!(as_json(391.0), json!(391));
        assert_eq!(as_json(0.5), json!(0.5));
        assert_eq!(as_json(1.0e20), json!(1.0e20));
    }
}
