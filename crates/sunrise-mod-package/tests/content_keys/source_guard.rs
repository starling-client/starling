//! Audit Serde constructs whose input schema is not necessarily exposed to
//! the probe. The key includes the source file, item and attribute arguments,
//! so a second use of a known workaround still needs its own evidence.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[path = "manifest_reviewed.rs"]
mod manifest_reviewed;

// Workspace types used in declaration inputs, by the paths the reader names them with
// (re-exports included). The guard follows each path to the file that defines its type.
// Registry crates such as semver, serde_json and toml are out of scope.
const EXTERNAL_DECLARATIONS: &[&str] = &[
    "sunrise_core::arm_pose::ArmPose",
    "mod_vocab::Side",
    "mod_vocab::DependencyOrder",
    "mod_vocab::DependencyOrder::After",
    // The field's custom deserializer maps names to this vocabulary instead of deriving Serde.
    "mod_vocab::NoteInstrument",
    "mod_vocab::NoteInstrument::ALL",
];

// Where each such crate keeps its sources.
const EXTERNAL_CRATES: &[(&str, &str)] = &[
    ("sunrise_core", "../sunrise-core/src"),
    ("mod_vocab", "../sunrise-mod-vocab/src"),
];

#[rustfmt::skip]
const NOT_DECLARATIONS: &[(&str, &str)] = &[
    ("mod_vocab::__private::DeclarationKind", "labels validation errors"),
    ("mod_vocab::DeclarationError", "reports validation errors"),
    ("mod_vocab::wire", "typed payload codec, not Serde"),
    ("mod_surface::uses::ManifestFacts", "typed scan input derived from a validated manifest"),
    ("mod_surface::uses::USES_FILE", "name of native fit metadata"),
    ("mod_surface::uses::MAX_USES_BYTES", "size limit for native fit metadata"),
    ("sunrise_mod_api_macros::Wire", "derive for the typed payload codec, not Serde"),
    ("sunrise_assets::blockstate::BlockStateFile", "asset reference check, not a declaration"),
    ("sunrise_assets::blockstate::VariantSet", "asset reference check, not a declaration"),
    ("sunrise_assets::item_model::ClientItem", "asset reference check, not a declaration"),
    ("sunrise_assets::model::CuboidModel", "asset reference check, not a declaration"),
    ("sunrise_assets::model::Loader", "asset reference check, not a declaration"),
    ("sunrise_assets::model::TextureSlot", "asset reference check, not a declaration"),
];

type Reviewed = (
    (&'static str, &'static str, &'static str),
    &'static [&'static str],
);

#[rustfmt::skip]
const REVIEWED: &[Reviewed] = &[
    (("content.rs", "push_reaction", "deserialize_with = \"deserialize_push_reaction\""), &["block_full", "refused_block_bad_push_reaction"]),
    (("content.rs", "offset", "deserialize_with = \"deserialize_offset\""), &["block_full", "refused_block_bad_offset"]),
    (("content.rs", "max_horizontal_offset", "deserialize_with = \"deserialize_horizontal_offset\""), &["block_full", "refused_block_horizontal_offset_range"]),
    (("content.rs", "max_vertical_offset", "deserialize_with = \"deserialize_vertical_offset\""), &["block_full", "refused_block_vertical_offset_range"]),
    (("content.rs", "instrument", "deserialize_with = \"deserialize_instrument\""), &["block_full", "refused_block_bad_instrument"]),
    (("content.rs", "ignited_by_lava", "deserialize_with = \"deserialize_ignited_by_lava\""), &["block_full", "refused_block_bad_property_type"]),
    (("content.rs", "replaceable", "deserialize_with = \"deserialize_replaceable\""), &["block_full"]),
    (("content.rs", "skip_same_block_faces", "deserialize_with = \"deserialize_skip_same_block_faces\""), &["block_full"]),
    (("content.rs", "RandomTicks", "untagged"), &["block_full"]),
    (("content.rs", "BlockShapes", "impl Deserialize"), &["block_full", "keys_block_forms", "refused_block_shape_bounds"]),
    (("content.rs", "StateProperty", "tag = \"type\""), &["block_full"]),
    (("content.rs", "open_sound", "deserialize_with = \"GateSound::deserialize_open\""), &["block_full", "keys_block_forms"]),
    (("content.rs", "close_sound", "deserialize_with = \"GateSound::deserialize_close\""), &["block_full", "keys_block_forms"]),
    (("content.rs", "duration", "deserialize_with = \"deserialize_stew_duration\""), &["block_full"]),
    (("content.rs", "GateSound", "untagged"), &["block_full", "keys_block_forms", "refused_block_gate_sound_site"]),
    (("content/entity_sounds.rs", "SoundOverride", "untagged"), &["entity_type_full", "keys_entity_sounds"]),
    (("content/entity_type.rs", "allowed_in_peaceful", "deserialize_with = \"deserialize_peaceful_setting\""), &["entity_type_full", "refused_entity_type_thrown_peaceful", "refused_entity_type_null_allowed_in_peaceful"]),
    (("content/entity_type.rs", "shadow_radius", "deserialize_with = \"deserialize_shadow\""), &["entity_type_full", "refused_entity_type_negative_shadow", "refused_entity_type_null_shadow_radius"]),
    (("content/entity_type.rs", "shadow_strength", "deserialize_with = \"deserialize_shadow\""), &["entity_type_full", "refused_entity_type_null_shadow_strength"]),
    (("content/entity_type.rs", "passenger_attachments", "deserialize_with = \"deserialize_passenger_attachments\""), &["entity_type_full", "refused_entity_type_bad_attachments", "refused_entity_type_attachment_length", "refused_entity_type_attachment_type"]),
];

