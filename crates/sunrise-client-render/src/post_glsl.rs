//! Vanilla's post-processing GLSL (`#version 330` with `layout(std140)` uniform blocks
//! and `uniform sampler2D`s, Vulkan-style `gl_VertexIndex`) turned into naga modules at
//! run time, from whichever resource root is active, the way Sunrise's shader packs turn
//! their GLSL into naga (`sunrise-shaders`' translator, which handles the compatibility
//! dialect; the post shaders need only the block and sampler binding rewrite below).
//!
//! `ShaderManager`/`GlslPreprocessor` read `assets/<ns>/shaders/<path>.{vsh,fsh}` and
//! resolve `#include <ns:path>` to `assets/<ns>/shaders/include/<path>` and a quoted one from
//! the stage's own folder, through the core shaders' resolver (`core_shader::expand_imports`).
//!
//! The rewrite gives every uniform block a binding of bind group 0, shared by both
//! stages, and splits each `uniform sampler2D NameSampler;` into a texture and a sampler
//! object at the next two bindings, `NameSampler` becoming the combined
//! `sampler2D(texture, sampler)` again where it is used. The vertex stage's `main` is
//! wrapped so that clip y is negated: targets keep GL's rows (row 0 is the bottom edge),
//! so `texCoord` reads the texel it would in vanilla. Each line keeps the file and line it came
//! from through the rewrite (`sunrise_glsl::Expanded`), so an error names them.
use naga::ShaderStage;
use naga::front::glsl::Frontend;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use sunrise_assets::resource_source::ResourceSource;
use sunrise_glsl::{Error, Expanded};

/// One program, ready for wgpu.
pub struct Program {
    pub vertex: naga::Module,
    pub fragment: naga::Module,
    /// Every uniform block either stage declares, with its binding.
    pub blocks: Vec<(String, u32)>,
    /// Every sampler (`NameSampler`): the texture's binding; the sampler object is the next.
    pub samplers: Vec<(String, u32)>,
    /// What each stage reads (`sunrise_glsl::bindings_read`, from the stages' validation).
    pub read: Vec<(u32, wgpu::ShaderStages)>,
    /// The code each stage was parsed from, with where its lines came from.
    code: [Expanded; 2],
}

impl Program {
    /// Where a stage declares the resource at `binding`, a stage reading it first
    /// (`sunrise_glsl::declared_at`): what a refusal of that declaration names.
    pub fn declared_at(&self, binding: u32) -> sunrise_glsl::Location {
        let stages = sunrise_glsl::stages_reading(&self.read, binding);
        sunrise_glsl::declared_at(
            [
                (&self.vertex, &self.code[0]),
                (&self.fragment, &self.code[1]),
            ],
            [
                stages.contains(wgpu::ShaderStages::VERTEX),
                stages.contains(wgpu::ShaderStages::FRAGMENT),
            ],
            binding,
        )
    }
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The source with its comments blanked (same length, so offsets stay valid).
fn blank_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                if b[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
            for slot in out.iter_mut().take((i + 2).min(b.len())).skip(i) {
                *slot = b' ';
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Why [`translate`] refuses a pass's shaders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A stage file missing or that can't be read (naming it).
    Source(String),
    /// A stage's includes (`core_shader::expand_imports`), at the directive refused.
    Imports(String, sunrise_glsl::ImportError),
    /// The stages declare more uniform blocks or samplers than the device can bind.
    Declares {
        what: &'static str,
        declared: usize,
        limit: usize,
    },
    /// The translator's errors for `stage` (`ns:post/name`), with the shader stage and
    /// positions the author wrote.
    Translator { stage: String, errors: Vec<Error> },
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(why) => f.write_str(why),
            Self::Translator { stage, errors } => {
                write!(f, "{stage}: {}", sunrise_glsl::describe(errors))
            }
            Self::Imports(id, refusal) => write!(f, "{id}: {refusal}"),
            Self::Declares {
                what,
                declared,
                limit,
            } => write!(
                f,
                "the shaders declare {declared} {what}, past the {limit} the device binds"
            ),
        }
    }
}

/// How many uniform blocks and samplers a pass's two stages may declare between them: what the
/// device binds to both (twice its per-stage limits). Checked before any analysis. With the
/// device's stage interface limits, which the stages' parse refuses past.
#[derive(Clone, Copy, Debug)]
pub struct Declarable {
    pub blocks: usize,
    pub samplers: usize,
    pub interface: naga::front::glsl::InterfaceLimits,
}

impl Declarable {
    /// For `limits`: each stage's limit, for both stages.
    pub fn of(limits: &wgpu::Limits) -> Self {
        Self {
            blocks: 2 * limits.max_uniform_buffers_per_shader_stage as usize,
            samplers: 2 * limits.max_sampled_textures_per_shader_stage as usize,
            interface: crate::core_shader::interface_limits(limits),
        }
    }
}

