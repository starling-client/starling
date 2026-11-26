//! The arithmetic of `#if` and `#elif` (GLSL 4.60 §3.3): `defined`, integer literals, macros
//! expanded to their values, and the operators of §3.3's table with its precedence. Float
//! literals count as numbers too (`#define AMOUNT 0.`): §3.3 takes integer expressions only, and
//! naga's preprocessor (pp-rs 0.2.1) refused them; this evaluator, moved here from
//! `sunrise-shaders`, reads them (#5533 step 3). An identifier that is not a macro counts as 0,
//! as in pp-rs. Parentheses and unary operators nest at most [`MAX_CONDITION_DEPTH`] deep.
//! [`Conditionals`] keeps the nesting of `#if` ... `#endif`; the caller evaluates
//! each condition, with its own rules for `#ifdef`.

use crate::macros::{Macros, is_ident, is_ident_start};

/// How deep parentheses and unary operators may nest in a condition: the evaluator reads each
/// level in frames of its own (thirteen per parenthesis, two per operator). A depth whose frames
/// fit in a 2 MiB thread (what Rust gives a spawned thread, such as the one a shader pack loads
/// on) in a debug build with half of it to spare, under the deepest includes that test a
/// condition: a 1 MiB thread held 134 bare parentheses with no bound, and
/// `deep_conditions_fit_a_small_thread` here, `conditions_nested_to_the_bound_fit_a_small_thread`
/// and `an_include_sixteen_deep_tests_a_condition_at_the_bound_on_a_small_thread` in
/// `directives_tests`, and `sunrise-shaders`' tests of its preprocessor and properties check it
/// on 1 MiB.
pub const MAX_CONDITION_DEPTH: usize = 64;

/// Why a condition nested deeper than [`MAX_CONDITION_DEPTH`] is refused.
pub fn too_deep() -> String {
    format!("parentheses and unary operators nest more than {MAX_CONDITION_DEPTH} deep")
}

/// The value of a condition; `Err` says what is wrong with it.
pub fn eval_condition(expr: &str, macros: &Macros) -> Result<i64, String> {
    let (tokens, real) = tokens(
        &resolve_defined(expr, |name| macros.is_defined(name))?,
        macros,
    )?;
    let mut parser = Parser::new(&tokens, real);
    let value = parser.binary(0)?;
    match parser.tokens.get(parser.at) {
        // A fraction is true, as any value but zero.
        None if value != 0.0 && value.abs() < 1.0 => Ok(value.signum() as i64),
        None => Ok(value as i64),
        Some(t) => Err(format!("unexpected `{}`", t.text())),
    }
}

/// The line number of a `#line` directive (`line` or `line source`, each an integer expression
/// over `macros`); `Err` says what is wrong with it.
pub fn eval_line(expr: &str, macros: &Macros) -> Result<i64, String> {
    let (tokens, real) = tokens(
        &resolve_defined(expr, |name| macros.is_defined(name))?,
        macros,
    )?;
    if real {
        return Err("a line number is an integer".into());
    }
    let mut parser = Parser::new(&tokens, real);
    let line = parser.binary(0)?;
    // The source string number, which names nothing here.
    if parser.at < parser.tokens.len() {
        parser.binary(0)?;
    }
    match parser.tokens.get(parser.at) {
        None => Ok(line as i64),
        Some(t) => Err(format!("unexpected `{}`", t.text())),
    }
}

/// One level of `#if` ... `#endif`.
struct Branch {
    /// This branch's lines are read.
    on: bool,
    /// A branch of this `#if` was already taken (the rest are skipped).
    taken: bool,
    /// The enclosing level is read.
    outer: bool,
    seen_else: bool,
}

/// The state of nested conditional directives.
#[derive(Default)]
pub struct Conditionals(Vec<Branch>);

impl Conditionals {
    /// Lines here are read.
    pub fn active(&self) -> bool {
        self.0.last().is_none_or(|b| b.on)
    }

    /// An `#if` is still open.
    pub fn open(&self) -> bool {
        !self.0.is_empty()
    }