#[test]
pub(crate) fn serde_blind_spots_are_reviewed() {
    crate::at_crate_root();
    let mut reviewed: BTreeMap<_, _> = REVIEWED
        .iter()
        .chain(manifest_reviewed::REVIEWED)
        .map(|&((file, item, feature), fixtures)| {
            ([file, item, feature].map(str::to_owned), fixtures)
        })
        .collect();
    assert_eq!(
        reviewed.len(),
        REVIEWED.len() + manifest_reviewed::REVIEWED.len(),
        "duplicate Serde exception"
    );
    let mut files = Vec::new();
    for stem in ["manifest", "content"] {
        files.push(format!("{stem}.rs"));
        sources(Path::new("src"), Path::new(stem), &mut files);
    }
    files.sort();
    let mut paths: Vec<_> = files
        .iter()
        .map(|file| (file.clone(), Path::new("src").join(file)))
        .collect();
    let external: BTreeSet<_> = EXTERNAL_DECLARATIONS
        .iter()
        .map(|path| {
            let mut segments = path.split("::");
            let krate = segments.next().unwrap();
            let src = EXTERNAL_CRATES
                .iter()
                .find(|&&(name, _)| name == krate)
                .unwrap_or_else(|| panic!("{path}: no source folder listed for {krate}"))
                .1;
            defining_file(Path::new(src), &segments.collect::<Vec<_>>())
                .unwrap_or_else(|e| panic!("{path}: {e}, so its Serde form can't be audited"))
        })
        .collect();
    paths.extend(
        external
            .into_iter()
            .map(|file| (file.to_str().unwrap().replace('\\', "/"), file)),
    );
    let known_paths: BTreeSet<_> = EXTERNAL_DECLARATIONS
        .iter()
        .copied()
        .chain(NOT_DECLARATIONS.iter().map(|&(path, _reason)| path))
        .collect();
    let corpus = crate::corpus::entries();
    let mut unclassified = Vec::new();
    let mut unreviewed = Vec::new();
    for (file, path) in paths {
        let source = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
        let source_tokens = tokens(&source);
        for path in workspace_paths(&source_tokens) {
            if !known_paths.contains(path.as_str()) {
                unclassified.push(format!("{file}: {path}"));
            }
        }
        for (item, feature) in blind_spots(&source_tokens) {
            if let Some(fixtures) =
                reviewed.remove(&[file.clone(), item.to_owned(), feature.clone()])
            {
                assert!(
                    !fixtures.is_empty(),
                    "{file}: {item}: {feature}: no fixtures"
                );
                for fixture in fixtures {
                    if let Some((source, test)) = fixture.split_once(':') {
                        let text = fs::read_to_string(source).unwrap();
                        assert!(
                            tokens(&text).windows(2).any(|pair| pair == ["fn", test]),
                            "missing reader test {fixture}"
                        );
                    } else {
                        let path = Path::new(
                            corpus
                                .get(*fixture)
                                .unwrap_or_else(|| panic!("missing corpus fixture {fixture}")),
                        );
                        assert!(
                            !path.is_dir() || path.join("sunrise-mod.toml").is_file(),
                            "missing manifest for corpus fixture {fixture}"
                        );
                    }
                }
            } else {
                unreviewed.push(format!("{file}: {item}: {feature}"));
            }
        }
    }
    assert!(
        unclassified.is_empty(),
        "Unclassified workspace paths (add the declaration source or explain the exception):\n{}",
        unclassified.join("\n")
    );
    assert!(
        unreviewed.is_empty(),
        "Serde uses without a coverage review:\n{}",
        unreviewed.join("\n")
    );
    assert!(
        reviewed.is_empty(),
        "stale Serde exceptions: {:?}",
        reviewed.keys()
    );
}

