//! Every program of a shader pack, loaded as the game loads it (`sunrise_shaders::load_post` with
//! the game's inputs), written as SPIR-V and hashed into a text manifest with the load's notes.
//! A change that must leave the translator's output alone (#5533 step 2) shows it by leaving the
//! manifests byte-identical.
//!
//! A manifest has one part per way the game reads the pack: its default options, then each
//! `profile.<name>` it declares, each without and with subgroup operations
//! (`Macros::define_subgroups`, which the game's `define_pack_macros` calls on a device with
//! them). A profile is read under each macro set and applied as the options screen applies it
//! (`choose_option`, which `ShaderOptions` calls, then `Pack::with_chosen`): a value its option
//! doesn't list is left out, and so is a value at its option's default. A part starts with the notes of reading
//! `shaders.properties`. Under it, one section per pack folder the game loads (the dimensions'
//! folders and a modded dimension's, each once) and per way the game draws terrain
//! ([`TerrainDraws`]): the load's notes, the custom uniforms' notes, a hash of `size.buffer.*`,
//! then each program's modules (vertex, fragment, a geometry program's vertex stage, compute) as
//! the SHA-256 of their SPIR-V words in little-endian bytes and their length, a hash of its
//! interface, of its settings (buffers, flips, blends, mipmaps, scale), and its translation notes.
//! A module that doesn't validate, or that the SPIR-V writer refuses, is hashed as naga's `Debug`
//! text of the module instead, and says so.
//!
//! - `spirv_dump_matches_the_committed_manifests` (runs in `verify`): the fuzzer's seed pack
//!   (`examples/shader_fuzz/seeds`), the geometry fixture (`tests/spirv_dump/geometry`) and the
//!   subgroup and profile fixture (`tests/spirv_dump/subgroups`), each dumped twice in one
//!   process, against `tests/spirv_dump/<pack>.txt`. To write a missing manifest, run this test
//!   alone with `SUNRISE_BLESS_SPIRV_DUMP=1`; it only creates missing files. A change meant to
//!   alter the output (a naga upgrade) deletes the manifests and blesses them again, and its PR
//!   says why they changed:
//!   `SUNRISE_BLESS_SPIRV_DUMP=1 cargo test -p sunrise-render-shaderpack --test spirv_dump -- --exact spirv_dump_matches_the_committed_manifests`
//! - `spirv_dump_real_packs` (manual): the packs in `SUNRISE_TEST_SHADER_PACKS` (zips or folders,
//!   and those one folder down), each dumped twice in one process, written to
//!   `SUNRISE_SPIRV_DUMP_OUT/<pack>.txt`. The output folder must not exist yet: the run makes it
//!   and writes nothing else. Compare two heads with `diff -r` of their output folders:
//!   `SUNRISE_TEST_SHADER_PACKS=<packs> SUNRISE_SPIRV_DUMP_OUT=<new folder> cargo test --release -p sunrise-render-shaderpack --test spirv_dump -- --ignored --exact spirv_dump_real_packs --nocapture`

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sunrise_render_shaderpack::pack_atlas::{TerrainDraws, deviceless_game_inputs};
use sunrise_shaders::{Macros, Pack, PostProgram, Properties, Screens, discover, load_post};

/// The game's dimensions, as their identifier and legacy number (`dimension_folder_named`), and
/// a modded dimension, which takes the folder `dimension.properties` maps `*` to.
const DIMENSIONS: [(&str, i32); 4] = [
    ("minecraft:overworld", 0),
    ("minecraft:the_nether", -1),
    ("minecraft:the_end", 1),
    ("sunrise:spirv_dump_modded", 0),
];

/// Every way the game gives terrain programs their vertices, each a different terrain module.
const DRAWS: [TerrainDraws; 4] = [
    TerrainDraws::Attributes,
    TerrainDraws::Pages,
    TerrainDraws::Normals,
    TerrainDraws::Flat,
];

/// The committed packs and the manifest of each, from the crate root.
const COMMITTED: [(&str, &str); 3] = [
    ("examples/shader_fuzz/seeds", "tests/spirv_dump/seeds.txt"),
    ("tests/spirv_dump/geometry", "tests/spirv_dump/geometry.txt"),
    (
        "tests/spirv_dump/subgroups",
        "tests/spirv_dump/subgroups.txt",
    ),
];

