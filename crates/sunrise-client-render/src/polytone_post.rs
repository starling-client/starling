//! Resource packs' Polytone post chains (#11486, wiki "Shaders"): vanilla `post_effect` chains a
//! pack runs over the world while their `activation_condition` holds, with uniform blocks from
//! expressions (`expression_uniforms`) and targets of their own (`post_targets`) kept between
//! frames. `post_chain` runs them; this reads what they add. Their passes, as any effect's, read
//! the game's `PolyGlobals` ([`PolyGlobals`], handed to `post_chain` with each frame's request).
use crate::environment::Frame;
use crate::post_chain::{Extras, InternalTarget};
use serde_json::Value as Json;
use std::sync::Arc;
use sunrise_polytone::expr::Program;

/// A `polytone/post_chains/<path>.json`.
struct Chain {
    /// The `post_effect` chain it runs.
    effect: String,
    activation: Option<Program>,
    /// Each expression uniform: its block's name and expression.
    uniforms: Vec<(String, Program)>,
}

/// The packs' post chains and targets.
pub struct PolytonePost {
    chains: Vec<Chain>,
    targets: std::collections::BTreeMap<String, InternalTarget>,
    configs: sunrise_polytone::pack::Configs,
}

/// What `PolyGlobals` holds (wiki "Extra Shader Uniforms").
#[derive(Clone, Copy, Debug, Default)]
pub struct PolyGlobals {
    /// The world's projection (reverse-Z, as 26.3's and Sunrise's are), and its model-view (the
    /// camera's turn).
    pub projection: glam::Mat4,
    pub model_view: glam::Mat4,
    /// The sun's angle in radians, 0 at the horizon.
    pub sun_angle: f32,
    /// The time of day in ticks (0 to 24000), and the game ticks this frame advanced.
    pub day_time: f32,
    pub delta_time: f32,
    /// The player's feet: `PolyPlayerBlockPos`, and `PolyPlayerOffset` as vanilla's
    /// `CameraOffset` is (the block position less the position, so `polyPlayerPos()` is the
    /// block position less the offset).
    pub player: [f64; 3],
}

impl PolyGlobals {
    /// The std140 block (`sunrise_polytone::globals`).
    pub fn bytes(&self) -> [u8; sunrise_polytone::globals::BLOCK_SIZE] {
        sunrise_polytone::globals::Globals {
            projection: self.projection.to_cols_array(),
            model_view: self.model_view.to_cols_array(),
            sun_angle: self.sun_angle,
            day_time: self.day_time,
            delta_time: self.delta_time,
            player: self.player,
        }
        .std140()
    }
}

/// Every `.json` under `assets/<namespace>/polytone/<kind>`, by its implicit id.
fn files(
    root: &sunrise_assets::resource_source::ResourceSource,
    kind: &str,
) -> Vec<(String, Json)> {
    let mut out = Vec::new();
    for relative in root.list("assets/") {
        let Some(id) = sunrise_polytone::implicit_id(&relative, kind) else {
            continue;
        };
        match root
            .read_to_string(&relative)
            .ok()
            .and_then(|s| serde_json::from_str::<Json>(&s).ok())
        {
            Some(json) => out.push((id, json)),
            None => log::warn!("{relative}: invalid post configuration"),
        }
    }
    out
}

fn compile(where_: &str, src: &Json) -> Option<Program> {
    let text = match src {
        Json::String(s) => s.clone(),
        Json::Number(n) => n.to_string(),
        Json::Bool(b) => b.to_string(),
        _ => return None,
    };
    match Program::compile(&text) {
        Ok(p) => Some(p),
        Err(e) => {
            log::warn!("{where_}: {e}");
            None
        }
    }
}