fn sources(root: &Path, dir: &Path, files: &mut Vec<String>) {
    for entry in fs::read_dir(root.join(dir)).unwrap() {
        let entry = entry.unwrap();
        let path = dir.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            sources(root, &path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path.to_str().unwrap().replace('\\', "/"));
        }
    }
}

/// The file that defines the type a declaration path names, by following the path: modules from
/// the crate root, then the type, with any variant or associated item after it left over. A name
/// the root does not define is followed one hop through the root's re-exports, which for
/// `mod_vocab` include the export fragments its build assembles.
///
/// This is not a resolver. Whatever it cannot decide from the parsed items is an error, never a
/// guess: an alias, an inline or `#[path]` module, any item on the way that carries a `cfg`, a
/// `::`-rooted or `super` re-export, and a glob whose target may get the name from elsewhere.
fn defining_file(src: &Path, path: &[&str]) -> Result<PathBuf, String> {
    let root = root_items(src)?;
    let (items, file, walked) = walk(src, &root, path)?;
    let Some(&name) = path.get(walked) else {
        return Err(format!("{} is a module", file.display()));
    };
    if walked > 0 {
        return defined(&items, &file, name, false)?
            .ok_or_else(|| format!("{} does not define {name}", file.display()));
    }
    if let Some(file) = defined(&root, &file, name, true)? {
        return Ok(file);
    }
    let mut exports = Vec::new();
    for item in &root {
        if let syn::Item::Use(item) = item {
            if item.leading_colon.is_some() {
                return Err("the crate root has a `::`-rooted use, which is not traced".into());
            }
            let conditional = gated(&item.attrs);
            use_tree(&item.tree, &mut Vec::new(), &mut |bound, target| {
                exports.push((bound, target, conditional));
            });
        }
    }
    let named: Vec<_> = exports
        .iter()
        .filter(|(bound, ..)| bound.as_deref() == Some(name))
        .collect();
    let mut found = BTreeSet::new();
    for (bound, target, conditional) in if named.is_empty() {
        exports
            .iter()
            .filter(|(bound, ..)| bound.is_none())
            .collect()
    } else {
        named
    } {
        if *conditional {
            return Err(format!(
                "a root re-export that may bind {name} carries a cfg, which is not evaluated"
            ));
        }
        let target: Vec<_> = target
            .iter()
            .map(String::as_str)
            .skip_while(|segment| matches!(*segment, "crate" | "self"))
            .collect();
        let (modules, item) = match bound {
            Some(_) => target.split_at(target.len().saturating_sub(1)),
            None => (target.as_slice(), std::slice::from_ref(&name)),
        };
        // Build output included into an inline module cannot be read here. Passing its glob
        // over is safe: if it also exported the name, the path would be ambiguous to rustc.
        if bound.is_none()
            && root.iter().any(|item| {
                matches!(item, syn::Item::Mod(module) if module.content.is_some()
                    && !gated(&module.attrs) && modules.first().is_some_and(|name| module.ident == name))
            })
        {
            continue;
        }
        let (items, file, walked) = walk(src, &root, modules)?;
        if walked != modules.len() || walked == 0 || item.is_empty() {
            return Err(format!(
                "the root re-export from {} is not traced",
                target.join("::")
            ));
        }
        match (defined(&items, &file, item[0], false)?, bound) {
            (Some(file), _) => found.insert(file),
            (None, None) => false,
            (None, Some(_)) => {
                return Err(format!("{} does not define {}", file.display(), item[0]));
            }
        };
    }
    match (found.pop_first(), found.is_empty()) {
        (Some(file), true) => Ok(file),
        (Some(_), false) => Err(format!("the crate root exports more than one {name}")),
        (None, _) => Err(format!(
            "the crate root neither defines nor traceably re-exports {name}"
        )),
    }
}