/// How many modules of each kind a dump wrote: vertex, fragment, geometry (a geometry program's
/// combined vertex stage), compute, and of those any that runs a subgroup operation; how many
/// were hashed as `Debug` text instead of SPIR-V; how many loads panicked; how many profiles the
/// pack declares, and how many of their values the options screen ignores (not in their option's
/// list).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Kinds {
    vertex: usize,
    fragment: usize,
    geometry: usize,
    compute: usize,
    subgroup: usize,
    not_spirv: usize,
    panics: usize,
    profiles: usize,
    ignored_profile_values: usize,
}

struct Dump {
    text: String,
    kinds: Kinds,
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn sha256_debug(value: &dyn std::fmt::Debug) -> String {
    sha256(format!("{value:?}").as_bytes())
}

/// SPIR-V options fixed here: naga's `Options::default()` adds `DEBUG` only in debug builds, so a
/// release run and a test run would write different words.
fn spirv_options() -> naga::back::spv::Options<'static> {
    use naga::back::spv::WriterFlags;
    naga::back::spv::Options {
        flags: WriterFlags::DEBUG
            | WriterFlags::ADJUST_COORDINATE_SPACE
            | WriterFlags::LABEL_VARYINGS
            | WriterFlags::CLAMP_FRAG_DEPTH,
        ..Default::default()
    }
}

/// Whether a statement of `module`'s functions or entry points is a subgroup operation.
fn runs_subgroup_operations(module: &naga::Module) -> bool {
    let bodies = module
        .functions
        .iter()
        .map(|(_, f)| &f.body)
        .chain(module.entry_points.iter().map(|e| &e.function.body));
    let mut blocks: Vec<&naga::Block> = bodies.collect();
    while let Some(block) = blocks.pop() {
        for statement in block.iter() {
            use naga::Statement as S;
            match statement {
                S::SubgroupBallot { .. }
                | S::SubgroupCollectiveOperation { .. }
                | S::SubgroupGather { .. } => return true,
                S::Block(b) => blocks.push(b),
                S::If { accept, reject, .. } => blocks.extend([accept, reject]),
                S::Loop {
                    body, continuing, ..
                } => blocks.extend([body, continuing]),
                S::Switch { cases, .. } => blocks.extend(cases.iter().map(|c| &c.body)),
                _ => {}
            }
        }
    }
    false
}

/// One module's manifest line: its SPIR-V's hash, or, when it doesn't validate or the writer
/// refuses it, the hash of its `Debug` text.
fn module_line(out: &mut String, kinds: &mut Kinds, what: &str, module: &naga::Module) {
    if runs_subgroup_operations(module) {
        kinds.subgroup += 1;
    }
    let validated = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(module);
    let words = match &validated {
        Ok(info) => naga::back::spv::write_vec(module, info, &spirv_options(), None)
            .map_err(|_| "spirv-refused"),
        Err(_) => Err("invalid"),
    };
    match words {
        Ok(words) => {
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            let _ = writeln!(
                out,
                "      {what} spirv {} words {}",
                sha256(&bytes),
                words.len()
            );
        }
        Err(why) => {
            kinds.not_spirv += 1;
            let _ = writeln!(out, "      {what} {why} module {}", sha256_debug(module));
        }
    }
}