impl PolytonePost {
    /// The post chains and targets of the packs merged at `root`; `None` without chains.
    pub fn read(
        root: &sunrise_assets::resource_source::ResourceSource,
        configs: sunrise_polytone::pack::Configs,
    ) -> Option<Self> {
        let chains: Vec<Chain> = files(root, "post_chains")
            .into_iter()
            .filter_map(|(id, json)| {
                let effect = json["post_chain"].as_str()?;
                let effect = if effect.contains(':') {
                    effect.to_owned()
                } else {
                    format!("minecraft:{effect}")
                };
                // `PolyGlobals` is the game's: a chain giving it is refused, named.
                let globals = sunrise_polytone::globals::BLOCK;
                if json["expression_uniforms"].get(globals).is_some() {
                    log::warn!(
                        "{id}: not used: its expression uniform {globals} is the game's block"
                    );
                    return None;
                }
                let activation = json
                    .get("activation_condition")
                    .and_then(|c| compile(&id, c));
                let uniforms = json["expression_uniforms"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter_map(|(name, src)| Some((name.clone(), compile(&id, src)?)))
                    .collect();
                Some(Chain {
                    effect,
                    activation,
                    uniforms,
                })
            })
            .collect();
        if chains.is_empty() {
            return None;
        }
        let targets = files(root, "post_targets")
            .into_iter()
            .map(|(id, json)| {
                let size = |key: &str| json[key].as_u64().filter(|n| *n > 0).map(|n| n as u32);
                let target = InternalTarget {
                    width: size("width"),
                    height: size("height"),
                    persistent: true,
                    clear: [0.0; 4],
                };
                (id, target)
            })
            .collect();
        Some(Self {
            chains,
            targets,
            configs,
        })
    }

    /// The chains to run this frame (those whose condition holds), and what they add. The
    /// game's `PolyGlobals` reach every post pass that reads them through the request
    /// (`post_chain::Request::poly_globals`), not here.
    pub fn frame(&self, frame: &Frame) -> (Vec<String>, Extras) {
        let mut effects = Vec::new();
        let mut extras = Extras {
            targets: self.targets.clone(),
            blocks: Default::default(),
        };
        for chain in &self.chains {
            let on = chain.activation.as_ref().is_none_or(|c| {
                crate::environment::eval_frame(c, frame, &self.configs).is_some_and(|v| v.truthy())
            });
            if !on {
                continue;
            }
            for (name, program) in &chain.uniforms {
                let value = crate::environment::eval_frame(program, frame, &self.configs)
                    .map_or(0.0, |v| v.num()) as f32;
                let mut block = value.to_le_bytes().to_vec();
                block.resize(16, 0);
                extras.blocks.insert(name.clone(), block);
            }
            effects.push(chain.effect.clone());
        }
        (effects, extras)
    }
}

/// The packs' `polytone/shader_modifiers` (`sunrise_polytone::shader_modifiers`): each shader's
/// expression uniforms, read once when the packs' core shaders load (`hud::pack_text`).
pub struct ShaderModifiers {
    /// Each shader's id (`minecraft:core/lightmap`) and its uniforms.
    shaders: Vec<(String, Vec<(String, Arc<Program>)>)>,
    configs: Arc<sunrise_polytone::pack::Configs>,
}

/// The largest `shader_modifiers` file read: far more than [`MAX_UNIFORMS`] expressions need.
///
/// [`MAX_UNIFORMS`]: sunrise_polytone::shader_modifiers::MAX_UNIFORMS
pub const MAX_SHADER_MODIFIERS_FILE: u64 = 64 * 1024;

/// One expression uniform of a pipeline, resolved when its shader loads: the block's binding
/// and the stages that read it, and the program that gives its value each frame.
#[derive(Clone)]
pub struct ExpressionUniform {
    pub name: String,
    pub binding: u32,
    pub stages: wgpu::ShaderStages,
    program: Arc<Program>,
    configs: Arc<sunrise_polytone::pack::Configs>,
}

impl ExpressionUniform {
    /// Its value in `frame` (the packs' expression context): the expression's, 0 when it fails.
    pub fn evaluate(&self, frame: &Frame) -> f32 {
        crate::environment::eval_frame(&self.program, frame, &self.configs)
            .map_or(0.0, |v| v.num() as f32)
    }
}

impl std::fmt::Debug for ExpressionUniform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExpressionUniform")
            .field("name", &self.name)
            .field("binding", &self.binding)
            .field("stages", &self.stages)
            .finish()
    }
}

