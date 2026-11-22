//! Layout metadata for external types named by native game internals.
use crate::internals::Internal;
use quote::ToTokens;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How a [`CROSSING_FOREIGN`] type's definition is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Foreign {
    /// A struct or enum, as written.
    Layout,
    /// A `bitflags!` struct, which has no field to read: it crosses as its bits, a one-field
    /// struct of the flags' integer (`bits()` / `from_bits_retain`).
    Bits,
}

/// The types of crates outside the workspace that cross by copy (#1409): the key and mouse
/// state the input internals take. Their definitions are read from the source Cargo locked for
/// the game, and listed in `internals.json` like the workspace's.
pub const CROSSING_FOREIGN: &[(&str, Foreign)] = &[
    ("winit::keyboard::KeyCode", Foreign::Layout),
    ("winit::keyboard::PhysicalKey", Foreign::Layout),
    ("winit::keyboard::NativeKeyCode", Foreign::Layout),
    ("winit::keyboard::KeyLocation", Foreign::Layout),
    ("winit::keyboard::ModifiersState", Foreign::Bits),
    ("winit::event::ElementState", Foreign::Layout),
    ("winit::event::MouseButton", Foreign::Layout),
    ("winit::event_loop::ControlFlow", Foreign::Layout),
];

/// How the [`CROSSING_FOREIGN`] type at `path` is read, if it is one.
pub fn foreign(path: &str) -> Option<Foreign> {
    CROSSING_FOREIGN
        .iter()
        .find(|(listed, _)| *listed == path)
        .map(|(_, foreign)| *foreign)
}

/// The checked-in [`CROSSING_FOREIGN`] layouts, written by `cargo run -p xtask -- mod-api` from
/// the locked source. Staging also runs where Cargo may not be yet, so it reads this file.
/// Path relative to the source tree being staged, not the tool's checkout.
pub const CROSSING_FOREIGN_FILE: &str = "crates/sunrise-mod-api-gen/crossing-foreign.json";

/// One [`CROSSING_FOREIGN`] type's layout, as its crate's source declares it.
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct ForeignLayout {
    path: String,
    kind: String,
    signature: String,
}