fn program_lines(out: &mut String, kinds: &mut Kinds, index: usize, p: &PostProgram) {
    let _ = writeln!(out, "    program {index} {} {:?}", p.name, p.group);
    let settings = (
        &p.buffers,
        &p.flips,
        &p.blend,
        p.vanilla_sky_blends,
        &p.buffer_blends,
        p.mipmaps,
        p.scale,
    );
    let _ = writeln!(out, "      settings {}", sha256_debug(&settings));
    if let Some(program) = &p.program {
        if p.geometry_triangles > 0 {
            kinds.geometry += 1;
            let _ = writeln!(out, "      geometry triangles {}", p.geometry_triangles);
            module_line(out, kinds, "geometry-vertex", &program.vertex);
        } else {
            kinds.vertex += 1;
            module_line(out, kinds, "vertex", &program.vertex);
        }
        kinds.fragment += 1;
        module_line(out, kinds, "fragment", &program.fragment);
        let _ = writeln!(out, "      interface {}", sha256_debug(&program.interface));
        for n in &program.notes {
            let _ = writeln!(out, "      note {}:{}: {}", n.file, n.line, n.message);
        }
    }
    for (name, c) in &p.compute {
        kinds.compute += 1;
        module_line(out, kinds, &format!("compute {name}"), &c.module);
        let interface = (&c.interface, c.work_groups, c.work_groups_render);
        let _ = writeln!(
            out,
            "      compute {name} interface {}",
            sha256_debug(&interface)
        );
        for n in &c.notes {
            let _ = writeln!(
                out,
                "      compute {name} note {}:{}: {}",
                n.file, n.line, n.message
            );
        }
    }
}

/// The macros the game reads a pack with (`define_pack_macros`): the standard set, the
/// renderer's at Mipmap Levels 4, and the subgroup set on a device with subgroup operations.
fn game_macros(subgroups: bool) -> Macros {
    let mut macros = Macros::standard();
    macros.define_renderer(4);
    if subgroups {
        macros.define_subgroups();
    }
    macros
}

/// A profile as the options screen applies it from the defaults (`ShaderOptions::next_profile`):
/// each of its values through `choose_option`, which leaves out a value its option doesn't list
/// and a value at its option's default.
struct Profile {
    name: String,
    chosen: BTreeMap<String, String>,
    /// The values `choose_option` left out because their option doesn't list them.
    ignored: usize,
}

/// The pack's profiles, in its order, with `shaders.properties` read under `macros` as the options
/// screen reads it.
fn profiles(pack: &Pack, macros: &Macros) -> Vec<Profile> {
    let options = sunrise_shaders::options(pack);
    let text = pack.read_text("shaders.properties").unwrap_or_default();
    let macros = sunrise_shaders::properties_macros(pack, macros);
    let screens = Screens::parse(&Properties::parse(&text, &macros), &options);
    screens
        .profiles
        .into_iter()
        .map(|(name, values)| {
            let mut chosen = BTreeMap::new();
            let mut ignored = 0;
            for (option, value) in values {
                let listed = options
                    .get(&option)
                    .is_none_or(|o| o.values.contains(&value));
                if !sunrise_shaders::choose_option(&mut chosen, &options, &option, value) && !listed
                {
                    ignored += 1;
                }
            }
            Profile {
                name,
                chosen,
                ignored,
            }
        })
        .collect()
}

/// One part of the manifest: `pack` read with `macros`, every folder the game loads, once, under
/// each of [`DRAWS`]. A load that panics is a `panic` line, and the dump goes on.
fn dump_part(out: &mut String, kinds: &mut Kinds, pack: &Pack, mut macros: Macros) {
    let (properties, notes) = sunrise_shaders::read_properties(pack, &mut macros);
    for note in &notes {
        let _ = writeln!(out, "  note {note}");
    }
    for note in &properties.notes {
        let _ = writeln!(
            out,
            "  note shaders.properties:{}: {}",
            note.line, note.message
        );
    }
    let mut folders: Vec<String> = Vec::new();
    for (id, legacy) in DIMENSIONS {
        let folder = sunrise_shaders::dimension_folder_named(pack, id, legacy);
        if !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    for folder in &folders {
        for draws in DRAWS {
            let _ = writeln!(out, "  folder {folder:?} draws {draws:?}");
            let inputs = deviceless_game_inputs(properties.separate_ao(), draws);
            let loaded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                load_post(pack, folder, &properties, &macros, &inputs)
            }));
            let post = match loaded {
                Ok(post) => post,
                Err(payload) => {
                    let message = payload
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| payload.downcast_ref::<&str>().copied())
                        .unwrap_or("(no message)");
                    kinds.panics += 1;
                    let _ = writeln!(out, "    panic {message}");
                    continue;
                }
            };
            for note in post.notes.iter().chain(&post.config.notes) {
                let _ = writeln!(out, "    note {note}");
            }
            for note in &post.custom.notes {
                let _ = writeln!(out, "    custom note {note}");
            }
            let _ = writeln!(out, "    sizes {}", sha256_debug(&post.sizes));
            for (index, p) in post.every_program().enumerate() {
                program_lines(out, kinds, index, p);
            }
        }
    }
}

