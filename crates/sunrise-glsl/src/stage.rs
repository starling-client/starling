//! One stage the caller has already preprocessed and written as the Vulkan-flavoured GLSL 450
//! naga reads, parsed and validated ([`compile_stage`]): the entry point of a caller that does
//! its own rewriting (`sunrise-shaders`' Iris/OptiFine programs), where [`crate::translate`] does
//! all of it for a vanilla-format program. The caller keeps its names and its rewrites; this
//! adds only what any GLSL needs from naga:
//! - a few things GL takes and naga's frontend refuses, rewritten where naga points and parsed
//!   again: a scalar's swizzle (`f.x`, GLSL 4.20), a matrix column's component written to
//!   (`m[1].y =`), `min`/`max`/`clamp` of a `uint` and integer literals, a function prototype
//!   declared again;
//! - in the parsed module, a function returning a value that reaches its end returns its type's
//!   zero, and `.length()` of a buffer's runtime-sized array is taken of the array itself;
//! - the resources the author's code declares are moved to their binding in the caller's
//!   [`BindingMap`];
//! - the module is validated with every capability and subgroup operation naga has.
//!
//! Errors name the stage and where [`Source::locate`] says each line came from.

use std::collections::BTreeMap;
use std::fmt::Write;

use naga::ShaderStage;
use naga::front::glsl::{Frontend, InterfaceLimits, Options};
use naga::valid::{Capabilities, ValidationFlags, Validator};

use crate::{Error, Location};

/// Where each resource the author's code declares at a `(group, binding)` (its
/// `layout(set = group, binding = binding)`, group 0 when it names no set) is bound instead.
/// A resource whose `(group, binding)` isn't a key keeps it.
pub type BindingMap = BTreeMap<naga::ResourceBinding, naga::ResourceBinding>;

/// One stage to compile ([`compile_stage`]).
pub struct Source<'a> {
    /// Vertex, fragment or compute: naga has no geometry stage, so a caller runs a geometry
    /// program inside the vertex stage it gives here.
    pub stage: ShaderStage,
    /// The caller's own declarations, first in the stage and at their final bindings: the
    /// [`BindingMap`] leaves what they declare alone.
    pub header: &'a str,
    /// The author's code as the caller wrote it again, with any code of the caller's after it.
    pub code: &'a str,
    /// Where line `n` (from 0) of `header` followed by `code` came from, which an error there
    /// names.
    pub locate: &'a dyn Fn(usize) -> Location,
}

/// How many times a stage is parsed, each parse after the first following a rewrite of one
/// refusal naga's frontend points at; a stage still refused after this many is refused with the
/// errors of its last parse. The bound `sunrise-shaders`' `compile` had (`tries < 256`), moved
/// here with it.
const MAX_PARSES: u32 = 256;

/// `source` parsed under `limits` and validated, the resources its code declares bound through
/// `bindings`. Diagnostics carry `source`'s stage and the caller's line mapping.
pub fn compile_stage(
    source: &Source,
    bindings: &BindingMap,
    limits: InterfaceLimits,
) -> Result<naga::Module, Vec<Error>> {
    let options = Options {
        interface_limits: limits,
        ..Options::from(source.stage)
    };
    let at = |text: &str, span: naga::Span, message: String| {
        let start = span.to_range().map_or(0, |r| r.start);
        let line = text[..start.min(text.len())].matches('\n').count();
        (source.locate)(line)
            .error(message)
            .with_stage(source.stage)
    };
    let mut text = String::with_capacity(source.header.len() + source.code.len());
    text.push_str(source.header);
    text.push_str(source.code);
    let mut parses = 0;
    let mut module = loop {
        parses += 1;
        match Frontend::default().parse(&options, &text) {
            Ok(m) => break m,
            Err(e) => match e.errors.iter().find_map(|e| {
                scalar_swizzle(&text, e)
                    .or_else(|| unsigned_literals(&text, e))
                    .or_else(|| repeated_prototype(&text, e))
            }) {
                Some(fixed) if parses < MAX_PARSES => text = fixed,
                _ => {
                    return Err(e
                        .errors
                        .into_iter()
                        .map(|e| at(&text, e.meta, e.kind.to_string()))
                        .collect());
                }
            },
        }
    };
    end_returns_give_zero(&mut module);
    lengths_of_loaded_arrays(&mut module);
    rebind(&mut module, source.header.len(), bindings);
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .subgroup_operations(naga::valid::SubgroupOperationSet::all())
        .subgroup_stages(naga::valid::ShaderStages::all())
        .validate(&module)
        .map_err(|e| {
            let span = e.spans().next().map_or(naga::Span::default(), |(s, _)| *s);
            vec![at(&text, span, validation_message(e.into_inner()))]
        })?;
    Ok(module)
}

