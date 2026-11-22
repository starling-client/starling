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
//! so `texCoord` reads the texel it would in vanilla.
use naga::ShaderStage;
use naga::front::glsl::{Frontend, Options};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use sunrise_assets::resource_source::ResourceSource;

/// One program, ready for wgpu.
pub struct Program {
    pub vertex: naga::Module,
    pub fragment: naga::Module,
    /// Every uniform block either stage declares, with its binding.
    pub blocks: Vec<(String, u32)>,
    /// Every sampler (`NameSampler`): the texture's binding; the sampler object is the next.
    pub samplers: Vec<(String, u32)>,
    /// What each stage reads (`core_shader::bindings_read`, from the stages' validation).
    pub read: Vec<(u32, wgpu::ShaderStages)>,
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
    /// A stage's includes (`core_shader::expand_imports`).
    Imports(String, crate::core_shader::ImportRefusal),
    /// The stages declare more uniform blocks or samplers than the device can bind.
    Declares {
        what: &'static str,
        declared: usize,
        limit: usize,
    },
    /// The translator's error.
    Translator(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(why) | Self::Translator(why) => f.write_str(why),
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
/// device binds to both (twice its per-stage limits). Checked before any analysis.
#[derive(Clone, Copy, Debug)]
pub struct Declarable {
    pub blocks: usize,
    pub samplers: usize,
}

impl Declarable {
    /// For `limits`: each stage's limit, for both stages.
    pub fn of(limits: &wgpu::Limits) -> Self {
        Self {
            blocks: 2 * limits.max_uniform_buffers_per_shader_stage as usize,
            samplers: 2 * limits.max_sampled_textures_per_shader_stage as usize,
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
/// bounded, and a missing `polytone_globals.glsl` the built-in one.
fn stage(root: &ResourceSource, id: &str, ext: &str) -> Result<String, Refused> {
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
    crate::core_shader::expand_imports(&text, &dir, &read)
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

/// The source with each declaration given its binding from the shared tables.
fn rewrite(src: &str, blocks: &[(String, u32)], samplers: &[(String, u32)]) -> String {
    let mut out = String::new();
    let mut pos = 0;
    // `translate` bounded the keywords before.
    for (start, end, ty, name, sampler) in declarations(src, usize::MAX).unwrap_or_default() {
        out.push_str(&src[pos..start]);
        pos = end;
        if sampler {
            let tex = samplers.iter().find(|(n, _)| *n == name).map_or(0, |s| s.1);
            let kind = if ty == "sampler2D" { "texture2D" } else { &ty };
            out.push_str(&format!(
                "layout(set = 0, binding = {tex}) uniform {kind} {name}_hz_tex; \
                 layout(set = 0, binding = {}) uniform sampler {name}_hz_smp;\n\
                 #define {name} sampler2D({name}_hz_tex, {name}_hz_smp)\n",
                tex + 1
            ));
        } else {
            let binding = blocks.iter().find(|(n, _)| *n == name).map_or(0, |s| s.1);
            out.push_str(&format!(
                "layout(std140, set = 0, binding = {binding}) uniform {name}"
            ));
        }
    }
    out.push_str(&src[pos..]);
    out
}

/// Replaces the word `main` of `void main(` with `hz_main`.
fn wrap_main(src: &str) -> Result<String, String> {
    let plain = blank_comments(src);
    let mut at = 0;
    while let Some(found) = plain[at..].find("main") {
        let p = at + found;
        at = p + 4;
        let b = plain.as_bytes();
        if (p > 0 && is_ident(b[p - 1])) || b.get(p + 4).is_some_and(|c| is_ident(*c)) {
            continue;
        }
        if plain[..p].trim_end().ends_with("void") && plain[p + 4..].trim_start().starts_with('(') {
            let mut out = String::with_capacity(src.len() + 80);
            out.push_str(&src[..p]);
            out.push_str("hz_main");
            out.push_str(&src[p + 4..]);
            out.push_str("\nvoid main() { hz_main(); gl_Position.y = -gl_Position.y; }\n");
            return Ok(out);
        }
    }
    Err("no main".into())
}

fn parse(
    stage: ShaderStage,
    text: &str,
) -> Result<(naga::Module, naga::valid::ModuleInfo), String> {
    let module = Frontend::default()
        .parse(&Options::from(stage), text)
        .map_err(|e| {
            e.errors
                .iter()
                .map(|e| e.kind.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|e| format!("{:?}", e.into_inner()))?;
    Ok((module, info))
}

/// Translates the vertex and fragment shader a pass names (`minecraft:post/invert`), refused
/// before any analysis when they declare more blocks or samplers than `declarable`.
pub fn translate(
    root: &ResourceSource,
    vertex: &str,
    fragment: &str,
    declarable: Declarable,
) -> Result<Program, Refused> {
    let vs = stage(root, vertex, "vsh")?;
    let fs = stage(root, fragment, "fsh")?;
    // Blocks and samplers are shared by name across the stages, so both agree on a binding.
    let mut blocks: Vec<(String, u32)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let keywords = declarable.blocks + declarable.samplers;
    for src in [&vs, &fs] {
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
    let vs = wrap_main(&rewrite(&vs, &blocks, &samplers)).map_err(Refused::Translator)?;
    let fs = rewrite(&fs, &blocks, &samplers);
    let vertex_module = parse(ShaderStage::Vertex, &format!("#version 450\n{vs}"))
        .map_err(|e| Refused::Translator(format!("{vertex}: {e}")))?;
    let fragment_module = parse(ShaderStage::Fragment, &format!("#version 450\n{fs}"))
        .map_err(|e| Refused::Translator(format!("{fragment}: {e}")))?;
    let read = crate::core_shader::bindings_read([
        (&vertex_module.0, &vertex_module.1),
        (&fragment_module.0, &fragment_module.1),
    ]);
    Ok(Program {
        vertex: vertex_module.0,
        fragment: fragment_module.0,
        blocks,
        samplers,
        read,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_shader::ImportRefusal;
    use std::path::Path;

    /// Room for any test shader's declarations.
    const ANY: Declarable = Declarable {
        blocks: 64,
        samplers: 64,
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
            Err(Refused::Imports(
                "p:post/a".into(),
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
            Err(Refused::Imports(
                "p:post/a".into(),
                ImportRefusal::NotFound("p:shaders/post/lib/c.glsl".into())
            ))
        );
        // Twice: imported once, included every time.
        let include = "assets/minecraft/shaders/include/c.glsl";
        let pasted = |directive: &str| {
            let fsh = format!("#version 330\n{directive} <c.glsl>\n{directive} <c.glsl>\n{main}");
            let text = stage(&source(&fsh, &[(include, color)]), "p:post/a", "fsh").unwrap();
            text.matches("const vec4 color").count()
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
            Err(Refused::Imports("p:post/a".into(), ImportRefusal::TooDeep))
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
            Err(Refused::Imports(_, ImportRefusal::Unreadable(why))) if why.contains(globals)
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

    #[test]
    fn sampler_and_block_bindings() {
        let src = "layout(std140) uniform A {\n float x;\n};\nuniform sampler2D InSampler; // c\nvoid main(){}\n";
        let d = declarations(src, usize::MAX).unwrap();
        assert_eq!(d.len(), 2);
        let out = rewrite(src, &[("A".into(), 0)], &[("InSampler".into(), 1)]);
        assert!(out.contains("layout(std140, set = 0, binding = 0) uniform A {"));
        assert!(out.contains("binding = 1) uniform texture2D InSampler_hz_tex;"));
        assert!(out.contains("binding = 2) uniform sampler InSampler_hz_smp;"));
        assert!(wrap_main("void main() {}").unwrap().contains("hz_main"));
    }
}
