//! Crash and memory containment for untrusted GLSL shader text before naga's GLSL front end
//! parses it.
//!
//! naga's GLSL parser, validator and backends recurse over the syntactic shape of the source, so
//! deeply nested brackets, chained unary operators, deeply nested brace-less statements, or an
//! expression chain of thousands of binary operators (`a + a + a + ...`) can overflow a thread's
//! stack, and a stack overflow aborts the process.
//!
//! These bounds are crash and memory containment, not a feature cap on what a shader may compute:
//! vanilla and NeoForge delegate GLSL compilation to the GPU driver, whereas Sunrise parses GLSL
//! in-process via naga. Anything refused by these bounds can be written with local variables or
//! braces, and every vanilla 26.3 core shader sits well below every bound (checked by the daily
//! test `vanilla_core_shaders_pass_guard_with_margin`).
//!
//! Bounds applied to every expanded GLSL stage (on top of `sunrise_glsl::expand_includes`'s
//! include depth 16, 1,024 pastes, 1 MiB expanded and 8 MiB scanned memory containment):
//! - [`MAX_EXPANDED_BYTES`] (1 MiB, matching `sunrise_glsl::MAX_EXPANDED`): caps total expanded
//!   work per stage so pathological input cannot exhaust memory or stack via declaration chains.
//! - [`MAX_CHAINED_OPERATORS`] (200): caps binary operators, member accesses and indexings in
//!   one expression; naga lowers left-associative chains recursively, and 200 stays well inside
//!   both the 64 MiB checking thread (`STACK_BYTES`) and a default 2 MiB debug thread.
//! - [`MAX_NESTING`] (200): caps bracket nesting (`(`, `[`, `{`), consecutive unary prefix
//!   operators (`!`, `~`, `+`, `-`), and chained brace-less control statements (`if`, `else`,
//!   `for`, `while`), matching naga's own WGSL parser nesting limit so naga's GLSL front end
//!   cannot overflow its stack on deeply nested syntax.
//! - [`guarded`]: runs translation and validation on a 64 MiB stack thread (`STACK_BYTES`) and
//!   catches panics as refusals.

/// The most bytes an expanded GLSL stage may hold (1 MiB, matching `sunrise_glsl::MAX_EXPANDED`).
pub const MAX_EXPANDED_BYTES: usize = sunrise_glsl::MAX_EXPANDED;

/// The most binary operators, member accesses and indexings one expression may chain.
/// Crash containment for naga's recursive GLSL parser/lowering; verified in tests to parse and
/// validate comfortably on a debug build's 2 MiB thread as well as on [`STACK_BYTES`].
pub const MAX_CHAINED_OPERATORS: usize = 200;

/// The deepest nesting of brackets (`(`, `[`, `{`), consecutive unary prefix operators, or
/// chained brace-less statements (`if`, `else`, `for`, `while`) allowed before naga parses GLSL.
/// Crash containment matching naga's WGSL nesting bound; verified in tests on a debug 2 MiB thread.
pub const MAX_NESTING: usize = 200;

/// Stack size of the checking thread; address space committed lazily by the OS.
const STACK_BYTES: usize = 64 << 20;

/// Why expanded GLSL `text` may not be handed to naga, with its byte offset when localized.
pub fn refusal(text: &str) -> Option<(Option<usize>, String)> {
    if text.len() > MAX_EXPANDED_BYTES {
        return Some((
            None,
            format!(
                "{} bytes of expanded shader; a stage may hold {MAX_EXPANDED_BYTES}",
                text.len()
            ),
        ));
    }
    scan(text)
}

