//! Cases: a corpus pack cut down to one or two programs (and what they include), then a few
//! mutations, each a named step re-derived from its own seed so a shrinker can drop any of them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use sunrise_shaders::Pack;

/// SplitMix64, as `crates/sunrise-core/examples/jmath_fuzz.rs` draws its inputs.
pub struct Stream(pub u64);

impl Stream {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n` > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// Case `index` of the run seeded `seed`: the seed `--replay` takes.
pub fn case_seed(seed: u64, index: u64) -> u64 {
    Stream(seed.wrapping_mul(1_000_003).wrapping_add(index)).next()
}

/// The dimensions the game asks a pack's folder for (`dimension_folder_named`).
pub const DIMENSIONS: [(&str, i32); 3] = [
    ("minecraft:overworld", 0),
    ("minecraft:the_nether", -1),
    ("minecraft:the_end", 1),
];

/// A corpus pack: its files by path inside `shaders`, and per dimension the folder the game
/// loads and the programs in it: both stages there, or compute shaders only.
pub struct Source {
    pub name: String,
    pub files: BTreeMap<String, Vec<u8>>,
    pub folders: [(String, Vec<String>); 3],
}

impl Source {
    pub fn open(path: &Path) -> Option<Source> {
        let pack = Pack::open(path).ok()?;
        let files: BTreeMap<String, Vec<u8>> = pack
            .all_files()
            .into_iter()
            .filter_map(|f| Some((f.clone(), pack.read(&f)?)))
            .collect();
        let folders = DIMENSIONS.map(|(id, legacy)| {
            let folder = sunrise_shaders::dimension_folder_named(&pack, id, legacy);
            let listed = pack.list(&folder);
            let has = |n: &str| pack.exists(&in_folder(&folder, n));
            // Programs with both stages (a geometry or tessellation stage goes with them), and
            // composite-style programs of compute shaders only (spec 1.3, 2.2).
            let mut programs: Vec<String> = listed
                .iter()
                .filter_map(|f| f.strip_suffix(".fsh"))
                .filter(|n| has(&format!("{n}.vsh")))
                .map(str::to_owned)
                .collect();
            programs.extend(
                listed
                    .iter()
                    .filter_map(|f| f.strip_suffix(".csh"))
                    .map(compute_owner)
                    .filter(|n| compute_only(n) && !has(&format!("{n}.fsh")))
                    .map(str::to_owned),
            );
            programs.sort();
            programs.dedup();
            (folder, programs)
        });
        Some(Source {
            name: pack.name().to_owned(),
            files,
            folders,
        })
    }
}

/// The program a compute shader belongs to: `composite1_b` → `composite1`.
fn compute_owner(stem: &str) -> &str {
    match stem.rsplit_once('_') {
        Some((owner, c)) if c.len() == 1 && c.as_bytes()[0].is_ascii_lowercase() => owner,
        _ => stem,
    }
}

/// A program that may be compute shaders alone: `setup`, and the composite-style passes
/// with their numbers (spec 2.2).
fn compute_only(name: &str) -> bool {
    let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
    [
        "setup",
        "begin",
        "prepare",
        "deferred",
        "composite",
        "shadowcomp",
    ]
    .contains(&base)
}

pub fn in_folder(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// The committed seed pack, then each pack (zip or folder) of `corpus`, by name.
pub fn corpus(corpus: Option<&Path>) -> Vec<Source> {
    let seeds = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/shader_fuzz/seeds");
    let mut out = vec![Source::open(&seeds).expect("the seed pack opens")];
    if let Some(dir) = corpus {
        for found in sunrise_shaders::discover(dir) {
            if found.problem.is_none()
                && let Some(source) = Source::open(&found.path)
            {
                out.push(source);
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Declaration,
    Include,
    Preprocessor,
    ArraySize,
    Collide,
    Numeric,
    Truncate,
    Encoding,
    Duplicate,
    Reorder,
    Properties,
}

pub const KINDS: [Kind; 11] = [
    Kind::Declaration,
    Kind::Include,
    Kind::Preprocessor,
    Kind::ArraySize,
    Kind::Collide,
    Kind::Numeric,
    Kind::Truncate,
    Kind::Encoding,
    Kind::Duplicate,
    Kind::Reorder,
    Kind::Properties,
];

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Declaration => "declaration",
            Kind::Include => "include",
            Kind::Preprocessor => "preprocessor",
            Kind::ArraySize => "array_size",
            Kind::Collide => "collide",
            Kind::Numeric => "numeric",
            Kind::Truncate => "truncate",
            Kind::Encoding => "encoding",
            Kind::Duplicate => "duplicate",
            Kind::Reorder => "reorder",
            Kind::Properties => "properties",
        }
    }
}

/// One mutation: its kind, the file it edits and the seed of its choices.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub kind: Kind,
    pub file: String,
    pub seed: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    pub seed: u64,
    pub pack: String,
    /// Index into [`DIMENSIONS`].
    pub dim: usize,
    /// Terrain as flat draws give it (vertices pulled) or as grid draws do (attributes).
    pub flat: bool,
    pub programs: Vec<String>,
    pub folder: String,
    pub files: BTreeMap<String, Vec<u8>>,
    pub steps: Vec<Step>,
}

impl Case {
    /// The pack the case loads: its files with every step applied, in order.
    pub fn materialize(&self) -> BTreeMap<String, Vec<u8>> {
        self.materialize_first(self.steps.len())
    }

    pub fn materialize_first(&self, n: usize) -> BTreeMap<String, Vec<u8>> {
        let mut files = self.files.clone();
        for step in &self.steps[..n] {
            apply(step, &mut files);
        }
        files
    }

    /// The case's own stage files of `program`: what a step may edit and still leave every other
    /// program as it was.
    pub fn owns(&self, program: &str, path: &str) -> bool {
        let Some(rest) = path
            .strip_prefix(&in_folder(&self.folder, ""))
            .filter(|_| !self.folder.is_empty())
            .or((self.folder.is_empty()).then_some(path))
        else {
            return false;
        };
        let Some((stem, ext)) = rest.rsplit_once('.') else {
            return false;
        };
        let compute = ext == "csh"
            && (stem == program
                || stem
                    .strip_prefix(program)
                    .and_then(|s| s.strip_prefix('_'))
                    .is_some_and(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_lowercase()));
        (stem == program && ["vsh", "fsh", "gsh", "tcs", "tes"].contains(&ext)) || compute
    }
}

const STAGES: [&str; 6] = ["vsh", "fsh", "gsh", "csh", "tcs", "tes"];

fn is_source(path: &str) -> bool {
    !path.ends_with(".properties")
        && !path.ends_with(".lang")
        && !path.ends_with(".json")
        && !path.ends_with(".png")
        && !path.ends_with(".bin")
        && !path.ends_with(".dat")
}

/// The include targets of `text` in file `from`, as the preprocessor resolves them.
fn includes(from: &str, text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim_start().strip_prefix('#')?.trim_start();
            let target = rest.strip_prefix("include")?.trim();
            let target = target.trim_matches(|c| c == '"' || c == '<' || c == '>');
            let joined = match target.strip_prefix('/') {
                Some(abs) => abs.to_owned(),
                None => in_folder(from.rsplit_once('/').map_or("", |(d, _)| d), target),
            };
            normalize(&joined)
        })
        .collect()
}

fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// The files of `source` a case on `programs` (in `folder`) needs: their stage files, what those
/// include (transitively), and the `.properties` files at the root.
pub fn subset(source: &Source, folder: &str, programs: &[String]) -> BTreeMap<String, Vec<u8>> {
    let mut wanted: BTreeSet<String> = source
        .files
        .keys()
        .filter(|f| !f.contains('/') && f.ends_with(".properties"))
        .cloned()
        .collect();
    let mut queue: Vec<String> = Vec::new();
    for p in programs {
        for ext in STAGES {
            queue.push(in_folder(folder, &format!("{p}.{ext}")));
        }
        for c in 'a'..='z' {
            queue.push(in_folder(folder, &format!("{p}_{c}.csh")));
        }
    }
    while let Some(path) = queue.pop() {
        let Some(bytes) = source.files.get(&path) else {
            continue;
        };
        if !wanted.insert(path.clone()) {
            continue;
        }
        queue.extend(includes(&path, &String::from_utf8_lossy(bytes)));
    }
    wanted
        .into_iter()
        .filter_map(|p| Some((p.clone(), source.files.get(&p)?.clone())))
        .collect()
}

/// The case of seed `seed`: a pack of the corpus, a dimension, one or two of its programs and
/// one to six steps. `steps: false` leaves the files as the pack has them.
pub fn generate(corpus: &[Source], seed: u64, steps: bool) -> Option<Case> {
    let mut s = Stream(seed);
    let source = &corpus[s.below(corpus.len())];
    let dim = s.below(DIMENSIONS.len());
    let (folder, programs) = &source.folders[dim];
    if programs.is_empty() {
        return None;
    }
    let mut chosen = vec![s.pick(programs).clone()];
    if s.chance(3) {
        let other = s.pick(programs).clone();
        if other != chosen[0] {
            chosen.push(other);
        }
    }
    chosen.sort();
    let files = subset(source, folder, &chosen);
    let mut case = Case {
        seed,
        pack: source.name.clone(),
        dim,
        flat: s.chance(2),
        programs: chosen,
        folder: folder.clone(),
        files,
        steps: Vec::new(),
    };
    if steps {
        let most = if s.chance(4) { 6 } else { 2 };
        let count = 1 + s.below(most);
        let mut current = case.files.clone();
        for _ in 0..count {
            let kind = *s.pick(&KINDS);
            let file = if kind == Kind::Properties {
                "shaders.properties".to_owned()
            } else {
                let sources: Vec<&String> = current.keys().filter(|f| is_source(f)).collect();
                // Mostly the programs' own stage files, else what they include.
                let own: Vec<&String> = sources
                    .iter()
                    .copied()
                    .filter(|f| case.programs.iter().any(|p| case.owns(p, f)))
                    .collect();
                if !own.is_empty() && !s.chance(3) {
                    (*s.pick(&own)).clone()
                } else if !sources.is_empty() {
                    (*s.pick(&sources)).clone()
                } else {
                    continue;
                }
            };
            let step = Step {
                kind,
                file,
                seed: s.next(),
            };
            apply(&step, &mut current);
            case.steps.push(step);
        }
    }
    Some(case)
}

// ---------------------------------------------------------------------------------------------
// The mutations. Each reads the file as it is now and edits it (and may add files); a step whose
// file is gone does nothing.

pub fn apply(step: &Step, files: &mut BTreeMap<String, Vec<u8>>) {
    let mut s = Stream(step.seed);
    let text = match files.get(&step.file) {
        Some(b) => String::from_utf8_lossy(b).into_owned(),
        None if step.kind == Kind::Properties => String::new(),
        None => return,
    };
    let out: Vec<u8> = match step.kind {
        Kind::Declaration => declaration(&mut s, &step.file, &text).into_bytes(),
        Kind::Include => include(&mut s, &step.file, &text, files).into_bytes(),
        Kind::Preprocessor => preprocessor(&mut s, &text).into_bytes(),
        Kind::ArraySize => array_size(&mut s, &text).into_bytes(),
        Kind::Collide => {
            // A rename runs through every source file, so the program stays consistent.
            if let Some((from, to)) = rename(&mut s, &text) {
                for (path, bytes) in files.iter_mut() {
                    if is_source(path) && path != &step.file {
                        let t = String::from_utf8_lossy(bytes);
                        *bytes = replace_word(&t, &from, to).into_bytes();
                    }
                }
                replace_word(&text, &from, to).into_bytes()
            } else {
                collide_declare(&mut s, &step.file, &text).into_bytes()
            }
        }
        Kind::Numeric => numeric(&mut s, &text).into_bytes(),
        Kind::Truncate => truncate(&mut s, &text).into_bytes(),
        Kind::Encoding => encoding(&mut s, &text),
        Kind::Duplicate => duplicate(&mut s, &text).into_bytes(),
        Kind::Reorder => reorder(&mut s, &text).into_bytes(),
        Kind::Properties => properties(&mut s, &text).into_bytes(),
    };
    files.insert(step.file.clone(), out);
}

fn lines(text: &str) -> Vec<String> {
    text.split('\n').map(str::to_owned).collect()
}

fn join(lines: &[String]) -> String {
    lines.join("\n")
}

/// Lines at global scope (brace depth 0 before them, after the leading directives), where a
/// declaration can go.
fn global_points(lines: &[String]) -> Vec<usize> {
    let mut depth = 0i32;
    let mut points = Vec::new();
    let mut header = true;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        let code = t.split("//").next().unwrap_or("");
        if header && (t.starts_with("#version") || t.starts_with("#extension") || t.is_empty()) {
            continue;
        }
        header = false;
        if depth == 0 && !t.starts_with('#') {
            let previous = lines[..i]
                .iter()
                .rev()
                .map(|l| l.trim())
                .find(|l| !l.is_empty());
            if previous.is_none_or(|p| {
                p.ends_with(';') || p.ends_with('}') || p.starts_with('#') || p.ends_with("*/")
            }) {
                points.push(i);
            }
        }
        depth += code.matches('{').count() as i32 - code.matches('}').count() as i32;
        depth = depth.max(0);
    }
    if points.is_empty() {
        points.push(lines.len());
    }
    points
}

/// The line just inside `main`'s body, if the file has one.
fn main_body(lines: &[String]) -> Option<usize> {
    let start = lines.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with("void main") || t.starts_with("void  main")
    })?;
    let open = (start..lines.len()).find(|&i| lines[i].contains('{'))?;
    Some(open + 1)
}

fn insert_global(s: &mut Stream, text: &str, decl: &str, used: Option<&str>) -> String {
    let mut ls = lines(text);
    if let (Some(u), Some(at)) = (used, main_body(&ls)) {
        ls.insert(at, format!("    {u}"));
    }
    let points = global_points(&ls);
    let at = *s.pick(&points);
    // Before `main`, so the use sees it.
    let at = match main_body(&ls) {
        Some(m) if at >= m => *points.iter().filter(|&&p| p < m).max().unwrap_or(&at),
        _ => at,
    };
    ls.insert(at.min(ls.len()), decl.to_owned());
    join(&ls)
}

fn vertex(file: &str) -> bool {
    file.ends_with(".vsh") || file.ends_with(".gsh")
}

/// Declarations in every form the scanner reads: mixed declarators, struct types declared
/// with variables, anonymous structs, interface, uniform and buffer blocks, arrays on the type
/// or the name; or a global the pack wrote, rewritten into one of those forms.
fn declaration(s: &mut Stream, file: &str, text: &str) -> String {
    let io = if vertex(file) { "out" } else { "in" };
    let forms: [(String, &str); 12] = [
        (
            "float fz_a, fz_b = 1.0, fz_c[2];".into(),
            "fz_a = fz_b + fz_c[1];",
        ),
        (
            "const float fz_k[2] = float[](1.0, 2.0), fz_j = fz_k[1];".into(),
            "float fz_u = fz_j;",
        ),
        (
            "struct FzS { vec3 p; float q[2]; } fz_s, fz_t[2];".into(),
            "fz_s.p = vec3(fz_t[1].q[0]);",
        ),
        (
            "struct { float x; vec2 y[3]; } fz_anon, fz_anon2;".into(),
            "fz_anon.x = fz_anon2.y[2].x;",
        ),
        (
            "uniform FzBlock { mat4 fz_m; vec4 fz_v[4]; float fz_f, fz_g; };".into(),
            "vec4 fz_w = fz_m * fz_v[3] * fz_g;",
        ),
        (
            "uniform FzNamed { vec4 v[2]; } fz_named;".into(),
            "vec4 fz_w2 = fz_named.v[1];",
        ),
        (
            "layout(std430, binding = 3) buffer FzBuf { uint fz_count; vec4 fz_data[]; } fz_buf;"
                .into(),
            "fz_buf.fz_data[0] = vec4(float(fz_buf.fz_count));",
        ),
        (
            "layout(std430, binding = 4) buffer FzBuf2 { int fz_items[]; };".into(),
            "fz_items[0] = 1;",
        ),
        (
            "float[3] fz_on_type, fz_on_both[2];".into(),
            "fz_on_type[2] = fz_on_both[1][0];",
        ),
        (
            format!("{io} FzIface {{ vec4 c; vec2 uv[2]; }} fz_io;"),
            if vertex(file) {
                "fz_io.c = vec4(1.0); fz_io.uv[1] = vec2(0.0);"
            } else {
                "vec4 fz_r = fz_io.c;"
            },
        ),
        (
            "varying vec3 fz_var1, fz_var2[2];".into(),
            if vertex(file) {
                "fz_var1 = fz_var2[1] = vec3(0.5);"
            } else {
                "vec3 fz_r2 = fz_var1 + fz_var2[1];"
            },
        ),
        (
            "uniform sampler2D fz_tex1, fz_tex2;".into(),
            "vec4 fz_t1 = texture2D(fz_tex1, vec2(0.0)) + texture2D(fz_tex2, vec2(0.5));",
        ),
    ];
    if s.chance(3)
        && let Some(rewritten) = rewrite_global(s, text)
    {
        return rewritten;
    }
    let (decl, used) = s.pick(&forms);
    let used = (!s.chance(4)).then_some(*used);
    insert_global(s, text, decl, used)
}

const TYPES: [&str; 22] = [
    "float",
    "int",
    "uint",
    "bool",
    "vec2",
    "vec3",
    "vec4",
    "ivec2",
    "ivec3",
    "ivec4",
    "uvec2",
    "uvec3",
    "uvec4",
    "mat2",
    "mat3",
    "mat4",
    "sampler2D",
    "sampler3D",
    "usampler2D",
    "sampler2DShadow",
    "isampler2D",
    "image2D",
];

const QUALIFIERS: [&str; 9] = [
    "uniform",
    "varying",
    "attribute",
    "in",
    "out",
    "const",
    "flat",
    "highp",
    "layout",
];

/// A global declaration of the pack's: its line, and the index of its type and name words.
fn globals(text: &str) -> Vec<(usize, String, String)> {
    let ls = lines(text);
    let mut out = Vec::new();
    for i in global_points(&ls) {
        let Some(line) = ls.get(i) else { continue };
        let t = line.trim();
        if !t.ends_with(';') || t.contains('(') && !t.starts_with("layout") {
            continue;
        }
        let words: Vec<&str> = t
            .trim_end_matches(';')
            .split(|c: char| c.is_whitespace() || c == ',' || c == '[' || c == '=')
            .filter(|w| !w.is_empty())
            .collect();
        let Some(ty) = words.iter().position(|w| TYPES.contains(w)) else {
            continue;
        };
        if words[..ty]
            .iter()
            .any(|w| !QUALIFIERS.contains(w) && !w.starts_with("layout") && !w.contains(')'))
        {
            continue;
        }
        if let Some(name) = words.get(ty + 1).filter(|w| is_ident(w)) {
            out.push((i, words[ty].to_owned(), (*name).to_owned()));
        }
    }
    out
}

fn is_ident(w: &str) -> bool {
    w.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn rewrite_global(s: &mut Stream, text: &str) -> Option<String> {
    let found = globals(text);
    if found.is_empty() {
        return None;
    }
    let (i, ty, name) = s.pick(&found).clone();
    let mut ls = lines(text);
    let line = ls[i].clone();
    let body = line.trim_end().trim_end_matches(';');
    ls[i] = match s.below(3) {
        // One more declarator of the same type.
        0 => format!("{body}, fz_more_{name};"),
        // An array of two, on the type.
        1 if !body.contains('[') && !body.contains('=') => {
            body.replacen(&format!("{ty} "), &format!("{ty}[2] "), 1) + ";"
        }
        // An array of two, on the name.
        _ if !body.contains('[') && !body.contains('=') => format!("{body}[2];"),
        _ => format!("{body}, fz_more2_{name}[3];"),
    };
    Some(join(&ls))
}

/// `#include` chains past the preprocessor's depth, cycles, diamonds, paths out of the pack.
fn include(
    s: &mut Stream,
    file: &str,
    text: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> String {
    let line = match s.below(8) {
        0 => {
            let depth = *s.pick(&[2usize, 5, 9, 10, 11, 40]);
            for i in 0..depth {
                let body = if i + 1 < depth {
                    format!("#include \"inc{}.glsl\"\n", i + 1)
                } else {
                    "float fz_chain() { return 1.0; }\n".to_owned()
                };
                files.insert(format!("fz/inc{i}.glsl"), body.into_bytes());
            }
            "#include \"/fz/inc0.glsl\"".to_owned()
        }
        1 => format!("#include \"/{file}\""),
        2 => {
            files.insert(
                "fz/a.glsl".into(),
                b"#include \"b.glsl\"\nfloat fz_a1;\n".to_vec(),
            );
            files.insert(
                "fz/b.glsl".into(),
                b"#include \"a.glsl\"\nfloat fz_b1;\n".to_vec(),
            );
            "#include \"/fz/a.glsl\"".to_owned()
        }
        3 => {
            let levels = *s.pick(&[3usize, 6, 9, 12]);
            for i in 0..levels {
                let body = if i + 1 < levels {
                    format!("#include \"d{0}.glsl\"\n#include \"d{0}.glsl\"\n", i + 1)
                } else {
                    "#define FZ_LEAF 1\n// a leaf, included 2^levels times\n".to_owned()
                };
                files.insert(format!("fz/d{i}.glsl"), body.into_bytes());
            }
            "#include \"/fz/d0.glsl\"".to_owned()
        }
        4 => (*s.pick(&[
            "#include \"../../../../etc/hostname\"",
            "#include \"/../shaders.properties\"",
            "#include \"\"",
            "#include",
            "#include \"/\"",
            "#include \"/fz\"",
        ]))
        .to_owned(),
        5 => "#include \"/fz/missing.glsl\"".to_owned(),
        6 => {
            files.insert(
                "fz/angle.glsl".into(),
                b"const float fz_angle = 2.0;\n".to_vec(),
            );
            "#include </fz/angle.glsl>".to_owned()
        }
        _ => {
            // An include of a file that already is in the case, a second time.
            let others: Vec<String> = files
                .keys()
                .filter(|f| f.ends_with(".glsl") || f.ends_with(".inc"))
                .cloned()
                .collect();
            match others.is_empty() {
                true => "#include \"/fz/missing.glsl\"".to_owned(),
                false => format!("#include \"/{}\"", s.pick(&others)),
            }
        }
    };
    let mut ls = lines(text);
    let points = global_points(&ls);
    let at = if s.chance(4) {
        s.below(ls.len() + 1)
    } else {
        *s.pick(&points)
    };
    ls.insert(at.min(ls.len()), line);
    join(&ls)
}

/// Nested and unbalanced conditionals, recursive and doubling macros, the directives packs
/// rarely write.
fn preprocessor(s: &mut Stream, text: &str) -> String {
    let mut ls = lines(text);
    let n = ls.len();
    match s.below(6) {
        0 => {
            let depth = *s.pick(&[1usize, 3, 50, 300]);
            let a = s.below(n + 1);
            let b = (a + s.below(8)).min(n);
            let opens = [
                "#if 1",
                "#ifdef MC_VERSION",
                "#ifndef FZ_UNDEFINED",
                "#if defined(MC_VERSION) || !defined(FZ_X)",
                "#if MC_VERSION > 0 && (1 << 3) == 8",
                "#elif 1",
            ];
            let open = (*s.pick(&opens)).to_owned();
            let else_branch = s.chance(3);
            let mut inner: Vec<String> = Vec::new();
            for _ in 0..depth {
                inner.push(open.clone());
            }
            inner.extend(ls[a..b].iter().cloned());
            for _ in 0..depth {
                if else_branch {
                    inner.push("#else".into());
                    inner.push("this is not glsl".into());
                }
                inner.push("#endif".into());
            }
            ls.splice(a..b, inner);
        }
        1 => {
            let line = *s.pick(&[
                "#endif",
                "#if 1",
                "#else",
                "#elif 1",
                "#ifdef",
                "#if",
                "#if (1",
                "#if 1/0",
                "#if 1 % 0",
                "#if 9223372036854775807 + 1",
                "#if -9223372036854775807 - 2",
                "#if (-9223372036854775807 - 1) / -1",
                "#if 1 << 64",
                "#if 1 << -1",
                "#if defined",
                "#if FZ_UNDEFINED(1)",
                "#if 99999999999999999999999",
                "#if 0x",
                "#if 1 ? 2",
            ]);
            ls.insert(s.below(n + 1), line.to_owned());
        }
        2 => {
            let at = *s.pick(&global_points(&ls));
            for (k, l) in [
                "#define FZ_A FZ_B",
                "#define FZ_B FZ_A",
                "#define FZ_SELF FZ_SELF + 1",
                "float fz_rec = float(FZ_A) + float(FZ_SELF);",
            ]
            .iter()
            .enumerate()
            {
                ls.insert((at + k).min(ls.len()), (*l).to_owned());
            }
        }
        3 => {
            // A macro doubling its argument, nested: 2^levels copies.
            let levels = *s.pick(&[4usize, 12, 24, 40]);
            let at = *s.pick(&global_points(&ls));
            let mut add = vec!["#define FZ_D0(x) x x".to_owned()];
            for i in 1..levels {
                add.push(format!("#define FZ_D{i}(x) FZ_D{0}(x) FZ_D{0}(x)", i - 1));
            }
            if s.chance(2) {
                add.push(format!("float fz_bomb = 0.0 FZ_D{}(+1.0);", levels - 1));
            } else {
                add.push("#define FZ_E0 1".into());
                for i in 1..levels {
                    add.push(format!("#define FZ_E{i} (FZ_E{0} + FZ_E{0})", i - 1));
                }
                add.push(format!("#if FZ_E{} > 0\n#endif", levels - 1));
            }
            let at = at.min(ls.len());
            ls.splice(at..at, add);
        }
        4 => {
            let line = *s.pick(&[
                "#undef MC_VERSION",
                "#define MC_VERSION 0",
                "#define MC_VERSION",
                "#undef __LINE__",
                "#line 99999999",
                "#line -1",
                "#version 9999",
                "#version",
                "#version 120 compatibility core",
                "#extension GL_FZ_unknown : require",
                "#extension all : enable",
                "#pragma optimize(off)",
                "#error fz",
                "#",
                "#define",
                "#define FZ(",
                "#define FZ_F(a) a\nfloat fz_args = FZ_F(1.0, 2.0);",
                "#define FZ_G(a, a) a",
                "#define 1FZ 2",
                "#unknown_directive",
            ]);
            ls.insert(s.below(n + 1), line.to_owned());
        }
        _ => {
            // The #version line gone, or a second one.
            match ls
                .iter()
                .position(|l| l.trim_start().starts_with("#version"))
            {
                Some(i) if s.chance(2) => {
                    ls.remove(i);
                }
                Some(i) => {
                    let again = ls[i].clone();
                    ls.insert(s.below(n + 1), again);
                }
                None => ls.insert(0, "#version 460 core".into()),
            }
        }
    }
    join(&ls)
}

const SIZES: [&str; 13] = [
    "0",
    "-1",
    "1",
    "2147483647",
    "2147483648",
    "4294967296",
    "99999999999",
    "1 << 30",
    "fz_undeclared",
    "",
    "2.5",
    "0x7fffffff",
    "-2147483648",
];

/// An array size of the pack's set to an extreme, or a new array of that size.
fn array_size(s: &mut Stream, text: &str) -> String {
    let spans: Vec<(usize, usize)> = text
        .match_indices('[')
        .filter_map(|(i, _)| {
            let close = text[i + 1..].find(']')? + i + 1;
            let inside = &text[i + 1..close];
            (!inside.is_empty() && inside.len() < 24 && inside.chars().all(|c| c.is_ascii_digit()))
                .then_some((i + 1, close))
        })
        .collect();
    let size = *s.pick(&SIZES);
    if spans.is_empty() || s.chance(4) {
        let decl = format!("float fz_sized[{size}];");
        return insert_global(s, text, &decl, Some("fz_sized[0] = 1.0;"));
    }
    let (a, b) = *s.pick(&spans);
    format!("{}{size}{}", &text[..a], &text[b..])
}

/// Names a pack may write that the game or GLSL already gives a meaning: the game's vertex
/// inputs, the translator's own names, and words later GLSL versions reserve.
pub const COLLISIONS: [&str; 30] = [
    "vaPosition",
    "vaNormal",
    "vaColor",
    "vaUV0",
    "vaUV1",
    "vaUV2",
    "mc_Entity",
    "mc_midTexCoord",
    "at_tangent",
    "at_midBlock",
    "at_velocity",
    "mc_chunkFade",
    "hz_Vertex",
    "hz_v_pos",
    "hz_Color",
    "hz_main",
    "hz_layer",
    "sample",
    "filter",
    "input",
    "output",
    "common",
    "partition",
    "active",
    "buffer",
    "shared",
    "precise",
    "patch",
    "subroutine",
    "resource",
];

const KEYWORDS: [&str; 30] = [
    "main",
    "if",
    "else",
    "for",
    "while",
    "do",
    "return",
    "break",
    "continue",
    "discard",
    "struct",
    "uniform",
    "in",
    "out",
    "inout",
    "const",
    "true",
    "false",
    "void",
    "varying",
    "attribute",
    "layout",
    "flat",
    "smooth",
    "noperspective",
    "centroid",
    "invariant",
    "highp",
    "mediump",
    "lowp",
];

/// A name the pack declares, to be renamed to a colliding one.
fn rename(s: &mut Stream, text: &str) -> Option<(String, &'static str)> {
    let mut names: BTreeSet<String> = globals(text).into_iter().map(|g| g.2).collect();
    // Locals and parameters: a word after a type word.
    let words: Vec<&str> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    for pair in words.windows(2) {
        if TYPES.contains(&pair[0]) && is_ident(pair[1]) {
            names.insert(pair[1].to_owned());
        }
    }
    names.retain(|n| {
        !n.starts_with("gl_") && !KEYWORDS.contains(&n.as_str()) && !TYPES.contains(&n.as_str())
    });
    if names.is_empty() || s.chance(3) {
        return None;
    }
    let names: Vec<String> = names.into_iter().collect();
    Some((s.pick(&names).clone(), *s.pick(&COLLISIONS)))
}

fn replace_word(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(from) {
        let before = rest[..i].chars().next_back();
        let after = rest[i + from.len()..].chars().next();
        let word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        out.push_str(&rest[..i]);
        out.push_str(if word(before) || word(after) {
            from
        } else {
            to
        });
        rest = &rest[i + from.len()..];
    }
    out.push_str(rest);
    out
}

/// A new global or local named like a game input, a translator name or a later keyword.
fn collide_declare(s: &mut Stream, file: &str, text: &str) -> String {
    let name = *s.pick(&COLLISIONS);
    if s.chance(2) {
        let ty = *s.pick(&["vec3", "float", "vec4", "int"]);
        let local = format!("{ty} {name} = {ty}(1);");
        let mut ls = lines(text);
        if let Some(at) = main_body(&ls) {
            ls.insert(at, format!("    {local}"));
            return join(&ls);
        }
    }
    let io = if vertex(file) { "in" } else { "uniform" };
    let decl = s
        .pick(&[
            format!("attribute vec4 {name};"),
            format!("{io} vec3 {name};"),
            format!("uniform vec2 {name};"),
            format!("const int {name} = 1;"),
            format!("vec4 {name}[2];"),
            format!("float {name}(float x) {{ return x; }}"),
        ])
        .clone();
    insert_global(s, text, &decl, Some(&format!("{name};")))
}

const NUMBERS: [&str; 16] = [
    "1e39",
    "3.4028235e38",
    "-0.0",
    "(0.0 / 0.0)",
    "(1.0 / 0.0)",
    "-2147483648",
    "4294967295u",
    "4294967296",
    "0x7fffffff",
    "0xffffffffff",
    "1e-45",
    "-1",
    "00",
    "1.0e",
    "99999999999999999999.0",
    "1.#INF",
];

/// A number literal of the pack's (also in a `#define` or `const`) set to an extreme.
fn numeric(s: &mut Stream, text: &str) -> String {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let prev_word = i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        if c.is_ascii_digit() && !prev_word {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                i += 1;
            }
            spans.push((start, i));
        } else {
            i += 1;
        }
    }
    let number = *s.pick(&NUMBERS);
    if spans.is_empty() {
        return insert_global(
            s,
            text,
            &format!("float fz_n = {number};"),
            Some("fz_n += 1.0;"),
        );
    }
    let (a, b) = *s.pick(&spans);
    format!("{}{number}{}", &text[..a], &text[b..])
}