    /// Track the active arm using the caller's expression evaluator. Skipped arms never
    /// evaluate their conditions.
    pub fn apply_with(
        &mut self,
        word: &str,
        mut evaluate: impl FnMut(&str) -> Result<bool, String>,
    ) -> Result<bool, String> {
        let on = self.active();
        match word {
            "if" | "ifdef" | "ifndef" => {
                let value = if on { evaluate(word) } else { Ok(false) };
                let v = *value.as_ref().unwrap_or(&false);
                self.0.push(Branch {
                    on: v,
                    taken: v,
                    outer: on,
                    seen_else: false,
                });
                value.map(|_| true)
            }
            "elif" | "else" => {
                let b = self
                    .0
                    .last_mut()
                    .ok_or_else(|| format!("`#{word}` without `#if`"))?;
                if b.seen_else {
                    return Err(format!("`#{word}` after `#else`"));
                }
                b.seen_else = word == "else";
                let take = b.outer && !b.taken;
                let value = match word {
                    "else" => Ok(take),
                    _ if take => evaluate("if"),
                    _ => Ok(false),
                };
                b.on = *value.as_ref().unwrap_or(&false);
                b.taken |= b.on;
                value.map(|_| true)
            }
            "endif" => self
                .0
                .pop()
                .map(|_| true)
                .ok_or_else(|| "`#endif` without `#if`".into()),
            _ => Ok(false),
        }
    }
}

/// `defined X` and `defined(X)` become `1` or `0` before anything is expanded.
pub fn resolve_defined(expr: &str, is_defined: impl Fn(&str) -> bool) -> Result<String, String> {
    let mut out = String::with_capacity(expr.len());
    let mut rest = expr;
    while let Some(at) = find_word(rest, "defined") {
        out.push_str(&rest[..at]);
        let after = rest[at + "defined".len()..].trim_start();
        let (inner, tail) = match after.strip_prefix('(') {
            Some(open) => {
                let close = open.find(')').ok_or("`defined(` without `)`")?;
                (open[..close].trim(), &open[close + 1..])
            }
            None => {
                let len = after.find(|c: char| !is_ident(c)).unwrap_or(after.len());
                (&after[..len], &after[len..])
            }
        };
        if inner.is_empty() || !inner.starts_with(is_ident_start) || !inner.chars().all(is_ident) {
            return Err("`defined` needs a macro name".into());
        }
        out.push_str(if is_defined(inner) { " 1 " } else { " 0 " });
        rest = tail;
    }
    out.push_str(rest);
    Ok(out)
}