/// Why a pack core shader's expression uniforms refuse it when it loads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpressionRefusal {
    /// A stage reads the block, and it is not one float.
    NotOneFloat(String),
    /// A stage reads the block, and no `shader_modifiers` file of the pipeline's shaders gives it.
    NotGiven(String),
    /// A `shader_modifiers` file of the pipeline's shaders gives a block the game fills
    /// (`LightmapInfo`, `PolyGlobals`).
    Reserved(String),
    /// The shaders declare `PolyGlobals` other than its layout
    /// (`expression_blocks::check_poly_globals`).
    Globals(crate::gfx::expression_blocks::GlobalsRefusal),
}

impl std::fmt::Display for ExpressionRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotOneFloat(name) => write!(f, "its block {name} is not one float"),
            Self::NotGiven(name) => {
                write!(f, "no shader_modifiers file of its shaders gives {name}")
            }
            Self::Reserved(name) => write!(
                f,
                "a shader_modifiers file of its shaders gives {name}, which the game fills"
            ),
            Self::Globals(e) => e.fmt(f),
        }
    }
}

impl ShaderModifiers {
    /// The packs' files at `root` (the selected packs' stack), each evaluated with `configs`.
    /// A file over [`MAX_SHADER_MODIFIERS_FILE`], not JSON, or refused by its contents is left
    /// out, named by its path in the log.
    pub fn read(
        root: &sunrise_assets::resource_source::ResourceSource,
        configs: sunrise_polytone::pack::Configs,
    ) -> Self {
        let shaders = Self::files(root, &mut |path, e| log::warn!("{path}: not used: {e}"));
        Self {
            shaders,
            configs: Arc::new(configs),
        }
    }