/// A validation error with every reason it nests, `: ` between them.
pub(crate) fn validation_message(error: naga::valid::ValidationError) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(s) = source {
        write!(message, ": {s}").unwrap();
        source = s.source();
    }
    message
}

/// The globals declared at or after byte `code` of the stage (the author's code) moved to their
/// binding in `bindings`.
fn rebind(module: &mut naga::Module, code: usize, bindings: &BindingMap) {
    let authored: Vec<_> = (module.global_variables.iter())
        .filter(|&(handle, _)| {
            let span = module.global_variables.get_span(handle);
            span.to_range().is_some_and(|r| r.start >= code)
        })
        .map(|(handle, _)| handle)
        .collect();
    for handle in authored {
        let global = &mut module.global_variables[handle];
        if let Some(to) = global.binding.and_then(|b| bindings.get(&b)) {
            global.binding = Some(*to);
        }
    }
}

/// `block.member.length()` of a buffer block's runtime-sized array: naga's frontend loads the
/// array and takes the length of the loaded value, which its validator refuses, since a
/// runtime-sized array can't be loaded. The length is taken of the array's pointer instead. The
/// load's slot becomes that length, and the old length expression a plain conversion of it, so
/// every handle and emitted range stays as it was.
fn lengths_of_loaded_arrays(module: &mut naga::Module) {
    use naga::Expression as E;
    let functions = module.functions.iter_mut().map(|(_, f)| f);
    for f in functions.chain(module.entry_points.iter_mut().map(|e| &mut e.function)) {
        let loads: Vec<_> = f
            .expressions
            .iter()
            .filter_map(|(at, e)| match *e {
                E::ArrayLength(loaded) => match f.expressions[loaded] {
                    E::Load { pointer } => Some((at, loaded, pointer)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        for (at, loaded, pointer) in loads {
            f.expressions[loaded] = E::ArrayLength(pointer);
            f.expressions[at] = E::As {
                expr: loaded,
                kind: naga::ScalarKind::Uint,
                convert: Some(4),
            };
        }
    }
}

/// GLSL lets a function that returns a value reach its end without a `return`: what it returns
/// then is undefined (GLSL 4.60, 6.1). naga's frontend ends every body that can reach its end
/// with a `return` of no value, which its validator refuses in such a function; those returns
/// give the result type's zero instead.
fn end_returns_give_zero(module: &mut naga::Module) {
    fn fill(block: &mut naga::Block, zero: &mut dyn FnMut() -> naga::Handle<naga::Expression>) {
        // The ends naga's frontend gives a `return` to (`naga::proc::ensure_block_returns`).
        match block.last_mut() {
            Some(naga::Statement::Block(b)) => fill(b, zero),
            Some(naga::Statement::If { accept, reject, .. }) => {
                fill(accept, zero);
                fill(reject, zero);
            }
            Some(naga::Statement::Switch { cases, .. }) => {
                for case in cases.iter_mut().filter(|c| !c.fall_through) {
                    fill(&mut case.body, zero);
                }
            }
            Some(naga::Statement::Return { value }) if value.is_none() => *value = Some(zero()),
            _ => {}
        }
    }
    for (_, f) in module.functions.iter_mut() {
        let Some(ty) = f.result.as_ref().map(|r| r.ty) else {
            continue;
        };
        let (body, expressions) = (&mut f.body, &mut f.expressions);
        let mut made = None;
        fill(body, &mut || {
            *made.get_or_insert_with(|| {
                expressions.append(naga::Expression::ZeroValue(ty), naga::Span::default())
            })
        });
    }
}

/// `text` without the scalar swizzle naga's frontend refused (`e`, GLSL 4.20's `f.x`, `f.xx`): one
/// component dropped, more a vector constructor of the scalar.
fn scalar_swizzle(text: &str, e: &naga::front::glsl::Error) -> Option<String> {
    if !e
        .kind
        .to_string()
        .starts_with("Can't lookup field on this type")
    {
        return None;
    }
    let r = e.meta.to_range()?;
    let field = text.get(r.clone())?.strip_prefix('.')?;
    let one = |c: char| field.chars().all(|f| f == c);
    let indexed = text[..r.start].trim_end().ends_with(']');
    let component = field.len() == 1 && "xyzwrgbastpq".contains(field);
    if field.is_empty()
        || field.len() > 4
        || !(one('x') || one('r') || one('s') || indexed && component)
    {
        return None;
    }
    let mut out = text.to_owned();
    if field.len() == 1 && indexed {
        // A matrix column's component, written to: `m[1].y` as `m[1][1]`.
        let k = "xyzwrgbastpq".find(field).map(|k| k % 4)?;
        out.replace_range(r, &format!("[{k}]"));
    } else if field.len() == 1 {
        out.replace_range(r, "");
    } else {
        let start = expression_start(text, r.start)?;
        out.replace_range(r, ")");
        out.insert_str(start, &format!("vec{}(", field.len()));
    }
    Some(out)
}

/// `text` with the integer literals of the `min`/`max`/`clamp` call naga found ambiguous (`e`)
/// made unsigned: with a `uint` beside them GL takes the `uint` overload.
fn unsigned_literals(text: &str, e: &naga::front::glsl::Error) -> Option<String> {
    let kind = e.kind.to_string();
    let f = kind
        .strip_prefix("Ambiguous best function for '")?
        .trim_end_matches('\'');
    if !matches!(f, "min" | "max" | "clamp") {
        return None;
    }
    let r = e.meta.to_range()?;
    let call = text.get(r.clone())?;
    let b = call.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'.';
    let mut out = String::with_capacity(call.len() + 8);
    let mut i = 0;
    let mut changed = false;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !word(b[i - 1])) {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            out.push_str(&call[start..i]);
            if i == b.len() || !(word(b[i])) {
                out.push('u');
                changed = true;
            }
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    changed.then(|| format!("{}{out}{}", &text[..r.start], &text[r.end..]))
}

/// `text` without the function prototype naga's frontend refused as declared before (`e`): GLSL
/// lets a function be declared again (packs include the same helper's prototype from two
/// files). The prototype and its `;` turn to spaces, keeping lines where they are; one with
/// qualifiers before its type (naga's span starts at the type) is left alone.
fn repeated_prototype(text: &str, e: &naga::front::glsl::Error) -> Option<String> {
    if e.kind.to_string() != "Prototype already defined" {
        return None;
    }
    let r = e.meta.to_range()?;
    let end = r.end + text.get(r.end..)?.find(';')? + 1;
    if !text.get(r.end..end - 1)?.trim().is_empty() {
        return None;
    }
    let before = text.get(..r.start)?.trim_end();
    let line = &before[before.rfind('\n').map_or(0, |n| n + 1)..];
    let boundary =
        before.is_empty() || before.ends_with([';', '}']) || line.trim_start().starts_with('#');
    if !boundary {
        return None;
    }
    let blank: String = text[r.start..end]
        .chars()
        .map(|c| if c == '\n' { '\n' } else { ' ' })
        .collect();
    Some(format!("{}{blank}{}", &text[..r.start], &text[end..]))
}

/// Where the operand that ends right before byte `end` begins: a name, a call or an index (with
/// what it is of), or a bracketed expression.
fn expression_start(text: &str, end: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = end;
    while i > 0 && b[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    loop {
        match b.get(i.checked_sub(1)?) {
            Some(b')') | Some(b']') => {
                let mut level = 0i32;
                loop {
                    i = i.checked_sub(1)?;
                    match b[i] {
                        b')' | b']' => level += 1,
                        b'(' | b'[' => {
                            level -= 1;
                            if level == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some(c) if c.is_ascii_alphanumeric() || *c == b'_' || *c == b'.' => i -= 1,
            _ => return Some(i),
        }
        if i == 0 {
            return Some(0);
        }
    }
}

#[cfg(test)]
#[path = "stage_tests.rs"]
mod tests;
