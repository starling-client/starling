//! Package inspection and the release attestation policy for `cargo sunrise-mod check <file>`.
use crate::bounded::{self, Symlinks};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use sunrise_attestation::{Attested, PUBLIC_GOOD_TRUSTED_ROOT};
use sunrise_mod_package::Package;
use sunrise_mod_package::index::{self, Facts, Kind};
use sunrise_mod_package::manifest::{MANIFEST, MAX_USES};

pub struct Report {
    pub kind: Kind,
    pub facts: Facts,
    pub sha256: String,
    pub size: u64,
    pub signed: bool,
    attestation: Option<Attested>,
}

impl Report {
    pub fn json(&self) -> Value {
        let requires: Vec<Value> = self
            .facts
            .requires
            .iter()
            .map(|r| json!({"id": r.id, "version": r.version, "sha256": r.sha256}))
            .collect();
        let mut report = json!({
            "kind": self.kind.name(),
            "id": self.facts.id,
            "version": self.facts.version,
            "api": self.facts.api,
            "side": self.facts.side,
            "capabilities": self.facts.capabilities,
            "game": self.facts.game,
            "dependencies": self.facts.dependencies,
            "requires": requires,
            "sha256": self.sha256,
            "size": self.size,
            "signed": self.signed,
        });
        if let Some(attested) = &self.attestation {
            report["attestation"] = json!({
                "repository_id": attested.repository_id,
                "repository_name": attested.repository_name,
                "signer_workflow_ref": attested.signer_workflow_ref,
                "source_ref": attested.source_ref,
                "source_commit": attested.source_commit,
                "predicate_type": attested.predicate_type,
            });
        }
        report
    }

    pub fn text(&self) -> String {
        let f = &self.facts;
        let mut out = format!(
            "{} {} ({}",
            f.id.as_deref().unwrap_or("resource pack"),
            f.version.as_deref().unwrap_or(""),
            self.kind.name()
        );
        if let Some(side) = f.side {
            let side = serde_json::to_value(side).unwrap_or_default();
            out += &format!(", API {}, {}", f.api, side.as_str().unwrap_or("?"));
        } else if !f.api.is_empty() {
            out += &format!(", format {}", f.api);
        }
        if let Some(game) = &f.game {
            out += &format!(", for {game}");
        }
        let signed = if self.signed {
            "signature verified"
        } else {
            "unsigned"
        };
        out += &format!(
            ")\n  {} bytes, SHA-256 {}, {signed}\n",
            self.size, self.sha256
        );
        if let Some(attested) = &self.attestation {
            out += &format!(
                "  repository: {} (ID {})\n  workflow: {}\n  commit: {}\n",
                attested.repository_name,
                attested.repository_id,
                attested.signer_workflow_ref,
                attested.source_commit
            );
        }
        for c in &f.capabilities {
            out += &format!("  may {}\n", c.describe());
        }
        for dep in &f.dependencies {
            let label = if dep.optional {
                "optional:"
            } else {
                "requires"
            };
            let side = match dep.side {
                Some(sunrise_mod_package::manifest::Side::Client) => " (client only)",
                Some(sunrise_mod_package::manifest::Side::Server) => " (dedicated server only)",
                _ => "",
            };
            out += &format!("  {label} {} {}{side}\n", dep.id, dep.version);
        }
        for r in &f.requires {
            out += &format!("  needs {} {} ({})\n", r.id, r.version, &r.sha256[..12]);
        }
        if self.kind == Kind::NativeMod {
            out += "  native source: unsandboxed; not executed by the registry check. Compile and test against the exact pinned Sunrise and Mixins revisions before review.\n";
        }
        if self.kind == Kind::Mod {
            out += "  no code: content only, nothing to run\n";
        }
        out
    }
}

/// The kind of a package, inspecting mod archives for their native manifest.
pub fn kind_of(path: &Path) -> Option<Kind> {
    match path.extension()?.to_str()? {
        "smod" if index::native::holds(path) => Some(Kind::NativeMod),
        "smod" => Some(Kind::Mod),
        "spack" => Some(Kind::Modpack),
        "zip" => Some(sunrise_mod_package::index::zip_kind(path)),
        _ => None,
    }
}

/// Inspect a downloaded package and require an attestation for mods. A mod is read once into a
/// private copy, and that copy is hashed and verified before its contents are inspected: the facts
/// reported are those of the attested bytes. Whether a file is a mod is decided by its contents
/// too: a `.smod`, a mod `--kind`, or any archive with `sunrise-mod.toml` at its root.
pub fn check_release(
    path: &Path,
    kind: Option<Kind>,
    attestation: Option<&Path>,
    repository: Option<u64>,
) -> Result<Report, String> {
    let bytes =
        bounded::read_bytes(path, index::MAX_FILE, Symlinks::Follow).map_err(|e| e.to_string())?;
    let holds_manifest = zip::ZipArchive::new(std::io::Cursor::new(&bytes))
        .is_ok_and(|mut archive| archive.by_name(MANIFEST).is_ok());
    let mod_archive = holds_manifest || path.extension().is_some_and(|ext| ext == "smod");
    let as_mod = kind
        .as_ref()
        .is_some_and(|kind| matches!(kind, Kind::Mod | Kind::NativeMod));
    if !mod_archive && !as_mod {
        if attestation.is_some() || repository.is_some() {
            return Err("--attestation and --repository apply only to mod packages".into());
        }
        let kind = kind
            .or_else(|| kind_of(path))
            .ok_or("pass --kind mod|modpack|resourcepack: the extension doesn't say")?;
        return inspect(path, kind);
    }
    if kind.as_ref().is_some_and(|_| !as_mod) {
        return Err("a mod package must be checked as a mod or native-mod".into());
    }
    let default_sidecar;
    let sidecar = match attestation {
        Some(sidecar) => sidecar,
        None => {
            default_sidecar = path.with_added_extension("sigstore.json");
            &default_sidecar
        }
    };
    // The registry downloads a release's attestation within its bound for one release file.
    let bundle = bounded::read_bytes(sidecar, index::MAX_FILE, Symlinks::Follow).map_err(|e| {
        if e.is_not_found() {
            format!(
                "not signed: the release workflow attests each mod package; bundle not found: {}",
                sidecar.display()
            )
        } else {
            e.to_string()
        }
    })?;
    let attested = sunrise_attestation::verify(
        &bundle,
        Sha256::digest(&bytes).into(),
        PUBLIC_GOOD_TRUSTED_ROOT,
    )
    .map_err(|e| e.to_string())
    .and_then(|attested| require_release_signer(attested, repository))
    .map_err(|e| format!("{}: {e}", sidecar.display()))?;
    // A fixed name: the download's own name never becomes part of a path written here.
    let private = tempfile::tempdir().map_err(|e| format!("check: {e}"))?;
    let copy = private.path().join("package.smod");
    std::fs::write(&copy, &bytes).map_err(|e| format!("{}: {e}", copy.display()))?;
    let kind = kind.unwrap_or_else(|| {
        if index::native::holds(&copy) {
            Kind::NativeMod
        } else {
            Kind::Mod
        }
    });
    let mut report = inspect(&copy, kind)?;
    report.attestation = Some(attested);
    report.signed = true;
    Ok(report)
}