fn parse(file: &Path) -> Result<Vec<syn::Item>, String> {
    let text = fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(syn::parse_file(&text)
        .map_err(|e| format!("{}: {e}", file.display()))?
        .items)
}

/// The crate root's own items: `lib.rs`, and the export fragments when it includes the root file
/// its build assembles from them. The fragments are the ones the generator itself enumerates.
fn root_items(src: &Path) -> Result<Vec<syn::Item>, String> {
    let mut items = parse(&src.join("lib.rs"))?;
    let assembled = format!("{}root.rs", sunrise_game_discovery::exports::PREFIX);
    if items.iter().any(|item| {
        matches!(item, syn::Item::Macro(item) if item.mac.path.is_ident("include")
            && item.mac.tokens.to_string().contains(&assembled))
    }) {
        let exports = src.join("exports");
        for area in sunrise_game_discovery::module_files(&exports)? {
            items.extend(parse(&exports.join(format!("{area}.rs")))?);
        }
    }
    Ok(items)
}

fn gated(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
}

/// Enter the modules a path starts with, as far as the scopes declare them: the innermost
/// module's items and file, and how many segments were modules.
fn walk(
    src: &Path,
    root: &[syn::Item],
    path: &[&str],
) -> Result<(Vec<syn::Item>, PathBuf, usize), String> {
    let (mut items, mut dir, mut file) = (root.to_vec(), src.to_path_buf(), src.join("lib.rs"));
    for (walked, name) in path.iter().enumerate() {
        let mut declared = items.iter().filter_map(|item| match item {
            syn::Item::Mod(module) if module.ident == name => Some(module),
            _ => None,
        });
        let Some(module) = declared.next() else {
            return Ok((items, file, walked));
        };
        if declared.next().is_some()
            || module.content.is_some()
            || gated(&module.attrs)
            || module.attrs.iter().any(|attr| attr.path().is_ident("path"))
        {
            return Err(format!(
                "{} declares `mod {name}` inline, redirected, or under a cfg, which is not traced",
                file.display()
            ));
        }
        let inner = [format!("{name}.rs"), format!("{name}/mod.rs")]
            .into_iter()
            .map(|file| dir.join(file))
            .find(|file| file.is_file())
            .ok_or_else(|| format!("no file for `mod {name}` of {}", file.display()))?;
        (items, dir, file) = (parse(&inner)?, dir.join(name), inner);
    }
    Ok((items, file, path.len()))
}

/// Whether these items, a file's own, define the type: `None` only when nothing in them can bind
/// the name for another crate. At the root (`uses_apart`) the caller resolves the `use` items.
fn defined(
    items: &[syn::Item],
    file: &Path,
    name: &str,
    uses_apart: bool,
) -> Result<Option<PathBuf>, String> {
    let public = |vis: &syn::Visibility| matches!(vis, syn::Visibility::Public(_));
    let mut definitions = 0;
    let mut unresolved = None;
    for item in items {
        let (ident, attrs, vis) = match item {
            syn::Item::Struct(item) => (&item.ident, &item.attrs, &item.vis),
            syn::Item::Enum(item) => (&item.ident, &item.attrs, &item.vis),
            syn::Item::Union(item) => (&item.ident, &item.attrs, &item.vis),
            syn::Item::Type(item) if item.ident == name => {
                unresolved = Some("makes it an alias; name its target");
                continue;
            }
            syn::Item::Use(item) if !uses_apart && public(&item.vis) => {
                use_tree(&item.tree, &mut Vec::new(), &mut |bound, _| {
                    if bound.as_deref() == Some(name) {
                        unresolved = Some("re-exports it; name its target");
                    } else if bound.is_none() {
                        unresolved.get_or_insert("may re-export it through a glob");
                    }
                });
                continue;
            }
            _ => continue,
        };
        if ident != name {
            continue;
        }
        if gated(attrs) {
            unresolved = Some("defines it under a cfg, which is not evaluated");
        } else if uses_apart || public(vis) {
            definitions += 1;
        }
    }
    match (definitions, unresolved) {
        (1, None | Some("may re-export it through a glob")) => Ok(Some(file.to_path_buf())),
        (0, None) => Ok(None),
        (_, Some(why)) => Err(format!("{}: {name}: {why}", file.display())),
        _ => Err(format!("{} defines {name} more than once", file.display())),
    }
}

