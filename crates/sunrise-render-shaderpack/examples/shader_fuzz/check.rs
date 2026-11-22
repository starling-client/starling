//! One case through the game's load path, twice, and the invariants every load must keep.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::hash::Hasher;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use sunrise_render_shaderpack::pack_atlas::{PackAtlas, game_inputs};
use sunrise_shaders::{Macros, Pack, Post, PostProgram};

use crate::cases::DIMENSIONS;

/// What a case asks of the checker besides its files.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub dim: usize,
    pub flat: bool,
    /// Programs that must load or be named by a note: the unmutated case loaded them and only
    /// their own stage files changed (see `expect` in main.rs).
    pub expect: Vec<String>,
    /// Bytes a stage file may grow to through `#include` and macros.
    pub size_limit: usize,
}

/// A broken invariant: its name, and what about it (a program, a location, an error kind).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Failure {
    pub invariant: String,
    pub detail: String,
}

impl Failure {
    pub fn new(invariant: &str, detail: impl Into<String>) -> Self {
        Self {
            invariant: invariant.to_owned(),
            detail: detail.into(),
        }
    }

    pub fn signature(&self) -> String {
        format!("{}|{}", self.invariant, self.detail)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub failures: Vec<Failure>,
    /// The programs that loaded, in the order `observe` lists them.
    pub loaded: Vec<String>,
    pub shadow_pass: bool,
}

/// What one load gave, reduced to what the invariants compare.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observed {
    pub loaded: Vec<String>,
    pub notes: Vec<String>,
    pub shadow_pass: bool,
    /// Each module that does not validate: `program stage`, and naga's error.
    pub invalid: Vec<(String, String)>,
    /// A hash of each part of the output, labelled.
    pub pieces: Vec<(String, u64)>,
}

/// The pack in `dir` loaded as the game loads a pack for a dimension (`shader_packs.rs`): the
/// standard macros and the renderer's at Mipmap Levels 4, `shaders.properties` with the
/// features it turns on, the dimension's folder, and the game's own inputs.
pub fn load(dir: &Path, request: &Request) -> Post {
    let pack = Pack::open(dir).expect("the case folder is a pack");
    let mut macros = Macros::standard();
    macros.define_renderer(4);
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let (id, legacy) = DIMENSIONS[request.dim];
    let folder = sunrise_shaders::dimension_folder_named(&pack, id, legacy);
    sunrise_shaders::load_post(&pack, &folder, &properties, &macros, &inputs(request.flat))
}

/// The game's inputs with a terrain atlas of 1024 x 1024 pixels in 16-pixel sprites: the
/// atlas's size only reaches the GLSL as constants. `flat` picks the game's flat draws.
pub fn inputs(flat: bool) -> sunrise_shaders::GameInputs {
    let terrain = PackAtlas::terrain_inputs_sized([1024, 1024], 64, 16, false, flat);
    game_inputs(Some(terrain), flat, &[3, 4])
}