/// The manifest of the pack at `path`: its default options, then each of its profiles, each
/// without and with the subgroup macros. The profiles are read under each macro set: a profile the
/// pack declares under one only is `absent` under the other, and one whose values differ under
/// the subgroup macros says so (`options with subgroups`).
fn dump(path: &Path) -> Result<Dump, String> {
    let base = Pack::open(path).map_err(|e| e.to_string())?;
    let read = [false, true].map(|subgroups| profiles(&base, &game_macros(subgroups)));
    let mut names: Vec<&str> = Vec::new();
    for p in read.iter().flatten() {
        if !names.contains(&p.name.as_str()) {
            names.push(&p.name);
        }
    }
    let mut kinds = Kinds {
        profiles: names.len(),
        ignored_profile_values: read.iter().flatten().map(|p| p.ignored).sum(),
        ..Kinds::default()
    };
    let none = BTreeMap::new();
    let mut choices: Vec<(String, [Option<&BTreeMap<String, String>>; 2])> =
        vec![("defaults".to_owned(), [Some(&none), Some(&none)])];
    for name in names {
        let chosen = |i: usize| read[i].iter().find(|p| p.name == name).map(|p| &p.chosen);
        choices.push((format!("profile.{name}"), [chosen(0), chosen(1)]));
    }
    let mut out = String::new();
    for (label, both) in choices {
        match both[0] {
            Some(c) => {
                let _ = writeln!(out, "options {label} {c:?}");
            }
            None => {
                let _ = writeln!(out, "options {label}");
            }
        }
        for (subgroups, chosen) in [false, true].into_iter().zip(both) {
            let _ = writeln!(out, "subgroups {subgroups}");
            let Some(chosen) = chosen else {
                let _ = writeln!(out, "  absent");
                continue;
            };
            if subgroups && both[0] != Some(chosen) {
                let _ = writeln!(out, "  options with subgroups {chosen:?}");
            }
            let pack = Pack::open(path)
                .map_err(|e| e.to_string())?
                .with_chosen(chosen.clone());
            dump_part(&mut out, &mut kinds, &pack, game_macros(subgroups));
        }
    }
    Ok(Dump { text: out, kinds })
}

/// Why a pack has no manifest.
#[derive(Debug)]
enum NoDump {
    /// It doesn't open, as `Pack::open` says.
    Open(String),
    /// Two dumps in one process wrote different bytes.
    Differ,
}