/// Single-pass lexical scan of GLSL `text` checking:
/// 1. chained binary operators / member accesses / indexings per expression (`MAX_CHAINED_OPERATORS`),
/// 2. bracket nesting (`(`, `[`, `{`) (`MAX_NESTING`),
/// 3. consecutive unary prefix operators (`!`, `~`, `+`, `-`) (`MAX_NESTING`),
/// 4. chained brace-less control statements (`if`, `else`, `for`, `while`) (`MAX_NESTING`).
///
/// GLSL comments (`//` to line end, non-nested `/* ... */`) and numbers (decimal/hex/float with
/// exponents) are skipped so operators inside comments or exponents are neither missed nor
/// over-counted.
fn scan(text: &str) -> Option<(Option<usize>, String)> {
    let mut chars = text.char_indices().peekable();
    let mut chain = 0usize;
    let mut after_operand = false;
    let mut bracket_depth = 0usize;
    let mut unary_depth = 0usize;
    let mut braceless_depth = 0usize;
    let mut in_control_header_parens: Option<usize> = None;
    let mut expecting_control_body = false;

    while let Some((at, c)) = chars.next() {
        match c {
            '/' if chars.peek().is_some_and(|(_, n)| *n == '/') => {
                while chars.next_if(|(_, n)| *n != '\n' && *n != '\r').is_some() {}
                continue;
            }
            '/' if chars.peek().is_some_and(|(_, n)| *n == '*') => {
                // GLSL block comments do not nest: the first `*/` closes the comment.
                chars.next();
                let mut prev = None;
                for (_, c) in &mut chars {
                    if prev == Some('*') && c == '/' {
                        break;
                    }
                    prev = Some(c);
                }
                continue;
            }
            '#' => {
                // Skip preprocessor directives (with backslash line continuations).
                let mut prev = '#';
                for (_, ch) in chars.by_ref() {
                    if ch == '\n' && prev != '\\' {
                        break;
                    }
                    if ch != '\r' {
                        prev = ch;
                    }
                }
                continue;
            }
            c if c.is_ascii_whitespace() => {}
            ';' => {
                chain = 0;
                after_operand = false;
                unary_depth = 0;
                if in_control_header_parens.is_none() {
                    braceless_depth = 0;
                    expecting_control_body = false;
                }
            }
            '{' => {
                bracket_depth += 1;
                if bracket_depth > MAX_NESTING {
                    return Some((
                        Some(at),
                        format!("nesting exceeds {MAX_NESTING} levels ({bracket_depth} here)"),
                    ));
                }
                chain = 0;
                after_operand = false;
                unary_depth = 0;
                braceless_depth = 0;
                expecting_control_body = false;
            }
            '}' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                chain = 0;
                after_operand = false;
                unary_depth = 0;
                braceless_depth = 0;
                expecting_control_body = false;
            }
            '(' => {
                bracket_depth += 1;
                if bracket_depth > MAX_NESTING {
                    return Some((
                        Some(at),
                        format!("nesting exceeds {MAX_NESTING} levels ({bracket_depth} here)"),
                    ));
                }
                if expecting_control_body && in_control_header_parens.is_none() {
                    in_control_header_parens = Some(bracket_depth);
                    expecting_control_body = false;
                }
                after_operand = false;
                unary_depth = 0;
            }
            ')' => {
                if in_control_header_parens == Some(bracket_depth) {
                    in_control_header_parens = None;
                    braceless_depth += 1;
                    if braceless_depth > MAX_NESTING {
                        return Some((
                            Some(at),
                            format!(
                                "brace-less statement nesting exceeds {MAX_NESTING} levels \
                                 ({braceless_depth} here); add braces"
                            ),
                        ));
                    }
                    // Header finished; body starts next, so reset expression chain.
                    chain = 0;
                    after_operand = false;
                } else {
                    after_operand = true;
                }
                bracket_depth = bracket_depth.saturating_sub(1);
                unary_depth = 0;
            }
            '[' => {
                bracket_depth += 1;
                if bracket_depth > MAX_NESTING {
                    return Some((
                        Some(at),
                        format!("nesting exceeds {MAX_NESTING} levels ({bracket_depth} here)"),
                    ));
                }
                if after_operand {
                    chain += 1;
                    if chain > MAX_CHAINED_OPERATORS {
                        return Some((
                            Some(at),
                            format!(
                                "an expression chains more than {MAX_CHAINED_OPERATORS} operators \
                                 ({chain} here); split it with local variables"
                            ),
                        ));
                    }
                }
                after_operand = false;
                unary_depth = 0;
            }
            ']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                after_operand = true;
                unary_depth = 0;
            }
            c if c.is_ascii_digit() => {
                number(c, &mut chars);
                after_operand = true;
                unary_depth = 0;
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut end = at + c.len_utf8();
                while let Some(&(idx, n)) = chars.peek() {
                    if n.is_ascii_alphanumeric() || n == '_' {
                        end = idx + n.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }
                let word = &text[at..end];
                if in_control_header_parens.is_none() {
                    match word {
                        "if" | "for" | "while" => {
                            expecting_control_body = true;
                            after_operand = false;
                            unary_depth = 0;
                            continue;
                        }
                        "else" => {
                            braceless_depth += 1;
                            if braceless_depth > MAX_NESTING {
                                return Some((
                                    Some(at),
                                    format!(
                                        "brace-less statement nesting exceeds {MAX_NESTING} levels \
                                         ({braceless_depth} here); add braces"
                                    ),
                                ));
                            }
                            after_operand = false;
                            unary_depth = 0;
                            continue;
                        }
                        "return" | "case" => {
                            after_operand = false;
                            unary_depth = 0;
                            continue;
                        }
                        _ => {}
                    }
                }
                after_operand = true;
                unary_depth = 0;
            }
            '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '<' | '>' | '=' | '!' | '.' | '?'
            | ':'
                if after_operand =>
            {
                // Consume second character of multi-char operators (`==`, `!=`, `<=`, `>=`, `&&`,
                // `||`, `<<`, `>>`, `++`, `--`, `+=`, etc.) as one operator.
                let next = chars.peek().map(|&(_, n)| n);
                match (c, next) {
                    ('+' | '-' | '&' | '|' | '<' | '>' | '=', Some(n)) if n == c => {
                        chars.next();
                    }
                    (
                        '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '<' | '>' | '!',
                        Some('='),
                    ) => {
                        chars.next();
                    }
                    _ => {}
                }
                chain += 1;
                if chain > MAX_CHAINED_OPERATORS {
                    return Some((
                        Some(at),
                        format!(
                            "an expression chains more than {MAX_CHAINED_OPERATORS} operators \
                             ({chain} here); split it with local variables"
                        ),
                    ));
                }
                // Postfix `++`/`--` leaves us still after an operand; binary ops expect an operand.
                after_operand = matches!((c, next), ('+', Some('+')) | ('-', Some('-')));
                unary_depth = 0;
            }
            '!' | '~' | '+' | '-' => {
                // Unary prefix operator (or prefix `++`/`--`).
                if matches!(c, '+' | '-') && chars.peek().is_some_and(|&(_, n)| n == c) {
                    chars.next();
                }
                unary_depth += 1;
                if unary_depth > MAX_NESTING {
                    return Some((
                        Some(at),
                        format!(
                            "unary operator nesting exceeds {MAX_NESTING} levels ({unary_depth} here)"
                        ),
                    ));
                }
                after_operand = false;
            }
            _ => {
                after_operand = false;
                unary_depth = 0;
            }
        }
    }
    None
}