/// Every program of `post` with a slot, by slot.
fn programs(post: &Post) -> Vec<(&'static str, &PostProgram)> {
    let slots: [(&'static str, &Option<PostProgram>); 23] = [
        ("terrain", &post.terrain),
        ("terrain_cutout", &post.terrain_cutout),
        ("water", &post.water),
        ("sky", &post.sky),
        ("sky_textured", &post.sky_textured),
        ("entities", &post.entities),
        ("block", &post.block),
        ("block_translucent", &post.block_translucent),
        ("entities_translucent", &post.entities_translucent),
        ("particles", &post.particles),
        ("particles_translucent", &post.particles_translucent),
        ("beacon_beam", &post.beacon_beam),
        ("damaged_block", &post.damaged_block),
        ("clouds", &post.clouds),
        ("lines", &post.lines),
        ("weather", &post.weather),
        ("hand", &post.hand),
        ("hand_water", &post.hand_water),
        ("shadow", &post.shadow),
        ("shadow_water", &post.shadow_water),
        ("shadow_water_plain", &post.shadow_water_plain),
        ("shadow_entities", &post.shadow_entities),
        ("shadow_block", &post.shadow_block),
    ];
    let mut out: Vec<(&'static str, &PostProgram)> =
        post.programs.iter().map(|p| ("pass", p)).collect();
    out.extend(post.shadow_comp.iter().map(|p| ("shadowcomp", p)));
    out.extend(
        slots
            .into_iter()
            .filter_map(|(slot, p)| Some((slot, p.as_ref()?))),
    );
    out
}

/// naga's error for `module`, validated as the translator validates (every check, every
/// capability), with its sources; `None` when it validates.
pub fn validate(module: &naga::Module) -> Option<String> {
    let flags = naga::valid::ValidationFlags::all();
    let caps = naga::valid::Capabilities::all();
    let error = naga::valid::Validator::new(flags, caps)
        .validate(module)
        .err()?;
    let mut text = error.as_inner().to_string();
    let mut source = std::error::Error::source(error.as_inner());
    while let Some(e) = source {
        let _ = write!(text, ": {e}");
        source = e.source();
    }
    Some(text)
}

/// Hashes what `Debug` writes, without holding the text.
struct HashWriter(std::hash::DefaultHasher);

impl std::fmt::Write for HashWriter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.write(s.as_bytes());
        Ok(())
    }
}

fn hash(value: &dyn std::fmt::Debug) -> u64 {
    let mut w = HashWriter(std::hash::DefaultHasher::new());
    let _ = write!(w, "{value:?}");
    w.0.finish()
}

pub fn observe(post: &Post) -> Observed {
    let mut seen = Observed {
        shadow_pass: post.shadow_pass,
        notes: post
            .notes
            .iter()
            .chain(&post.config.notes)
            .cloned()
            .collect(),
        ..Default::default()
    };
    seen.pieces.push(("notes".into(), hash(&seen.notes)));
    for (slot, p) in programs(post) {
        if !seen.loaded.contains(&p.name) {
            seen.loaded.push(p.name.clone());
        }
        let label = format!("{slot} {}", p.name);
        let parts = (
            &p.buffers,
            &p.flips,
            &p.blend,
            &p.buffer_blends,
            p.mipmaps,
            p.scale,
        );
        seen.pieces
            .push((format!("{label} settings"), hash(&parts)));
        let mut modules: Vec<(String, &naga::Module)> = Vec::new();
        if let Some(program) = &p.program {
            modules.push((format!("{label} vertex"), &program.vertex));
            modules.push((format!("{label} fragment"), &program.fragment));
            seen.pieces
                .push((format!("{label} interface"), hash(&program.interface)));
        }
        for (name, c) in &p.compute {
            modules.push((format!("{label} compute {name}"), &c.module));
            let groups = (&c.interface, c.work_groups, c.work_groups_render);
            seen.pieces
                .push((format!("{label} compute {name} interface"), hash(&groups)));
        }
        for (what, module) in modules {
            seen.pieces.push((what.clone(), hash(module)));
            if let Some(error) = validate(module) {
                seen.invalid.push((what, error));
            }
        }
    }
    seen
}

/// A note starts with a subject: a non-empty `` `key` ``, or a word without spaces (a program,
/// a key, a `file:line`) before its first `: `. Whether that word is one of the pack's names is
/// not checked. The one note about the whole pack, that it has no program for a kind of
/// geometry (`no program for terrain; nothing draws it`), refuses nothing the pack wrote.
pub fn named(note: &str) -> bool {
    if note.starts_with("no program for ") {
        return true;
    }
    if let Some(rest) = note.strip_prefix('`') {
        return rest.find('`').is_some_and(|end| end > 0);
    }
    let Some((subject, _)) = note.split_once(": ") else {
        return false;
    };
    let subject = subject.split(':').next().unwrap_or("");
    !subject.is_empty() && !subject.contains(char::is_whitespace)
}

/// Text that groups alike: numbers (handles, lines, sizes) read `#`, and at most 120 chars.
pub fn shape(text: &str) -> String {
    let mut out = String::new();
    let mut digits = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            if !digits {
                out.push('#');
            }
            digits = true;
        } else {
            digits = false;
            out.push(c);
        }
        if out.len() >= 120 {
            break;
        }
    }
    out
}

