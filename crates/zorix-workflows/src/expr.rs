//! The small expression language of conditions and webhook filters (ADR-0033).
//!
//! `severity >= 3 and steps.triage.output.text contains "outage"`. Paths, numbers, strings, `true`, `false`, `null`, the
//! comparisons `== != < <= > >=`, `contains`, `and`, `or`, `not` and parentheses. Nothing else: no calls, no assignment, no
//! loops, so an expression always ends and can do nothing but read.

use serde_json::Value;

/// Why an expression could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ExprError(pub String);

fn err<T>(message: impl Into<String>) -> Result<T, ExprError> {
    Err(ExprError(message.into()))
}

/// One step of a path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    /// A key of an object.
    Key(String),
    /// An item of an array.
    Index(usize),
}

/// Read a path such as `trigger.payload.items[0].name`.
pub fn parse_path(text: &str) -> Result<Vec<Segment>, ExprError> {
    let chars: Vec<char> = text.trim().chars().collect();
    if chars.is_empty() {
        return err("A path is empty.");
    }
    let mut segments = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '.' => {
                i += 1;
                if i >= chars.len() || chars[i] == '.' || chars[i] == '[' {
                    return err(format!("“{text}” has a misplaced dot."));
                }
            }
            '[' => {
                let end = chars[i..].iter().position(|c| *c == ']').map(|p| p + i);
                let Some(end) = end else { return err(format!("“{text}” has a [ that is never closed.")) };
                let digits: String = chars[i + 1..end].iter().collect();
                let index: usize = digits.parse().map_err(|_| ExprError(format!("“[{digits}]” is not an index.")))?;
                segments.push(Segment::Index(index));
                i = end + 1;
                if i < chars.len() && chars[i] != '.' && chars[i] != '[' {
                    return err(format!("“{text}” needs a dot after ]."));
                }
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | '-')) {
                    i += 1;
                }
                segments.push(Segment::Key(chars[start..i].iter().collect()));
            }
            other => return err(format!("“{text}” has an unexpected “{other}”.")),
        }
    }
    if segments.is_empty() {
        return err("A path is empty.");
    }
    Ok(segments)
}

