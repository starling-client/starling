//! Backend translation checks and located diagnostics for mod render pipeline GLSL programs.
use naga::ShaderStage;
use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};
use std::fmt;

/// What is wrong, and where: a file by its asset path (`assets/<namespace>/<path>`, an `#include`d
/// file's too), with the 1-based line and column when known (0 when not). Its display names the
/// shader stage when the problem is specific to one stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub file: String,
    pub line: usize,
    pub column: usize,
    /// The shader stage responsible, or `None` for a program-wide or unattributed problem.
    pub stage: Option<ShaderStage>,
    /// The refusal without a stage prefix.
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = sunrise_glsl::shader_message(self.stage, &self.message);
        if self.line == 0 {
            write!(f, "{}: {message}", self.file)
        } else {
            write!(f, "{}:{}:{}: {message}", self.file, self.line, self.column)
        }
    }
}

/// A refusal at the file as a whole, attributed to `stage` when known.
pub fn problem(file: &str, stage: Option<ShaderStage>, message: impl Into<String>) -> Problem {
    Problem {
        file: file.to_owned(),
        line: 0,
        column: 0,
        stage,
        message: message.into(),
    }
}

/// Validates `module` and translates every entry point for Vulkan (SPIR-V), Metal (MSL) and
/// DirectX 12 (HLSL), catching backend panics so a mod shader never takes down the game or
/// `cargo sunrise-mod check`. Returns each refusal as `<backend> refuses it: <reason>`.
pub fn translate_backends(module: &naga::Module) -> Vec<String> {
    let info = match Validator::new(ValidationFlags::all(), Capabilities::all()).validate(module) {
        Ok(info) => info,
        Err(err) => return vec![format!("validation failed: {err}")],
    };
    translate_validated(module, &info)
}

fn translate_validated(module: &naga::Module, info: &ModuleInfo) -> Vec<String> {
    let mut problems = Vec::new();
    for e in &module.entry_points {
        let fail = |backend: &str, err: String| format!("{backend} refuses it: {err}");
        let spv = refusal(|| {
            naga::back::spv::write_vec(
                module,
                info,
                &naga::back::spv::Options::default(),
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: e.stage,
                    entry_point: e.name.clone(),
                }),
            )
        });
        let msl = refusal(|| {
            naga::back::msl::write_string(
                module,
                info,
                &naga::back::msl::Options {
                    lang_version: (2, 2),
                    ..Default::default()
                },
                &naga::back::msl::PipelineOptions {
                    entry_point: Some((e.stage, e.name.clone())),
                    ..Default::default()
                },
            )
        });
        let hlsl = refusal(|| {
            naga::back::hlsl::Writer::new(
                &mut String::new(),
                &naga::back::hlsl::Options::default(),
                &naga::back::hlsl::PipelineOptions {
                    entry_point: Some((e.stage, e.name.clone())),
                },
            )
            .write(module, info, None)
        });
        for (backend, err) in [
            ("Vulkan (SPIR-V)", spv),
            ("Metal", msl),
            ("DirectX 12 (HLSL)", hlsl),
        ] {
            if let Some(err) = err {
                problems.push(fail(backend, err));
            }
        }
        if !problems.is_empty() {
            return problems;
        }
    }
    problems
}

/// Why a backend refused: its error, or the panic of a backend that cannot translate a shader it
/// was given (a mod's shader must never take the game or the tool down).
fn refusal<T, E: fmt::Display>(translate: impl FnOnce() -> Result<T, E>) -> Option<String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(translate)) {
        Ok(result) => result.err().map(|e| e.to_string()),
        Err(panic) => Some(format!(
            "it panicked: {}",
            panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or(panic.downcast_ref::<&str>().copied())
                .unwrap_or("no reason given")
        )),
    }
}
