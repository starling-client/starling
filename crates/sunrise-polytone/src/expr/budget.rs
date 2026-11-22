//! How large an expression may be. The parser recurses, and so do the program it builds and every
//! walk over it (the fast path's lowering, `reads_only`, evaluation, dropping the tree): one stack
//! frame a level. A pack's expression must not nest deep enough to overflow the stack, which
//! aborts the process rather than unwinding. So the parser refuses an expression past these as it
//! goes: by its tokens before it starts, by its own recursion, and by the height of each node of
//! the tree as it is built (one more than its highest child), so no tree higher is ever built.

/// The most tokens an expression may have: the work its parse and each evaluation do.
pub const MAX_TOKENS: usize = 1024;
/// The highest a tree may be: what every walk over it recurses into. A walk takes about 3 KiB
/// of stack a level in a debug build.
pub const MAX_HEIGHT: usize = 64;
/// The deepest the parser may recurse (an expression, a statement, or a unary operator or `^`
/// within one). The parser takes about 11 KiB of stack a level in a debug build, through its
/// five functions: 16 levels (14 brackets inside a statement) fit a 256 KiB stack.
pub const MAX_NESTING: usize = 16;

/// Why an expression is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverBudget {
    Tokens(usize),
    Height,
    Nesting,
}

impl std::fmt::Display for OverBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tokens(n) => write!(
                f,
                "{n} tokens, more than the {MAX_TOKENS} an expression may have"
            ),
            Self::Height => write!(
                f,
                "its tree is higher than the {MAX_HEIGHT} levels an expression may have"
            ),
            Self::Nesting => write!(
                f,
                "it nests deeper than the {MAX_NESTING} levels an expression may have"
            ),
        }
    }
}

/// Whether there are few enough `tokens` to parse.
pub(super) fn check_tokens<T>(tokens: &[T]) -> Result<(), OverBudget> {
    if tokens.len() > MAX_TOKENS {
        return Err(OverBudget::Tokens(tokens.len()));
    }
    Ok(())
}
