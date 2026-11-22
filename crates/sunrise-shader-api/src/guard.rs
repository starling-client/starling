//! Keeps a mod's or a pack's shader from crashing the game. naga's WGSL parser, validator and
//! backends recurse over the shape of the source, so an expression chain of ten thousand terms
//! (`a + a + a + ...`) overflows a thread's stack, and a stack overflow aborts the process: a
//! shader file (a resource pack's, possibly pushed by a server) could take the game or the
//! check tool down.
//!
//! Three bounds, which together hold for any input:
//! - [`MAX_SOURCE_BYTES`]: a shader longer than that is refused before it is parsed. naga's
//!   other recursion (a chain of structs, aliases or calls) grows with the length of the
//!   source, and a shader this long is far inside what a thread's stack holds.
//! - [`MAX_CHAINED_OPERATORS`]: naga limits the nesting of parentheses, blocks and unary
//!   operators itself, but not a chain of binary operators, member accesses or indexings,
//!   which it lowers by recursion as deep as the chain is long. [`refusal`] counts them in
//!   the text, before it is parsed. This is the bound that makes a shader which passes safe
//!   to parse again on any thread, as wgpu does when it builds the pipeline.
//! - [`guarded`]: the parse, validation and translation run on a thread with a large stack,
//!   so a shape the two bounds above did not foresee still does not reach the limit of the
//!   caller's own stack, and a panic comes back as a refusal.

/// The most bytes of WGSL a shader (the prelude, the files and the entry points as composed)
/// may hold.
pub const MAX_SOURCE_BYTES: usize = 128 * 1024;

/// The most binary operators, member accesses and indexings one expression may chain. The
/// count is generous for written code (a polynomial of 90 terms has about 180) and the deepest
/// parse it allows takes about a megabyte of a debug build's stack.
pub const MAX_CHAINED_OPERATORS: usize = 200;

/// The stack of the thread a shader is checked on. It is address space the system commits as
/// it is used.
const STACK_BYTES: usize = 64 << 20;

/// Why `text` may not be parsed, and the byte offset it is found at when it has one: it is too
/// long, or an expression chains more than [`MAX_CHAINED_OPERATORS`] operators.
pub fn refusal(text: &str) -> Option<(Option<usize>, String)> {
    if text.len() > MAX_SOURCE_BYTES {
        return Some((
            None,
            format!(
                "{} bytes of shader; a shader may hold {MAX_SOURCE_BYTES}",
                text.len()
            ),
        ));
    }
    let (at, count) = longest_chain(text)?;
    Some((
        Some(at),
        format!(
            "an expression chains more than {MAX_CHAINED_OPERATORS} operators ({count} here); \
             split it with `let`"
        ),
    ))
}