pub(crate) fn require_release_signer(
    attested: Attested,
    repository: Option<u64>,
) -> Result<Attested, String> {
    const WORKFLOW: &str = "sunrise-org/sunrise-mods/.github/workflows/release.yml";
    // The shared workflow's tags are `release-v<number>` (sunrise-mods registry/release-workflow.toml).
    const TAG_PREFIX: &str = "refs/tags/release-v";
    if let Some(expected) = repository
        && attested.repository_id != expected
    {
        return Err(format!(
            "expected repository ID {expected}, attestation names {} (ID {})",
            attested.repository_name, attested.repository_id
        ));
    }
    let allowed =
        attested
            .signer_workflow_ref
            .split_once('@')
            .is_some_and(|(workflow, git_ref)| {
                workflow == WORKFLOW
                    && git_ref.strip_prefix(TAG_PREFIX).is_some_and(|version| {
                        !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit())
                    })
            });
    if !allowed {
        return Err(format!(
            "expected workflow {WORKFLOW}@{TAG_PREFIX}<number>, attestation names {}",
            attested.signer_workflow_ref
        ));
    }
    Ok(attested)
}

/// Validate a package's contents, independent of signature or registry policy.
pub(crate) fn inspect(path: &Path, kind: Kind) -> Result<Report, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if size > index::MAX_FILE {
        return Err(format!("over {} MiB", index::MAX_FILE >> 20));
    }
    index::open(&kind)?;
    let facts = index::inspect(&kind, path)?;
    if kind == Kind::NativeMod {
        uses_list(path)?;
    }
    Ok(Report {
        kind,
        facts,
        sha256: index::sha256_file(path)?,
        size,
        signed: false,
        attestation: None,
    })
}

/// A native mod carries the list of what it uses, made for the build it pins: without it the mod
/// works on that one build only. A mod with rows no list can judge
/// ([`sunrise_native_build::game_uses::unjudged`]) is made for one build by nature and needs none.
fn uses_list(path: &Path) -> Result<(), String> {
    use sunrise_mod_package::mod_surface::uses::{USES_FILE, UsesTable};
    let manifest = Package::read(path)?.manifest;
    if sunrise_native_build::game_uses::unjudged(&manifest).is_some() {
        return Ok(());
    }
    let pin = manifest
        .native
        .ok_or_else(|| format!("no [native] table in {MANIFEST}"))?
        .sunrise;
    let repack = "Pack the mod with `cargo sunrise-mod build`, which writes the list; if it says it cannot read the mod's code, fix what it names first.";
    let Some(bytes) = Package::entry(path, USES_FILE, MAX_USES)? else {
        return Err(format!(
            "no {USES_FILE}: the package does not list what the mod uses, so it would work on game build {pin} only and stop at the next one. {repack}"
        ));
    };
    let list = UsesTable::read(&bytes).map_err(|e| format!("{USES_FILE}: {e}. {repack}"))?;
    if list.made_for != pin {
        return Err(format!(
            "{USES_FILE} was made for game build {}, and the mod pins {pin}, so the list says nothing about the pinned build. {repack}",
            list.made_for
        ));
    }
    Ok(())
}

/// Loads every listed `[[jar]]` from the local jar store, fetching a missing or changed jar from
/// Modrinth.
/// Returns `None` when the manifest lists no jars or when any jar cannot be fetched or read,
/// printing one note so `check` and `build` still succeed offline.
fn load_jar_layer(
    manifest: &sunrise_mod_package::Manifest,
) -> Option<sunrise_mod_package::content::JarLayer> {
    if manifest.jar.is_empty() {
        return None;
    }
    let store = crate::jar::default_store();
    let mut layer = sunrise_mod_package::content::JarLayer::new();
    for jar in &manifest.jar {
        let cached = jar.cache_path(&store);
        // A cached file is used only when its bytes still match the listed SHA-512, as `run`
        // checks before staging; otherwise it is fetched again, which replaces it.
        let verified = std::fs::File::open(&cached)
            .and_then(|file| sunrise_jar_fetch::hash_reader::<sha2::Sha512>(file, std::io::sink()))
            .is_ok_and(|hash| format!("{hash:x}") == jar.sha512());
        if !verified
            && let Err(error) = sunrise_jar_fetch::fetch(
                &crate::jar::ProxyHttp,
                crate::jar::MODRINTH,
                jar.sha512(),
                &cached,
            )
        {
            eprintln!(
                "note: could not fetch jar {} {} ({error}); its resources were not checked",
                jar.id(),
                jar.version()
            );
            return None;
        }
        if let Err(error) = layer.add_archive(&cached) {
            eprintln!(
                "note: could not read jar {} {} ({error}); its resources were not checked",
                jar.id(),
                jar.version()
            );
            return None;
        }
    }
    Some(layer)
}

