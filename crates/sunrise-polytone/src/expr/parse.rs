//! Parses tokens into a tree whose names are already resolved: locals to slots, functions to
//! the math table or the host, roots (`o`, `player`...) to [`Root`]s. An unknown name is an
//! error here, when the pack loads, not when the expression runs.
use super::lex::{Token, tokens};
use super::{MATH, MathFn, Root, Value, legacy};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug)]
pub(super) enum Expr {
    Const(Value),
    Local(usize),
    Root(Root),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    /// A math or color function.
    Call(MathFn, Vec<Expr>),
    /// A global function the game answers (`config`, `colormap`...).
    Host(Arc<str>, Vec<Expr>),
    /// `target.name(args)`; `p.age`, `p.age()` and `p.getAge()` all name `age`.
    Method(Box<Expr>, Arc<str>, Vec<Expr>),
}

#[derive(Debug)]
pub(super) enum Stmt {
    Expr(Expr),
    /// `slot = value`, or `slot op= value`.
    Assign(usize, Option<BinOp>, Expr),
    If(Vec<(Expr, Vec<Stmt>)>, Vec<Stmt>),
}

/// Global functions the game provides rather than this crate.
const HOST_FUNCTIONS: [&str; 11] = [
    "colormap",
    "config",
    "modOn",
    "modLoader",
    "dateYear",
    "dateMonth",
    "dateDay",
    "dateHour",
    "dateMinute",
    "dateTime",
    "dateDayOfYear",
];

struct Parser {
    tokens: Vec<(Token, usize)>,
    pos: usize,
    locals: Vec<String>,
    /// A legacy math expression: `lerp`, `smoothstep` and `color` take their old argument order.
    legacy: bool,
    /// How deep the parser has recursed (`expr` and `statement` calls now open).
    nesting: usize,
}

/// A tree being built and its height: 1 for a leaf, one more than its highest child otherwise.
type Built = (Expr, usize);

/// The statements and how many local slots they use.
pub(super) fn parse(src: &str) -> Result<(Vec<Stmt>, usize), String> {
    let tokens = tokens(src)?;
    super::budget::check_tokens(&tokens).map_err(|e| e.to_string())?;
    let mut p = Parser {
        legacy: legacy::is_legacy(&tokens),
        tokens,
        pos: 0,
        locals: Vec::new(),
        nesting: 0,
    };
    let body = p.statements(false)?;
    Ok((body, p.locals.len()))
}

fn binary(op: &str) -> Option<(BinOp, u8)> {
    Some(match op {
        "==" => (BinOp::Eq, 4),
        "!=" => (BinOp::Ne, 4),
        "<" => (BinOp::Lt, 5),
        "<=" => (BinOp::Le, 5),
        ">" => (BinOp::Gt, 5),
        ">=" => (BinOp::Ge, 5),
        "+" => (BinOp::Add, 6),
        "-" => (BinOp::Sub, 6),
        "*" => (BinOp::Mul, 7),
        "/" => (BinOp::Div, 7),
        "%" => (BinOp::Rem, 7),
        _ => return None,
    })
}