/// Where `word` starts in `text` as a whole identifier.
fn find_word(text: &str, word: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = text[from..].find(word).map(|i| i + from) {
        let before = text[..i].chars().next_back().is_some_and(is_ident);
        let after = text[i + word.len()..].chars().next().is_some_and(is_ident);
        if !before && !after {
            return Some(i);
        }
        from = i + word.len();
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Num(f64),
    Op(&'static str),
}

impl Token {
    fn text(&self) -> String {
        match self {
            Token::Num(n) => n.to_string(),
            Token::Op(op) => (*op).to_owned(),
        }
    }
}

const OPS: [&str; 22] = [
    "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "(", ")", "!", "~", "+", "-", "*", "/", "%",
    "<", ">", "&", "^", "|",
];

fn tokens(expr: &str, macros: &Macros) -> Result<(Vec<Token>, bool), String> {
    let expanded = macros.expand_condition(expr)?;
    let mut out = Vec::new();
    let mut real = false;
    let mut rest = expanded.trim_start();
    while let Some(c) = rest.chars().next() {
        let fraction = c == '.' && rest[1..].starts_with(|c: char| c.is_ascii_digit());
        if c.is_ascii_digit() || fraction {
            let hex = rest.starts_with("0x") || rest.starts_with("0X");
            let len = rest
                .char_indices()
                .find(|&(i, c)| {
                    !(c.is_ascii_alphanumeric()
                        || c == '.'
                        || !hex
                            && matches!(c, '+' | '-')
                            && i > 0
                            && matches!(rest.as_bytes()[i - 1], b'e' | b'E'))
                })
                .map_or(rest.len(), |(i, _)| i);
            let text = &rest[..len];
            if !hex && (text.contains('.') || text.contains(['e', 'E'])) {
                real = true;
                let text = text.trim_end_matches(['f', 'F']);
                let n = text
                    .parse::<f64>()
                    .map_err(|_| format!("`{text}` is not a number"))?;
                out.push(Token::Num(n));
                rest = &rest[len..];
            } else {
                out.push(Token::Num(int_literal(&rest[..len])? as f64));
                rest = &rest[len..];
            }
        } else if is_ident_start(c) {
            // Not a macro: 0, as the C preprocessor counts it.
            let len = rest.find(|c: char| !is_ident(c)).unwrap_or(rest.len());
            out.push(Token::Num(0.0));
            rest = &rest[len..];
        } else {
            let op = OPS
                .iter()
                .find(|op| rest.starts_with(**op))
                .ok_or_else(|| format!("unexpected `{c}`"))?;
            out.push(Token::Op(op));
            rest = &rest[op.len()..];
        }
        rest = rest.trim_start();
    }
    Ok((out, real))
}

/// Decimal, `0x` hexadecimal or leading-zero octal, with an optional `u` suffix.
pub fn int_literal(text: &str) -> Result<i64, String> {
    let digits = text.strip_suffix(['u', 'U']).unwrap_or(text);
    let parsed = if let Some(hex) = digits.strip_prefix("0x").or(digits.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16)
    } else if digits.len() > 1 && digits.starts_with('0') {
        i64::from_str_radix(&digits[1..], 8)
    } else {
        digits.parse()
    };
    parsed.map_err(|_| format!("`{text}` is not an integer"))
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    /// A float literal appears: `/` and `%` divide as floats.
    real: bool,
    /// Inside a side that `&&` / `||` skips: a division by zero is not an error.
    quiet: bool,
    /// How many parentheses and unary operators are open ([`MAX_CONDITION_DEPTH`]).
    depth: usize,
}

/// Binary operators from the loosest to the tightest (GLSL §5.1 / §3.3).
const LEVELS: [&[&str]; 10] = [
    &["||"],
    &["&&"],
    &["|"],
    &["^"],
    &["&"],
    &["==", "!="],
    &["<", ">", "<=", ">="],
    &["<<", ">>"],
    &["+", "-"],
    &["*", "/", "%"],
];

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token], real: bool) -> Self {
        Self {
            tokens,
            at: 0,
            real,
            quiet: false,
            depth: 0,
        }
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.at).copied();
        self.at += 1;
        t
    }

    fn binary(&mut self, level: usize) -> Result<f64, String> {
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(&Token::Op(op)) = self.tokens.get(self.at) {
            if !LEVELS[level].contains(&op) {
                break;
            }
            self.at += 1;
            // C does not evaluate the side `&&` / `||` skips, so a guarded division by zero
            // there is no error.
            let skipped = (op == "||" && left != 0.0) || (op == "&&" && left == 0.0);
            let outer = self.quiet;
            self.quiet |= skipped;
            let right = self.binary(level + 1);
            self.quiet = outer;
            left = apply(op, left, right?, self.real, skipped || outer)?;
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<f64, String> {
        let token = self.next();
        let opens = matches!(token, Some(Token::Op("(" | "-" | "+" | "!" | "~")));
        if opens {
            // Refused before its frames are taken.
            if self.depth == MAX_CONDITION_DEPTH {
                return Err(too_deep());
            }
            self.depth += 1;
        }
        let value = self.operand(token);
        if opens {
            self.depth -= 1;
        }
        value
    }

    /// The operand `token` starts, after it; one level deeper when it opens a level.
    fn operand(&mut self, token: Option<Token>) -> Result<f64, String> {
        match token {
            Some(Token::Num(n)) => Ok(n),
            Some(Token::Op("(")) => {
                let value = self.binary(0)?;
                match self.next() {
                    Some(Token::Op(")")) => Ok(value),
                    _ => Err("missing `)`".into()),
                }
            }
            Some(Token::Op("-")) => Ok(-self.unary()?),
            Some(Token::Op("+")) => self.unary(),
            Some(Token::Op("!")) => Ok(f64::from(u8::from(self.unary()? == 0.0))),
            Some(Token::Op("~")) => Ok(!(self.unary()? as i64) as f64),
            Some(t) => Err(format!("unexpected `{}`", t.text())),
            None => Err("the condition ends too early".into()),
        }
    }
}

fn apply(op: &str, a: f64, b: f64, real: bool, quiet: bool) -> Result<f64, String> {
    let truth = |v: bool| f64::from(u8::from(v));
    let (i, j) = (a as i64, b as i64);
    Ok(match op {
        "||" => truth(a != 0.0 || b != 0.0),
        "&&" => truth(a != 0.0 && b != 0.0),
        "|" => (i | j) as f64,
        "^" => (i ^ j) as f64,
        "&" => (i & j) as f64,
        "==" => truth(a == b),
        "!=" => truth(a != b),
        "<" => truth(a < b),
        ">" => truth(a > b),
        "<=" => truth(a <= b),
        ">=" => truth(a >= b),
        "<<" => i.wrapping_shl(j as u32) as f64,
        ">>" => i.wrapping_shr(j as u32) as f64,
        "+" => a + b,
        "-" => a - b,
        "*" => a * b,
        "/" | "%" if b == 0.0 && quiet => 0.0,
        "/" | "%" if b == 0.0 => return Err("division by zero".into()),
        "/" if real => a / b,
        "%" if real => a % b,
        "/" => i.wrapping_div(j) as f64,
        _ => i.wrapping_rem(j) as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expr: &str) -> Result<i64, String> {
        let mut m = Macros::default();
        m.define("SHADOW_RES", "2048");
        m.define("WAVES", "");
        m.define("QUALITY", "SHADOW_RES / 1024");
        eval_condition(expr, &m)
    }

    #[test]
    fn precedence_and_operators_follow_c() {
        assert_eq!(eval("1 + 2 * 3"), Ok(7));
        assert_eq!(eval("(1 + 2) * 3"), Ok(9));
        assert_eq!(eval("1 << 2 + 1"), Ok(8));
        assert_eq!(eval("6 & 3 == 3"), Ok(0));
        assert_eq!(eval("1 || 0 && 0"), Ok(1));
        assert_eq!(eval("-7 / 2"), Ok(-3));
        assert_eq!(eval("-7 % 2"), Ok(-1));
        assert_eq!(eval("!0 + ~0"), Ok(0));
        assert_eq!(eval("0x10 + 010 + 3u"), Ok(27));
    }

    #[test]
    fn macros_and_defined() {
        assert_eq!(eval("QUALITY == 2"), Ok(1));
        assert_eq!(eval("defined WAVES && defined(SHADOW_RES)"), Ok(1));
        assert_eq!(eval("!defined NOPE"), Ok(1));
        assert_eq!(eval("NOPE == 0"), Ok(1));
        assert_eq!(eval("undefinedFeature"), Ok(0));
    }

    /// Float values count (`#if AMOUNT != 0` with `#define AMOUNT 0.0`).
    #[test]
    fn float_values_count() {
        assert_eq!(eval("0. != 0"), Ok(0));
        assert_eq!(eval("4. != 0"), Ok(1));
        assert_eq!(eval("0.7 > 0.5"), Ok(1));
        assert_eq!(eval(".25 * 4.0 == 1"), Ok(1));
        assert_eq!(eval("1.5f"), Ok(1));
        assert_eq!(eval("7 / 2"), Ok(3));
        assert_eq!(eval("SHADOW_RES == 2048.0"), Ok(1));
    }

    #[test]
    fn scientific_literals_and_macro_values_count() {
        for expr in [
            "1e-3 > 0",
            "1.0e-3 == 0.001",
            "1.0e+3 == 1000",
            ".25E2 == 25",
            "1E3 / 4e2 == 2.5",
            "1e0 / 2 > 0",
            "1e-3+1e-3 == 2e-3",
            "1e+3-1 == 999",
            "0x1E-1 == 29",
            "0xEE+010 == 246",
        ] {
            assert_eq!(eval(expr), Ok(1), "{expr}");
        }
        let mut macros = Macros::default();
        macros.define("TINY", "1e-3");
        macros.define("WHOLE_REAL", "1e0");
        assert_eq!(eval_condition("TINY > 0", &macros), Ok(1));
        assert_eq!(eval_condition("WHOLE_REAL / 2 > 0", &macros), Ok(1));
        for expr in ["1e", "1e+", "1e--3"] {
            assert!(eval(expr).is_err(), "{expr}");
        }
    }

    #[test]
    fn short_circuit_skips_guarded_division() {
        let mut m = Macros::default();
        m.define("AMOUNT", "0");
        assert_eq!(eval_condition("AMOUNT != 0 && 100 / AMOUNT > 5", &m), Ok(0));
        assert_eq!(eval_condition("AMOUNT == 0 || 100 / AMOUNT > 5", &m), Ok(1));
        assert_eq!(eval_condition("AMOUNT && (1 % AMOUNT)", &m), Ok(0));
        // The unskipped side still errors.
        assert!(eval_condition("AMOUNT == 0 && 100 / AMOUNT > 5", &m).is_err());
        assert!(eval_condition("AMOUNT != 0 || 100 / AMOUNT > 5", &m).is_err());
    }

    #[test]
    fn bad_conditions_say_why() {
        assert!(eval("1 / 0").is_err());
        assert!(eval("SHADOW_RES == 0.5 0").is_err());
        assert!(eval("(1 + 2").is_err());
        assert!(eval("1 2").is_err());
        assert!(eval("").is_err());
        assert!(eval("defined").is_err());
        assert!(eval("1 @ 2").is_err());
    }

    /// `run` on a thread of 1 MiB: half the 2 MiB Rust gives a spawned thread.
    fn on_small_thread<T: Send + 'static>(run: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(run)
            .unwrap()
            .join()
            .unwrap()
    }

    /// Parentheses and unary operators nested to `MAX_CONDITION_DEPTH` evaluate on a thread of
    /// 1 MiB in this (debug) build; one level more is refused by name, whatever opens it.
    #[test]
    fn deep_conditions_fit_a_small_thread() {
        let at = MAX_CONDITION_DEPTH;
        let parens = |n: usize| format!("{}7{}", "(".repeat(n), ")".repeat(n));
        // `-(` pairs: `-` and `(` each open a level, an even count of `-` keeps the value.
        let negated = |pairs: usize| format!("{}7{}", "-(".repeat(pairs), ")".repeat(pairs));
        let cases = [
            (parens(at), Ok(7)),
            (parens(at + 1), Err(too_deep())),
            (format!("{}5", "~".repeat(at)), Ok(5)),
            (format!("{}5", "~".repeat(at + 1)), Err(too_deep())),
            (format!("{}3", "!".repeat(at)), Ok(1)),
            (format!("{}3", "!".repeat(at + 1)), Err(too_deep())),
            (format!("{}6", "+".repeat(at)), Ok(6)),
            (format!("{}6", "-".repeat(at + 1)), Err(too_deep())),
            (negated(at / 2), Ok(7)),
            (format!("-{}", negated(at / 2)), Err(too_deep())),
            // Refused on the side `&&` skips too: the depth is the text's, not the value's.
            (format!("0 && {}", parens(at + 1)), Err(too_deep())),
        ];
        let got = on_small_thread(move || {
            cases
                .into_iter()
                .map(|(expr, want)| (eval(&expr), want))
                .collect::<Vec<_>>()
        });
        for (n, (got, want)) in got.into_iter().enumerate() {
            assert_eq!(got, want, "case {n}");
        }
    }

    /// The depth is how many levels are open at once: a long condition of shallow parts is
    /// read whole.
    #[test]
    fn long_shallow_conditions_are_not_deep() {
        let many = 4 * MAX_CONDITION_DEPTH;
        let sums = vec!["(1)"; many].join(" + ");
        assert_eq!(eval(&format!("{sums} == {many}")), Ok(1));
        let tests = vec!["!defined NOPE"; many].join(" && ");
        assert_eq!(eval(&tests), Ok(1));
        let unary = vec!["-(-2)"; many].join(" + ");
        assert_eq!(eval(&format!("{unary} == {}", 2 * many)), Ok(1));
    }

    /// Parentheses a macro's replacement makes count as written ones; so does `#line`'s second
    /// number.
    #[test]
    fn parentheses_from_macros_and_lines_count() {
        let mut m = Macros::default();
        m.define_line("P(x) (x)").unwrap();
        let wrapped = |n: usize| format!("{}9{}", "P(".repeat(n), ")".repeat(n));
        let at = MAX_CONDITION_DEPTH;
        let parens = |n: usize| format!("{}4{}", "(".repeat(n), ")".repeat(n));
        let (deepest, past, line, line_past) = on_small_thread(move || {
            (
                eval_condition(&wrapped(at), &m),
                eval_condition(&wrapped(at + 1), &m),
                eval_line(&format!("{} {}", parens(at), parens(at)), &m),
                eval_line(&format!("{} {}", parens(at), parens(at + 1)), &m),
            )
        });
        assert_eq!(deepest, Ok(9));
        assert_eq!(past, Err(too_deep()));
        assert_eq!(line, Ok(4));
        assert_eq!(line_past, Err(too_deep()));
    }

    /// Calls nested in a condition stop at `MAX_ARGUMENT_DEPTH` in the shader packs' mode too,
    /// before the parser: 400 levels, which overflowed a 1 MiB thread, are refused by name.
    #[test]
    fn calls_nested_in_a_condition_stop_at_the_depth() {
        let nested = |depth: usize| format!("{}5{}", "F(".repeat(depth), ")".repeat(depth));
        let (deepest, past) = on_small_thread(move || {
            let mut m = Macros::default();
            m.define_line("F(x) x").unwrap();
            (
                eval_condition(&nested(crate::macros::MAX_ARGUMENT_DEPTH), &m),
                eval_condition(&nested(400), &m),
            )
        });
        assert_eq!(deepest, Ok(5));
        assert_eq!(
            past,
            Err(crate::directives::stopped(crate::macros::Stop::Depth))
        );
    }
}
