//! `cargo sunrise-mod check <file>`: what the registry checks of a submission,
//! run anywhere: its kind's own checks (docs/modding/src/reference/registry-index.md), its signature
//! when given a key, and for a `.smod` whether it contains native source or only content.
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use sunrise_mod_package::Package;
use sunrise_mod_package::index::{self, Facts, Kind};
use sunrise_mod_package::manifest::{MANIFEST, MAX_USES};
use sunrise_shader_api::mod_pipeline;

pub struct Report {
    pub kind: Kind,
    pub facts: Facts,
    pub sha256: String,
    pub size: u64,
    pub signed: bool,
}

impl Report {
    pub fn json(&self) -> Value {
        let requires: Vec<Value> = self
            .facts
            .requires
            .iter()
            .map(|r| json!({"id": r.id, "version": r.version, "sha256": r.sha256}))
            .collect();
        json!({
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
        })
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
        } else if self.kind == Kind::Shader {
            out += &format!(", shader API {}", f.api);
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
        "sshader" => Some(Kind::Shader),
        _ => None,
    }
}

/// Checks the file at `path` as `kind`; `key` and `signature` (the
/// `.minisig` text) check its signature.
pub fn check(
    path: &Path,
    kind: Kind,
    key: Option<&str>,
    signature: Option<&str>,
) -> Result<Report, String> {
    check_package(path, kind, key, signature, true)
}

/// Inspect a reviewed source when adding only a build. An older source without a uses list
/// stays exact to its pin; the registry still verifies its source and judges the appended build.
pub(crate) fn check_reviewed_source(
    path: &Path,
    kind: Kind,
    key: Option<&str>,
    signature: Option<&str>,
) -> Result<Report, String> {
    check_package(path, kind, key, signature, false)
}