    /// Each file's shader and uniforms; `refused` hears of each one left out, by its path.
    fn files(
        root: &sunrise_assets::resource_source::ResourceSource,
        refused: &mut dyn FnMut(&str, String),
    ) -> Vec<(String, Vec<(String, Arc<Program>)>)> {
        use sunrise_polytone::shader_modifiers::ShaderModifier;
        let mut shaders = Vec::new();
        for namespace in root.namespaces("assets") {
            for path in root.list(&format!("assets/{namespace}/polytone/shader_modifiers/")) {
                if !path.ends_with(".json") {
                    continue;
                }
                let Some(id) = sunrise_polytone::implicit_id(&path, "shader_modifiers") else {
                    refused(&path, "its path names no shader".to_owned());
                    continue;
                };
                let read = root
                    .read_limited(&path, MAX_SHADER_MODIFIERS_FILE)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| {
                        serde_json::from_slice::<Json>(&bytes).map_err(|e| e.to_string())
                    })
                    .and_then(|json| {
                        // A pack's expressions are untrusted, as its GLSL is (`hud::pack_text`):
                        // a fault compiling one refuses the file, and the game goes on.
                        crate::crash::contain(|| ShaderModifier::parse(&json))
                            .map_err(|panic| format!("the expression compiler failed: {panic}"))?
                            .map_err(|e| e.to_string())
                    });
                match read {
                    Ok(modifier) => {
                        let uniforms = modifier.uniforms.into_iter();
                        let uniforms = uniforms.map(|(n, p)| (n, Arc::new(p))).collect();
                        shaders.push((id, uniforms));
                    }
                    Err(e) => refused(&path, e),
                }
            }
        }
        shaders
    }

    /// The expression `name` the file of shader `id` gives.
    fn uniform(&self, id: &str, name: &str) -> Option<&Arc<Program>> {
        let (_, uniforms) = self.shaders.iter().find(|(s, _)| s == id)?;
        uniforms.iter().find(|(n, _)| n == name).map(|(_, p)| p)
    }

    /// The uniform `name` of any file, as if a pipeline's stages `stages` read it at
    /// `binding`.
    #[cfg(test)]
    pub(crate) fn uniform_for_test(
        &self,
        name: &str,
        binding: u32,
        stages: wgpu::ShaderStages,
    ) -> ExpressionUniform {
        let program = (self.shaders.iter())
            .find_map(|(id, _)| self.uniform(id, name))
            .expect("a file gives it");
        ExpressionUniform {
            name: name.to_owned(),
            binding,
            stages,
            program: program.clone(),
            configs: self.configs.clone(),
        }
    }

    /// The names the files of `shaders` (a pipeline's stages' ids) give: the blocks the loader
    /// lets the pipeline's shaders declare.
    pub fn names(&self, shaders: &[&str]) -> Vec<String> {
        let mut names: Vec<String> = (self.shaders.iter())
            .filter(|(id, _)| shaders.contains(&id.as_str()))
            .flat_map(|(_, uniforms)| uniforms.iter().map(|(n, _)| n.clone()))
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// The expression uniforms of `program`, a pipeline whose vertex stage is shader
    /// `vertex` and whose fragment stage is `fragment` (their ids): each of its blocks but
    /// `own` that a stage reads, with that stage's file's expression. A block both stages
    /// read, or that both files give, takes the fragment shader's file's first, then the
    /// vertex shader's. One no stage reads is not bound; one that isn't one float refuses
    /// the shader, named. So does a file of these shaders giving one of `own` (the game's), and
    /// a `PolyGlobals` declared other than its layout, the same rule as the post shaders'.
    pub fn resolve(
        &self,
        (vertex, fragment): (&str, &str),
        program: &crate::core_shader::CoreProgram,
        own: &[&str],
    ) -> Result<Vec<ExpressionUniform>, ExpressionRefusal> {
        if let Some(name) = (own.iter()).find(|name| {
            [vertex, fragment]
                .iter()
                .any(|id| self.uniform(id, name).is_some())
        }) {
            return Err(ExpressionRefusal::Reserved((*name).to_owned()));
        }
        if let Some(i) = (program.other.iter()).position(|n| n == sunrise_polytone::globals::BLOCK)
        {
            let binding = crate::core_shader::other_binding(i);
            crate::gfx::expression_blocks::check_poly_globals(
                [&program.vertex, &program.fragment],
                binding,
            )
            .map_err(ExpressionRefusal::Globals)?;
        }
        let mut uniforms = Vec::new();
        for (i, name) in program.other.iter().enumerate() {
            if own.contains(&name.as_str()) {
                continue;
            }
            let binding = crate::core_shader::other_binding(i);
            let stages = program.stages_reading(binding);
            if stages.is_empty() {
                continue;
            }
            if !program.is_float_block(binding) {
                return Err(ExpressionRefusal::NotOneFloat(name.clone()));
            }
            let order = if stages.contains(wgpu::ShaderStages::FRAGMENT) {
                [fragment, vertex]
            } else {
                [vertex, fragment]
            };
            let program = (order.iter())
                .find_map(|id| self.uniform(id, name))
                .ok_or_else(|| ExpressionRefusal::NotGiven(name.clone()))?;
            uniforms.push(ExpressionUniform {
                name: name.clone(),
                binding,
                stages,
                program: program.clone(),
                configs: self.configs.clone(),
            });
        }
        Ok(uniforms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Packs in memory: `files` (path, text).
    fn pack(files: &[(&str, String)]) -> sunrise_assets::resource_source::ResourceSource {
        use sunrise_assets::resource_source::{Resource, ResourceSource};
        ResourceSource::indexed(
            files
                .iter()
                .map(|(p, t)| ((*p).to_owned(), Resource::bytes(t.clone().into_bytes())))
                .collect(),
        )
    }

    const LIGHTMAP: &str = "assets/minecraft/polytone/shader_modifiers/core/lightmap.json";
    const SCREENQUAD: &str = "assets/minecraft/polytone/shader_modifiers/core/screenquad.json";

    /// A lightmap pipeline's program whose vertex stage reads blocks `vertex` and whose
    /// fragment stage reads `fragment` (each `uniform Name { float v; };`, or `vec4` for one
    /// named `Wide`), besides `LightmapInfo` and the blocks in `unused`, declared and not read.
    fn program(
        vertex: &[&str],
        fragment: &[&str],
        unused: &[&str],
    ) -> crate::core_shader::CoreProgram {
        let block = |name: &&str| {
            let ty = if *name == "Wide" { "vec4" } else { "float" };
            format!(
                "layout(std140) uniform {name} {{\n    {ty} {}v;\n}};\n",
                name.to_lowercase()
            )
        };
        let sum = |names: &[&str]| -> String {
            names
                .iter()
                .map(|n| {
                    let v = format!("{}v", n.to_lowercase());
                    if *n == "Wide" {
                        format!(" + {v}.x")
                    } else {
                        format!(" + {v}")
                    }
                })
                .collect()
        };
        let vsh = format!(
            "#version 330\n{}{}layout(location = 0) out float sum;\n\
             void main() {{\n    sum = 0.0{};\n    gl_Position = vec4(0.0, 0.0, 0.0, 1.0);\n}}\n",
            vertex.iter().map(block).collect::<String>(),
            unused.iter().map(block).collect::<String>(),
            sum(vertex)
        );
        let fsh = format!(
            "#version 330\nlayout(std140) uniform LightmapInfo {{\n    float SkyFactor;\n}} info;\n\
             {}layout(location = 0) in float sum;\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {{\n    fragColor = vec4(sum{}, info.SkyFactor, 0.0, 1.0);\n}}\n",
            fragment.iter().map(block).collect::<String>(),
            sum(fragment)
        );
        crate::core_shader::translate_core_with(
            &vsh,
            &fsh,
            &[],
            &|_| None,
            &crate::gfx::core_lightmap::GLUE,
            &|_| true,
        )
        .expect("the shaders translate")
    }

    fn resolved(
        modifiers: &ShaderModifiers,
        program: &crate::core_shader::CoreProgram,
    ) -> Result<Vec<(String, wgpu::ShaderStages, f32)>, ExpressionRefusal> {
        let [vertex, fragment] = crate::gfx::LIGHTMAP_SHADERS;
        let uniforms = modifiers.resolve((vertex, fragment), program, &["LightmapInfo"])?;
        let frame = Frame::default();
        Ok(uniforms
            .iter()
            .map(|u| (u.name.clone(), u.stages, u.evaluate(&frame)))
            .collect())
    }

    /// A pipeline's expression uniforms resolve at load from its stages' files: a block a
    /// stage reads takes its value from that stage's file, the fragment shader's first when
    /// both read it or both files give it; one no stage reads is not bound; one that is no
    /// single float (a `vec4`, the same 16 bytes) refuses the shader.
    #[test]
    fn a_pipelines_expression_uniforms_resolve_from_its_stages_files() {
        let modifiers = ShaderModifiers::read(
            &pack(&[
                (
                    LIGHTMAP,
                    r#"{"Both": "0.25", "Frag": "0.5", "Wide": "1"}"#.into(),
                ),
                (
                    SCREENQUAD,
                    r#"{"Both": "0.75", "Vert": "0.125", "Unused": "1"}"#.into(),
                ),
                (
                    "assets/minecraft/polytone/shader_modifiers/core/sky.json",
                    r#"{"Sky": "1"}"#.into(),
                ),
            ]),
            Default::default(),
        );
        let lightmap = crate::gfx::LIGHTMAP_SHADERS;
        assert_eq!(
            modifiers.names(&lightmap),
            ["Both", "Frag", "Unused", "Vert", "Wide"]
        );
        assert_eq!(modifiers.names(&["minecraft:core/sky"]), ["Sky"]);
        let (v, f) = (wgpu::ShaderStages::VERTEX, wgpu::ShaderStages::FRAGMENT);
        let split = program(&["Vert", "Both"], &["Frag", "Both"], &["Unused"]);
        assert_eq!(
            resolved(&modifiers, &split).unwrap(),
            [
                ("Vert".to_owned(), v, 0.125),
                ("Both".to_owned(), v | f, 0.25),
                ("Frag".to_owned(), f, 0.5),
            ]
        );
        // Read by the vertex stage only, `Both` takes the vertex shader's file's.
        let vertex_only = program(&["Both"], &[], &[]);
        assert_eq!(
            resolved(&modifiers, &vertex_only).unwrap(),
            [("Both".to_owned(), v, 0.75)]
        );
        // Only one file gives `Frag`: a vertex stage reading it takes that one.
        let across = program(&["Frag"], &[], &[]);
        assert_eq!(
            resolved(&modifiers, &across).unwrap(),
            [("Frag".to_owned(), v, 0.5)]
        );
        let wide = program(&[], &["Wide"], &[]);
        assert_eq!(
            resolved(&modifiers, &wide).unwrap_err(),
            ExpressionRefusal::NotOneFloat("Wide".into())
        );
        // A `vec4` the vertex stage only declares binds nothing: the fragment stage's float of
        // that name is what the pipeline reads.
        let declared = crate::core_shader::translate_core_with(
            "#version 330\nlayout(std140) uniform Frag {\n    vec4 unused;\n};\n\
             layout(location = 0) out float sum;\n\
             void main() {\n    sum = 0.0;\n    gl_Position = vec4(0.0, 0.0, 0.0, 1.0);\n}\n",
            "#version 330\nlayout(std140) uniform Frag {\n    float fragv;\n};\n\
             layout(location = 0) in float sum;\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {\n    fragColor = vec4(sum + fragv);\n}\n",
            &[],
            &|_| None,
            &crate::gfx::core_lightmap::GLUE,
            &|_| true,
        )
        .expect("the shaders translate");
        assert_eq!(
            resolved(&modifiers, &declared).unwrap(),
            [("Frag".to_owned(), f, 0.5)]
        );
        // A block a stage reads that no file of the pipeline's shaders gives.
        let ungiven = program(&[], &["Sky"], &[]);
        assert_eq!(
            resolved(&modifiers, &ungiven).unwrap_err(),
            ExpressionRefusal::NotGiven("Sky".into())
        );
    }

    /// A file over the read budget, not JSON, or refused by its contents is left out, named by
    /// its resource path: its uniforms are no shader's, so a shader reading one is refused
    /// when it loads.
    #[test]
    fn a_bad_shader_modifiers_file_is_left_out_by_its_path() {
        let budget = MAX_SHADER_MODIFIERS_FILE as usize;
        let padded = |len: usize| {
            let body = r#"{"Tint": "1"}"#;
            format!("{body}{}", " ".repeat(len - body.len()))
        };
        let read = |text: String| {
            let mut refused = Vec::new();
            let shaders = ShaderModifiers::files(&pack(&[(LIGHTMAP, text)]), &mut |path, _| {
                refused.push(path.to_owned())
            });
            (shaders.len(), refused)
        };
        assert_eq!(read(padded(budget)), (1, vec![]));
        for bad in [
            padded(budget + 1),
            "{\"Tint\": ".into(),
            r#"{"gl_Tint": "1"}"#.into(),
        ] {
            assert_eq!(read(bad), (0, vec![LIGHTMAP.to_owned()]));
        }
        // A file whose path names no shader (an id may not have capitals) is named too.
        let bad_path = "assets/minecraft/polytone/shader_modifiers/core/Lightmap.json";
        let mut refused = Vec::new();
        let files = pack(&[(bad_path, r#"{"Tint": "1"}"#.into())]);
        let shaders = ShaderModifiers::files(&files, &mut |path, _| refused.push(path.to_owned()));
        assert_eq!((shaders.len(), refused), (0, vec![bad_path.to_owned()]));
    }

    #[test]
    fn poly_globals_follow_std140() {
        let g = PolyGlobals {
            projection: glam::Mat4::from_diagonal(glam::Vec4::splat(2.0)),
            sun_angle: 1.5,
            day_time: 6000.0,
            delta_time: 0.5,
            player: [10.25, -3.5, 7.0],
            ..Default::default()
        };
        let b = g.bytes();
        let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let i = |at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        assert_eq!(b.len(), 176);
        assert_eq!(f(0), 2.0);
        assert_eq!((f(128), f(132), f(136)), (1.5, 6000.0, 0.5));
        // ivec3 at 144, vec3 at 160 (std140 aligns both to 16). The offset is the block position
        // less the position, as vanilla's CameraOffset: the block position less it is the position.
        assert_eq!((i(144), i(148), i(152)), (10, -4, 7));
        assert_eq!((f(160), f(164), f(168)), (-0.25, -0.5, 0.0));
    }

    #[test]
    fn chains_run_while_their_condition_holds_with_their_uniforms() {
        let root =
            std::env::temp_dir().join(format!("sunrise-polytone-post-{}", std::process::id()));
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "assets/p/polytone/post_chains/glare.json",
            r#"{"activation_condition": "g.rain() < 0.5", "post_chain": "p:glare",
                "expression_uniforms": {"Weather": "1 - g.rain()"}}"#,
        );
        write(
            "assets/p/polytone/post_targets/history.json",
            r#"{"width": 1, "height": 1}"#,
        );
        let post = PolytonePost::read(
            &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
            Default::default(),
        )
        .unwrap();
        let mut frame = Frame {
            rain: 0.25,
            ..Frame::default()
        };
        let (effects, extras) = post.frame(&frame);
        assert_eq!(effects, ["p:glare"]);
        // `g.rain` is the mean of rain (0.25) and thunder (0): 0.125.
        assert_eq!(
            f32::from_le_bytes(extras.blocks["Weather"][..4].try_into().unwrap()),
            0.875
        );
        assert!(extras.targets["p:history"].persistent);
        frame.rain = 1.0;
        assert!(post.frame(&frame).0.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// `PolyGlobals` is the game's: a chain giving an expression uniform of that name is refused
    /// (the others still run), and no chain's block of that name is handed on.
    #[test]
    fn a_chain_cannot_give_poly_globals() {
        use sunrise_assets::resource_source::{Resource, ResourceSource};
        let chain = |effect: &str, uniform: &str| {
            format!(r#"{{"post_chain": "{effect}", "expression_uniforms": {{"{uniform}": "1"}}}}"#)
        };
        let pack = ResourceSource::indexed(
            [
                (
                    "assets/p/polytone/post_chains/a.json",
                    chain("p:a", "PolyGlobals"),
                ),
                (
                    "assets/p/polytone/post_chains/b.json",
                    chain("p:b", "Weather"),
                ),
            ]
            .into_iter()
            .map(|(path, text)| (path.to_owned(), Resource::bytes(text.into_bytes())))
            .collect(),
        );
        let post = PolytonePost::read(&pack, Default::default()).unwrap();
        let (effects, extras) = post.frame(&Frame::default());
        assert_eq!(effects, ["p:b"]);
        assert!(!extras.blocks.contains_key("PolyGlobals"));
    }
}