/// A shader file of the packs at `root`, through the core shaders' bounded reader
/// (`hud::pack_text::read_shader_file`): none when they haven't it, else the text or why it
/// can't be read, named by its path.
fn read_file(root: &ResourceSource, path: &str) -> Result<Option<String>, String> {
    crate::hud::pack_text::read_shader_file(root, path).map_err(|e| e.to_string())
}

/// The stage `id` (`ns:post/name`) with its includes, through the core shaders' resolver
/// (`core_shader::expand_imports`): the stage and each include read through the bounded
/// reader, quoted ones from the stage's own folder, each `#moj_import` once, the expansion
/// bounded, and a missing `polytone_globals.glsl` the built-in one. Each line keeps its file
/// (`ns:shaders/post/name.fsh` for the stage) and line.
fn stage(root: &ResourceSource, id: &str, ext: &str) -> Result<Expanded, Refused> {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    let file = format!("assets/{ns}/shaders/{path}.{ext}");
    let text = read_file(root, &file)
        .map_err(Refused::Source)?
        .ok_or_else(|| Refused::Source(format!("{file}: not found")))?;
    let dir = format!(
        "{ns}:shaders/{}",
        &path[..path.rfind('/').map_or(0, |i| i + 1)]
    );
    let read = |file: &str| {
        let (ns, path) = file.split_once(':').ok_or(format!("bad include {file}"))?;
        read_file(root, &format!("assets/{ns}/{path}"))
    };
    crate::core_shader::expand_imports(&text, &format!("{ns}:shaders/{path}.{ext}"), &dir, &read)
        .map_err(|e| Refused::Imports(id.to_owned(), e))
}

/// Every `uniform` declaration of a source: (start of the declaration including a
/// `layout(...)` before it, end of the type or of the whole `;`, type, name, is a sampler).
/// Refused, with how many it has found, once the source holds more than `max` `uniform`
/// keywords: each one costs a look back for its `layout(...)`, so their count is bounded before
/// the source is scanned further.
fn declarations(src: &str, max: usize) -> Result<Vec<(usize, usize, String, String, bool)>, usize> {
    let plain = blank_comments(src);
    let b = plain.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    let mut keywords = 0;
    while let Some(found) = plain[at..].find("uniform") {
        let start = at + found;
        at = start + 7;
        if (start > 0 && is_ident(b[start - 1])) || b.get(start + 7).is_some_and(|c| is_ident(*c)) {
            continue;
        }
        keywords += 1;
        if keywords > max {
            return Err(keywords);
        }
        let word = |from: usize| -> Option<(usize, usize)> {
            let mut s = from;
            while s < b.len() && b[s].is_ascii_whitespace() {
                s += 1;
            }
            let mut e = s;
            while e < b.len() && is_ident(b[e]) {
                e += 1;
            }
            (e > s).then_some((s, e))
        };
        let Some((ts, te)) = word(start + 7) else {
            continue;
        };
        let ty = plain[ts..te].to_owned();
        // The `layout(...)` right before `uniform`, if any.
        let before = plain[..start].trim_end();
        let decl_start = before
            .ends_with(')')
            .then(|| before.rfind("layout"))
            .flatten()
            .filter(|l| {
                before[*l + 6..].trim_start().starts_with('(') && !before[*l..].contains(';')
            })
            .unwrap_or(start);
        if ty.starts_with("sampler") {
            let Some((ns, ne)) = word(te) else { continue };
            let end = plain[ne..].find(';').map_or(ne, |p| ne + p + 1);
            out.push((decl_start, end, ty, plain[ns..ne].to_owned(), true));
        } else {
            let mut s = te;
            while s < b.len() && b[s].is_ascii_whitespace() {
                s += 1;
            }
            if b.get(s) == Some(&b'{') {
                out.push((decl_start, te, String::new(), ty, false));
            }
        }
    }
    Ok(out)
}

/// The source with each declaration given its binding from the shared tables; the lines a
/// sampler's adds come from its declaration's.
fn rewrite(src: &mut Expanded, blocks: &[(String, u32)], samplers: &[(String, u32)]) {
    // `translate` bounded the keywords before.
    let declared = declarations(src.text(), usize::MAX).unwrap_or_default();
    let edits = declared
        .into_iter()
        .map(|(start, end, ty, name, sampler)| {
            let with = if sampler {
                let tex = samplers.iter().find(|(n, _)| *n == name).map_or(0, |s| s.1);
                let kind = if ty == "sampler2D" { "texture2D" } else { &ty };
                format!(
                    "layout(set = 0, binding = {tex}) uniform {kind} {name}_hz_tex; \
                     layout(set = 0, binding = {}) uniform sampler {name}_hz_smp;\n\
                     #define {name} sampler2D({name}_hz_tex, {name}_hz_smp)\n",
                    tex + 1
                )
            } else {
                let binding = blocks.iter().find(|(n, _)| *n == name).map_or(0, |s| s.1);
                format!("layout(std140, set = 0, binding = {binding}) uniform {name}")
            };
            (start..end, with)
        })
        .collect();
    src.edit(edits);
}