fn char_boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The file cut short: at any character, mid-line, by its last lines, or emptied.
fn truncate(s: &mut Stream, text: &str) -> String {
    match s.below(4) {
        0 => String::new(),
        1 => {
            let at = char_boundary(text, s.below(text.len() + 1));
            text[..at].to_owned()
        }
        2 => {
            let ls = lines(text);
            let keep = ls.len().saturating_sub(1 + s.below(4));
            join(&ls[..keep])
        }
        _ => {
            let mut ls = lines(text);
            let i = s.below(ls.len());
            let cut = char_boundary(&ls[i], ls[i].len() / 2);
            ls[i].truncate(cut);
            join(&ls[..=i])
        }
    }
}

/// Bytes packs have that are not plain UTF-8 lines: a byte-order mark, CRLF, invalid UTF-8,
/// a NUL, a lone CR, tabs and form feeds.
fn encoding(s: &mut Stream, text: &str) -> Vec<u8> {
    match s.below(6) {
        0 => [b"\xEF\xBB\xBF".as_slice(), text.as_bytes()].concat(),
        1 => text.replace('\n', "\r\n").into_bytes(),
        2 => {
            let at = char_boundary(text, s.below(text.len() + 1));
            [
                &text.as_bytes()[..at],
                b"\xFF\xFE\xC0",
                &text.as_bytes()[at..],
            ]
            .concat()
        }
        3 => {
            let at = char_boundary(text, s.below(text.len() + 1));
            [&text.as_bytes()[..at], b"\0", &text.as_bytes()[at..]].concat()
        }
        4 => text.replace('\n', "\r").into_bytes(),
        _ => text.replace("    ", "\t\x0c").into_bytes(),
    }
}

