//! Each stage shares macro definitions with its includes. For branch arithmetic, hand naga's
//! GLSL preprocessor the condition and its referenced definitions; unrelated source and
//! earlier conditional arms never have to be replayed.
use super::{MAX_SCANNED, is_ident, replace_ident};
use pp_rs::{pp::Preprocessor, token::TokenValue};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct Conditions {
    macros: HashMap<String, String>,
    scanned: usize,
    has_line_macro: bool,
}

impl Conditions {
    pub(super) fn new(defines: &[&str]) -> Self {
        let mut state = Self::default();
        for define in defines {
            state.define(define);
        }
        state
    }

    pub(super) fn define(&mut self, definition: &str) {
        let end = definition
            .find(|c| !is_ident(c))
            .unwrap_or(definition.len());
        let name = &definition[..end];
        self.has_line_macro |= definition.contains("__LINE__");
        self.macros.insert(name.to_owned(), definition.to_owned());
    }

    pub(super) fn undef(&mut self, name: &str) {
        self.macros.remove(name);
    }

    /// Object-like and function-like definitions can refer to later definitions. Resolve
    /// their names at use, leaving actual token expansion to the GLSL frontend.
    fn definitions<'a>(&'a self, text: &'a str) -> Vec<&'a str> {
        let mut pending = vec![text];
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        while let Some(source) = pending.pop() {
            for name in source.split(|c| !is_ident(c)).filter(|s| !s.is_empty()) {
                if let Some(definition) = self.macros.get(name)
                    && seen.insert(name)
                {
                    found.push(definition.as_str());
                    pending.push(definition);
                }
            }
        }
        found
    }

    pub(super) fn uses_line(&self, text: &str) -> bool {
        text.contains("__LINE__")
            || (self.has_line_macro
                && self
                    .definitions(text)
                    .iter()
                    .any(|d| d.contains("__LINE__")))
    }

    pub(super) fn test(&mut self, kind: &str, rest: &str, line: u32) -> Result<bool, String> {
        match kind {
            "ifdef" | "ifndef" => {
                if rest.is_empty() || !rest.chars().all(is_ident) {
                    return Err(format!("invalid #{kind} {rest}"));
                }
                let defined = rest == "__LINE__" || self.macros.contains_key(rest);
                Ok(defined == (kind == "ifdef"))
            }
            _ => self.number(rest, line, false).map(|n| n != 0),
        }
    }

    pub(super) fn line(&mut self, rest: &str, line: u32) -> Result<u32, String> {
        self.number(rest, line, true)
    }

    fn number(&mut self, expression: &str, line: u32, line_directive: bool) -> Result<u32, String> {
        let expression = sunrise_shaders::resolve_defined(expression, |name| {
            name == "__LINE__" || self.macros.contains_key(name)
        })?;
        let mut source = String::new();
        for definition in self.definitions(&expression) {
            source.push_str("#define ");
            source.push_str(definition);
            source.push('\n');
        }
        source.push_str(&expression);
        // Expand macros as ordinary GLSL tokens. In particular, function-like macros take
        // their arguments here before the conditional expression parser evaluates the result.
        let source = replace_ident(&source, "__LINE__", &line.to_string());
        self.charge(source.len())?;
        let mut code = String::from(if line_directive { "#line " } else { "#if " });
        for token in Preprocessor::new(&source) {
            let token = token.map_err(|(why, _)| format!("GLSL directive: {why:?}"))?;
            let spelling = &source[token.location.start as usize..token.location.end as usize];
            self.charge(spelling.len() + 1)?;
            code.push_str(spelling);
            code.push(' ');
        }
        if line_directive {
            code.push_str("\n__LINE__\n");
        } else {
            code.push_str("\n1\n#else\n0\n#endif\n");
        }
        match Preprocessor::new(&code).next() {
            Some(Ok(token)) => match token.value {
                TokenValue::Integer(n) => n
                    .value
                    .try_into()
                    .map_err(|_| "line number outside GLSL range".into()),
                _ => Err("expected an integer from a GLSL directive".into()),
            },
            Some(Err((why, _))) => Err(format!("GLSL directive: {why:?}")),
            None => Err("empty GLSL directive".into()),
        }
    }

    fn charge(&mut self, bytes: usize) -> Result<(), String> {
        self.scanned += bytes;
        if self.scanned > MAX_SCANNED {
            return Err(super::ImportRefusal::TooMuchScanned.to_string());
        }
        Ok(())
    }
}