/// `getAge` and `age` are the same method.
fn method_name(name: &str) -> String {
    match name.strip_prefix("get") {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_uppercase()) => {
            let mut chars = rest.chars();
            let first = chars.next().unwrap_or_default().to_ascii_lowercase();
            std::iter::once(first).chain(chars).collect()
        }
        _ => name.to_owned(),
    }
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|t| &t.0)
    }
    fn at(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Token::Punct(q)) if *q == p)
    }
    fn eat(&mut self, p: &str) -> bool {
        let found = self.at(p);
        self.pos += usize::from(found);
        found
    }
    fn error(&self, what: &str) -> String {
        match self.tokens.get(self.pos) {
            Some((t, at)) => format!("{what}, found {t:?} at {at}"),
            None => format!("{what} at the end"),
        }
    }
    fn expect(&mut self, p: &str) -> Result<(), String> {
        if self.eat(p) {
            Ok(())
        } else {
            Err(self.error(&format!("expected {p:?}")))
        }
    }

    /// One more level of the parser's recursion, refused past [`MAX_NESTING`].
    ///
    /// [`MAX_NESTING`]: super::budget::MAX_NESTING
    fn enter(&mut self) -> Result<(), String> {
        self.nesting += 1;
        if self.nesting > super::budget::MAX_NESTING {
            return Err(super::budget::OverBudget::Nesting.to_string());
        }
        Ok(())
    }

    /// `e`, whose highest child is `children` high: refused past [`MAX_HEIGHT`], so no tree the
    /// walks recurse into is ever built higher.
    ///
    /// [`MAX_HEIGHT`]: super::budget::MAX_HEIGHT
    fn node(e: Expr, children: usize) -> Result<Built, String> {
        let height = children + 1;
        if height > super::budget::MAX_HEIGHT {
            return Err(super::budget::OverBudget::Height.to_string());
        }
        Ok((e, height))
    }

    fn statements(&mut self, in_block: bool) -> Result<Vec<Stmt>, String> {
        let mut out = Vec::new();
        loop {
            while self.eat(";") {}
            match self.peek() {
                None if !in_block => return Ok(out),
                Some(Token::Punct("}")) if in_block => return Ok(out),
                None => return Err(self.error("expected \"}\"")),
                _ => out.push(self.statement()?),
            }
        }
    }

    fn block(&mut self) -> Result<Vec<Stmt>, String> {
        self.expect("{")?;
        let body = self.statements(true)?;
        self.expect("}")?;
        Ok(body)
    }

    fn statement(&mut self) -> Result<Stmt, String> {
        self.enter()?;
        let statement = self.statement_inner();
        self.nesting -= 1;
        statement
    }

    fn statement_inner(&mut self) -> Result<Stmt, String> {
        if matches!(self.peek(), Some(Token::Ident(s)) if s == "if") {
            let mut branches = Vec::new();
            let mut otherwise = Vec::new();
            loop {
                self.pos += 1;
                self.expect("(")?;
                let condition = self.expr(0)?.0;
                self.expect(")")?;
                branches.push((condition, self.block()?));
                if !matches!(self.peek(), Some(Token::Ident(s)) if s == "else") {
                    break;
                }
                self.pos += 1;
                if !matches!(self.peek(), Some(Token::Ident(s)) if s == "if") {
                    otherwise = self.block()?;
                    break;
                }
            }
            return Ok(Stmt::If(branches, otherwise));
        }
        if let (Some(Token::Ident(name)), Some((Token::Punct(op), _))) =
            (self.peek(), self.tokens.get(self.pos + 1))
        {
            let op = match *op {
                "=" => Some(None),
                "+=" => Some(Some(BinOp::Add)),
                "-=" => Some(Some(BinOp::Sub)),
                "*=" => Some(Some(BinOp::Mul)),
                "/=" => Some(Some(BinOp::Div)),
                _ => None,
            };
            if let Some(op) = op {
                let name = name.clone();
                self.pos += 2;
                let value = self.expr(0)?.0;
                let slot = match self.locals.iter().position(|l| *l == name) {
                    Some(slot) => slot,
                    None if op.is_none() => {
                        self.locals.push(name);
                        self.locals.len() - 1
                    }
                    None => return Err(format!("{name} is updated before it is set")),
                };
                return Ok(Stmt::Assign(slot, op, value));
            }
        }
        Ok(Stmt::Expr(self.expr(0)?.0))
    }

    /// Precedence climbing: ternary 1, `||` 2, `&&` 3, equality 4, comparison 5, `+ -` 6,
    /// `* / %` 7, unary 8, `^` 9 (right to left, so `-2^2` is -4). Each node's height is known
    /// as it is built.
    fn expr(&mut self, min: u8) -> Result<Built, String> {
        self.enter()?;
        let e = self.expr_inner(min);
        self.nesting -= 1;
        e
    }

    fn expr_inner(&mut self, min: u8) -> Result<Built, String> {
        let (mut left, mut height) = self.unary()?;
        loop {
            let Some(Token::Punct(op)) = self.peek().cloned() else {
                return Ok((left, height));
            };
            (left, height) = if op == "?" && min <= 1 {
                self.pos += 1;
                let (then, a) = self.expr(1)?;
                self.expect(":")?;
                let (otherwise, b) = self.expr(1)?;
                let e = Expr::Cond(Box::new(left), Box::new(then), Box::new(otherwise));
                Self::node(e, height.max(a).max(b))?
            } else if op == "||" && min <= 2 {
                self.pos += 1;
                let (right, h) = self.expr(3)?;
                Self::node(Expr::Or(Box::new(left), Box::new(right)), height.max(h))?
            } else if op == "&&" && min <= 3 {
                self.pos += 1;
                let (right, h) = self.expr(4)?;
                Self::node(Expr::And(Box::new(left), Box::new(right)), height.max(h))?
            } else if let Some((bin, prec)) = binary(op).filter(|b| b.1 >= min) {
                self.pos += 1;
                let (right, h) = self.expr(prec + 1)?;
                Self::node(
                    Expr::Binary(bin, Box::new(left), Box::new(right)),
                    height.max(h),
                )?
            } else {
                return Ok((left, height));
            };
        }
    }

    fn unary(&mut self) -> Result<Built, String> {
        if self.eat("-") {
            self.enter()?;
            let (operand, h) = self.unary()?;
            self.nesting -= 1;
            return Self::node(Expr::Neg(Box::new(operand)), h);
        }
        if self.eat("!") {
            self.enter()?;
            let (operand, h) = self.unary()?;
            self.nesting -= 1;
            return Self::node(Expr::Not(Box::new(operand)), h);
        }
        self.eat("+");
        let (base, height) = self.postfix()?;
        if self.eat("^") {
            self.enter()?;
            let (exponent, h) = self.unary()?;
            self.nesting -= 1;
            let e = Expr::Binary(BinOp::Pow, Box::new(base), Box::new(exponent));
            return Self::node(e, height.max(h));
        }
        Ok((base, height))
    }

    /// The arguments, and the highest of them (0 for none).
    fn arguments(&mut self) -> Result<(Vec<Expr>, usize), String> {
        let mut args = Vec::new();
        let mut highest = 0;
        if self.eat(")") {
            return Ok((args, highest));
        }
        loop {
            let (arg, h) = self.expr(0)?;
            args.push(arg);
            highest = highest.max(h);
            if self.eat(")") {
                return Ok((args, highest));
            }
            self.expect(",")?;
        }
    }

    fn postfix(&mut self) -> Result<Built, String> {
        let (mut e, mut height) = self.primary()?;
        while self.eat(".") {
            let Some(Token::Ident(name)) = self.peek().cloned() else {
                return Err(self.error("expected a method name"));
            };
            self.pos += 1;
            let (args, h) = if self.eat("(") {
                self.arguments()?
            } else {
                (Vec::new(), 0)
            };
            let method = Expr::Method(Box::new(e), method_name(&name).into(), args);
            (e, height) = Self::node(method, height.max(h))?;
        }
        Ok((e, height))
    }

    fn primary(&mut self) -> Result<Built, String> {
        let token = self
            .peek()
            .cloned()
            .ok_or_else(|| self.error("expected a value"))?;
        self.pos += 1;
        let leaf = |e: Expr| Ok((e, 1));
        match token {
            Token::Num(n) => leaf(Expr::Const(Value::Num(n))),
            Token::Str(s) => leaf(Expr::Const(Value::Str(s.into()))),
            Token::Punct("(") => {
                let e = self.expr(0)?;
                self.expect(")")?;
                Ok(e)
            }
            Token::Ident(name) if self.eat("(") => {
                let (args, highest) = self.arguments()?;
                let legacy = legacy::FUNCTIONS.iter().filter(|_| self.legacy);
                if let Some(&(_, fewest, most, f)) =
                    legacy.chain(MATH).find(|entry| entry.0 == name)
                {
                    if !(fewest..=most).contains(&args.len()) {
                        let takes = if fewest == most {
                            fewest.to_string()
                        } else {
                            format!("{fewest} to {most}")
                        };
                        return Err(format!(
                            "{name} takes {takes} arguments, not {}",
                            args.len()
                        ));
                    }
                    Self::node(Expr::Call(f, args), highest)
                } else if HOST_FUNCTIONS.contains(&name.as_str()) {
                    Self::node(Expr::Host(name.into(), args), highest)
                } else if let Some(draw) = legacy::random(&name, args) {
                    // `Random.name(args)`: the root under it, a level above its arguments.
                    Self::node(draw, highest.max(1))
                } else {
                    Err(format!("unknown function {name}"))
                }
            }
            Token::Ident(name) => match name.as_str() {
                "true" => leaf(Expr::Const(Value::Bool(true))),
                "false" => leaf(Expr::Const(Value::Bool(false))),
                "PI" => leaf(Expr::Const(Value::Num(std::f64::consts::PI))),
                "TAU" => leaf(Expr::Const(Value::Num(std::f64::consts::TAU))),
                "E" => leaf(Expr::Const(Value::Num(std::f64::consts::E))),
                "PSI" => leaf(Expr::Const(Value::Num(std::f64::consts::GOLDEN_RATIO))),
                _ => match (
                    Root::parse(&name),
                    self.locals.iter().position(|l| *l == name),
                ) {
                    (_, Some(slot)) => leaf(Expr::Local(slot)),
                    (Some(root), None) => leaf(Expr::Root(root)),
                    // A legacy variable's tree is a fixed one, at most six high (`SPEED`).
                    (None, None) => match legacy::variable(&name) {
                        Some(e) => Ok((e?, legacy::VARIABLE_HEIGHT)),
                        None => Err(format!("unknown name {name}")),
                    },
                },
            },
            Token::Punct(_) => {
                self.pos -= 1;
                Err(self.error("expected a value"))
            }
        }
    }
}
