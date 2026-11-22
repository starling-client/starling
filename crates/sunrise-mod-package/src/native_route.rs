//! Classify native build inputs without loading the SDK or running Cargo.
use crate::{Manifest, manifest::Native};

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
}

/// Classify a mod from its parsed manifest and Cargo metadata. Registry dependencies take
/// precedence over game code, since they need the whole SDK as well as Cargo's build.
pub fn requirement<'a>(manifest: &Manifest, cargo: &'a toml::Table) -> Requirement<'a> {
    if let Some(package) = dependency_tables(cargo)
        .flat_map(|(_, deps)| deps.iter().map(|(key, value)| package_name(key, value)))
        .find(|name| !PACKAGES.iter().any(|(package, _)| package == name))
    {
        Requirement::Registry(package)
    } else if let Some(reason) = changes_game(manifest, cargo) {
        Requirement::Engine(reason)
    } else {
        Requirement::Api
    }
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
        state,
        r#loop,
    }) = &manifest.native
    {
        for (empty, table) in [
            (mixin.is_empty(), "[[native.mixin]]"),
            (module.is_empty(), "[[native.module]]"),
            (bridge.is_empty(), "[[native.bridge]]"),
            (state.is_empty(), "[[native.state]]"),
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