fn check_package(
    path: &Path,
    kind: Kind,
    key: Option<&str>,
    signature: Option<&str>,
    require_uses: bool,
) -> Result<Report, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if size > index::MAX_FILE {
        return Err(format!("over {} MiB", index::MAX_FILE >> 20));
    }
    index::open(&kind)?;
    let facts = index::inspect(&kind, path)?;
    if require_uses && kind == Kind::NativeMod {
        uses_list(path)?;
    }
    let signed = match (key, signature) {
        (Some(key), Some(sig)) => {
            index::verify_signature(key, sig, path)?;
            true
        }
        (None, None) => false,
        _ => return Err("--key and --signature go together".into()),
    };
    Ok(Report {
        kind,
        facts,
        sha256: index::sha256_file(path)?,
        size,
        signed,
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

/// The references among a project's content files ([`sunrise_mod_package::content::check_references`]):
/// warnings are printed, errors fail with one line each.
pub fn references(root: &Path, manifest: &sunrise_mod_package::Manifest) -> Result<(), String> {
    let content = sunrise_mod_package::Content::read(root, manifest)?;
    let findings = sunrise_mod_package::content::check_references(&content, manifest)?;
    for warning in &findings.warnings {
        eprintln!("warning: {warning}");
    }
    if findings.errors.is_empty() {
        Ok(())
    } else {
        Err(findings.errors.join("\nerror: "))
    }
}

/// Composes and validates every render pipeline fragment file in a mod's `files`
/// (`assets/<namespace>/shaders/pipeline/<path>.wgsl`) with the game's own code
/// (`sunrise_shader_api::mod_pipeline`), returning how many there are and each refusal as
/// `<file>:<line>:<column>: <message>` (`<file>: <message>` where it has no span), the text the game logs after the pipeline's id. Whether a pipeline is
/// textured is in the mod's code, which this cannot see: a file that calls `pipeline_texture`
/// is composed as textured, any other as untextured.
pub fn pipeline_shaders(files: &BTreeMap<String, Vec<u8>>) -> (usize, Vec<String>) {
    let mut count = 0;
    let mut refused = Vec::new();
    for (name, bytes) in files {
        let mut parts = name.splitn(3, '/');
        let in_folder = parts.next() == Some("assets")
            && parts.next().is_some()
            && parts.next().is_some_and(|rest| {
                rest.strip_prefix(mod_pipeline::FOLDER)
                    .is_some_and(|rest| rest.starts_with('/') && rest.ends_with(".wgsl"))
            });
        if !in_folder {
            continue;
        }
        count += 1;
        let composed = std::str::from_utf8(bytes)
            .map_err(|e| e.to_string())
            .map_err(|e| vec![format!("{name}: {e}")])
            .and_then(|text| {
                mod_pipeline::compose_with(name, text, text.contains("pipeline_texture"))
                    .map_err(|problems| problems.iter().map(ToString::to_string).collect())
            });
        if let Err(reasons) = composed {
            refused.extend(reasons);
        }
    }
    (count, refused)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_files_are_composed_as_the_game_does() {
        let glow =
            "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return input.color; }";
        let sample = "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return pipeline_texture(input.uv); }";
        let files: BTreeMap<String, Vec<u8>> = [
            ("assets/m/shaders/pipeline/glow.wgsl", glow),
            ("assets/m/shaders/pipeline/deep/sample.wgsl", sample),
            ("assets/m/shaders/pipeline/broken.wgsl", "fn nothing() {}"),
            (
                "assets/m/shaders/pipeline/typo.wgsl",
                "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {\n    let x: f32 = input.uv;\n    return vec4<f32>(x);\n}",
            ),
            // Not a pipeline file: other shaders are not this check's.
            ("assets/m/shaders/post/other.wgsl", "garbage"),
            ("assets/m/lang/en_us.json", "{}"),
        ]
        .map(|(name, text)| (name.to_owned(), text.as_bytes().to_vec()))
        .into();
        let (count, refused) = pipeline_shaders(&files);
        assert_eq!(count, 4);
        assert_eq!(
            refused,
            [
                "assets/m/shaders/pipeline/broken.wgsl: no function `pipeline_fragment`: expected `fn pipeline_fragment(input: PipelineInput) -> vec4<f32>`",
                // The binding `x` is at line 2, column 9 of the mod's own file.
                "assets/m/shaders/pipeline/typo.wgsl:2:9: the type of `x` is expected to be `f32`, but got `vec2<f32>`",
            ]
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
        let report = check(&pack, kind_of(&pack).unwrap(), None, None).unwrap();
        assert_eq!(report.facts.requires[0].id, "coords_hud");
        assert!(
            report
                .text()
                .contains("cozy 1.0.0 (modpack, format 1, for 26.3)")
        );
        // A signed content mod passes; a bad signature is still refused.
        let smod = fixture("coords_hud-0.1.0.smod");
        let key = std::fs::read_to_string(fixture("test.pub")).unwrap();
        let sig = std::fs::read_to_string(fixture("coords_hud-0.1.0.smod.minisig")).unwrap();
        let key = key.lines().last().unwrap();
        let report = check(&smod, Kind::Mod, Some(key), Some(&sig)).unwrap();
        assert!(report.signed);
        assert!(report.text().contains("no code: content only"));
        let e = check(
            &smod,
            Kind::Mod,
            Some(key),
            Some("untrusted comment: x\nRUQ=\n"),
        )
        .err()
        .unwrap();
        assert!(e.contains("signature"), "{e}");
        let e = check(&pack, Kind::Shader, None, None).err().unwrap();
        assert!(e.contains("sunrise-shaders.toml"), "{e}");
        assert!(check(&pack, Kind::Mod, None, None).is_err());
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
        };
        let text = report.text();
        for word in ["creature", "^0.1", "jei", "^0.5"] {
            assert!(text.contains(word), "{text}");
        }
        assert_eq!(report.json()["dependencies"], dependencies);
    }
}