/// The value a path points at, or none.
pub fn lookup<'a>(root: &'a Value, path: &[Segment]) -> Option<&'a Value> {
    let mut at = root;
    for segment in path {
        at = match (segment, at) {
            (Segment::Key(key), Value::Object(map)) => map.get(key)?,
            (Segment::Index(index), Value::Array(items)) => items.get(*index)?,
            _ => return None,
        };
    }
    Some(at)
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Literal(Value),
    Path(Vec<Segment>),
    Not(Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    Compare(Box<Node>, Op, Box<Node>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Text(String),
    Word(String),
    Op(Op),
    Open,
    Close,
}

const MAX_DEPTH: usize = 24;

fn tokenize(text: &str) -> Result<Vec<Token>, ExprError> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '(' => {
                tokens.push(Token::Open);
                i += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                i += 1;
            }
            '"' | '\'' => {
                let quote = c;
                let mut out = String::new();
                i += 1;
                loop {
                    let Some(&ch) = chars.get(i) else { return err("A quoted text is never closed.") };
                    i += 1;
                    if ch == quote {
                        break;
                    }
                    if ch == '\\' {
                        let Some(&next) = chars.get(i) else { return err("A quoted text ends with a backslash.") };
                        i += 1;
                        out.push(match next {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                    } else {
                        out.push(ch);
                    }
                }
                tokens.push(Token::Text(out));
            }
            '=' | '!' | '<' | '>' => {
                let two = chars.get(i + 1) == Some(&'=');
                let op = match (c, two) {
                    ('=', true) => Op::Eq,
                    ('!', true) => Op::Ne,
                    ('<', true) => Op::Le,
                    ('>', true) => Op::Ge,
                    ('<', false) => Op::Lt,
                    ('>', false) => Op::Gt,
                    _ => return err(format!("“{c}” is not an operator here (use == != < <= > >=).")),
                };
                i += if two { 2 } else { 1 };
                tokens.push(Token::Op(op));
            }
            c if c.is_ascii_digit() || (c == '-' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) => {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let raw: String = chars[start..i].iter().collect();
                tokens.push(Token::Number(raw.parse().map_err(|_| ExprError(format!("“{raw}” is not a number.")))?));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | '-' | '.' | '[' | ']'))
                {
                    i += 1;
                }
                tokens.push(Token::Word(chars[start..i].iter().collect()));
            }
            other => return err(format!("Unexpected “{other}”.")),
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn word(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Word(w)) if w == word)
    }

    fn or(&mut self) -> Result<Node, ExprError> {
        let mut left = self.and()?;
        while self.word("or") {
            self.at += 1;
            left = Node::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Node, ExprError> {
        let mut left = self.not()?;
        while self.word("and") {
            self.at += 1;
            left = Node::And(Box::new(left), Box::new(self.not()?));
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Node, ExprError> {
        if self.word("not") {
            self.at += 1;
            self.depth += 1;
            if self.depth > MAX_DEPTH {
                return err("The expression is nested too deeply.");
            }
            let inner = self.not()?;
            self.depth -= 1;
            return Ok(Node::Not(Box::new(inner)));
        }
        self.compare()
    }

    fn compare(&mut self) -> Result<Node, ExprError> {
        let left = self.atom()?;
        let op = match self.peek() {
            Some(Token::Op(op)) => Some(*op),
            Some(Token::Word(w)) if w == "contains" => Some(Op::Contains),
            _ => None,
        };
        let Some(op) = op else { return Ok(left) };
        self.at += 1;
        let right = self.atom()?;
        Ok(Node::Compare(Box::new(left), op, Box::new(right)))
    }

    fn atom(&mut self) -> Result<Node, ExprError> {
        let Some(token) = self.peek().cloned() else { return err("The expression ends too soon.") };
        self.at += 1;
        match token {
            Token::Number(n) => {
                Ok(Node::Literal(serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null)))
            }
            Token::Text(t) => Ok(Node::Literal(Value::String(t))),
            Token::Open => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return err("The expression is nested too deeply.");
                }
                let inner = self.or()?;
                self.depth -= 1;
                match self.peek() {
                    Some(Token::Close) => {
                        self.at += 1;
                        Ok(inner)
                    }
                    _ => err("A ( is never closed."),
                }
            }
            Token::Word(word) => match word.as_str() {
                "true" => Ok(Node::Literal(Value::Bool(true))),
                "false" => Ok(Node::Literal(Value::Bool(false))),
                "null" => Ok(Node::Literal(Value::Null)),
                "and" | "or" | "not" | "contains" => err(format!("“{word}” is in the wrong place.")),
                _ => Ok(Node::Path(parse_path(&word)?)),
            },
            Token::Close => err("A ) has no matching (."),
            Token::Op(_) => err("An operator needs a value before it."),
        }
    }
}

/// A parsed expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr(Node);

impl Expr {
    /// Read an expression.
    pub fn parse(text: &str) -> Result<Expr, ExprError> {
        let tokens = tokenize(text)?;
        if tokens.is_empty() {
            return err("The expression is empty.");
        }
        let mut parser = Parser { tokens, at: 0, depth: 0 };
        let node = parser.or()?;
        if parser.at != parser.tokens.len() {
            return err("There is something left over after the expression.");
        }
        Ok(Expr(node))
    }

    /// The value of the expression over `root`. A path that points at nothing is `null`.
    pub fn eval(&self, root: &Value) -> Value {
        eval(&self.0, root)
    }

    /// Whether the expression is true over `root`.
    pub fn test(&self, root: &Value) -> bool {
        truthy(&self.eval(root))
    }