/// Each name a `use` item binds (`None` for a glob) with the path it comes from.
fn use_tree(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    bind: &mut impl FnMut(Option<String>, Vec<String>),
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            use_tree(&path.tree, prefix, bind);
            prefix.pop();
        }
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                use_tree(tree, prefix, bind);
            }
        }
        syn::UseTree::Glob(_) => bind(None, prefix.clone()),
        syn::UseTree::Name(syn::UseName { ident })
        | syn::UseTree::Rename(syn::UseRename { ident, .. }) => {
            let mut target = prefix.clone();
            if ident != "self" {
                target.push(ident.to_string());
            }
            let bound = match tree {
                syn::UseTree::Rename(rename) => rename.rename.to_string(),
                _ => target.last().cloned().unwrap_or_default(),
            };
            bind(Some(bound), target);
        }
    }
}

#[test]
fn a_same_named_type_elsewhere_is_not_the_declaration() {
    let src = Path::new(env!("CARGO_TARGET_TMPDIR")).join("source-guard-decoy");
    let _ = fs::remove_dir_all(&src);
    fs::create_dir_all(src.join("exports")).unwrap();
    fs::create_dir_all(src.join("content")).unwrap();
    let put = |file: &str, text: &str| fs::write(src.join(file), text).unwrap();
    let lib = "mod content; mod decoy; mod forwarding; mod generated { include!(\"out.rs\"); } \
               include!(concat!(env!(\"OUT_DIR\"), \"/mod_api_exports_root.rs\"));";
    let side = src.join("content/side.rs");
    let reset = || {
        put("lib.rs", lib);
        put("exports/sides.rs", "pub use content::side::Side;");
        put("decoy.rs", "pub enum Side {}");
        put("forwarding.rs", "");
        put("content/mod.rs", "pub(crate) mod side;");
        put("content/side.rs", "pub enum Side {}");
        let _ = fs::remove_file(src.join("exports/mod.rs"));
        let _ = fs::remove_file(src.join("exports/.rs"));
    };
    let found = || defining_file(&src, &["Side", "Client"]);
    reset();
    assert_eq!(found(), Ok(side.clone()));
    assert_eq!(
        defining_file(&src, &["decoy", "Side"]),
        Ok(src.join("decoy.rs"))
    );

    // The referenced definition moved out of reach: the decoy must not stand in for it.
    for moved in [
        "pub use elsewhere::Side;",
        "pub type Side = crate::decoy::Side;",
        "mod tests { pub enum Side {} }",
        // Which of two conditional items is compiled is not decided here.
        "#[cfg(test)] pub enum Side {} #[cfg(not(test))] pub use crate::decoy::Side;",
        // A brace inside an attribute does not end the item's attributes.
        "#[cfg(test)] #[doc = stringify!({})] pub enum Side {}",
    ] {
        put("content/side.rs", moved);
        assert!(found().is_err(), "{moved}");
    }
    reset();
    // A stale file beside a redirected or conditional module is not that module.
    for module in [
        "#[path = \"moved/side.rs\"] pub(crate) mod side;",
        "#[cfg(test)] pub(crate) mod side;",
    ] {
        put("content/mod.rs", module);
        assert!(found().is_err(), "{module}");
    }
    reset();
    // Files the build leaves out export nothing: the compiled glob decides.
    put(
        "exports/sides.rs",
        "pub use generated::*; pub use content::side::*;",
    );
    put("exports/mod.rs", "pub use decoy::Side;");
    put("exports/.rs", "pub use decoy::Side;");
    assert_eq!(found(), Ok(side.clone()));
    // So does a crate root that does not include the fragments.
    put("lib.rs", "mod content; mod decoy;");
    assert!(found().is_err());
    reset();
    // A named export this guard cannot place is refused, not passed over for a glob.
    for export in [
        "#[cfg(test)] pub use content::side::Side;",
        "#[cfg(test)] #[doc = stringify!({})] pub use decoy::Side; pub use content::side::*;",
        "pub use ::dependency::Side; pub use content::side::*;",
        "pub use dependency::Side; pub use content::side::*;",
        // A glob that may bind the name is refused when its target cannot be read for certain.
        "#[cfg(test)] pub use decoy::*; pub use content::side::*;",
        "pub use dependency::*; pub use content::side::*;",
        "pub use forwarding::*; pub use content::side::*;",
    ] {
        put("exports/sides.rs", export);
        put("forwarding.rs", "pub use crate::content::side::Side;");
        assert!(found().is_err(), "{export}");
    }
    // A private type of the same name is not what a glob exports.
    put(
        "exports/sides.rs",
        "pub use decoy::*; pub use content::side::*;",
    );
    put("decoy.rs", "enum Side {}");
    assert_eq!(found(), Ok(side));
}