/// The invariants over two loads of the same case (each a panic message when it panicked).
pub fn judge(
    first: Result<Observed, String>,
    second: Result<Observed, String>,
    request: &Request,
) -> Outcome {
    let first = match first {
        Ok(o) => o,
        Err(panic) => {
            return Outcome {
                failures: vec![Failure::new("panic", panic)],
                ..Default::default()
            };
        }
    };
    let mut failures = Vec::new();
    for (what, error) in &first.invalid {
        let program = what.split(' ').nth(1).unwrap_or("");
        failures.push(Failure::new("naga", format!("{program}: {}", shape(error))));
    }
    for note in first.notes.iter().filter(|n| !named(n)) {
        failures.push(Failure::new(
            "refusal",
            format!("unnamed note: {}", shape(note)),
        ));
    }
    for name in &request.expect {
        let gone = !first.loaded.contains(name);
        let named = first
            .notes
            .iter()
            .any(|n| n.starts_with(&format!("{name}:")));
        // The shadow pass may be off now: its programs then don't load, by design.
        let shadow = name.starts_with("shadow") && !first.shadow_pass;
        if gone && !named && !shadow {
            failures.push(Failure::new(
                "refusal",
                format!("{name}: not loaded, no note"),
            ));
        }
    }
    match second {
        Err(panic) => failures.push(Failure::new("determinism", format!("second load: {panic}"))),
        Ok(second) => {
            let differs = first
                .pieces
                .iter()
                .zip(&second.pieces)
                .find(|(a, b)| a != b)
                .map(|(a, _)| a.0.clone())
                .or_else(|| {
                    (first.pieces.len() != second.pieces.len()).then(|| "program list".into())
                });
            if let Some(piece) = differs {
                failures.push(Failure::new("determinism", piece));
            }
        }
    }
    failures.sort();
    failures.dedup();
    Outcome {
        failures,
        loaded: first.loaded,
        shadow_pass: first.shadow_pass,
    }
}

thread_local! {
    /// Where the last panic on this thread happened, set by the worker's hook.
    pub static PANIC_AT: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(at) = PANIC_AT.with(|p| p.borrow_mut().take()) {
        return at;
    }
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    shape(&message)
}

/// Every stage file's preprocessed size, by the preprocessor the loader runs: the files over
/// `limit` bytes.
pub fn oversized(dir: &Path, request: &Request) -> Vec<Failure> {
    let Ok(pack) = Pack::open(dir) else {
        return Vec::new();
    };
    let mut macros = Macros::standard();
    macros.define_renderer(4);
    let _ = sunrise_shaders::read_properties(&pack, &mut macros);
    let stages = [".vsh", ".fsh", ".gsh", ".csh"];
    pack.all_files()
        .into_iter()
        .filter(|f| stages.iter().any(|e| f.ends_with(e)))
        .filter_map(|f| {
            let pp = sunrise_shaders::preprocess(&pack, &f, &macros);
            (pp.code.len() > request.size_limit).then(|| {
                let file = f.rsplit('/').next().unwrap_or(&f).to_owned();
                Failure::new("size", format!("{file}: preprocessed past the limit"))
            })
        })
        .collect()
}

