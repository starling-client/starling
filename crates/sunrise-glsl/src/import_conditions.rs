//! Each stage shares macro definitions with its includes: the walk that pastes them keeps the
//! macros of the lines before ([`Macros`]) and tests each condition with them
//! ([`directives::test`]), so that only the includes the compiler reaches are read. The lines
//! it keeps still hold their `#define` and `#undef` for [`directives::run_directives`], which
//! expands the code.
use super::{MAX_SCANNED, directives};
use crate::condition::eval_line;
use crate::macros::Macros;

pub(super) struct Conditions {
    macros: Macros,
}

impl Conditions {
    /// The conditions of a stage whose file declares `version` ([`super::stage_version`]),
    /// under its program's `defines`.
    pub(super) fn new(defines: &[&str], version: u32) -> Self {
        Self {
            macros: directives::macros(defines, MAX_SCANNED, version),
        }
    }

    /// The rest of a `#define` line. One that defines nothing is left to the directives' run,
    /// which refuses it.
    pub(super) fn define(&mut self, definition: &str) {
        let _ = self.macros.define_line(definition);
    }

    pub(super) fn undef(&mut self, name: &str) {
        self.macros.undef(name);
    }

    /// Whether `text` reads `__LINE__`, itself or through a macro.
    pub(super) fn uses_line(&self, text: &str) -> bool {
        self.macros.mentions(text, "__LINE__")
    }

    pub(super) fn test(&mut self, kind: &str, rest: &str, line: u32) -> Result<bool, String> {
        self.at_line(line, |macros| directives::test(kind, rest, macros))
    }

    /// The number of the line after `#line rest`, as GLSL numbers it: the directive's, plus one.
    pub(super) fn line(&mut self, rest: &str, line: u32) -> Result<u32, String> {
        self.at_line(line, |macros| {
            let value = eval_line(rest, macros)?;
            u32::try_from(value)
                .ok()
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| "line number outside GLSL range".into())
        })
    }

    /// `run` with `__LINE__` defined as `line`; refused once the macros' work runs out.
    fn at_line<T>(
        &mut self,
        line: u32,
        run: impl FnOnce(&Macros) -> Result<T, String>,
    ) -> Result<T, String> {
        self.macros.define("__LINE__", line.to_string());
        let result = run(&self.macros);
        self.macros.undef("__LINE__");
        match self.macros.stopped() {
            Some(stop) => Err(directives::stopped(stop)),
            None => result,
        }
    }
}

pub(super) use crate::condition::Conditionals;