/// The references among a project's content files ([`sunrise_mod_package::content::check_references`]):
/// warnings are printed, errors fail with one line each. When the manifest lists `[[jar]]`, each
/// jar is read from the local jar store (fetched from Modrinth if missing) so references resolve
/// against the mod and its jars together.
pub fn references(root: &Path, manifest: &sunrise_mod_package::Manifest) -> Result<(), String> {
    let content = sunrise_mod_package::Content::read(root, manifest)?;
    let jars = load_jar_layer(manifest);
    let findings =
        sunrise_mod_package::content::check_references(&content, manifest, jars.as_ref())?;
    for warning in findings
        .warnings
        .iter()
        .chain(&renamed_folders(&content.files))
    {
        eprintln!("warning: {warning}");
    }
    if findings.errors.is_empty() {
        Ok(())
    } else {
        Err(findings.errors.join("\nerror: "))
    }
}

/// The include that opts a program into the order-independent transparency passes.
const OIT_INCLUDE: &str = "minecraft:shaders/include/oit.glsl";

/// Translates and validates every GLSL core shader program in a mod's `files`
/// (`assets/<namespace>/shaders/core/<path>.vsh` and `.fsh`) through `sunrise_mod_pipeline`
/// (the same code the game uses), returning how many programs were checked and each refusal as
/// `<file>[:<line>:<column>]: <message>`.
///
/// Includes resolve from the mod's own `files`, then from `resources` (prepared vanilla 26.3
/// resources, when available), and finally from Sunrise's interface-only stubs for `minecraft:*`
/// (§6 & §13 Q3: nothing from Mojang's files is shipped in the SDK). When a `.fsh` has no
/// matching `.vsh` beside it in `files`, `minecraft:shaders/core/position_tex_color.vsh` is used
/// as the default vertex stage (matching the common `.vertex_shader("minecraft:core/position_tex_color")`
/// case). A vertex-only file is checked with a stand-in fragment shader. Whether a pipeline is
/// textured is in the mod's Rust code, so a program that uses `sampleSprite`, `SpriteRect` or
/// `Sampler0` (or includes `pipeline_sprite.glsl`) in the text the translator reads, with its
/// `#include`s inlined and its comments removed, is checked as textured; a name only in a
/// comment is not. The vertex inputs are those of the vertex stage with its includes inlined
/// ([`sunrise_mod_pipeline::ProgramBindings::vertex_inputs`]), and a program that includes
/// `oit.glsl`, in its own text or through another include, is also checked against the
/// order-independent transparency passes. A program whose includes cannot be expanded is refused.
pub fn pipeline_shaders(
    files: &BTreeMap<String, Vec<u8>>,
    resources: Option<&Path>,
) -> (usize, Vec<String>) {
    let programs = pair_programs(files);
    let assets = Assets { files, resources };
    let mut refused = Vec::new();
    for program in &programs {
        if let Err(problems) = check_program(program, &assets) {
            refused.extend(problems);
        }
    }
    (programs.len(), refused)
}

/// The fragment stage of a vertex-only file: it only lets the vertex stage be validated.
const STAND_IN_FRAGMENT: &str =
    "#version 330\nout vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }\n";

/// One program [`pipeline_shaders`] checks: the two stage files of a `shaders/core` stem and the
/// folders their quoted includes are relative to.
struct Program {
    vertex_file: String,
    vertex_dir: String,
    fragment_file: String,
    fragment_dir: String,
    /// The file has no `.fsh`: the fragment stage is [`STAND_IN_FRAGMENT`] (a vertex stage shared
    /// across pipelines via `.vertex_shader(...)`) and is no OIT program.
    stand_in: bool,
}

/// Pairs the `.vsh` and `.fsh` files of `files` by namespace and stem. A fragment-only file takes
/// `minecraft:core/position_tex_color` as its vertex stage.
fn pair_programs(files: &BTreeMap<String, Vec<u8>>) -> Vec<Program> {
    let mut stems: BTreeMap<(String, String), (Option<String>, Option<String>)> = BTreeMap::new();
    let folder = sunrise_mod_pipeline::CORE_FOLDER;
    for name in files.keys() {
        let mut parts = name.splitn(3, '/');
        let (Some("assets"), Some(ns), Some(rest)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Some(after_folder) = rest.strip_prefix(folder).and_then(|r| r.strip_prefix('/')) else {
            continue;
        };
        if let Some(stem) = after_folder.strip_suffix(".vsh") {
            stems.entry((ns.to_owned(), stem.to_owned())).or_default().0 = Some(name.clone());
        } else if let Some(stem) = after_folder.strip_suffix(".fsh") {
            stems.entry((ns.to_owned(), stem.to_owned())).or_default().1 = Some(name.clone());
        }
    }
    stems
        .into_iter()
        .map(|((ns, stem), (vsh_file, fsh_file))| {
            let folder_part = stem.rfind('/').map_or("", |i| &stem[..=i]);
            let dir = format!("{ns}:{folder}/{folder_part}");
            let (vertex_file, vertex_dir) = vsh_file.map_or_else(
                || {
                    (
                        "assets/minecraft/shaders/core/position_tex_color.vsh".to_owned(),
                        "minecraft:shaders/core/".to_owned(),
                    )
                },
                |file| (file, dir.clone()),
            );
            Program {
                stand_in: fsh_file.is_none(),
                fragment_file: fsh_file.unwrap_or_else(|| vertex_file.clone()),
                fragment_dir: dir,
                vertex_file,
                vertex_dir,
            }
        })
        .collect()
}

/// Where a program's files and includes come from: the mod's own `files`, then prepared vanilla
/// `resources`, and finally Sunrise's interface-only stubs for `minecraft:*`.
struct Assets<'a> {
    files: &'a BTreeMap<String, Vec<u8>>,
    resources: Option<&'a Path>,
}

