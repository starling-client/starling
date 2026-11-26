//! What a code mod uses of the surface (design-fit.md 10.2): the scan of its source against the
//! pinned SDK's `surface.json`, the table `build` packs as `uses.json`, and `check`'s summary.
use crate::sdk_fetch::PinnedSdk;
use mod_source::mod_uses::{self, ReadError};
use mod_surface::child::ChildEntry;
use mod_surface::uses::{Built, ManifestFacts, Origin, Uses, UsesTable, uses_contained};
use mod_surface::{ReadError as SurfaceError, Surface, leaf};
use std::collections::BTreeMap;
use std::path::Path;
use sunrise_mod_package::Manifest;
use sunrise_native_build::game_uses;

/// The surface export a fetched SDK holds. A schema this tool does not read names the two ways
/// out: a build this tool reads, or the tool that SDK's release ships.
pub(crate) fn surface_of(sdk: &PinnedSdk) -> Result<Surface, String> {
    read_surface(sdk)?.map_err(|e| match e {
        SurfaceError::Schema(_) => format!(
            "{}: {e}. Pin the mod to a build this tool reads with `cargo sunrise-mod pin <build>`, or use the tool in sdk-{}",
            sdk.surface.display(),
            sdk.build
        ),
        _ => format!("{}: {e}", sdk.surface.display()),
    })
}

/// A fetched SDK's surface export as this tool reads it: the outer error is the file's, the inner
/// one the surface's.
pub(crate) fn read_surface(sdk: &PinnedSdk) -> Result<Result<Surface, SurfaceError>, String> {
    let bytes =
        std::fs::read(&sdk.surface).map_err(|e| format!("{}: {e}", sdk.surface.display()))?;
    Ok(Surface::read(&bytes))
}

/// What the scan of a mod folder found.
pub(crate) enum Scanned {
    /// What the mod uses of the surface.
    Uses(Uses),
    /// The mod has rows no list can judge ([`game_uses::unjudged`]), so it is made for exactly
    /// one build: why.
    Exact(String),
    /// The scan cannot judge the source (a computed `include!`, a file it does not read, more
    /// source than it reads, a file it cannot parse): why. Such a mod has no table and counts as
    /// made for its build only; it may still build.
    Unread(String),
}