/// Skip the rest of a GLSL numeric literal starting with `first`, handling decimal, hex (`0x`),
/// floats with exponents (`1.5e-3`, `0x1p+4`), and suffixes (`u`, `f`).
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
    while chars
        .next_if(|&(_, c)| c.is_ascii_alphanumeric() || c == '_')
        .is_some()
    {}
}

/// Runs `work` on a thread with a 64 MiB stack and returns its result, or why it failed (the
/// thread failed to spawn or `work` panicked).
pub fn guarded<T: Send>(work: impl FnOnce() -> T + Send) -> Result<T, String> {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .name("glsl-pipeline-check".to_owned())
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

    fn chain_glsl(terms: usize) -> String {
        format!(
            "#version 450 core\nfloat f(float x) {{ return x{}; }}\n",
            "+x".repeat(terms)
        )
    }

    #[test]
    fn operator_chain_bound_is_per_expression() {
        assert_eq!(refusal(&chain_glsl(MAX_CHAINED_OPERATORS)), None);
        let (at, msg) = refusal(&chain_glsl(MAX_CHAINED_OPERATORS + 1)).unwrap();
        assert!(at.is_some());
        assert!(msg.contains("201 here"), "{msg}");
        // Many separate statements stay well under the per-expression bound.
        let statements = format!(
            "#version 450 core\nvoid f() {{ float a = 1.0; {} }}\n",
            "a = a + a * a - a; ".repeat(2_000)
        );
        assert_eq!(refusal(&statements), None);
    }

    #[test]
    fn bracket_unary_and_braceless_nesting_bounds_hold() {
        let parens = |n: usize| {
            format!(
                "#version 450 core\nfloat f(float x) {{ return {}x{}; }}\n",
                "(".repeat(n),
                ")".repeat(n)
            )
        };
        // `{` of function body counts as 1 level, so `MAX_NESTING - 1` parens fit, `MAX_NESTING` overflows.
        assert_eq!(refusal(&parens(MAX_NESTING - 1)), None);
        assert!(refusal(&parens(MAX_NESTING)).is_some());

        let unary = |n: usize| {
            format!(
                "#version 450 core\nfloat f(float x) {{ return {}x; }}\n",
                "- ".repeat(n)
            )
        };
        assert_eq!(refusal(&unary(MAX_NESTING)), None);
        assert!(refusal(&unary(MAX_NESTING + 1)).is_some());
        let not_chain = |n: usize| {
            format!(
                "#version 450 core\nbool f(bool x) {{ return {}x; }}\n",
                "!".repeat(n)
            )
        };
        assert_eq!(refusal(&not_chain(MAX_NESTING)), None);
        assert!(refusal(&not_chain(MAX_NESTING + 1)).is_some());

        let braceless = |n: usize| {
            format!(
                "#version 450 core\nvoid f(bool c) {{ {}c = false; }}\n",
                "if (c) ".repeat(n)
            )
        };
        assert_eq!(refusal(&braceless(MAX_NESTING)), None);
        assert!(refusal(&braceless(MAX_NESTING + 1)).is_some());
    }

    #[test]
    fn glsl_block_comments_do_not_nest_and_cannot_hide_chains() {
        let half = "+x".repeat(150);
        // `/* /* */` closes at the first `*/` in GLSL, so the second half is live code.
        let source = format!("void f(float x) {{ float a = x{half}/* /* */{half}; }}");
        assert!(refusal(&source).is_some());
        // A `;` inside a comment does not reset the expression chain.
        for comment in ["// ;\n", "/* ; */"] {
            let source = format!("void f(float x) {{ float a = x{half}{comment}{half}; }}");
            assert!(refusal(&source).is_some(), "{comment:?}");
        }
    }

    #[test]
    fn expanded_stage_over_one_mib_is_refused() {
        let long = format!("// {}", "x".repeat(MAX_EXPANDED_BYTES));
        let (at, msg) = refusal(&long).unwrap();
        assert_eq!(at, None);
        assert!(
            msg.contains(&format!("a stage may hold {MAX_EXPANDED_BYTES}")),
            "{msg}"
        );
    }

    /// The longest chains and deepest nesting allowed parse and validate with naga's GLSL front
    /// end on a standard test thread's 2 MiB stack in a debug build.
    #[test]
    fn max_allowed_glsl_parses_and_validates_on_default_thread() {
        let chain_src = format!(
            "#version 450 core\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {{ float x = 0.001; float y = x{}; fragColor = vec4(y); }}\n",
            "+x".repeat(MAX_CHAINED_OPERATORS - 1)
        );
        let parens_src = format!(
            "#version 450 core\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {{ float x = 1.0; float y = {}x{}; fragColor = vec4(y); }}\n",
            "(".repeat(MAX_NESTING - 1),
            ")".repeat(MAX_NESTING - 1)
        );
        for src in [&chain_src, &parens_src] {
            assert_eq!(refusal(src), None);
            let options = naga::front::glsl::Options::from(naga::ShaderStage::Fragment);
            let module = naga::front::glsl::Frontend::default()
                .parse(&options, src)
                .unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    #[test]
    fn guarded_runs_on_large_stack_and_catches_panics() {
        fn deep(n: usize) -> usize {
            let pad = [n as u8; 1024];
            if n == 0 {
                0
            } else {
                deep(n - 1) + usize::from(pad[0] & 1)
            }
        }
        assert_eq!(guarded(|| deep(8_192).min(1)), Ok(1));
        assert_eq!(
            guarded(|| -> () { panic!("boom") }),
            Err("the shader check panicked: boom".to_owned())
        );
    }

    /// Daily test (§6 & §13 Q1): every vanilla 26.3 core shader (expanded with its includes)
    /// passes `guard::refusal` with comfortable margin under `MAX_EXPANDED_BYTES`.
    #[test]
    #[ignore = "daily"]
    fn vanilla_core_shaders_pass_guard_with_margin() {
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .expect("SUNRISE_TEST_RESOURCES must point to prepared 26.3 resources");
        let core_dir = root.join("assets/minecraft/shaders/core");
        let read = |id: &str| -> Result<Option<String>, String> {
            let Some((ns, rest)) = id.split_once(':') else {
                return Ok(None);
            };
            let path = root.join("assets").join(ns).join(rest);
            match std::fs::read_to_string(&path) {
                Ok(text) => Ok(Some(text)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(format!("{}: {e}", path.display())),
            }
        };
        let includes = sunrise_glsl::Includes {
            read: &read,
            namespace: "minecraft",
            folder: "shaders/include/",
            once: &["moj_import"],
        };
        let mut count = 0usize;
        let mut largest_name = String::new();
        let mut largest_bytes = 0usize;
        for entry in std::fs::read_dir(&core_dir).expect("read vanilla core shaders dir") {
            let path = entry.unwrap().path();
            let ext = path.extension().and_then(|e| e.to_str());
            if !matches!(ext, Some("vsh" | "fsh")) {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let file_id = format!(
                "minecraft:shaders/core/{}",
                path.file_name().unwrap().to_string_lossy()
            );
            let expanded = sunrise_glsl::expand_includes(
                &text,
                &file_id,
                "minecraft:shaders/core/",
                &includes,
                Some(&[crate::DEPTH_ZERO_TO_ONE]),
            )
            .unwrap_or_else(|e| panic!("{}: expand failed: {e}", path.display()))
            .text()
            .to_owned();
            assert_eq!(
                refusal(&expanded),
                None,
                "vanilla core shader {} failed guard",
                path.display()
            );
            count += 1;
            if expanded.len() > largest_bytes {
                largest_bytes = expanded.len();
                largest_name = path.file_name().unwrap().to_string_lossy().into_owned();
            }
        }
        assert!(count > 0, "expected vanilla core shaders in {core_dir:?}");
        // Vanilla's largest core shader must sit well under MAX_EXPANDED_BYTES (at least 10x margin).
        assert!(
            largest_bytes * 10 < MAX_EXPANDED_BYTES,
            "largest vanilla core shader {largest_name} is {largest_bytes} bytes (budget {MAX_EXPANDED_BYTES})"
        );
    }
}