/// The [`CROSSING_FOREIGN`] entries of `internals.json`, from the checked-in layouts.
pub fn foreign_internals(root: &Path) -> Result<Vec<Internal>, String> {
    let path = root.join(CROSSING_FOREIGN_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let layouts: Vec<ForeignLayout> =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(layouts
        .into_iter()
        .map(|layout| Internal {
            path: layout.path,
            kind: if layout.kind == "enum" {
                "enum"
            } else {
                "struct"
            },
            cfg: String::new(),
            hash: crate::internals::signature_hash(&layout.signature),
            signature: layout.signature,
            shape_only: true,
            opaque_reason: None,
            refs: BTreeMap::new(),
        })
        .collect())
}

/// Write the [`CROSSING_FOREIGN`] layouts of `root`'s checkout, read from the source of each
/// crate as the game crate resolves it in the lock file.
pub fn write_foreign_layouts(root: &Path) -> Result<(), String> {
    let path = root.join(CROSSING_FOREIGN_FILE);
    std::fs::write(&path, locked_foreign_layouts(root)?)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The text of [`CROSSING_FOREIGN_FILE`] that `root`'s locked source gives.
fn locked_foreign_layouts(root: &Path) -> Result<String, String> {
    let metadata = cargo_metadata(root)?;
    let layouts = read_foreign(CROSSING_FOREIGN, |krate| locked_lib(&metadata, krate))?;
    serde_json::to_string_pretty(&layouts)
        .map(|text| text + "\n")
        .map_err(|e| e.to_string())
}

fn cargo_metadata(root: &Path) -> Result<serde_json::Value, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = std::process::Command::new(cargo)
        .current_dir(root)
        .args([
            "metadata",
            "--format-version=1",
            "--locked",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("cargo metadata: {e}"))
}

/// The library root of the game crate's dependency `krate`, from `cargo metadata`.
fn locked_lib(metadata: &serde_json::Value, krate: &str) -> Result<PathBuf, String> {
    let packages = metadata["packages"].as_array().into_iter().flatten();
    let package =
        |pick: &dyn Fn(&serde_json::Value) -> bool| packages.clone().find(|package| pick(package));
    let game = package(&|p| p["name"] == "sunrise-client").ok_or("no sunrise-client package")?;
    let id = metadata["resolve"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|node| node["id"] == game["id"])
        .and_then(|node| node["deps"].as_array())
        .into_iter()
        .flatten()
        .find(|dep| dep["name"] == krate)
        .map(|dep| &dep["pkg"])
        .ok_or_else(|| format!("{krate}: not a dependency of sunrise-client"))?;
    package(&|p| p["id"] == *id)
        .and_then(|p| p["targets"].as_array())
        .into_iter()
        .flatten()
        .find(|target| {
            target["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "lib"))
        })
        .and_then(|target| target["src_path"].as_str())
        .map(PathBuf::from)
        .ok_or_else(|| format!("{krate}: no library in the locked package"))
}

/// The `listed` types' layouts, each read from its crate's library root at `lib(crate)`. A
/// listed path its source no longer has is an error naming it.
fn read_foreign(
    listed: &[(&str, Foreign)],
    lib: impl Fn(&str) -> Result<PathBuf, String>,
) -> Result<Vec<ForeignLayout>, String> {
    let mut libs = BTreeMap::new();
    let mut layouts = Vec::new();
    for (path, foreign) in listed {
        let mut segments: Vec<&str> = path.split("::").collect();
        let name = segments.pop().expect("a listed path");
        let krate = segments.remove(0);
        if !libs.contains_key(krate) {
            libs.insert(krate, lib(krate)?);
        }
        let stale = |why: &str| format!("{path}: {why} in the locked {krate} source");
        let items = module_items(&libs[krate], &segments).map_err(|e| stale(&e))?;
        let item = match foreign {
            Foreign::Layout => items.into_iter().find(|item| match item {
                syn::Item::Struct(s) => s.ident == name,
                syn::Item::Enum(e) => e.ident == name,
                _ => false,
            }),
            Foreign::Bits => items.iter().find_map(|item| match item {
                syn::Item::Macro(m) if m.mac.path.is_ident("bitflags") => {
                    bits_struct(&m.mac.tokens, name)
                }
                _ => None,
            }),
        }
        .ok_or_else(|| stale("not found"))?;
        let kind = if matches!(item, syn::Item::Enum(_)) {
            "enum"
        } else {
            "struct"
        };
        layouts.push(ForeignLayout {
            path: (*path).to_owned(),
            kind: kind.to_owned(),
            signature: crate::internals::shape_signature(&item)
                .expect("a struct or an enum")
                .to_token_stream()
                .to_string(),
        });
    }
    Ok(layouts)
}

/// The items of module `modules` (from the crate root) of the crate whose root is `lib`.
fn module_items(lib: &Path, modules: &[&str]) -> Result<Vec<syn::Item>, String> {
    let parse = |file: &Path| {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        syn::parse_file(&text)
            .map(|file| file.items)
            .map_err(|e| format!("{}: {e}", file.display()))
    };
    let mut items = parse(lib)?;
    let mut dir = lib.parent().expect("a source file has a folder").to_owned();
    for module in modules {
        let child = items
            .iter()
            .find_map(|item| match item {
                syn::Item::Mod(m) if m.ident == module => Some(m.content.clone()),
                _ => None,
            })
            .ok_or_else(|| format!("no module {module}"))?;
        items = match child {
            Some((_, inner)) => inner,
            None if dir.join(format!("{module}.rs")).is_file() => {
                parse(&dir.join(format!("{module}.rs")))?
            }
            None => parse(&dir.join(module).join("mod.rs"))?,
        };
        dir = dir.join(module);
    }
    Ok(items)
}

/// `struct name(Bits);` for the flags `name` in a `bitflags!` body, which declares them as
/// `struct Name: Bits { ... }`.
fn bits_struct(tokens: &proc_macro2::TokenStream, name: &str) -> Option<syn::Item> {
    use proc_macro2::TokenTree;
    let tokens: Vec<TokenTree> = tokens.clone().into_iter().collect();
    tokens.windows(4).find_map(|window| match window {
        [
            TokenTree::Ident(keyword),
            TokenTree::Ident(ident),
            TokenTree::Punct(colon),
            TokenTree::Ident(bits),
        ] if keyword == "struct" && ident == name && colon.as_char() == ':' => {
            syn::parse_str(&format!("struct {ident}({bits});")).ok()
        }
        _ => None,
    })
}