    /// The paths the expression reads.
    pub fn paths(&self) -> Vec<Vec<Segment>> {
        fn walk(node: &Node, out: &mut Vec<Vec<Segment>>) {
            match node {
                Node::Path(p) => out.push(p.clone()),
                Node::Literal(_) => {}
                Node::Not(n) => walk(n, out),
                Node::And(a, b) | Node::Or(a, b) | Node::Compare(a, _, b) => {
                    walk(a, out);
                    walk(b, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.0, &mut out);
        out
    }
}

/// What counts as true: not `null`, `false`, zero, an empty text, array or object.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn eval(node: &Node, root: &Value) -> Value {
    match node {
        Node::Literal(v) => v.clone(),
        Node::Path(p) => lookup(root, p).cloned().unwrap_or(Value::Null),
        Node::Not(n) => Value::Bool(!truthy(&eval(n, root))),
        Node::And(a, b) => Value::Bool(truthy(&eval(a, root)) && truthy(&eval(b, root))),
        Node::Or(a, b) => Value::Bool(truthy(&eval(a, root)) || truthy(&eval(b, root))),
        Node::Compare(a, op, b) => Value::Bool(compare(&eval(a, root), *op, &eval(b, root))),
    }
}

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

fn compare(left: &Value, op: Op, right: &Value) -> bool {
    match op {
        Op::Eq => same(left, right),
        Op::Ne => !same(left, right),
        Op::Contains => match (left, right) {
            (Value::String(a), Value::String(b)) => a.to_lowercase().contains(&b.to_lowercase()),
            (Value::Array(items), needle) => items.iter().any(|i| same(i, needle)),
            _ => false,
        },
        Op::Lt | Op::Le | Op::Gt | Op::Ge => {
            let order = match (left, right) {
                (Value::Number(a), Value::Number(b)) => a.as_f64().partial_cmp(&b.as_f64()),
                (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
                _ => None,
            };
            let Some(order) = order else { return false };
            match op {
                Op::Lt => order.is_lt(),
                Op::Le => order.is_le(),
                Op::Gt => order.is_gt(),
                _ => order.is_ge(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn run(text: &str, root: &Value) -> bool {
        Expr::parse(text).unwrap().test(root)
    }

    #[test]
    fn paths_read_keys_and_indexes() {
        let root = json!({"trigger": {"payload": {"labels": ["bug", "incident"], "n": 4}}, "steps": {"post-it": {"output": "x"}}});
        assert_eq!(lookup(&root, &parse_path("trigger.payload.labels[1]").unwrap()), Some(&json!("incident")));
        assert_eq!(lookup(&root, &parse_path("steps.post-it.output").unwrap()), Some(&json!("x")));
        assert_eq!(lookup(&root, &parse_path("trigger.payload.nope").unwrap()), None);
        assert!(parse_path("a..b").is_err());
        assert!(parse_path("a[x]").is_err());
        assert!(parse_path("a[1").is_err());
        assert!(parse_path("").is_err());
    }

    #[test]
    fn comparisons_and_logic() {
        let root =
            json!({"severity": 4, "text": "Checkout OUTAGE in the EU", "tags": ["a", "b"], "ok": true, "none": null});
        assert!(run("severity >= 3", &root));
        assert!(!run("severity > 4", &root));
        assert!(run("severity == 4 and ok", &root));
        assert!(run("severity < 3 or ok", &root));
        assert!(run("not (severity < 3)", &root));
        assert!(run("text contains \"outage\"", &root));
        assert!(run("tags contains 'b'", &root));
        assert!(!run("tags contains 'z'", &root));
        assert!(run("none == null", &root));
        assert!(run("missing == null", &root));
        assert!(!run("missing", &root));
        assert!(run("text != 'x'", &root));
        assert!(run("severity == 4.0", &root));
    }

    #[test]
    fn a_comparison_between_different_types_is_false_not_an_error() {
        let root = json!({"a": "10", "b": 9});
        assert!(!run("a > b", &root));
        assert!(!run("a < b", &root));
        assert!(!run("a == b", &root));
    }

    #[test]
    fn precedence_is_not_then_and_then_or() {
        let root = json!({"a": true, "b": false, "c": true});
        assert!(run("a or b and not c", &root));
        assert!(!run("(a or b) and not c", &root));
    }

    #[test]
    fn mistakes_are_explained() {
        for bad in ["", "a ==", "a and", "(a", "a)", "== 1", "a = 1", "a b", "\"open", "a ! b", "and", "1 + 2"] {
            assert!(Expr::parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn it_cannot_nest_forever() {
        let deep = format!("{}a{}", "(".repeat(200), ")".repeat(200));
        assert!(Expr::parse(&deep).is_err());
        let nots = format!("{}a", "not ".repeat(200));
        assert!(Expr::parse(&nots).is_err());
    }

    #[test]
    fn the_paths_an_expression_reads_are_listed() {
        let expr = Expr::parse("steps.a.output.x > 1 and trigger.payload.y").unwrap();
        assert_eq!(expr.paths().len(), 2);
    }
}