/// Where the first expression of `text` that chains more than [`MAX_CHAINED_OPERATORS`] binary
/// operators, member accesses or indexings gets past them (its byte offset) and how many it
/// chains there. An expression is what lies between `;`, `{` and `}`, which no expression
/// contains. Comments and numbers are read as naga reads them, so that nothing naga takes for
/// an operator is hidden from the count; what it counts beyond them (such as the angle
/// brackets of a template) only makes the bound stricter.
fn longest_chain(text: &str) -> Option<(usize, usize)> {
    let mut chars = text.char_indices().peekable();
    let (mut count, mut after_operand) = (0, false);
    while let Some((at, c)) = chars.next() {
        match c {
            '/' if chars.peek().is_some_and(|(_, n)| *n == '/') => {
                while chars.next_if(|(_, n)| !is_line_end(*n)).is_some() {}
                continue;
            }
            '/' if chars.peek().is_some_and(|(_, n)| *n == '*') => {
                // naga's own scan: the character after `/*` only starts what follows it, so
                // `/**/` ends at once and `/*/*` opens a nested comment.
                chars.next();
                let (mut depth, mut prev) = (1, chars.next().map(|(_, c)| c));
                for (_, c) in &mut chars {
                    match (prev, c) {
                        (Some('*'), '/') => {
                            prev = None;
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        (Some('/'), '*') => {
                            prev = None;
                            depth += 1;
                        }
                        _ => prev = Some(c),
                    }
                }
                continue;
            }
            ';' | '{' | '}' => {
                count = 0;
                after_operand = false;
            }
            c if is_blank(c) => {}
            c if c.is_ascii_digit() => {
                number(c, &mut chars);
                after_operand = true;
            }
            c if c.is_alphanumeric() || c == '_' || !c.is_ascii() => {
                while chars
                    .next_if(|(_, n)| n.is_alphanumeric() || *n == '_' || !n.is_ascii())
                    .is_some()
                {}
                after_operand = true;
            }
            ')' | ']' => after_operand = true,
            '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '<' | '>' | '=' | '!' | '.' | '['
                if after_operand =>
            {
                count += 1;
                if count == MAX_CHAINED_OPERATORS + 1 {
                    return Some((at, count));
                }
                after_operand = false;
            }
            _ => after_operand = false,
        }
    }
    None
}

/// The rest of a number whose first digit, `first`, was read, tokenized by its radix as naga
/// does, so that no operator is taken into it: a decimal number's digits, point and fraction,
/// then an exponent after `e`/`E` whose sign belongs to it (`1.5e-3`); a hexadecimal number
/// (`0x`), whose `e` is a digit, with its exponent only after `p`/`P` (`0x1p+4`). After the
/// exponent's sign only digits and a suffix follow, so `0x1e+0x1e` is two numbers and an
/// operator.
fn number(first: char, chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) {
    let hex = first == '0' && chars.next_if(|&(_, c)| matches!(c, 'x' | 'X')).is_some();
    let digit = |c: char| {
        if hex {
            c.is_ascii_hexdigit()
        } else {
            c.is_ascii_digit()
        }
    };
    while chars.next_if(|&(_, c)| digit(c) || c == '.').is_some() {}
    let exponent = if hex { ['p', 'P'] } else { ['e', 'E'] };
    if chars.next_if(|&(_, c)| exponent.contains(&c)).is_some() {
        chars.next_if(|&(_, c)| matches!(c, '+' | '-'));
    }
    // The exponent's digits and a suffix (`f`, `h`, `i`, `u`), or the rest of a malformed
    // number, which naga refuses: never a sign.
    while chars
        .next_if(|&(_, c)| c.is_alphanumeric() || c == '_')
        .is_some()
    {}
}

/// What ends a `//` comment in WGSL.
fn is_line_end(c: char) -> bool {
    matches!(c, '\u{a}'..='\u{d}' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// WGSL's blank space.
fn is_blank(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t'..='\r' | '\u{85}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}'
    )
}

/// Runs `work` on a thread with a large stack and gives its result, or why it could not: the
/// thread would not start, or `work` panicked.
pub fn guarded<T: Send>(work: impl FnOnce() -> T + Send) -> Result<T, String> {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .name("shader-check".to_owned())
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, work)
            .map_err(|e| format!("the shader check could not start: {e}"))?;
        thread.join().map_err(|panic| {
            format!(
                "the shader check panicked: {}",
                panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or(panic.downcast_ref::<&str>().copied())
                    .unwrap_or("no reason given")
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(terms: usize) -> String {
        format!("fn f(x: f32) -> f32 {{ return x{}; }}", "+x".repeat(terms))
    }

    /// The count is of the chain, not of the file: the longest chain that passes has exactly
    /// the cap's operators, and a statement ends the count.
    #[test]
    fn the_cap_is_on_one_expression_exactly() {
        assert_eq!(refusal(&chain(MAX_CHAINED_OPERATORS)), None);
        let (at, message) = refusal(&chain(MAX_CHAINED_OPERATORS + 1)).unwrap();
        assert!(message.contains("201 here"), "{message}");
        // The 201st `+`: after `fn f(x: f32) -> f32 { return x` (30 bytes) and 200 `+x`.
        assert_eq!(at, Some(30 + 2 * 200));
        // Many statements of a chain each are many expressions.
        let statements = format!("var a = 1.0; {}", "a = a + a * a - a; ".repeat(5_000));
        assert_eq!(refusal(&statements), None);
    }

    /// A hexadecimal number's `e` is a digit, not an exponent: an unspaced chain of hex terms
    /// is counted term by term, so it cannot pass as one long number.
    #[test]
    fn hex_numbers_cannot_hide_a_chain() {
        for (term, first) in [
            ("+0x1e", "0x1e"),
            ("-0x1E", "0x1E"),
            ("+0xe", "0xe"),
            ("+1e+1", "1e+1"),
        ] {
            // The `=` is one of the chain's operators, as the other tests count it.
            let refused = format!("let a = {first}{};", term.repeat(MAX_CHAINED_OPERATORS));
            assert!(refusal(&refused).is_some(), "{term}");
            let accepted = format!("let a = {first}{};", term.repeat(MAX_CHAINED_OPERATORS - 1));
            assert_eq!(refusal(&accepted), None, "{term}");
        }
        // Exponent signs still belong to their numbers: one operator per term.
        let floats = format!(
            "let a = 0x1p+4{};",
            "+0x1p-4".repeat(MAX_CHAINED_OPERATORS - 1)
        );
        assert_eq!(refusal(&floats), None);
        let floats = format!("let a = 1e+4{};", "+1e-4f".repeat(MAX_CHAINED_OPERATORS));
        assert!(refusal(&floats).is_some());
    }

    /// What is not a chain is not counted: unary signs (negative table entries), the points and
    /// exponent signs of numbers, comments, and words with a digit before a point.
    #[test]
    fn only_binary_operators_members_and_indexings_count() {
        let table = format!("const T = array<f32, 600>({});", "-1.5e-3, ".repeat(600));
        assert_eq!(refusal(&table), None);
        let points = format!("let a = 0.5{};", "+0.25".repeat(MAX_CHAINED_OPERATORS - 1));
        assert_eq!(refusal(&points), None);
        let comments = format!(
            "let a = x{};\n// {}\n/* {} /* nested {} */ */",
            "+x".repeat(100),
            "+x".repeat(500),
            "+x".repeat(500),
            "+x".repeat(500)
        );
        assert_eq!(refusal(&comments), None);
        // A member after a name ending in a digit is one, as are indexings and swizzles.
        for tail in [".x1", "[0]", ".xy"] {
            let source = format!("let a = v{};", tail.repeat(MAX_CHAINED_OPERATORS + 1));
            assert!(refusal(&source).is_some(), "{tail}");
        }
        // A non-ASCII letter ends a name; the operator after it still counts.
        let accented = format!("let a = e\u{301}{};", "+e\u{301}".repeat(300));
        assert!(refusal(&accented).is_some());
    }

    /// A comment is read as naga reads it: what hides a chain's rest from the count would
    /// have to end where naga's comment does not, and every end of a line is one.
    #[test]
    fn a_comment_cannot_hide_or_break_a_chain() {
        let half = "+x".repeat(150);
        // `;` inside a comment does not end the expression.
        for comment in ["// ;\n", "/* ; */", "/* /* ; */ ; */", "// {\u{85}"] {
            let source = format!("let a = x{half}{comment}{half};");
            assert!(refusal(&source).is_some(), "{comment:?}");
        }
        // An unterminated comment hides what follows it from naga as well.
        assert_eq!(refusal(&format!("/* {}", "+x".repeat(500))), None);
        // The end of a line other than `\n` ends a `//` comment, so the chain after it counts.
        let source = format!("let a = x{half}// c\u{2028}{half};");
        assert!(refusal(&source).is_some());
    }

    /// What the cap lets through parses and validates on an ordinary test thread's 2 MiB stack,
    /// in a debug build, whose frames are the largest: the claim that a shader which passed
    /// can be parsed again anywhere.
    #[test]
    fn the_longest_chains_allowed_parse_on_a_small_stack() {
        let wgsl = |tail: &str| {
            format!(
                "fn f(x: array<f32, 1>) -> f32 {{ var v = vec2<f32>(1.0); return {}; }}",
                tail
            )
        };
        let sources = [
            chain(MAX_CHAINED_OPERATORS),
            wgsl(&format!(
                "x[0]{}",
                "+x[0]".repeat(MAX_CHAINED_OPERATORS / 3)
            )),
            wgsl(&format!("v{}.x", ".yx".repeat(MAX_CHAINED_OPERATORS - 1))),
        ];
        for source in sources {
            assert_eq!(refusal(&source), None);
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::default(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    #[test]
    fn a_long_shader_is_refused_before_parsing() {
        let long = format!("{}// {}", chain(3), "x".repeat(MAX_SOURCE_BYTES));
        let (at, message) = refusal(&long).unwrap();
        assert_eq!(at, None);
        assert!(message.ends_with("a shader may hold 131072"), "{message}");
    }

    #[test]
    fn guarded_work_runs_on_a_large_stack_and_a_panic_is_a_refusal() {
        // Nested calls a thread's default 2 MiB cannot hold: 8 MiB of frames.
        fn deep(n: usize) -> usize {
            let pad = [n as u8; 1024];
            if n == 0 {
                0
            } else {
                deep(n - 1) + usize::from(pad[0] & 1)
            }
        }
        assert_eq!(guarded(|| deep(8_192).min(1)), Ok(1));
        let reason = guarded(|| -> () { panic!("boom") }).unwrap_err();
        assert_eq!(reason, "the shader check panicked: boom");
    }
}