/// Replaces the word `main` of `void main(` with `hz_main`, and calls it from a `main` of
/// Sunrise's own.
fn wrap_main(src: &mut Expanded) -> Result<(), String> {
    let plain = blank_comments(src.text());
    let mut at = 0;
    while let Some(found) = plain[at..].find("main") {
        let p = at + found;
        at = p + 4;
        let b = plain.as_bytes();
        if (p > 0 && is_ident(b[p - 1])) || b.get(p + 4).is_some_and(|c| is_ident(*c)) {
            continue;
        }
        if plain[..p].trim_end().ends_with("void") && plain[p + 4..].trim_start().starts_with('(') {
            src.edit(vec![(p..p + 4, "hz_main".to_owned())]);
            src.wrap(
                "",
                "\nvoid main() { hz_main(); gl_Position.y = -gl_Position.y; }\n",
            );
            return Ok(());
        }
    }
    Err("no main".into())
}

/// Runs the directives of one stage, with the backend's defines every program has
/// (`core_shader::run_directives`: a post pass has none of its own), then parses and validates
/// it. Refusals name the stage and the authored position, without quoting the rewritten line;
/// success includes the parsed code.
fn parse(
    stage: ShaderStage,
    code: &Expanded,
    limits: naga::front::glsl::InterfaceLimits,
) -> Result<(naga::Module, naga::valid::ModuleInfo, Expanded), Vec<Error>> {
    let code =
        crate::core_shader::run_directives(code, &[]).map_err(|e| vec![e.with_stage(stage)])?;
    let module = Frontend::default()
        .parse(&sunrise_glsl::glsl_options(stage, limits), code.text())
        .map_err(|e| code.errors_in(stage, &e, |kind, _line| kind.to_string()))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|e| {
            // The innermost span naga gives (an expression inside the function the error names).
            let span = (e.spans().map(|(s, _)| *s))
                .filter(|s| s.to_range().is_some())
                .last();
            let span = span.unwrap_or_default();
            vec![
                code.error_in(span, format!("{:?}", e.into_inner()))
                    .with_stage(stage),
            ]
        })?;
    Ok((module, info, code))
}