/// The case in `dir` checked: no panic, every program valid, every refusal named, the same
/// output twice, stage files within the size limit. Time and memory bounds are the worker's.
pub fn check(dir: &Path, request: &Request) -> Outcome {
    let once =
        || catch_unwind(AssertUnwindSafe(|| observe(&load(dir, request)))).map_err(panic_text);
    let first = once();
    let second = if first.is_ok() {
        once()
    } else {
        Err(String::new())
    };
    let mut outcome = judge(first, second, request);
    if outcome.failures.iter().all(|f| f.invariant != "panic") {
        match catch_unwind(AssertUnwindSafe(|| oversized(dir, request))) {
            Ok(over) => outcome.failures.extend(over),
            Err(p) => outcome.failures.push(Failure::new("panic", panic_text(p))),
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(expect: &[&str]) -> Request {
        Request {
            dim: 0,
            flat: true,
            expect: expect.iter().map(|s| (*s).to_owned()).collect(),
            size_limit: 1 << 24,
        }
    }

    fn seen(loaded: &[&str], notes: &[&str]) -> Observed {
        Observed {
            loaded: loaded.iter().map(|s| (*s).to_owned()).collect(),
            notes: notes.iter().map(|s| (*s).to_owned()).collect(),
            shadow_pass: false,
            invalid: Vec::new(),
            pieces: vec![("composite fragment".into(), 7)],
        }
    }

    #[test]
    fn notes_name_their_subject() {
        for ok in [
            "`size.buffer.colortex9`: not a buffer and a size",
            "composite: skipped",
            "lib/a.glsl:12: unknown directive `#foo`",
            "gbuffers_water: uses a tessellation stage (unsupported); skipped",
            "`fzA` reads 0: unknown",
            "no program for translucent terrain; nothing draws it",
        ] {
            assert!(named(ok), "{ok}");
        }
        for unnamed in [
            "skipped",
            ": skipped",
            "``: not an image",
            " composite: x",
            "a b: c",
        ] {
            assert!(!named(unnamed), "{unnamed}");
        }
    }

    #[test]
    fn a_panic_is_the_only_failure_of_its_load() {
        let out = judge(
            Err("post.rs:1".into()),
            Err(String::new()),
            &request(&["final"]),
        );
        assert_eq!(out.failures, [Failure::new("panic", "post.rs:1")]);
    }

    #[test]
    fn a_program_dropped_without_a_note_is_an_unnamed_refusal() {
        let req = request(&["composite", "final", "shadow"]);
        let out = judge(
            Ok(seen(&["deferred"], &["final: skipped"])),
            Ok(seen(&["deferred"], &["final: skipped"])),
            &req,
        );
        // `final` is named and the shadow pass is off: only `composite` went silently.
        assert_eq!(
            out.failures,
            [Failure::new("refusal", "composite: not loaded, no note")]
        );
        let out = judge(
            Ok(seen(&[], &["skipped"])),
            Ok(seen(&[], &["skipped"])),
            &request(&[]),
        );
        assert_eq!(
            out.failures,
            [Failure::new("refusal", "unnamed note: skipped")]
        );
    }

    #[test]
    fn two_loads_that_differ_name_the_first_part() {
        let mut other = seen(&["composite"], &[]);
        other.pieces[0].1 = 8;
        let out = judge(Ok(seen(&["composite"], &[])), Ok(other), &request(&[]));
        assert_eq!(
            out.failures,
            [Failure::new("determinism", "composite fragment")]
        );
        let out = judge(
            Ok(seen(&["composite"], &[])),
            Err("x".into()),
            &request(&[]),
        );
        assert_eq!(
            out.failures,
            [Failure::new("determinism", "second load: x")]
        );
    }

    #[test]
    fn an_invalid_module_fails_validation_by_program() {
        // A uniform without a binding.
        let mut module = naga::Module::default();
        let ty = module.types.insert(
            naga::Type {
                name: None,
                inner: naga::TypeInner::Scalar(naga::Scalar::F32),
            },
            naga::Span::UNDEFINED,
        );
        module.global_variables.append(
            naga::GlobalVariable {
                name: Some("x".into()),
                space: naga::AddressSpace::Uniform,
                binding: None,
                ty,
                init: None,
                memory_decorations: Default::default(),
            },
            naga::Span::UNDEFINED,
        );
        let error = validate(&module).expect("a uniform needs a binding");
        assert_eq!(validate(&naga::Module::default()), None);
        let mut bad = seen(&["composite"], &[]);
        bad.invalid
            .push(("pass composite fragment".into(), error.clone()));
        let out = judge(Ok(bad.clone()), Ok(bad), &request(&[]));
        assert_eq!(
            out.failures,
            [Failure::new(
                "naga",
                format!("composite: {}", shape(&error))
            )]
        );
    }

    #[test]
    fn numbers_do_not_split_groups() {
        assert_eq!(shape("Function [12] at 3:4"), "Function [#] at #:#");
    }
}