/// The sources of the mod folder at `root`, or what the scan can answer without them.
pub(crate) fn read(
    root: &Path,
    manifest: &Manifest,
) -> Result<Result<Vec<(String, String)>, Scanned>, String> {
    if let Some(table) = game_uses::unjudged(manifest) {
        return Ok(Err(Scanned::Exact(format!("it has a {table} table"))));
    }
    // A helper is compiled inside the game's code, where a name is the game's own, never the
    // mod API's: it is not read, and what it uses is declared by its row. A helper the mod's own
    // source also brings in cannot be judged either way, and the read says so.
    let helpers: Vec<&str> = manifest.native.as_ref().map_or_else(Vec::new, |native| {
        native
            .mixin
            .iter()
            .map(|row| row.helper.as_str())
            .chain(native.r#loop.iter().map(|row| row.helper.as_str()))
            .collect()
    });
    match mod_uses::read_sources(root, &helpers) {
        Ok(sources) => Ok(Ok(sources)),
        Err(e @ ReadError::Io { .. }) => Err(e.to_string()),
        Err(e) => Ok(Err(Scanned::Unread(e.to_string()))),
    }
}

/// What the mod at `root` uses of `surface`.
pub(crate) fn scan(
    entry: ChildEntry,
    root: &Path,
    manifest: &Manifest,
    surface: &Surface,
) -> Result<Scanned, String> {
    let sources = match read(root, manifest)? {
        Ok(sources) => sources,
        Err(scanned) => return Ok(scanned),
    };
    Ok(
        match uses_contained(
            entry,
            sources.iter().map(|(p, t)| (p.as_str(), t.as_str())),
            &ManifestFacts::from(manifest),
            surface,
        ) {
            Ok(uses) => Scanned::Uses(uses),
            Err(e) => Scanned::Unread(e.to_string()),
        },
    )
}

/// The table the mod pinned to `sdk` packs: what the scan found, then the game code
/// its manifest declares, each with its hash at the pinned build ([`game_uses::entries`]). The
/// build data is fetched only for a mod that declares some, from the signed SDK itself into a
/// private folder: the project's own cache is the checkout's to edit, so neither packing nor a fit
/// verdict reads it.
pub(crate) fn table(
    uses: &Uses,
    manifest: &Manifest,
    sdk: &PinnedSdk,
) -> Result<UsesTable, String> {
    let mut table = uses.table(sdk.build.clone());
    if game_uses::declared(manifest) {
        let staging = tempfile::tempdir().map_err(|e| format!("build data: {e}"))?;
        crate::sdk_fetch::release_data(
            &crate::sdk_fetch::Base::from_env(),
            &crate::sdk_fetch::keys(),
            &sdk.build,
            staging.path(),
            true,
        )?;
        let data = staging
            .path()
            .join(&sdk.build)
            .join(crate::sdk_fetch::BUILD_DATA);
        // An item the pinned build lacks is refused by the build, by name, and one its build data
        // cannot fingerprint is never shown unchanged; the list records the rest.
        table.entries.extend(
            game_uses::entries(manifest, &data, &sdk.build)?
                .into_iter()
                .filter_map(|(id, now)| now.into_entry(id)),
        );
        if table.entries.iter().any(|entry| entry.game_code.is_some()) {
            table.schema = mod_surface::uses::USES_SCHEMA;
        }
    }
    Ok(table)
}

/// `Greeter uses 14 items of the API (6 hooks, 5 methods, 3 types). Made for nightly-….`, then
/// what the author should know about how it builds each struct.
pub(crate) fn summary(name: &str, uses: &Uses, sdk: &PinnedSdk) -> String {
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for used in &uses.items {
        *kinds.entry(kind_word(&used.entry.id)).or_default() += 1;
    }
    let counts: Vec<String> = kinds
        .iter()
        .map(|(word, count)| format!("{count} {}", plural(word, *count)))
        .collect();
    let mut text = format!(
        "{name} uses {} {} of the API ({}). Made for {}.\n",
        uses.items.len(),
        plural("item", uses.items.len()),
        counts.join(", "),
        sdk.name
    );
    for note in uses.items.iter().filter_map(|used| used.note.as_deref()) {
        text.push_str(note);
        text.push('\n');
    }
    text
}

/// `check --uses`: each item with the place that brought it in.
pub(crate) fn list(uses: &Uses) -> String {
    let mut text = String::new();
    for used in &uses.items {
        let built = match used.entry.built {
            Some(Built::Full) => "  built field by field",
            Some(Built::Base) => "  built with `..`",
            _ => "",
        };
        text.push_str(&format!(
            "{}  {}{built}\n",
            used.entry.id,
            place(&used.origin)
        ));
    }
    text
}

/// Where an item came into the list.
pub(crate) fn place(origin: &Origin) -> String {
    match origin {
        Origin::Source { file, line } => format!("{file}:{line}"),
        Origin::Manifest => sunrise_mod_package::manifest::MANIFEST.to_owned(),
        Origin::Via(id) => format!("through {}", leaf(id)),
    }
}

/// The word for an id's kind, as the summary counts it.
fn kind_word(id: &str) -> &'static str {
    match id.split_once(':') {
        Some(("hook", rest)) if rest.contains('.') => "hook argument",
        Some(("hook", _)) => "hook",
        Some(("trait", _)) => "trait",
        Some(("method" | "handle", _)) => "method",
        Some(("assoc", _)) => "associated item",
        Some(("fn", _)) => "function",
        Some(("macro", _)) => "macro",
        Some(("part", _)) => "Part kind",
        Some(("capability", _)) => "capability",
        Some(("type", rest)) if rest.contains('.') => "field or variant",
        Some(("type", _)) => "type",
        _ => "item",
    }
}