/// Translates the vertex and fragment shader a pass names (`minecraft:post/invert`), refused
/// before any analysis when they declare more blocks or samplers than `declarable`. A parse,
/// directive or validation refusal names the stage as well as its authored position.
pub fn translate(
    root: &ResourceSource,
    vertex: &str,
    fragment: &str,
    declarable: Declarable,
) -> Result<Program, Refused> {
    let mut vs = stage(root, vertex, "vsh")?;
    let mut fs = stage(root, fragment, "fsh")?;
    // Blocks and samplers are shared by name across the stages, so both agree on a binding.
    let mut blocks: Vec<(String, u32)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let keywords = declarable.blocks + declarable.samplers;
    for src in [vs.text(), fs.text()] {
        let declared = declarations(src, keywords).map_err(|found| Refused::Declares {
            what: "uniform declarations",
            declared: found,
            limit: keywords,
        })?;
        for (_, _, _, name, sampler) in declared {
            if sampler {
                if !names.contains(&name) {
                    names.push(name);
                }
            } else if !blocks.iter().any(|(n, _)| *n == name) {
                blocks.push((name, blocks.len() as u32));
            }
            for (what, declared, limit) in [
                ("uniform blocks", blocks.len(), declarable.blocks),
                ("samplers", names.len(), declarable.samplers),
            ] {
                if declared > limit {
                    return Err(Refused::Declares {
                        what,
                        declared,
                        limit,
                    });
                }
            }
        }
    }
    let first = blocks.len() as u32;
    let samplers: Vec<(String, u32)> = names.into_iter().zip((first..).step_by(2)).collect();
    let refused = |stage: &str, errors| Refused::Translator {
        stage: stage.to_owned(),
        errors,
    };
    rewrite(&mut vs, &blocks, &samplers);
    wrap_main(&mut vs).map_err(|message| {
        refused(
            vertex,
            vec![vs.error_at(None, message).with_stage(ShaderStage::Vertex)],
        )
    })?;
    rewrite(&mut fs, &blocks, &samplers);
    vs.wrap("#version 450\n", "");
    fs.wrap("#version 450\n", "");
    let (vertex_module, vertex_info, vs) = parse(ShaderStage::Vertex, &vs, declarable.interface)
        .map_err(|errors| refused(vertex, errors))?;
    let (fragment_module, fragment_info, fs) =
        parse(ShaderStage::Fragment, &fs, declarable.interface)
            .map_err(|errors| refused(fragment, errors))?;
    let read = sunrise_glsl::bindings_read([
        (&vertex_module, &vertex_info),
        (&fragment_module, &fragment_info),
    ]);
    Ok(Program {
        vertex: vertex_module,
        fragment: fragment_module,
        blocks,
        samplers,
        read,
        code: [vs, fs],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use sunrise_glsl::{ImportError, ImportRefusal};

    /// `refusal` at `file`'s `line`, as the post shader `stage` refuses it.
    fn imports(stage: &str, file: &str, line: u32, refusal: ImportRefusal) -> Refused {
        Refused::Imports(
            stage.into(),
            ImportError {
                file: file.into(),
                line,
                refusal,
            },
        )
    }

    /// Room for any test shader's declarations.
    const ANY: Declarable = Declarable {
        blocks: 64,
        samplers: 64,
        interface: naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
    };

    fn root() -> Option<std::path::PathBuf> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)?
            .join(".local/sunrise-client/26.3");
        dir.join("assets/minecraft/shaders/post/blit.fsh")
            .exists()
            .then_some(dir)
    }

    #[test]
    fn shader_sources_and_includes_translate_without_files() {
        use sunrise_assets::resource_source::Resource;
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            ("assets/custom/shaders/post/test.vsh".into(), Resource::bytes(b"#version 330\nvoid main() { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }".to_vec())),
            ("assets/custom/shaders/post/test.fsh".into(), Resource::bytes(b"#version 330\n#include <custom:color.glsl>\nlayout(location=0) out vec4 fragColor;\nvoid main() { fragColor = color; }".to_vec())),
            ("assets/custom/shaders/include/color.glsl".into(), Resource::bytes(b"const vec4 color = vec4(0.2, 0.4, 0.6, 1.0);".to_vec())),
        ]));
        translate(&source, "custom:post/test", "custom:post/test", ANY).unwrap();
        assert!(matches!(
            translate(&source, "custom:missing", "custom:post/test", ANY),
            Err(Refused::Source(_))
        ));
    }

    /// The value of `module`'s constant `name`.
    fn constant(module: &naga::Module, name: &str) -> i64 {
        let (_, c) = (module.constants.iter())
            .find(|(_, c)| c.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no constant {name}"));
        match module.global_expressions[c.init] {
            naga::Expression::Literal(naga::Literal::I32(n)) => n.into(),
            ref other => panic!("{name} is {other:?}"),
        }
    }

    /// A post shader's `__VERSION__` is its own file's `#version` (not the `#version 450` the
    /// translator writes for naga), in its conditions and its code, each stage its own.
    #[test]
    fn a_post_shader_reads_its_version() {
        use sunrise_assets::resource_source::Resource;
        let vsh = "#version 330\nconst int seen = __VERSION__;\n\
            void main() { gl_Position = vec4(float(seen)); }";
        let fsh = "#version 150\n#if __VERSION__ != 150\n#error not 150\n#endif\n\
            #ifdef __VERSION__\nconst int seen = __VERSION__;\n#endif\n\
            #define NEXT (__VERSION__ + 1)\nconst int next = NEXT;\n\
            layout(location=0) out vec4 fragColor;\n\
            void main() { fragColor = vec4(float(seen + next)); }";
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            (
                "assets/custom/shaders/post/v.vsh".into(),
                Resource::bytes(vsh.as_bytes().to_vec()),
            ),
            (
                "assets/custom/shaders/post/v.fsh".into(),
                Resource::bytes(fsh.as_bytes().to_vec()),
            ),
        ]));
        let program = translate(&source, "custom:post/v", "custom:post/v", ANY).unwrap();
        let read = [
            constant(&program.vertex, "seen"),
            constant(&program.fragment, "seen"),
            constant(&program.fragment, "next"),
        ];
        assert_eq!(read, [330, 150, 151]);
    }

    /// A post shader's include reads the stage's `__VERSION__` in a condition that gates another
    /// include (pasted with every branch, then dropped by the directives), in `#elif`, in
    /// `#ifndef` and in its code; a post stage without `#version` reads 110.
    #[test]
    fn a_post_shaders_include_and_a_stage_without_version_read_it() {
        use sunrise_assets::resource_source::Resource;
        let a = "#if __VERSION__ == 150\n#include \"scale.glsl\"\n#endif\n\
            const int a = __VERSION__;\n\
            #if __VERSION__ == 100\nconst int e = 1;\n#elif __VERSION__ == 150\nconst int e = 2;\n#endif\n\
            #ifndef __VERSION__\nconst int n = 1;\n#else\nconst int n = 4;\n#endif\n";
        let vsh = "const int seen = __VERSION__;\n\
            void main() { gl_Position = vec4(float(seen)); }";
        let fsh = "#version 150\n#include <custom:a.glsl>\n\
            layout(location=0) out vec4 fragColor;\n\
            void main() { fragColor = vec4(SCALE * float(a + e + n)); }";
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            (
                "assets/custom/shaders/post/n.vsh".into(),
                Resource::bytes(vsh.as_bytes().to_vec()),
            ),
            (
                "assets/custom/shaders/post/n.fsh".into(),
                Resource::bytes(fsh.as_bytes().to_vec()),
            ),
            (
                "assets/custom/shaders/include/a.glsl".into(),
                Resource::bytes(a.as_bytes().to_vec()),
            ),
            (
                "assets/custom/shaders/include/scale.glsl".into(),
                Resource::bytes(b"#define SCALE 2.0\n".to_vec()),
            ),
        ]));
        let program = translate(&source, "custom:post/n", "custom:post/n", ANY).unwrap();
        let read = [
            constant(&program.vertex, "seen"),
            constant(&program.fragment, "a"),
            constant(&program.fragment, "e"),
            constant(&program.fragment, "n"),
        ];
        assert_eq!(read, [110, 150, 2, 4]);
    }

    /// A stage with an array too large to lay out (#4134) is the translator's refusal naming
    /// that stage, not a panic at the next array.
    #[test]
    fn an_array_too_large_to_lay_out_is_refused() {
        use sunrise_assets::resource_source::Resource;
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            ("assets/custom/shaders/post/big.vsh".into(), Resource::bytes(b"#version 330\nvoid main() { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }".to_vec())),
            ("assets/custom/shaders/post/big.fsh".into(), Resource::bytes(b"#version 330\nfloat big[0x7fffffff];\nfloat after[2];\nlayout(location=0) out vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }".to_vec())),
        ]));
        let Err(refused) = translate(&source, "custom:post/big", "custom:post/big", ANY) else {
            panic!("the fragment shader's oversized array is refused");
        };
        let displayed = refused.to_string();
        assert!(
            displayed.starts_with("custom:post/big: custom:shaders/post/big.fsh:2:"),
            "{displayed}"
        );
        assert!(displayed.contains(": fragment shader: "), "{displayed}");
    }

    /// `#moj_import`, as Polytone's wiki writes it, and its `polytone_globals.glsl` built in:
    /// the `PolyGlobals` block without the pack shipping the file, which the pack's own file
    /// replaces; any other missing import still fails.
    #[test]
    fn a_post_shader_imports_poly_globals_built_in() {
        use sunrise_assets::resource_source::Resource;
        let vsh = b"#version 330\nvoid main() { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }";
        let fsh = |import: &str| {
            format!(
                "#version 330\n#moj_import <{import}>\nlayout(location=0) out vec4 fragColor;\n\
                 void main() {{ fragColor = vec4(PolySunAngle); }}"
            )
            .into_bytes()
        };
        let pack = |import: &str, shipped: Option<&[u8]>| {
            let mut files = std::collections::BTreeMap::from([
                (
                    "assets/p/shaders/post/a.vsh".into(),
                    Resource::bytes(vsh.to_vec()),
                ),
                (
                    "assets/p/shaders/post/a.fsh".into(),
                    Resource::bytes(fsh(import)),
                ),
            ]);
            if let Some(text) = shipped {
                files.insert(
                    "assets/minecraft/shaders/include/polytone_globals.glsl".into(),
                    Resource::bytes(text.to_vec()),
                );
            }
            ResourceSource::indexed(files)
        };
        let blocks = |source: &ResourceSource| {
            translate(source, "p:post/a", "p:post/a", ANY).map(|p| {
                p.blocks
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>()
            })
        };
        let built_in = blocks(&pack("polytone_globals.glsl", None)).unwrap();
        assert_eq!(built_in, ["PolyGlobals"]);
        let own = b"layout(std140) uniform Own {\n    float PolySunAngle;\n};\n";
        let shipped = blocks(&pack("polytone_globals.glsl", Some(own))).unwrap();
        assert_eq!(shipped, ["Own"]);
        assert_eq!(
            blocks(&pack("other.glsl", None)),
            Err(imports(
                "p:post/a",
                "p:shaders/post/a.fsh",
                2,
                ImportRefusal::NotFound("minecraft:shaders/include/other.glsl".into())
            ))
        );
    }

    /// The post shaders' imports go through the core shaders' resolver: a quoted one from the
    /// stage's own folder, a `#moj_import` pasted once and an `#include` every time, a cycle
    /// refused, and the packs' `polytone_globals.glsl` that can't be read refused by its path
    /// instead of replaced by the built-in.
    #[test]
    fn post_imports_resolve_like_the_core_shaders() {
        use sunrise_assets::resource_source::Resource;
        let vsh = b"#version 330\nvoid main() { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }";
        let source = |fsh: &str, files: &[(&str, &[u8])]| {
            let mut all = std::collections::BTreeMap::from([
                (
                    "assets/p/shaders/post/a.vsh".into(),
                    Resource::bytes(vsh.to_vec()),
                ),
                (
                    "assets/p/shaders/post/a.fsh".into(),
                    Resource::bytes(fsh.as_bytes().to_vec()),
                ),
            ]);
            for (path, bytes) in files {
                all.insert((*path).to_owned(), Resource::bytes(bytes.to_vec()));
            }
            ResourceSource::indexed(all)
        };
        let translated = |fsh: &str, files: &[(&str, &[u8])]| {
            translate(&source(fsh, files), "p:post/a", "p:post/a", ANY).map(|_| ())
        };
        let main = "layout(location=0) out vec4 fragColor;\nvoid main() { fragColor = color; }\n";
        let color = b"const vec4 color = vec4(0.2, 0.4, 0.6, 1.0);\n".as_slice();
        // Quoted: beside the stage, not under shaders/include.
        let quoted = format!("#version 330\n#moj_import \"lib/c.glsl\"\n{main}");
        assert_eq!(
            translated(&quoted, &[("assets/p/shaders/post/lib/c.glsl", color)]),
            Ok(())
        );
        let elsewhere = translated(&quoted, &[("assets/p/shaders/include/lib/c.glsl", color)]);
        assert_eq!(
            elsewhere,
            Err(imports(
                "p:post/a",
                "p:shaders/post/a.fsh",
                2,
                ImportRefusal::NotFound("p:shaders/post/lib/c.glsl".into())
            ))
        );
        // Twice: imported once, included every time.
        let include = "assets/minecraft/shaders/include/c.glsl";
        let pasted = |directive: &str| {
            let fsh = format!("#version 330\n{directive} <c.glsl>\n{directive} <c.glsl>\n{main}");
            let text = stage(&source(&fsh, &[(include, color)]), "p:post/a", "fsh").unwrap();
            text.text().matches("const vec4 color").count()
        };
        assert_eq!((pasted("#moj_import"), pasted("#include")), (1, 2));
        // A file including itself is refused when it nests too deep.
        let looping = b"#include <loop.glsl>\n".as_slice();
        let cycle = translated(
            &format!("#version 330\n#include <loop.glsl>\n{main}"),
            &[("assets/minecraft/shaders/include/loop.glsl", looping)],
        );
        assert_eq!(
            cycle,
            Err(imports(
                "p:post/a",
                "minecraft:shaders/include/loop.glsl",
                1,
                ImportRefusal::TooDeep
            ))
        );
        // The packs' own globals that aren't text: refused by path, not the built-in.
        let globals = "assets/minecraft/shaders/include/polytone_globals.glsl";
        let reads = "#version 330\n#moj_import <polytone_globals.glsl>\n\
            layout(location=0) out vec4 fragColor;\nvoid main() { fragColor = vec4(PolySunAngle); }\n";
        assert_eq!(translated(reads, &[]), Ok(()));
        // Named by its path, so not the built-in standing in.
        let unreadable = translated(reads, &[(globals, &[0xff, 0xfe])]);
        assert!(matches!(
            unreadable,
            Err(Refused::Imports(_, ImportError { refusal: ImportRefusal::Unreadable(why), .. }))
                if why.contains(globals)
        ));
    }

    /// A pass declaring more blocks or samplers than the device binds to both stages is refused
    /// before any analysis, however few it reads; up to that many, used or not, it translates.
    /// So is a source with more `uniform` keywords than both caps together, before the rest of
    /// it is scanned.
    #[test]
    fn a_pass_declaring_past_the_device_is_refused_first() {
        use sunrise_assets::resource_source::Resource;
        let declarable = Declarable {
            blocks: 24,
            samplers: 4,
            interface: naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
        };
        let translated = |blocks: usize, samplers: usize| {
            let declared: String = (0..blocks)
                .map(|i| format!("layout(std140) uniform B{i} {{ float b{i}; }};\n"))
                .chain((0..samplers).map(|i| format!("uniform sampler2D S{i}Sampler;\n")))
                .collect();
            let fsh = format!(
                "#version 330\n{declared}layout(location=0) out vec4 fragColor;\n\
                 void main() {{ fragColor = vec4(0.0); }}\n"
            );
            let source = ResourceSource::indexed(std::collections::BTreeMap::from([
                (
                    "assets/p/shaders/post/a.vsh".into(),
                    Resource::bytes(
                        b"#version 330\nvoid main() { gl_Position = vec4(0.0); }".to_vec(),
                    ),
                ),
                (
                    "assets/p/shaders/post/a.fsh".into(),
                    Resource::bytes(fsh.into_bytes()),
                ),
            ]));
            translate(&source, "p:post/a", "p:post/a", declarable).map(|_| ())
        };
        assert_eq!(translated(24, 4), Ok(()));
        assert_eq!(
            translated(25, 0),
            Err(Refused::Declares {
                what: "uniform blocks",
                declared: 25,
                limit: 24
            })
        );
        assert_eq!(
            translated(0, 5),
            Err(Refused::Declares {
                what: "samplers",
                declared: 5,
                limit: 4
            })
        );
        // A megabyte of `uniform` keywords after a `)` (each a look back for its `layout`) is
        // refused at the first past what the two caps allow, before the rest is scanned.
        let crowded = format!(
            "#version 330\n{}layout(location=0) out vec4 fragColor;\n\
             void main() {{ fragColor = vec4(0.0); }}\n",
            "f()uniform sampler2D S;\n".repeat(40_000)
        );
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            (
                "assets/p/shaders/post/a.vsh".into(),
                Resource::bytes(b"#version 330\nvoid main() { gl_Position = vec4(0.0); }".to_vec()),
            ),
            (
                "assets/p/shaders/post/a.fsh".into(),
                Resource::bytes(crowded.into_bytes()),
            ),
        ]));
        assert_eq!(
            translate(&source, "p:post/a", "p:post/a", declarable).map(|_| ()),
            Err(Refused::Declares {
                what: "uniform declarations",
                declared: 29,
                limit: 28
            })
        );
    }

    #[test]
    fn vanilla_post_shaders_translate() {
        let Some(root) = root() else { return };
        let vs = ["minecraft:core/screenquad", "minecraft:post/rotscale"];
        let fs = [
            "bits",
            "blit",
            "box_blur",
            "color_convolve",
            "entity_outline_box_blur",
            "entity_sobel",
            "invert",
            "spiderclip",
        ];
        for f in fs {
            for v in vs {
                let name = format!("minecraft:post/{f}");
                let p = translate(&ResourceSource::directory(root.clone()), v, &name, ANY);
                // rotscale's outputs only match spiderclip, but each stage translates alone.
                if let Err(e) = &p {
                    panic!("{v} + {name}: {e}");
                }
            }
        }
    }

    /// `GlslCompiler` compiles a post pass's stages, as every program, with
    /// `RENDERPEARL_DEPTH_IS_ZERO_TO_ONE` on a backend whose depth runs from 0 to 1: of two
    /// blocks the fragment stage declares, it reads the one its `#ifdef` keeps, in either stage.
    #[test]
    fn a_post_shader_has_the_zero_to_one_depth_define() {
        use sunrise_assets::resource_source::Resource;
        let branch = |out: &str| {
            format!(
                "layout(std140) uniform ZeroToOne {{\n    vec4 zeroToOne;\n}};\n\
                 layout(std140) uniform MinusOneToOne {{\n    vec4 minusOneToOne;\n}};\n\
                 {out}\nvoid main() {{\n\
                 #ifdef RENDERPEARL_DEPTH_IS_ZERO_TO_ONE\n    v = zeroToOne;\n\
                 #else\n    v = minusOneToOne;\n#endif\n"
            )
        };
        let vsh = format!(
            "#version 330\n{}    gl_Position = vec4(0.0, 0.0, 0.0, 1.0);\n}}",
            branch("layout(location = 0) out vec4 v;")
        );
        let fsh = format!(
            "#version 330\nlayout(location = 0) in vec4 w;\nlayout(location = 0) out vec4 fragColor;\n{}    fragColor = v + w;\n}}",
            branch("vec4 v;")
        );
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            (
                "assets/p/shaders/post/a.vsh".into(),
                Resource::bytes(vsh.into_bytes()),
            ),
            (
                "assets/p/shaders/post/a.fsh".into(),
                Resource::bytes(fsh.into_bytes()),
            ),
        ]));
        let program = translate(&source, "p:post/a", "p:post/a", ANY).unwrap();
        let binding = |name: &str| program.blocks.iter().find(|(n, _)| n == name).unwrap().1;
        let reading = |name: &str| sunrise_glsl::stages_reading(&program.read, binding(name));
        assert_eq!(reading("ZeroToOne"), wgpu::ShaderStages::VERTEX_FRAGMENT);
        assert_eq!(reading("MinusOneToOne"), wgpu::ShaderStages::NONE);
    }

    #[test]
    fn sampler_and_block_bindings() {
        let src = "layout(std140) uniform A {\n float x;\n};\nuniform sampler2D InSampler; // c\nvoid main(){}\n";
        let d = declarations(src, usize::MAX).unwrap();
        assert_eq!(d.len(), 2);
        let expanded = |text: &str| {
            crate::core_shader::expand_imports(
                text,
                "p:shaders/post/a.fsh",
                "p:shaders/post/",
                &|_| Ok(None),
            )
            .unwrap()
        };
        let mut out = expanded(src);
        rewrite(&mut out, &[("A".into(), 0)], &[("InSampler".into(), 1)]);
        let out = out.text();
        assert!(out.contains("layout(std140, set = 0, binding = 0) uniform A {"));
        assert!(out.contains("binding = 1) uniform texture2D InSampler_hz_tex;"));
        assert!(out.contains("binding = 2) uniform sampler InSampler_hz_smp;"));
        let mut main = expanded("void main() {}");
        wrap_main(&mut main).unwrap();
        assert!(main.text().contains("hz_main"));
    }

    /// An error in a file an included file includes names that file and the line it is on, past
    /// the lines a sampler's rewrite adds before it, in the vertex stage (whose `main` is
    /// wrapped) and in the fragment stage; one in the stage's own file names it.
    #[test]
    fn an_error_in_a_nested_include_names_its_file_and_line() {
        use sunrise_assets::resource_source::Resource;
        let source = |vsh: &str, fsh: &str| {
            ResourceSource::indexed(std::collections::BTreeMap::from([
                (
                    "assets/p/shaders/post/a.vsh".into(),
                    Resource::bytes(vsh.as_bytes().to_vec()),
                ),
                (
                    "assets/p/shaders/post/a.fsh".into(),
                    Resource::bytes(fsh.as_bytes().to_vec()),
                ),
                (
                    "assets/p/shaders/include/outer.glsl".into(),
                    Resource::bytes(b"// outer\n#include \"inner/deep.glsl\"\n".to_vec()),
                ),
                (
                    "assets/p/shaders/include/inner/deep.glsl".into(),
                    Resource::bytes(
                        b"uniform sampler2D DeepSampler;\n/* two\n   lines */\nvec4 deep() { return broken; }\n"
                            .to_vec(),
                    ),
                ),
            ]))
        };
        let vsh = "#version 330\nvoid main() { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }\n";
        let fsh = "#version 330\nuniform sampler2D InSampler;\n#include <p:outer.glsl>\n\
            layout(location=0) out vec4 fragColor;\nvoid main() { fragColor = deep(); }\n";
        let located = |refused: Result<Program, Refused>| match refused {
            Err(Refused::Translator { stage, errors }) => {
                let at = errors.iter().map(|e| (e.file.clone(), e.line)).collect();
                (stage, at)
            }
            other => panic!("not the translator's refusal: {:?}", other.err()),
        };
        let deep = ("p:shaders/include/inner/deep.glsl".to_owned(), 4);
        let fragment = translate(&source(vsh, fsh), "p:post/a", "p:post/a", ANY);
        assert_eq!(
            located(fragment),
            ("p:post/a".to_owned(), vec![deep.clone()])
        );
        let vertex = "#version 330\n#include <p:outer.glsl>\n\
            void main() { gl_Position = deep(); }\n";
        let fsh_ok = "#version 330\nlayout(location=0) out vec4 fragColor;\n\
            void main() { fragColor = vec4(1.0); }\n";
        let refused = translate(&source(vertex, fsh_ok), "p:post/a", "p:post/a", ANY);
        assert_eq!(located(refused), ("p:post/a".to_owned(), vec![deep]));
        let own = "#version 330\nuniform sampler2D InSampler;\nlayout(location=0) out vec4 fragColor;\n\
            void main() { fragColor = missing; }\n";
        let refused = translate(&source(vsh, own), "p:post/a", "p:post/a", ANY);
        assert_eq!(
            located(refused),
            (
                "p:post/a".to_owned(),
                vec![("p:shaders/post/a.fsh".to_owned(), 4)]
            )
        );
    }

    /// An error naga's validator finds (the stage parses) in a file an include includes names
    /// that file and the line its span starts on.
    #[test]
    fn a_validation_error_in_a_nested_include_names_its_file_and_line() {
        use sunrise_assets::resource_source::Resource;
        let files = [
            (
                "assets/p/shaders/post/a.vsh",
                "#version 330\nvoid main() { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }\n",
            ),
            (
                "assets/p/shaders/post/a.fsh",
                "#version 330\n#include <p:outer.glsl>\nlayout(location=0) out vec4 fragColor;\n\
                 void main() { fragColor = vec4(pick()); }\n",
            ),
            (
                "assets/p/shaders/include/outer.glsl",
                "#include \"lib/pick.glsl\"\n",
            ),
            (
                "assets/p/shaders/include/lib/pick.glsl",
                "// pick\nfloat pick() {\n    float a[2] = float[2](1.0, 2.0);\n    return a[5];\n}\n",
            ),
        ];
        let source = ResourceSource::indexed(
            (files.iter())
                .map(|(p, t)| ((*p).to_owned(), Resource::bytes(t.as_bytes().to_vec())))
                .collect(),
        );
        let Err(Refused::Translator { stage, errors }) =
            translate(&source, "p:post/a", "p:post/a", ANY)
        else {
            panic!("not the translator's refusal");
        };
        assert_eq!(stage, "p:post/a");
        let located: Vec<_> = errors
            .iter()
            .map(|e| (e.file.as_str(), e.line, e.stage))
            .collect();
        assert_eq!(
            located,
            [(
                "p:shaders/include/lib/pick.glsl",
                4,
                Some(ShaderStage::Fragment)
            )]
        );
    }
}