/// The span of a function's definition starting at line `i` (to its closing brace).
fn function_at(ls: &[String], i: usize) -> Option<(usize, usize)> {
    let t = ls[i].trim();
    if !t.contains('(') || t.ends_with(';') || t.starts_with('#') {
        return None;
    }
    let mut depth = 0i32;
    let mut opened = false;
    for (j, l) in ls.iter().enumerate().skip(i) {
        let code = l.split("//").next().unwrap_or("");
        depth += code.matches('{').count() as i32;
        opened |= code.contains('{');
        depth -= code.matches('}').count() as i32;
        if opened && depth <= 0 {
            return Some((i, j + 1));
        }
        if j > i + 400 {
            break;
        }
    }
    None
}

/// A line, a global declaration or a whole function written twice.
fn duplicate(s: &mut Stream, text: &str) -> String {
    let mut ls = lines(text);
    let points = global_points(&ls);
    let functions: Vec<(usize, usize)> = points
        .iter()
        .filter_map(|&i| ls.get(i).and_then(|_| function_at(&ls, i)))
        .collect();
    match s.below(3) {
        0 if !functions.is_empty() => {
            let (a, b) = *s.pick(&functions);
            let copy: Vec<String> = ls[a..b].to_vec();
            ls.splice(b..b, copy);
        }
        1 => {
            let decls: Vec<usize> = points
                .iter()
                .copied()
                .filter(|&i| ls.get(i).is_some_and(|l| l.trim().ends_with(';')))
                .collect();
            if decls.is_empty() {
                return text.to_owned();
            }
            let i = *s.pick(&decls);
            let copy = ls[i].clone();
            ls.insert(i + 1, copy);
        }
        _ => {
            let filled: Vec<usize> = (0..ls.len())
                .filter(|&i| !ls[i].trim().is_empty())
                .collect();
            if filled.is_empty() {
                return text.to_owned();
            }
            let i = *s.pick(&filled);
            let copy = ls[i].clone();
            ls.insert(i + 1, copy);
        }
    }
    join(&ls)
}