fn plural(word: &str, count: usize) -> String {
    match (word, count) {
        (_, 1) => word.to_owned(),
        ("field or variant", _) => "fields or variants".to_owned(),
        ("associated item", _) => "associated items".to_owned(),
        ("hook argument", _) => "hook arguments".to_owned(),
        ("Part kind", _) => "Part kinds".to_owned(),
        _ => format!("{word}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_folder(root: &Path) -> Result<Result<Vec<(String, String)>, Scanned>, String> {
        read(root, &crate::read_manifest(root)?)
    }

    #[test]
    fn a_mod_including_build_script_output_is_unread_not_an_error() {
        let dir = std::env::temp_dir().join(format!("sunrise-mod-unread-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("sunrise-mod.toml"),
            "id = \"bindings\"\nname = \"Bindings\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src/lib.rs"),
            "include!(concat!(env!(\"OUT_DIR\"), \"/b.rs\"));\n",
        )
        .unwrap();
        let read = read_folder(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(read, Ok(Err(Scanned::Unread(_)))));
    }

    #[test]
    fn a_helper_compiled_inside_the_game_is_not_scanned_and_a_module_row_is_exact() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = format!(
            "id = \"blur\"\nname = \"Blur\"\nversion = \"0.1.0\"\napi = \"0.2\"\n[native]\ngame = \"26.3\"\nsunrise = \"{}\"\n[[native.mixin]]\nid = \"m\"\ntarget = \"game::f\"\nkind = \"HEAD\"\nhelper = \"src/mixins/m.rs\"\n",
            "a".repeat(40)
        );
        std::fs::create_dir_all(dir.path().join("src/mixins")).unwrap();
        std::fs::write(dir.path().join("sunrise-mod.toml"), &manifest).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn a() {}").unwrap();
        std::fs::write(
            dir.path().join("src/mixins/m.rs"),
            "{ World::game_only(); }",
        )
        .unwrap();
        let sources = read_folder(dir.path()).unwrap().ok().unwrap();
        let files: Vec<&str> = sources.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(files, ["src/lib.rs"]);
        // A helper's own includes are the game's to resolve, and never fail the mod's scan.
        std::fs::write(
            dir.path().join("src/mixins/m.rs"),
            "include!(concat!(env!(\"OUT_DIR\"), \"/g.rs\"));",
        )
        .unwrap();
        assert!(read_folder(dir.path()).unwrap().is_ok());
        // A helper the mod's own source also brings in is both: the scan says it cannot judge.
        std::fs::write(dir.path().join("src/lib.rs"), "include!(\"mixins/m.rs\");").unwrap();
        assert!(matches!(
            read_folder(dir.path()),
            Ok(Err(Scanned::Unread(why))) if why.contains("src/mixins/m.rs")
        ));
        // And so is one a plain `mod` declaration may load.
        std::fs::write(dir.path().join("src/lib.rs"), "mod mixins;").unwrap();
        std::fs::write(dir.path().join("src/mixins.rs"), "mod m;").unwrap();
        assert!(matches!(
            read_folder(dir.path()),
            Ok(Err(Scanned::Unread(why))) if why.contains("src/mixins/m.rs")
        ));
        std::fs::remove_file(dir.path().join("src/mixins.rs")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn a() {}").unwrap();
        std::fs::write(
            dir.path().join("sunrise-mod.toml"),
            manifest.clone()
                + "[[native.module]]\nid = \"x\"\nfile = \"game.rs\"\nname = \"x\"\nsource = \"src/x.rs\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/x.rs"), "pub fn x() {}").unwrap();
        assert!(matches!(
            read_folder(dir.path()),
            Ok(Err(Scanned::Exact(why))) if why.contains("[[native.module]]")
        ));
    }

    #[test]
    fn the_summary_counts_by_kind_in_words() {
        let used = |id: &str| mod_surface::uses::Used {
            entry: mod_surface::uses::Entry {
                id: id.into(),
                hash: "h".into(),
                game_code: None,
                impls: vec![],
                built: None,
                fields_hash: None,
                fields: vec![],
                built_at: None,
                paths: vec![],
                reached_from: vec![],
                version: None,
            },
            origin: Origin::Source {
                file: "src/lib.rs".into(),
                line: 3,
            },
            note: None,
        };
        let uses = Uses {
            items: vec![
                used("hook:player_logged_in"),
                used("hook:block_broken"),
                used("method:ServerHost::give_item"),
                used("type:ItemStack"),
            ],
        };
        let sdk = PinnedSdk {
            build: "a".repeat(40),
            name: "nightly-2026-11-13 (afe471c)".into(),
            dir: "sdk".into(),
            surface: "sdk/surface.json".into(),
        };
        assert_eq!(
            summary("Greeter", &uses, &sdk),
            "Greeter uses 4 items of the API (2 hooks, 1 method, 1 type). Made for nightly-2026-11-13 (afe471c).\n"
        );
        assert_eq!(
            list(&uses).lines().next(),
            Some("hook:player_logged_in  src/lib.rs:3")
        );
    }
}