impl Assets<'_> {
    fn asset(&self, asset: &str) -> Result<Option<String>, String> {
        if let Some(bytes) = self.files.get(asset) {
            return std::str::from_utf8(bytes)
                .map(|s| Some(s.to_owned()))
                .map_err(|e| format!("{asset}: {e}"));
        }
        if let Some(dir) = self.resources {
            let path = dir.join(asset);
            match std::fs::read_to_string(&path) {
                Ok(text) => return Ok(Some(text)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        if let Some(rest) = asset.strip_prefix("assets/")
            && let Some((ns, path)) = rest.split_once('/')
            && let Some(stub) = sunrise_mod_pipeline::stubs::read(&format!("{ns}:{path}"))
        {
            return Ok(Some(stub.to_owned()));
        }
        Ok(None)
    }

    /// The text of an `#include <ns:path>`, `None` when no source has it.
    fn include(&self, id: &str) -> Result<Option<String>, String> {
        let Some((ns, rest)) = id.split_once(':') else {
            return Ok(None);
        };
        self.asset(&format!("assets/{ns}/{rest}"))
    }

    /// The text of a stage file, which must exist.
    fn stage(&self, file: &str) -> Result<String, String> {
        self.asset(file)?
            .ok_or_else(|| format!("{file}: not found"))
    }
}

/// Reads, translates and validates one program, as [`pipeline_shaders`] documents: every refusal
/// as `<file>[:<line>:<column>]: [<stage> shader: ]<message>` when its position and stage
/// are known.
///
/// The check has no `RenderPipeline`, so it cannot know which vertex format the mod registers:
/// the game's registration decides that. It takes the vertex `in`s the program declares, each on
/// its own, and refuses only one of a type no vertex attribute format supplies; which of them the
/// game binds, and from which vertex bindings, the pipeline's registered format decides at
/// registration.
fn check_program(program: &Program, assets: &Assets<'_>) -> Result<(), Vec<String>> {
    let fragment = if program.stand_in {
        STAND_IN_FRAGMENT.to_owned()
    } else {
        assets.stage(&program.fragment_file).map_err(|e| vec![e])?
    };
    let vertex = assets.stage(&program.vertex_file).map_err(|e| vec![e])?;
    let sources = sunrise_mod_pipeline::ProgramSources {
        vertex_file: &program.vertex_file,
        vertex_dir: &program.vertex_dir,
        vertex: &vertex,
        fragment_file: &program.fragment_file,
        fragment_dir: &program.fragment_dir,
        fragment: &fragment,
    };
    let include = |id: &str| assets.include(id);
    // Whether a pipeline is textured is in the mod's Rust code; the program says so in the
    // text the translator reads.
    let bindings = sunrise_mod_pipeline::extract_program_bindings(&sources, &[], &include)
        .map_err(|e| vec![e.to_string()])?;
    let mod_bindings: Vec<sunrise_mod_pipeline::ModBinding<'_>> = bindings
        .bindings
        .iter()
        .map(|(name, kind)| sunrise_mod_pipeline::ModBinding {
            name: name.as_str(),
            kind: *kind,
        })
        .collect();
    let vertex_inputs: Vec<(&str, &str)> = (bindings.vertex_inputs.iter())
        .map(|(name, ty)| (name.as_str(), ty.as_str()))
        .collect();
    let options = sunrise_mod_pipeline::Options {
        textured: bindings.textured,
        vertex_inputs: &vertex_inputs,
        mod_bindings: &mod_bindings,
        push_constant_size: bindings.push_constant_size,
        check_backends: true,
        ..Default::default()
    };
    let refusals = |problems: Vec<sunrise_mod_pipeline::check::Problem>| {
        problems.iter().map(ToString::to_string).collect::<Vec<_>>()
    };
    sunrise_mod_pipeline::translate(&sources, &options, &include).map_err(refusals)?;
    // A program joins the OIT passes by including `oit.glsl`, directly or through another
    // include: `bindings.includes` lists every file its stages' expansion read.
    if !program.stand_in && bindings.includes.iter().any(|id| id == OIT_INCLUDE) {
        sunrise_mod_pipeline::translate_oit(&sources, &options, false, &include)
            .map_err(refusals)?;
    }
    Ok(())
}

/// One line per data folder that holds files under 26.2's name for a folder 26.3 reads under
/// another ([`sunrise_port_pack::upgrade::RENAMED_FOLDERS`]): the game reads nothing there, so
/// the files are silently left out.
fn renamed_folders(files: &[String]) -> Vec<String> {
    let mut found: BTreeMap<(&str, &str, &str), usize> = BTreeMap::new();
    for file in files {
        let Some((namespace, rest)) = file
            .strip_prefix("data/")
            .and_then(|rest| rest.split_once('/'))
        else {
            continue;
        };
        for (old, new) in sunrise_port_pack::upgrade::RENAMED_FOLDERS {
            if rest
                .strip_prefix(old)
                .is_some_and(|path| path.starts_with('/'))
            {
                *found.entry((namespace, old, new)).or_default() += 1;
            }
        }
    }
    found
        .into_iter()
        .map(|((namespace, old, new), count)| {
            format!(
                "data/{namespace}/{old}/: 26.3 does not read this folder, so its {count} file(s) are left out; 26.3 reads data/{namespace}/{new}/, in 26.3's format"
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_in_a_folder_26_3_renamed_are_named_once_per_folder() {
        let files = [
            "data/b/worldgen/configured_feature/arch.json",
            "data/b/worldgen/configured_feature/deep/rope.json",
            "data/b/worldgen/configured_carver/gap.json",
            "data/b/worldgen/feature/plank.json",
            "data/b/worldgen/configured_features/typo.json",
            "assets/b/worldgen/configured_feature/not_data.json",
        ]
        .map(String::from);
        assert_eq!(
            renamed_folders(&files),
            [
                "data/b/worldgen/configured_carver/: 26.3 does not read this folder, so its 1 file(s) are left out; 26.3 reads data/b/worldgen/carver/, in 26.3's format",
                "data/b/worldgen/configured_feature/: 26.3 does not read this folder, so its 2 file(s) are left out; 26.3 reads data/b/worldgen/feature/, in 26.3's format",
            ]
        );
    }

    #[test]
    fn pipeline_files_are_translated_as_the_game_does() {
        let vsh = "#version 330\n#include <minecraft:dynamictransforms.glsl>\n\
            #include <minecraft:projection.glsl>\n\
            in vec3 Position;\nin vec2 UV0;\nin vec4 Color;\n\
            out vec2 texCoord0;\nout vec4 vertexColor;\n\
            void main() { gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0); texCoord0 = UV0; vertexColor = Color; }\n";
        let glow_fsh = "#version 330\n#include <m:helper.glsl>\n\
            in vec2 texCoord0;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() { fragColor = tint(vertexColor); }\n";
        let sample_fsh = "#version 330\n#include <sunrise:pipeline_sprite.glsl>\n\
            in vec2 texCoord0;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() { fragColor = sampleSprite(texCoord0) * vertexColor; }\n";
        let broken_fsh =
            "#version 330\nout vec4 fragColor;\nvoid main() { fragColor = unknown_var; }\n";
        let files: BTreeMap<String, Vec<u8>> = [
            (
                "assets/m/shaders/include/helper.glsl",
                "vec4 tint(vec4 c) { return c * 0.5; }\n",
            ),
            ("assets/m/shaders/core/glow.vsh", vsh),
            ("assets/m/shaders/core/glow.fsh", glow_fsh),
            // Fragment-only program reusing default minecraft:core/position_tex_color.vsh stub:
            ("assets/m/shaders/core/deep/sample.fsh", sample_fsh),
            ("assets/m/shaders/core/broken.vsh", vsh),
            ("assets/m/shaders/core/broken.fsh", broken_fsh),
            // Old WGSL pipeline files and post shaders are not checked by the GLSL core rule.
            ("assets/m/shaders/pipeline/legacy.wgsl", "garbage"),
            ("assets/m/shaders/post/other.fsh", "garbage"),
            ("assets/m/lang/en_us.json", "{}"),
        ]
        .map(|(name, text)| (name.to_owned(), text.as_bytes().to_vec()))
        .into();
        let (count, refused) = pipeline_shaders(&files, None);
        assert_eq!(count, 3);
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert!(
            refused[0].starts_with("assets/m/shaders/core/broken.fsh:3:"),
            "{refused:?}"
        );
    }

    /// A uniform block and a sampler that an `#include` file declares are the program's own, as
    /// they are once the game inlines the include: the check takes them as the mod's bindings
    /// rather than refusing them (#5905).
    #[test]
    fn a_block_and_sampler_an_include_declares_are_the_programs() {
        let vsh = "#version 330\n#include <minecraft:dynamictransforms.glsl>\n\
            #include <minecraft:projection.glsl>\n\
            in vec3 Position;\nin vec2 UV0;\nin vec4 Color;\n\
            out vec2 texCoord0;\nout vec4 vertexColor;\n\
            void main() { gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0); texCoord0 = UV0; vertexColor = Color; }\n";
        let fsh = "#version 330\n#include <m:params.glsl>\n\
            in vec2 texCoord0;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() { fragColor = vertexColor * Strength * texture(Noise, texCoord0); }\n";
        let files: BTreeMap<String, Vec<u8>> = [
            (
                "assets/m/shaders/include/params.glsl",
                "layout(std140) uniform Params {\n    vec4 Strength;\n};\nuniform sampler2D Noise;\n",
            ),
            ("assets/m/shaders/core/blur.vsh", vsh),
            ("assets/m/shaders/core/blur.fsh", fsh),
        ]
        .map(|(name, text)| (name.to_owned(), text.as_bytes().to_vec()))
        .into();
        let (count, refused) = pipeline_shaders(&files, None);
        assert_eq!((count, refused), (1, Vec::<String>::new()));
    }

    /// A vertex shader of the default format that writes what the fragment shaders below read.
    const MOD_VSH: &str = "#version 330\n#include <minecraft:dynamictransforms.glsl>\n\
        #include <minecraft:projection.glsl>\n\
        in vec3 Position;\nin vec2 UV0;\nin vec4 Color;\n\
        out vec2 texCoord0;\nout vec4 vertexColor;\n\
        void main() { gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0); texCoord0 = UV0; vertexColor = Color; }\n";

    /// The files of one program `m:core/blur` with the vertex shader above and `fsh`.
    fn blur_program(fsh: &str) -> BTreeMap<String, Vec<u8>> {
        [
            ("assets/m/shaders/core/blur.vsh", MOD_VSH),
            ("assets/m/shaders/core/blur.fsh", fsh),
        ]
        .map(|(name, text)| (name.to_owned(), text.as_bytes().to_vec()))
        .into()
    }

    /// A fragment shader whose set-1 uniform block and sampler stand as the mod's own layout
    /// declares them passes, as the game binds them; the same shader with a syntax error in
    /// `main` is refused at that line, and not for its bindings (#6022).
    #[test]
    fn a_set_1_block_and_sampler_pass_and_a_syntax_error_is_still_refused() {
        let fsh = |statement_end: &str| {
            format!(
                "#version 330\nlayout(set = 1, binding = 0, std140) uniform Params {{ vec4 Strength; }};\n\
                uniform sampler2D Noise;\n\
                in vec2 texCoord0;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
                void main() {{ fragColor = vertexColor * Strength * texture(Noise, texCoord0){statement_end} }}\n"
            )
        };
        let (count, refused) = pipeline_shaders(&blur_program(&fsh(";")), None);
        assert_eq!((count, refused), (1, Vec::<String>::new()));

        let (count, refused) = pipeline_shaders(&blur_program(&fsh("")), None);
        assert_eq!(count, 1);
        assert!(!refused.is_empty(), "a syntax error must be refused");
        assert!(
            refused[0].starts_with("assets/m/shaders/core/blur.fsh:7:"),
            "{refused:?}"
        );
        assert!(
            refused
                .iter()
                .all(|r| !r.contains("not a block or sampler")),
            "{refused:?}"
        );
    }

    /// The game binds a declaration whatever else is on its line and whatever qualifier stands
    /// before `uniform`, so the check takes each as the mod's (#6022).
    #[test]
    fn declarations_the_game_binds_are_the_mods_whatever_their_line_looks_like() {
        let programs = [
            (
                "a qualifier before uniform",
                "uniform Scene { vec4 tint; };\nhighp uniform sampler2D Tex;",
                "tint * texture(Tex, texCoord0)",
            ),
            (
                "two samplers on one line",
                "uniform sampler2D TexA; uniform sampler2D TexB;",
                "texture(TexA, texCoord0) * texture(TexB, texCoord0)",
            ),
            (
                "a block after a sampler on one line",
                "uniform sampler2D TexA; uniform Scene { vec4 tint; };",
                "tint * texture(TexA, texCoord0)",
            ),
            (
                "a storage buffer after a sampler on one line",
                "uniform sampler2D TexA; layout(std430) readonly buffer Data { vec4 d[]; };",
                "d[0] * texture(TexA, texCoord0)",
            ),
        ];
        for (what, declarations, colour) in programs {
            let fsh = format!(
                "#version 330\n{declarations}\nin vec2 texCoord0;\nin vec4 vertexColor;\n\
                out vec4 fragColor;\nvoid main() {{ fragColor = vertexColor * {colour}; }}\n"
            );
            let (count, refused) = pipeline_shaders(&blur_program(&fsh), None);
            assert_eq!((count, refused), (1, Vec::<String>::new()), "{what}");
        }
    }

    /// The same authored include position can fail in either stage; the CLI names which one.
    #[test]
    fn a_shared_include_error_names_the_stage_that_failed() {
        let include = "#include <m:shared.glsl>\n";
        let shared = "#ifdef BROKEN\nvec4 sharedColor() {\n    return missing_color;\n}\n\
                      #else\nvec4 sharedColor() { return vec4(1.0); }\n#endif\n";
        for (vertex_define, fragment_define, failing_stage) in [
            ("#define BROKEN\n", "", "vertex"),
            ("", "#define BROKEN\n", "fragment"),
        ] {
            let vertex = format!(
                "#version 330\n{vertex_define}{include}void main() {{ gl_Position = sharedColor(); }}\n"
            );
            let fragment = format!(
                "#version 330\n{fragment_define}{include}out vec4 fragColor;\n\
                 void main() {{ fragColor = sharedColor(); }}\n"
            );
            let files = BTreeMap::from([
                (
                    "assets/m/shaders/core/shared.vsh".to_owned(),
                    vertex.into_bytes(),
                ),
                (
                    "assets/m/shaders/core/shared.fsh".to_owned(),
                    fragment.into_bytes(),
                ),
                (
                    "assets/m/shaders/include/shared.glsl".to_owned(),
                    shared.as_bytes().to_vec(),
                ),
            ]);
            let (count, refused) = pipeline_shaders(&files, None);
            assert_eq!((count, refused.len()), (1, 1), "{refused:?}");
            let prefix =
                format!("assets/m/shaders/include/shared.glsl:3:12: {failing_stage} shader: ");
            assert!(refused[0].starts_with(&prefix), "{refused:?}");
        }
    }

    /// A program is textured when the text the translator reads uses a sprite name, as it does
    /// once the game inlines the includes: a fragment shader that reaches `sampleSprite` only
    /// through its own include file is checked as textured, not refused for a missing sprite
    /// texture.
    #[test]
    fn a_sprite_an_include_samples_makes_the_program_textured() {
        let fsh = "#version 330\n#include <m:tinted.glsl>\n\
            in vec2 texCoord0;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() { fragColor = tintedSprite(texCoord0) * vertexColor; }\n";
        let mut files = blur_program(fsh);
        files.insert(
            "assets/m/shaders/include/tinted.glsl".to_owned(),
            b"#include <sunrise:pipeline_sprite.glsl>\n\
                vec4 tintedSprite(vec2 uv) { return sampleSprite(uv) * 0.5; }\n"
                .to_vec(),
        );
        let (count, refused) = pipeline_shaders(&files, None);
        assert_eq!((count, refused), (1, Vec::<String>::new()));
    }

    /// The vertex inputs are those of the stage with its includes inlined, as the translator
    /// validates them: a vertex shader whose extra `in` comes from its own include is checked
    /// against that input, not refused as undeclared.
    #[test]
    fn a_vertex_input_an_include_declares_is_the_programs() {
        let vsh = "#version 330\n#include <minecraft:dynamictransforms.glsl>\n\
            #include <minecraft:projection.glsl>\n#include <m:offset.glsl>\n\
            in vec3 Position;\nin vec2 UV0;\nin vec4 Color;\n\
            out vec2 texCoord0;\nout vec4 vertexColor;\n\
            void main() { gl_Position = ProjMat * ModelViewMat * vec4(Position + Offset, 1.0); texCoord0 = UV0; vertexColor = Color; }\n";
        let fsh = "#version 330\n\
            in vec2 texCoord0;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() { fragColor = vertexColor; }\n";
        let files: BTreeMap<String, Vec<u8>> = [
            ("assets/m/shaders/include/offset.glsl", "in vec3 Offset;\n"),
            ("assets/m/shaders/core/blur.vsh", vsh),
            ("assets/m/shaders/core/blur.fsh", fsh),
        ]
        .map(|(name, text)| (name.to_owned(), text.as_bytes().to_vec()))
        .into();
        let (count, refused) = pipeline_shaders(&files, None);
        assert_eq!((count, refused), (1, Vec::<String>::new()));
    }

    /// A vertex shader with `vsh_in` as its inputs and `main_body` as its body, with
    /// `blur.fsh` of the default format's outputs.
    fn vertex_program(vsh_in: &str, main_body: &str) -> BTreeMap<String, Vec<u8>> {
        let vsh = format!(
            "#version 330\n#include <minecraft:dynamictransforms.glsl>\n\
            #include <minecraft:projection.glsl>\n{vsh_in}\
            out vec2 texCoord0;\nout vec4 vertexColor;\n\
            void main() {{ {main_body} }}\n"
        );
        let fsh = "#version 330\nin vec2 texCoord0;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() { fragColor = vertexColor; }\n";
        [
            ("assets/m/shaders/core/blur.vsh", vsh.as_str()),
            ("assets/m/shaders/core/blur.fsh", fsh),
        ]
        .map(|(name, text)| (name.to_owned(), text.as_bytes().to_vec()))
        .into()
    }

    /// A program that declares only `Position` is checked against that input alone: it passes,
    /// and reading `UV0` it never declared is refused, as the game (which builds the inputs from
    /// the format's declared ones) would; checked as if it declared all three defaults, the
    /// undeclared read compiled.
    #[test]
    fn a_program_declaring_one_default_input_is_checked_against_that_input() {
        let body = "gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);";
        let (count, refused) = pipeline_shaders(&vertex_program("in vec3 Position;\n", body), None);
        assert_eq!((count, refused), (1, Vec::<String>::new()));

        let reads_uv0 = format!("{body} texCoord0 = UV0;");
        let (count, refused) =
            pipeline_shaders(&vertex_program("in vec3 Position;\n", &reads_uv0), None);
        assert_eq!(count, 1);
        assert!(
            refused
                .iter()
                .any(|r| r.starts_with("assets/m/shaders/core/blur.vsh")),
            "{refused:?}"
        );
    }

    /// The check cannot see the format a pipeline registers, so an `in` of any type a vertex
    /// attribute can have passes, whatever its name: `Normal`, which vanilla formats have, and a
    /// custom `Tangent` a mod's own binding would supply.
    #[test]
    fn a_vertex_input_of_any_attribute_type_passes_whatever_its_name() {
        for (inputs, body) in [
            (
                "in vec3 Position;\nin vec3 Normal;\n",
                "gl_Position = ProjMat * ModelViewMat * vec4(Position + Normal, 1.0);",
            ),
            (
                "in vec3 Position;\nin vec4 Tangent;\n",
                "gl_Position = ProjMat * ModelViewMat * vec4(Position + Tangent.xyz, 1.0);",
            ),
        ] {
            let (count, refused) = pipeline_shaders(&vertex_program(inputs, body), None);
            assert_eq!((count, refused), (1, Vec::<String>::new()), "{inputs}");
        }
    }

    /// An `in` of a type no vertex attribute format supplies is refused, naming it, at its line.
    #[test]
    fn a_vertex_input_no_attribute_format_supplies_is_refused() {
        let body = "gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0) * Bones;";
        let inputs = "in vec3 Position;\nin mat4 Bones;\n";
        let (count, refused) = pipeline_shaders(&vertex_program(inputs, body), None);
        assert_eq!(count, 1);
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert!(
            refused[0].starts_with("assets/m/shaders/core/blur.vsh:5:")
                && refused[0].contains("`mat4` for `Bones`"),
            "{refused:?}"
        );
    }

    /// A fragment shader that includes `oit.glsl` only through an include of its own is checked
    /// against the OIT passes: its body is refused there, as it is with the direct include.
    #[test]
    fn an_oit_include_reached_through_another_include_is_checked_against_the_oit_passes() {
        let fsh = "#version 330\n#include <m:wrapped.glsl>\n\
            in vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() {\n#ifdef OIT_ALPHA_ONLY\n\
            executeAlphaOnlyPhase(gl_FragCoord.z, vertexColor.a);\n#else\n\
            fragColor = sampleColorForAccumulation(vertexColor);\n#endif\n}\n";
        let mut files = blur_program(fsh);
        files.insert(
            "assets/m/shaders/include/wrapped.glsl".to_owned(),
            b"#include <minecraft:oit.glsl>\n".to_vec(),
        );
        let (count, refused) = pipeline_shaders(&files, None);
        assert_eq!(count, 1);
        assert!(
            refused
                .iter()
                .any(|r| r.contains("blur.fsh") && r.contains("OIT depth_bounds")),
            "{refused:?}"
        );
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sunrise-mod-package/tests/index")
            .join(name)
    }

    #[test]
    fn checks_each_kind_its_own_way() {
        let pack = fixture("cozy-1.0.0.spack");
        let report = inspect(&pack, kind_of(&pack).unwrap()).unwrap();
        assert_eq!(report.facts.requires[0].id, "coords_hud");
        assert!(
            report
                .text()
                .contains("cozy 1.0.0 (modpack, format 1, for 26.3)")
        );
        let smod = fixture("coords_hud-0.1.0.smod");
        let report = inspect(&smod, Kind::Mod).unwrap();
        assert!(report.text().contains("no code: content only"));
        assert!(inspect(&pack, Kind::Mod).is_err());
    }

    #[test]
    fn unsigned_smod_is_refused_and_an_explicit_bundle_is_read() {
        let smod = fixture("coords_hud-0.1.0.smod");
        let missing = check_release(&smod, Some(Kind::Mod), None, None)
            .err()
            .unwrap();
        assert!(missing.ends_with("coords_hud-0.1.0.smod.sigstore.json"));

        let bundle = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sunrise-attestation/tests/fixtures/ruff-source.sigstore.json");
        let mismatch = check_release(&smod, None, Some(&bundle), None)
            .err()
            .unwrap();
        assert!(
            mismatch.contains("artifact hash does not match"),
            "{mismatch}"
        );
        // A file that isn't even an archive fails on its attestation, before anything parses it.
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.smod");
        std::fs::write(&junk, b"not an archive").unwrap();
        let refused = check_release(&junk, None, Some(&bundle), None)
            .err()
            .unwrap();
        assert!(
            refused.contains("artifact hash does not match"),
            "{refused}"
        );
    }

    #[test]
    fn a_mod_package_needs_its_attestation_whatever_its_name_or_kind() {
        let dir = tempfile::tempdir().unwrap();
        let renamed = dir.path().join("pack.zip");
        std::fs::copy(fixture("coords_hud-0.1.0.smod"), &renamed).unwrap();
        for kind in [None, Some(Kind::ResourcePack)] {
            assert!(check_release(&renamed, kind, None, None).is_err());
        }
        let missing = check_release(&renamed, None, None, None).err().unwrap();
        assert!(missing.ends_with("pack.zip.sigstore.json"), "{missing}");
        let smod = fixture("coords_hud-0.1.0.smod");
        assert!(check_release(&smod, Some(Kind::ResourcePack), None, None).is_err());
    }

    #[test]
    fn release_signer_must_be_the_shared_workflow_at_a_release_tag() {
        // Test policy on verified claims; the verifier crate exercises real signed bundles.
        let attested = Attested {
            repository_id: 123456,
            owner_id: 654321,
            repository_name: "author/repo".into(),
            signer_workflow_ref:
                "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/tags/release-v1".into(),
            source_ref: "refs/tags/v1.0.0".into(),
            source_commit: "cc67ad5440ad86ceddef5be9c7b1f931d5688a2a".into(),
            predicate_type: "https://slsa.dev/provenance/v1".into(),
        };
        assert_eq!(
            require_release_signer(attested.clone(), Some(123456)),
            Ok(attested.clone())
        );
        for signer_workflow_ref in [
            "author/repo/.github/workflows/release.yml@refs/tags/release-v1",
            "sunrise-org/sunrise-mods/.github/workflows/publish.yml@refs/tags/release-v1",
            "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/heads/release-v1",
            "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/tags/release-vnext",
            "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/tags/release-v1.2",
        ] {
            let error = require_release_signer(
                Attested {
                    signer_workflow_ref: signer_workflow_ref.into(),
                    ..attested.clone()
                },
                None,
            )
            .unwrap_err();
            assert!(error.contains(signer_workflow_ref), "{error}");
        }
        let repository = require_release_signer(attested, Some(1)).unwrap_err();
        assert_eq!(
            repository,
            "expected repository ID 1, attestation names author/repo (ID 123456)"
        );
    }

    #[test]
    fn dependencies_are_reported_as_required_or_optional() {
        let dependencies = json!([
            {"id": "creature", "version": "^0.1"},
            {"id": "jei", "version": "^0.5", "optional": true, "side": "client"}
        ]);
        let report = Report {
            kind: Kind::NativeMod,
            facts: Facts {
                id: Some("fireflies".into()),
                version: Some("1.0.0".into()),
                dependencies: serde_json::from_value(dependencies.clone()).unwrap(),
                ..Facts::default()
            },
            sha256: "0".repeat(64),
            size: 1,
            signed: false,
            attestation: None,
        };
        let text = report.text();
        for word in ["creature", "^0.1", "jei", "^0.5"] {
            assert!(text.contains(word), "{text}");
        }
        assert_eq!(report.json()["dependencies"], dependencies);
    }

    /// #6022: a shader using a set-1 uniform block and a sampler of the mod's own layout passes
    /// `check`, declared in the shader or in an include, as the game accepts it; the same shader
    /// with a real syntax error is still refused.
    #[test]
    fn pipeline_shaders_take_the_mods_own_set_1_block_and_sampler() {
        const DECLARE: &str =
            "layout(std140, set = 1, binding = 0) uniform SceneUniform { vec4 Tint; };
layout(set = 1, binding = 1) uniform sampler2D SsaoTex;
";
        const BODY: &str = "in vec2 texCoord0;
out vec4 fragColor;
void main() { fragColor = Tint * texture(SsaoTex, texCoord0); }
";
        let check = |fsh: String| {
            let mut files = BTreeMap::new();
            files.insert(
                "assets/example/shaders/core/ssao.fsh".to_owned(),
                fsh.into_bytes(),
            );
            files.insert(
                "assets/example/shaders/include/scene.glsl".to_owned(),
                DECLARE.as_bytes().to_vec(),
            );
            pipeline_shaders(&files, None)
        };
        for fsh in [
            format!(
                "#version 330
{DECLARE}{BODY}"
            ),
            format!(
                "#version 330
#moj_import <example:scene.glsl>
{BODY}"
            ),
            format!(
                "#version 330
#include <example:scene.glsl>
{BODY}"
            ),
        ] {
            let (count, refused) = check(fsh.clone());
            assert_eq!((count, refused), (1, Vec::<String>::new()), "{fsh}");
        }
        let broken = format!(
            "#version 330
{DECLARE}{}",
            BODY.replace("Tint *", "Tint * *")
        );
        let (_, refused) = check(broken);
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert!(
            refused[0].starts_with("assets/example/shaders/core/ssao.fsh:")
                && !refused[0].contains("is not a block or sampler"),
            "{refused:?}"
        );
    }

    #[test]
    fn pipeline_shaders_validates_oit_variants_and_refuses_alpha_only_color_output() {
        let mut files = BTreeMap::new();
        files.insert(
            "assets/example/shaders/core/good_oit.fsh".to_owned(),
            b"\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:oit.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
#ifndef OIT_ALPHA_ONLY
out vec4 fragColor;
#endif
void main() {
    vec4 color = vertexColor * ColorModulator;
#ifdef OIT_ALPHA_ONLY
    executeAlphaOnlyPhase(gl_FragCoord.z, color.a);
#else
#ifdef OIT_ACCUMULATE
    color = sampleColorForAccumulation(color);
#endif
    fragColor = color;
#endif
}
"
            .to_vec(),
        );
        let (count, refused) = pipeline_shaders(&files, None);
        assert_eq!(count, 1);
        assert!(
            refused.is_empty(),
            "expected valid OIT shader to pass: {refused:?}"
        );

        files.insert(
            "assets/example/shaders/core/bad_oit.fsh".to_owned(),
            b"\
#version 330
#include <minecraft:oit.glsl>
in vec4 vertexColor;
out vec4 fragColor;
void main() {
#ifdef OIT_ALPHA_ONLY
    executeAlphaOnlyPhase(gl_FragCoord.z, vertexColor.a);
#else
    fragColor = sampleColorForAccumulation(vertexColor);
#endif
}
"
            .to_vec(),
        );
        let (count, refused) = pipeline_shaders(&files, None);
        assert_eq!(count, 2);
        assert!(
            refused
                .iter()
                .any(|r| r.contains("bad_oit.fsh") && r.contains("OIT depth_bounds")),
            "expected bad OIT shader refusal at depth_bounds, got: {refused:?}"
        );
    }
}