/// Two lines swapped, a function moved to the end (after its uses), or a declaration moved
/// above the `#version` line.
fn reorder(s: &mut Stream, text: &str) -> String {
    let mut ls = lines(text);
    if ls.len() < 2 {
        return text.to_owned();
    }
    let points = global_points(&ls);
    let functions: Vec<(usize, usize)> = points
        .iter()
        .filter_map(|&i| ls.get(i).and_then(|_| function_at(&ls, i)))
        .collect();
    match s.below(3) {
        0 if !functions.is_empty() => {
            let (a, b) = *s.pick(&functions);
            let moved: Vec<String> = ls.drain(a..b).collect();
            ls.extend(moved);
        }
        1 => {
            let decls: Vec<usize> = points
                .iter()
                .copied()
                .filter(|&i| ls.get(i).is_some_and(|l| l.trim().ends_with(';')))
                .collect();
            if decls.is_empty() {
                return text.to_owned();
            }
            let line = ls.remove(*s.pick(&decls));
            ls.insert(0, line);
        }
        _ => {
            let i = s.below(ls.len() - 1);
            ls.swap(i, i + 1);
        }
    }
    join(&ls)
}

const VALUES: [&str; 14] = [
    "",
    "-1",
    "99999999",
    "NaN",
    "1e40",
    "true false",
    "((((",
    "if(",
    "-2147483649",
    "0.0/0.0",
    " = = ",
    "\\",
    "sin(",
    "frameTimeCounter frameTimeCounter",
];