fn workspace_paths(tokens: &[&str]) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for (i, &name) in tokens.iter().enumerate() {
        // These two libraries use short `[lib] name`s, not their Cargo package names.
        if (name.starts_with("sunrise_") || matches!(name, "mod_vocab" | "mod_surface"))
            && (tokens[i + 1..].starts_with(&[":", ":"])
                || tokens[i + 1..].starts_with(&["as"])
                || i > 0 && tokens[i - 1] == "use")
        {
            collect_path(&mut &tokens[i..], "", &mut paths);
        }
    }
    paths
}

// Split grouped imports down to individual paths, keeping the same precision as
// for a fully qualified field type. `as` does not change the source being used.
fn collect_path(tokens: &mut &[&str], prefix: &str, paths: &mut BTreeSet<String>) {
    if let ["{", ..] = *tokens {
        *tokens = &tokens[1..];
        while !tokens.starts_with(&["}"]) {
            collect_path(tokens, prefix, paths);
            if let ["as", _, tail @ ..] = *tokens {
                *tokens = tail;
            }
            if let [",", tail @ ..] = *tokens {
                *tokens = tail;
            } else {
                break;
            }
        }
        assert!(
            tokens.starts_with(&["}"]),
            "unclosed grouped workspace import"
        );
        *tokens = &tokens[1..];
        return;
    }
    let (name, rest) = tokens.split_first().expect("workspace path segment");
    *tokens = rest;
    let path = match (*name, prefix) {
        (_, "") => (*name).to_owned(),
        ("self", prefix) => prefix.to_owned(),
        (name, prefix) => format!("{prefix}::{name}"),
    };
    // A turbofish starts type arguments, not another segment.
    if let [":", ":", next, ..] = rest
        && *next != "<"
    {
        *tokens = &rest[2..];
        collect_path(tokens, &path, paths);
    } else {
        paths.insert(path);
    }
}

