//! Expression uniforms on core shaders (`polytone/shader_modifiers/<shader>.json`, wiki
//! "Shaders", "Expression Uniforms on Any Shader"): a file's path names the shader
//! (`assets/minecraft/polytone/shader_modifiers/core/lightmap.json` is
//! `minecraft:core/lightmap`), and its body maps uniform block names to expressions. Each is a
//! std140 block of one float (`layout(std140) uniform Name { float value; };`) that a pipeline
//! drawing with that shader, as its vertex or fragment stage, is given, the expression's value
//! each frame.
use crate::expr::Program;
use serde_json::Value as Json;

/// The most uniforms one file may give its shader: each is evaluated every frame.
pub const MAX_UNIFORMS: usize = 16;
/// The longest uniform name: a GLSL identifier, as long as any vanilla block's name needs.
pub const MAX_NAME: usize = 64;

/// A shader's expression uniforms, in name order.
#[derive(Debug)]
pub struct ShaderModifier {
    pub uniforms: Vec<(String, Program)>,
}

/// Why a `shader_modifiers` file is refused; the whole file is, so a shader never draws with
/// part of what its pack meant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The body is not a JSON object of names to expressions.
    NotAnObject,
    /// More uniforms than [`MAX_UNIFORMS`].
    TooMany { count: usize },
    /// A name that is no GLSL identifier (or is longer than [`MAX_NAME`], or starts `gl_`).
    Name(String),
    /// A value that is no expression (a string, a number or a boolean) or doesn't compile.
    Expression { name: String, error: String },
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnObject => write!(f, "not an object of uniform names to expressions"),
            Self::TooMany { count } => {
                write!(
                    f,
                    "{count} uniforms, more than the {MAX_UNIFORMS} a shader may have"
                )
            }
            Self::Name(name) => write!(f, "{name:?} is no uniform block name"),
            Self::Expression { name, error } => write!(f, "{name}: {error}"),
        }
    }
}

/// A GLSL identifier a pack may name a block with.
fn block_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= MAX_NAME
        && !name.starts_with("gl_")
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl ShaderModifier {
    /// A file's body.
    pub fn parse(json: &Json) -> Result<Self, Refused> {
        let object = json.as_object().ok_or(Refused::NotAnObject)?;
        if object.len() > MAX_UNIFORMS {
            return Err(Refused::TooMany {
                count: object.len(),
            });
        }
        let mut uniforms = Vec::with_capacity(object.len());
        for (name, value) in object {
            if !block_name(name) {
                return Err(Refused::Name(name.clone()));
            }
            let expression = |error: String| Refused::Expression {
                name: name.clone(),
                error,
            };
            let program = match value {
                Json::String(s) => Program::compile(s),
                Json::Number(n) => Program::compile(&n.to_string()),
                Json::Bool(b) => Program::compile(if *b { "1" } else { "0" }),
                _ => Err(format!("{value} is not an expression")),
            }
            .map_err(expression)?;
            uniforms.push((name.clone(), program));
        }
        Ok(Self { uniforms })
    }
}

#[cfg(test)]
#[path = "shader_modifiers_tests.rs"]
mod tests;