const KEYS: [&str; 22] = [
    "size.buffer.colortex0=99999 99999",
    "size.buffer.colortex1=-1 0",
    "size.buffer.colortex99=0.5 0.5",
    "bufferObject.0=99999999999",
    "bufferObject.4294967296=16",
    "bufferObject.1=16 fz/missing.bin",
    "image.fzimg=fzsampler RGBA8 RGBA UNSIGNED_BYTE false false 999999 999999 9999",
    "image.fzimg2=fzsampler NOTAFORMAT RGBA UNSIGNED_BYTE false true -1 -1",
    "image.=x",
    "uniform.float.fzLoop=fzLoop + 1.0",
    "variable.float.fzP=fzQ\nvariable.float.fzQ=fzP",
    "uniform.vec3.fzBad=vec3(1.0, 2.0)",
    "uniform.float.frameTimeCounter=1.0",
    "scale.composite=0 0 0",
    "scale.composite=-1 2 2",
    "blend.gbuffers_terrain=NOT A BLEND",
    "blend.composite.colortex99=ONE ZERO ONE ZERO",
    "alphaTest.gbuffers_terrain=GREATER NaN",
    "texture.composite.colortex1=../../../../etc/hostname",
    "texture.noise=minecraft:textures/fz/missing.png",
    "=fz",
    "#if 1\nshadow.enabled=false",
];