fn blind_spots<'a>(tokens: &[&'a str]) -> Vec<(&'a str, String)> {
    let mut out = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let tail = &tokens[i + 1..];
        if *token == "#" && tail.first() == Some(&"[") {
            let attr_end = group_end(tail, "[", "]");
            let attr = &tail[1..attr_end];
            // Includes `cfg_attr(..., serde(...))`. Only top-level Serde
            // meta keys count, not identifiers in another key's arguments.
            for (n, pair) in attr.windows(2).enumerate() {
                if pair != ["serde", "("] {
                    continue;
                }
                let args = &attr[n + 1..];
                let mut args = &args[1..group_end(args, "(", ")")];
                while !args.is_empty() {
                    let mut end = 0;
                    while end < args.len() && args[end] != "," {
                        end += match args[end] {
                            "(" => group_end(&args[end..], "(", ")") + 1,
                            _ => 1,
                        };
                    }
                    if matches!(
                        args[0],
                        "flatten"
                            | "alias"
                            | "tag"
                            | "untagged"
                            | "deserialize_with"
                            | "with"
                            | "from"
                            | "try_from"
                            | "other"
                    ) {
                        out.push((
                            attribute_target(&tail[attr_end + 1..]),
                            args[..end].join(" "),
                        ));
                    }
                    args = &args[(end + 1).min(args.len())..];
                }
            }
        } else if *token == "impl" {
            let mut header = tail;
            if header.first() == Some(&"<") {
                header = &header[group_end(header, "<", ">") + 1..];
            }
            let trait_path = header
                .iter()
                .take_while(|&&t| t != "<" && t != "for" && t != "{" && t != "where");
            if trait_path.copied().last() == Some("Deserialize") {
                let header = header.split(|t| *t == "{").next().unwrap();
                if let Some(index) = header.iter().position(|t| *t == "for") {
                    let target = header[index + 1..]
                        .iter()
                        .copied()
                        .take_while(|&t| t != "<" && t != "where")
                        .last()
                        .unwrap();
                    out.push((target, "impl Deserialize".into()));
                }
            }
        }
    }
    out
}

fn attribute_target<'a>(mut tail: &[&'a str]) -> &'a str {
    while tail.starts_with(&["#", "["]) {
        tail = &tail[group_end(&tail[1..], "[", "]") + 2..];
    }
    if tail.first() == Some(&"pub") {
        tail = &tail[1..];
        if tail.first() == Some(&"(") {
            tail = &tail[group_end(tail, "(", ")") + 1..];
        }
    }
    if matches!(tail.first(), Some(&("enum" | "struct"))) {
        tail = &tail[1..];
    }
    tail.first().copied().unwrap_or("<unknown item>")
}

fn group_end(tokens: &[&str], open: &str, close: &str) -> usize {
    let mut depth = 0;
    for (i, token) in tokens.iter().enumerate() {
        if *token == open {
            depth += 1;
        } else if *token == close {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
    }
    panic!("unclosed {open} in reader source");
}

// Tokenize only what the audit needs: words, punctuation, and opaque literals.
// Comments and string contents must not be mistaken for code, including fixture
// strings in the reader's own tests. Whitespace is immaterial.
fn tokens(mut text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    while !text.is_empty() {
        text = text.trim_start();
        let Some(first) = text.chars().next() else {
            break;
        };
        if text.starts_with("//") {
            text = text.split_once('\n').map_or("", |(_, rest)| rest);
            continue;
        }
        if let Some(mut comment) = text.strip_prefix("/*") {
            let mut depth = 1;
            while depth > 0 {
                let next = comment.find(['/', '*']).expect("unclosed block comment");
                comment = &comment[next..];
                if let Some(rest) = comment.strip_prefix("/*") {
                    depth += 1;
                    comment = rest;
                } else if let Some(rest) = comment.strip_prefix("*/") {
                    depth -= 1;
                    comment = rest;
                } else {
                    comment = &comment[1..];
                }
            }
            text = comment;
            continue;
        }
        let raw = ["r", "br", "cr"].into_iter().find_map(|prefix| {
            let suffix = text.strip_prefix(prefix)?;
            let hashes = suffix.bytes().take_while(|b| *b == b'#').count();
            let body = suffix[hashes..].strip_prefix('"')?;
            let delimiter = format!("\"{}", "#".repeat(hashes));
            Some(
                text.len() - body.len()
                    + body.find(&delimiter).expect("unclosed raw string")
                    + delimiter.len(),
            )
        });
        let literal = |quote| {
            let mut escaped = false;
            text[1..].char_indices().find_map(|(i, c)| {
                let end = !escaped && c == quote;
                escaped = !escaped && c == '\\';
                end.then_some(i + 1 + c.len_utf8())
            })
        };
        let len = if let Some(len) = raw {
            len
        } else if first == '"'
            || (first == '\''
                && (text[1..].starts_with('\\') || text[1..].chars().nth(1) == Some('\'')))
        {
            literal(first).expect("unclosed literal")
        } else if first.is_alphanumeric() || first == '_' {
            text.find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(text.len())
        } else {
            first.len_utf8()
        };
        tokens.push(&text[..len]);
        text = &text[len..];
    }
    tokens
}