/// `dump` twice: the second must write the same bytes as the first.
fn dump_twice(path: &Path) -> Result<Dump, NoDump> {
    let first = dump(path).map_err(NoDump::Open)?;
    let second = dump(path).map_err(NoDump::Open)?;
    if first.text != second.text {
        return Err(NoDump::Differ);
    }
    Ok(first)
}

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
#[ignore = "daily"]
fn spirv_dump_matches_the_committed_manifests() {
    let bless = std::env::var("SUNRISE_BLESS_SPIRV_DUMP").as_deref() == Ok("1");
    let mut total = Kinds::default();
    let mut mismatches = Vec::new();
    for (pack, manifest) in COMMITTED {
        let dump = dump_twice(&crate_root().join(pack)).unwrap_or_else(|e| panic!("{pack}: {e:?}"));
        assert!(
            dump.kinds.not_spirv == 0 && dump.kinds.panics == 0,
            "{pack}: every committed load runs and every module writes as SPIR-V\n{}",
            dump.text
        );
        total.vertex += dump.kinds.vertex;
        total.fragment += dump.kinds.fragment;
        total.geometry += dump.kinds.geometry;
        total.compute += dump.kinds.compute;
        total.subgroup += dump.kinds.subgroup;
        total.profiles += dump.kinds.profiles;
        let golden = crate_root().join(manifest);
        if bless {
            // Creating is the check: an existing manifest is never opened for writing.
            match std::fs::File::create_new(&golden) {
                Ok(mut file) => {
                    std::io::Write::write_all(&mut file, dump.text.as_bytes())
                        .unwrap_or_else(|e| panic!("{manifest}: {e}"));
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => panic!("{manifest}: {e}"),
            }
        }
        let expected = std::fs::read(&golden).unwrap_or_else(|e| {
            panic!("{manifest}: {e} (write it with SUNRISE_BLESS_SPIRV_DUMP=1)")
        });
        if expected != dump.text.as_bytes() {
            let expected = String::from_utf8_lossy(&expected);
            let first = expected
                .lines()
                .zip(dump.text.lines())
                .position(|(a, b)| a != b)
                .unwrap_or(expected.lines().count().min(dump.text.lines().count()));
            mismatches.push(format!(
                "{manifest}: differs from line {}\n  committed: {:?}\n  now:       {:?}",
                first + 1,
                expected.lines().nth(first),
                dump.text.lines().nth(first)
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    // Every kind of program the translator writes is in the committed packs, a subgroup
    // operation among them, and a pack with a profile.
    assert!(
        total.vertex > 0
            && total.fragment > 0
            && total.geometry > 0
            && total.compute > 0
            && total.subgroup > 0
            && total.profiles > 0,
        "a kind of module is missing from the committed packs: {total:?}"
    );
}

/// The packs of `dir` and of the folders right inside it, as `real_packs_read` finds them.
fn real_packs(dir: &Path) -> Vec<sunrise_shaders::Found> {
    let mut found = discover(dir);
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !p.join("shaders").is_dir())
        .collect();
    subs.sort();
    for sub in subs {
        found.extend(discover(&sub));
    }
    found
}

#[test]
#[ignore = "needs SUNRISE_TEST_SHADER_PACKS"]
fn spirv_dump_real_packs() {
    let packs = std::env::var_os("SUNRISE_TEST_SHADER_PACKS").expect("SUNRISE_TEST_SHADER_PACKS");
    let out = std::env::var_os("SUNRISE_SPIRV_DUMP_OUT").expect("SUNRISE_SPIRV_DUMP_OUT");
    let out = PathBuf::from(out);
    // A folder of this run's own: an existing one is refused, never written into.
    std::fs::create_dir(&out).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    let (mut dumped, mut skipped, mut differ) = (0, Vec::new(), Vec::new());
    for f in real_packs(Path::new(&packs)) {
        if let Some(problem) = &f.problem {
            skipped.push(format!("{}: {problem}", f.name));
            continue;
        }
        // The manifest's name is the pack's own file or folder name, kept inside `out`.
        let plain = Path::new(&f.name)
            .file_name()
            .is_some_and(|n| n == f.name.as_str());
        if !plain || f.name.starts_with('.') {
            skipped.push(format!("{}: not a plain file name", f.name));
            continue;
        }
        let start = std::time::Instant::now();
        // A panic outside a load (reading the pack's properties) costs only this pack.
        let twice = std::panic::catch_unwind(|| dump_twice(&f.path));
        let dump = match twice {
            Ok(Ok(dump)) => dump,
            Ok(Err(NoDump::Differ)) => {
                differ.push(f.name.clone());
                continue;
            }
            Ok(Err(NoDump::Open(e))) => {
                skipped.push(format!("{}: {e}", f.name));
                continue;
            }
            Err(_) => {
                skipped.push(format!("{}: panicked outside a load", f.name));
                continue;
            }
        };
        let file = out.join(format!("{}.txt", f.name));
        match std::fs::File::create_new(&file) {
            Ok(mut w) => std::io::Write::write_all(&mut w, dump.text.as_bytes())
                .unwrap_or_else(|e| panic!("{}: {e}", file.display())),
            Err(e) => {
                skipped.push(format!("{}: {e}", f.name));
                continue;
            }
        }
        dumped += 1;
        println!(
            "{}: {:?}, dumped twice in {:.1} s",
            f.name,
            dump.kinds,
            start.elapsed().as_secs_f64()
        );
    }
    println!(
        "dumped {dumped} packs, skipped {}, not deterministic {}",
        skipped.len(),
        differ.len()
    );
    for s in &skipped {
        println!("  skipped {s}");
    }
    assert!(
        differ.is_empty(),
        "two dumps in one process differ: {differ:?}"
    );
    assert!(dumped > 0, "no pack was dumped");
}