/// `shaders.properties` with a value set to an extreme, or a key added (never a `program.`
/// key: those turn programs off without a note, by design).
fn properties(s: &mut Stream, text: &str) -> String {
    let mut ls = lines(text);
    let entries: Vec<usize> = (0..ls.len())
        .filter(|&i| {
            let t = ls[i].trim();
            t.contains('=') && !t.starts_with('#') && !t.contains("program.")
        })
        .collect();
    if !entries.is_empty() && s.chance(2) {
        let i = *s.pick(&entries);
        let key = ls[i].split('=').next().unwrap_or("").to_owned();
        let value = if s.chance(8) {
            "x".repeat(4096)
        } else {
            (*s.pick(&VALUES)).to_owned()
        };
        ls[i] = format!("{key}={value}");
    } else {
        ls.push((*s.pick(&KEYS)).to_owned());
    }
    join(&ls)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeds() -> Vec<Source> {
        corpus(None)
    }

    #[test]
    fn the_seed_pack_offers_its_programs_in_every_dimension() {
        let corpus = seeds();
        let names = [
            "composite",
            "composite1",
            "deferred",
            "final",
            "gbuffers_terrain",
            "gbuffers_water",
            "shadow",
        ];
        for (folder, programs) in &corpus[0].folders {
            assert_eq!(folder, "");
            assert_eq!(programs, &names);
        }
    }

    #[test]
    fn compute_shaders_alone_make_a_composite_style_program() {
        assert_eq!(compute_owner("shadowcomp_b"), "shadowcomp");
        assert_eq!(compute_owner("composite12"), "composite12");
        assert_eq!(compute_owner("composite_ab"), "composite_ab");
        for name in [
            "setup",
            "begin",
            "prepare1",
            "deferred",
            "composite12",
            "shadowcomp",
        ] {
            assert!(compute_only(name), "{name}");
        }
        for name in ["final", "gbuffers_terrain", "shadow", "composite_ab"] {
            assert!(!compute_only(name), "{name}");
        }
    }

    #[test]
    fn a_case_takes_its_programs_and_what_they_include() {
        let corpus = seeds();
        let files = subset(&corpus[0], "", &["composite".to_owned()]);
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            [
                "composite.fsh",
                "composite.vsh",
                "lib/common.glsl",
                "shaders.properties"
            ]
        );
        let files = subset(&corpus[0], "", &["composite1".to_owned()]);
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            ["composite1.csh", "shaders.properties"]
        );
    }

    #[test]
    fn generation_is_seeded_and_reaches_every_kind() {
        let corpus = seeds();
        let a = generate(&corpus, case_seed(1, 0), true).unwrap();
        assert_eq!(Some(&a), generate(&corpus, case_seed(1, 0), true).as_ref());
        assert_ne!(Some(&a), generate(&corpus, case_seed(1, 1), true).as_ref());
        assert_eq!(a.materialize(), a.clone().materialize());
        let mut kinds = BTreeSet::new();
        for i in 0..300 {
            let case = generate(&corpus, case_seed(1, i), true).unwrap();
            assert!((1..=6).contains(&case.steps.len()));
            kinds.extend(case.steps.iter().map(|s| s.kind));
        }
        assert_eq!(kinds.into_iter().collect::<Vec<_>>(), KINDS);
    }

    #[test]
    fn a_rename_reaches_every_source_file_of_the_case() {
        let mut files: BTreeMap<String, Vec<u8>> = [
            ("a.fsh", "uniform float fade;\nvoid main() { x = fade; }\n"),
            ("lib/b.glsl", "float g() { return fade + faded; }\n"),
        ]
        .map(|(p, t)| (p.to_owned(), t.as_bytes().to_vec()))
        .into();
        // Seed 1's choices over this file: rename `fade`.
        let step = (0..64)
            .map(|seed| Step {
                kind: Kind::Collide,
                file: "a.fsh".into(),
                seed,
            })
            .find(|step| {
                let mut f = files.clone();
                apply(step, &mut f);
                !f["lib/b.glsl"].starts_with(b"float g() { return fade ")
            })
            .unwrap();
        apply(&step, &mut files);
        let a = String::from_utf8(files["a.fsh"].clone()).unwrap();
        let b = String::from_utf8(files["lib/b.glsl"].clone()).unwrap();
        let name = a
            .strip_prefix("uniform float ")
            .and_then(|r| r.split(';').next())
            .unwrap();
        assert!(COLLISIONS.contains(&name), "{a}");
        assert_eq!(b, format!("float g() {{ return {name} + faded; }}\n"));
    }

    #[test]
    fn ownership_is_the_programs_stage_files() {
        let case = Case {
            seed: 0,
            pack: String::new(),
            dim: 0,
            flat: true,
            programs: vec!["composite".into()],
            folder: "world0".into(),
            files: BTreeMap::new(),
            steps: Vec::new(),
        };
        for own in [
            "world0/composite.fsh",
            "world0/composite.csh",
            "world0/composite_b.csh",
        ] {
            assert!(case.owns("composite", own), "{own}");
        }
        for other in [
            "composite.fsh",
            "world0/composite1.fsh",
            "world0/composite_bb.csh",
            "world0/lib/composite.fsh",
            "shaders.properties",
        ] {
            assert!(!case.owns("composite", other), "{other}");
        }
    }
}
