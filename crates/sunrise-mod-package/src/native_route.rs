//! Classify native build inputs without loading the SDK or running Cargo.
use crate::{Manifest, manifest::Native};
use std::path::Path;

/// SDK package for the mod API.
pub const API_PACKAGE: &str = "sunrise-mod-api";
/// SDK package for the per-mod internals facade.
pub const INTERNALS_PACKAGE: &str = "sunrise-internals";
/// SDK packages a mod may link and the names of their libraries.
pub const PACKAGES: [(&str, &str); 2] =
    [(API_PACKAGE, "mod_api"), (INTERNALS_PACKAGE, "internals")];

/// Whether selecting `candidate` for the same registry crate would move the game's pin.
/// Cargo unifies versions on the same major line, `0.minor` line, or exact `0.0.patch` line.
/// An advisory without Cargo resolution can call this with a candidate its requirement accepts.
pub fn game_registry_pin_conflicts(game: &semver::Version, candidate: &semver::Version) -> bool {
    game != candidate
        && game.major == candidate.major
        && (game.major != 0
            || (game.minor == candidate.minor
                && (game.minor != 0 || game.patch == candidate.patch)))
}

/// How one mod must be compiled, from least to most demanding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement<'a> {
    Api,
    /// A game-code manifest table or the internals facade.
    Engine(&'static str),
    /// The name of a dependency outside the SDK.
    Registry(&'a str),
    /// A build script or build dependencies, which only Cargo's build runs.
    BuildScript,
}

/// Classify a mod from its parsed manifest, its Cargo metadata and its folder `dir`. Registry
/// dependencies and build scripts take precedence over game code, since they need the whole SDK
/// as well as Cargo's build.
pub fn requirement<'a>(manifest: &Manifest, cargo: &'a toml::Table, dir: &Path) -> Requirement<'a> {
    if let Some(package) = dependency_tables(cargo)
        .flat_map(|(_, deps)| deps.iter().map(|(key, value)| package_name(key, value)))
        .find(|name| !PACKAGES.iter().any(|(package, _)| package == name))
    {
        Requirement::Registry(package)
    } else if runs_build_script(cargo, dir) {
        Requirement::BuildScript
    } else if let Some(reason) = changes_game(manifest, cargo) {
        Requirement::Engine(reason)
    } else {
        Requirement::Api
    }
}

/// Whether Cargo would run a build script for the mod in `dir`: `package.build` names one, or
/// `build.rs` is beside the manifest and neither `package.build = false` nor
/// `package.autobuild = false` switches it off; or it declares build dependencies, in any
/// `[build-dependencies]` table.
fn runs_build_script(cargo: &toml::Table, dir: &Path) -> bool {
    let package = cargo.get("package");
    let script = match package.and_then(|package| package.get("build")) {
        Some(build) => build.as_bool() != Some(false),
        None => {
            package
                .and_then(|package| package.get("autobuild"))
                .and_then(toml::Value::as_bool)
                != Some(false)
                && dir.join("build.rs").is_file()
        }
    };
    let targets = cargo
        .get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .filter_map(|(_, config)| config.as_table());
    script
        || std::iter::once(cargo).chain(targets).any(|table| {
            // Cargo still reads the underscore spelling before edition 2024.
            ["build-dependencies", "build_dependencies"]
                .into_iter()
                .filter_map(|kind| table.get(kind)?.as_table())
                .any(|dependencies| !dependencies.is_empty())
        })
}

/// The game-code table or internals facade that ties this mod to its exact game build.
pub fn changes_game(manifest: &Manifest, cargo: &toml::Table) -> Option<&'static str> {
    manifest_game_code(manifest).or_else(|| {
        dependency_tables(cargo)
            .any(|(_, deps)| {
                deps.iter()
                    .any(|(key, value)| package_name(key, value) == INTERNALS_PACKAGE)
            })
            .then_some("internals")
    })
}

/// The game-code table declared by a manifest, independent of its Cargo dependencies.
pub fn manifest_game_code(manifest: &Manifest) -> Option<&'static str> {
    // Every row kind by name: a new one does not compile until it is routed here.
    if let Some(Native {
        game: _,
        sunrise: _,
        mixins: _,
        serves_content: _,
        mixin,
        module,
        bridge,
        r#loop,
    }) = &manifest.native
    {
        for (empty, table) in [
            (mixin.is_empty(), "[[native.mixin]]"),
            (module.is_empty(), "[[native.module]]"),
            (bridge.is_empty(), "[[native.bridge]]"),
            (r#loop.is_empty(), "[[native.loop]]"),
        ] {
            if !empty {
                return Some(table);
            }
        }
    }
    (!manifest.internals.is_empty()).then_some("internals")
}

fn package_name<'a>(key: &'a str, value: &'a toml::Value) -> &'a str {
    value
        .get("package")
        .and_then(toml::Value::as_str)
        .unwrap_or(key)
}

/// The manifest's dependency tables: `[dependencies]` (no target), then each
/// `[target.<cfg>.dependencies]`.
pub fn dependency_tables(
    cargo: &toml::Table,
) -> impl Iterator<Item = (Option<&str>, &toml::Table)> {
    let targets = cargo
        .get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .map(|(target, config)| (Some(target.as_str()), config.get("dependencies")));
    std::iter::once((None, cargo.get("dependencies")))
        .chain(targets)
        .filter_map(|(target, table)| Some((target, table?.as_table()?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_runs_a_build_script_when_the_manifest_or_the_folder_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let scripted = |manifest: &str| {
            runs_build_script(&manifest.parse::<toml::Table>().unwrap(), dir.path())
        };
        let package = "[package]\nname = 'm'\nversion = '1.0.0'\n";
        assert!(!scripted(package));
        assert!(scripted(&format!("{package}build = 'tools/make.rs'\n")));
        assert!(scripted(&format!(
            "{package}[build-dependencies]\ncc = '1'\n"
        )));
        assert!(scripted(&format!(
            "{package}[target.'cfg(unix)'.build-dependencies]\ncc = '1'\n"
        )));
        assert!(!scripted(&format!("{package}[build-dependencies]\n")));
        std::fs::write(dir.path().join("build.rs"), "fn main() {}").unwrap();
        assert!(scripted(package));
        assert!(!scripted(&format!("{package}build = false\n")));
        assert!(!scripted(&format!("{package}autobuild = false\n")));
    }
}
