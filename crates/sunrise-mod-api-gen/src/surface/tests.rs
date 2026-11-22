//! The export over a small fixture API, and over the real tree.
use super::*;
use std::collections::BTreeMap;

/// The fixture: path below the root, and text.
const FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        "[workspace]\nmembers = [\"engine-fixture\", \"crates/sunrise-mod-vocab\", \"crates/sunrise-mod-api\", \"crates/sunrise-mod-api-macros\", \"crates/sunrise-mod-runtime\"]\n",
    ),
    (
        "engine-fixture/Cargo.toml",
        "[package]\nname = \"engine-package\"\n[lib]\nname = \"engine_library\"\n",
    ),
    (
        "engine-fixture/src/lib.rs",
        "pub struct Secret;\npub trait Trait { type Out; }\nimpl Trait for u32 { type Out = Secret; }\nstruct Option;\n#[cfg(feature = \"extra\")] pub struct Result<T, E>(T, E);\n#[cfg(windows)] pub mod conditional { pub struct WindowsActive; }\n#[cfg(unix)] pub mod conditional { pub struct UnixActive; }\n#[cfg(test)] pub struct TestOnly;\npub mod nested { pub struct Hidden; }\npub mod option { pub struct Option<T>(T); }\n",
    ),
    (
        "crates/sunrise-mod-vocab/Cargo.toml",
        "[package]\nname = \"sunrise-mod-vocab\"\n[lib]\nname = \"mod_vocab\"\n[dependencies]\nengine = { package = \"engine-package\", path = \"../../engine-fixture\" }\nsunrise-mod-runtime = { path = \"../sunrise-mod-runtime\" }\n",
    ),
    (
        "crates/sunrise-mod-api/Cargo.toml",
        "[package]\nname = \"sunrise-mod-api\"\n[lib]\nname = \"mod_api\"\n[dependencies]\nsunrise-mod-vocab = { path = \"../sunrise-mod-vocab\" }\n",
    ),
    (
        "crates/sunrise-mod-api-macros/Cargo.toml",
        "[package]\nname = \"sunrise-mod-api-macros\"\n[lib]\nproc-macro = true\n",
    ),
    (
        "crates/sunrise-mod-runtime/Cargo.toml",
        "[package]\nname = \"sunrise-mod-runtime\"\n",
    ),
    (
        "crates/sunrise-mod-runtime/src/lib.rs",
        "pub mod storage { pub struct Query; }",
    ),
    (
        "crates/sunrise-mod-api/src/lib.rs",
        "extern crate self as mod_api;\npub use mod_vocab::*;\n",
    ),
    (
        "crates/sunrise-mod-vocab/src/lib.rs",
        r#"
pub mod entity;
pub use entity::EntityRef;
pub use sunrise_mod_api_macros::{Wire, sunrise_mod};
#[doc(hidden)]
pub use events::*;
#[doc(hidden)]
pub mod internal {
    pub struct Secret;
}
mod events {
    use super::*;
    /// The event of `ping`.
    pub struct PingEvent<'a> {
        pub who: EntityRef,
        pub lines: &'a mut Vec<(Text, u32)>,
        pub cancelled: bool,
    }
    /// The event of `tick_block`.
    pub struct TickBlockEvent {
        pub pos: Point,
    }
}
/// A text: parts of parts.
#[derive(Clone, Debug, Wire)]
pub struct Text {
    pub parts: Vec<Text>,
    style: u8,
}
/// A point.
#[derive(Clone, Copy, Wire)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}
/// Limits a mod reads but cannot build.
#[non_exhaustive]
pub struct Limits {
    pub max: u32,
}
/// A colour.
#[non_exhaustive]
pub enum Colour {
    Red,
    Rgb(u8, u8, u8),
}
/// A mode.
pub enum Mode {
    On,
    Off { reason: String },
}
/// What a mod answers.
pub trait Hooks {
    /// Pinged.
    fn ping(&self, host: &mut dyn ServerHost, event: &mut PingEvent<'_>) {
        let _ = (host, event);
    }
    /// Must be written.
    fn name(&self) -> &'static str;
}
/// The game.
pub trait Host {
    /// The time.
    fn time(&self) -> u64;
}
/// The server's game.
pub trait ServerHost: Host {
    /// Saves.
    ///
    /// It writes
    /// files.
    fn save(&mut self, slot: u8);
    /// Sends a text.
    fn send(&mut self, text: Text) -> bool;
}
/// A block's part.
pub trait BlockPart {}
/// Finds the ground.
pub fn ground(at: EntityRef) -> Option<Point> {
    let _ = at;
    None
}
/// The first part.
pub fn first<'a>(text: &'a Text) -> &'a Text {
    text
}
/// A capability key.
pub struct CapabilityKey<T: ?Sized> {
    pub name: &'static str,
    pub handler: std::marker::PhantomData<T>,
}
/// Fluids, through a handler.
pub trait FluidHandler {}
/// Fluids.
pub const FLUIDS: CapabilityKey<dyn FluidHandler> = CapabilityKey {
    name: "fixture:fluids",
    handler: std::marker::PhantomData,
};
/// Fluids, made by a constructor.
pub const MORE_FLUIDS: CapabilityKey<dyn FluidHandler> = CapabilityKey::new("fixture:more_fluids");
pub const LIMIT: u32 = 3;
/// Makes a key.
#[macro_export]
macro_rules! key {
    () => {};
}
/// A result of the fixture.
pub type Outcome<T> = Result<T, Colour>;
impl Default for Point {
    fn default() -> Self {
        Self { x: 0, y: 0 }
    }
}
impl Point {
    /// The origin.
    pub const ORIGIN: Point = Point { x: 0, y: 0 };
}
/// A key whose Wire impl a macro writes.
pub struct SlotKey(u32);
macro_rules! slot_keys {
    ($width:ty; $($key:ty),+ $(,)?) => {$(
        impl Wire for $key {
            layout_fields!("slot" => $width);
        }
    )+};
}
slot_keys!(u32; SlotKey);
/// An identifier, encoded by hand.
pub struct Ident {
    cache: u64,
    pub namespace: String,
}
impl Wire for Ident {
    layout_fields!("namespace" => String);
}
/// A name that crosses as a string.
pub struct Name(String);
impl Wire for Name {
    layout_via!(String);
}
/// Context that crosses by a hand-written impl alone.
pub struct Context;
impl Wire for Context {}
/// Where Parts are attached.
pub trait Registrar {}
impl dyn Registrar + '_ {
    /// Attaches a part to the block `key` names.
    pub fn attach(&mut self, key: SlotKey) {
        let _ = key;
    }
}
/// Data with an entry type.
pub trait DataMap {
    /// Its entry.
    type Entry;
}
macro_rules! moves {
    ($to:ident, $from:ident) => {
        impl Point {
            /// Moves to.
            pub fn $to(&self) {}
            /// Moves from.
            pub fn $from(&self) {}
        }
    };
}
moves!(move_to, move_from);
macro_rules! draws {
    ($($host:ident),+) => { $(
        impl dyn $host + '_ {
            /// Draws.
            pub fn draw(&self) {}
        }
    )+ };
}
draws!(Host, Registrar);
macro_rules! outer {
    ($name:ident) => {
        moves!($name, inner_from);
        missing_inner!();
    };
}
outer!(outer_to);
unknown!(Point);
#[cfg(feature = "extra")]
impl Point {
    /// Only with `extra`.
    pub fn extra(&self) {}
}
/// A module reached by more routes than the walk follows.
pub mod shared {
    /// A thing.
    pub struct Thing;
}
pub use shared as s1;
pub use shared as s2;
pub use shared as s3;
pub use shared as s4;
pub use shared as s5;
pub use shared as s6;
pub use shared as s7;
pub use shared as s8;
pub use shared as s9;
pub use shared as s10;
pub use shared as s11;
pub use shared as s12;
pub use shared as s13;
pub use shared as s14;
pub use shared as s15;
pub use shared as s16;
pub use shared as s17;
"#,
    ),
    (
        "crates/sunrise-mod-vocab/src/entity.rs",
        r#"
/// A live entity.
#[derive(Clone, Copy, Wire)]
pub struct EntityRef(u32);
impl EntityRef {
    /// Its id.
    pub fn id(self) -> u32 {
        self.0
    }
    /// Its file.
    ///
    /// It reads a file.
    pub fn path(self) -> crate::Outcome<String> {
        Ok(String::new())
    }
    fn private(self) {}
}
"#,
    ),
    (
        "crates/sunrise-mod-api-macros/src/lib.rs",
        r#"
/// The derive.
#[proc_macro_derive(Wire, attributes(wire))]
pub fn derive_wire(input: TokenStream) -> TokenStream { input }
/// Marks the mod.
#[proc_macro_attribute]
pub fn sunrise_mod(_: TokenStream, input: TokenStream) -> TokenStream { input }
"#,
    ),
    (
        "crates/sunrise-mod-vocab/extension-tags/items.json",
        r#""crates/sunrise-mod-package/src/fixture.extension-tags.json""#,
    ),
    (
        "crates/sunrise-mod-vocab/extension-tags/blocks.json",
        r#"[{"registry": "block", "id": "sunrise:fixture", "doc": "Fixture blocks."}]"#,
    ),
    (
        "crates/unlisted/src/ignored.extension-tags.json",
        "invalid JSON: the export must not read unrelated declarations",
    ),
    (
        "crates/sunrise-mod-package/src/fixture.extension-tags.json",
        r#"[{"registry": "item", "id": "sunrise:fixture", "doc": "Fixture items."}]"#,
    ),
    (
        "crates/sunrise-mod-package/src/manifest.rs",
        r#"
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// New registries.
    Registries,
}
"#,
    ),
];

const HOOKS: &str = r#"
pub trait Hooks {
    /// Something pinged.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancelled",
        target = "sunrise_client::game::Game::ping",
        neoforge = "PingEvent"
    )]
    fn ping();
    /// A block ticks.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "pos",
        target = "sunrise_client::game::GameLevel::tick_block",
        neoforge = "none"
    )]
    fn tick_block();
}
"#;

fn boundary() -> Boundary {
    let mut boundary = Boundary::default();
    boundary.roles.push(("Hooks".into(), Role::Implement));
    boundary.roles.push(("Host".into(), Role::Call));
    boundary.roles.push(("ServerHost".into(), Role::Call));
    boundary.sides.push(("ServerHost".into(), Side::Server));
    boundary
}

/// An edit that moves its file to the path given as `to`.
const MOVE: &str = "<move>";

/// The export of the fixture with `edits` (file, from, to) applied.
fn export_fixture(edits: &[(&str, &str, &str)], hooks: &str) -> Export {
    try_export_fixture(edits, hooks).unwrap()
}

fn export_fixture_with_includes(
    edits: &[(&str, &str, &str)],
    hooks: &str,
    included: &[(&str, &str)],
) -> Export {
    try_export_fixture_with_includes(edits, hooks, included).unwrap()
}

fn try_export_fixture(edits: &[(&str, &str, &str)], hooks: &str) -> Result<Export, String> {
    try_export_fixture_with_includes(edits, hooks, &[])
}

fn try_export_fixture_with_includes(
    edits: &[(&str, &str, &str)],
    hooks: &str,
    included: &[(&str, &str)],
) -> Result<Export, String> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for (path, text) in FILES {
        let mut text = text.to_string();
        let mut path = *path;
        for (file, from, to) in edits {
            if *file != path {
                continue;
            }
            if *from == MOVE {
                path = to;
            } else {
                assert!(text.contains(from), "{from} not in {path}");
                text = text.replacen(from, to, 1);
            }
        }
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let catalog = Catalog::parse(&[("hooks/ping.rs".into(), hooks.into())]).unwrap();
    let mut index = crate::index::game(root, included);
    build(root, &mut index, &catalog.hooks, &boundary())
}

const LIB: &str = "crates/sunrise-mod-vocab/src/lib.rs";

#[test]
fn a_signature_cannot_hide_a_workspace_type_behind_an_import() {
    let edited = "use engine::Secret as Imported;\npub fn leak() -> Imported { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture_with_includes(&[(LIB, "/// A colour.", edited)], HOOKS, &[])
        .err()
        .expect("the leak must fail the export");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_signature_cannot_hide_a_workspace_type_behind_a_private_glob_import() {
    let edited = "mod imported { use engine::*; pub fn leak() -> Secret { unimplemented!() } }\npub use imported::leak;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the glob-imported leak must fail the export");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_private_glob_through_a_local_module_alias_cannot_hide_a_workspace_type() {
    let edited = "mod inner { pub(super) type Alias = engine::Secret; }\nuse self::inner as h;\nuse h::*;\npub fn leak() -> Alias { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the glob-imported local type alias must be expanded");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_glob_through_a_local_module_alias_cannot_import_an_invisible_type() {
    let edited = "mod inner { type Option = engine::Secret; }\nuse self::inner as h;\nuse h::*;\npub fn safe() -> Option<u8> { unimplemented!() }\n/// A colour.";
    try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("the child module's private Option is not visible in the importing module");
}

#[test]
fn a_rooted_import_chain_ignores_a_local_module_with_the_dependency_name() {
    let edited = "mod engine { pub struct Secret; }\nuse ::engine::Secret as Imported;\nuse self::Imported as Alias;\npub fn leak() -> Alias { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the rooted use chain must resolve the workspace dependency, not the local type");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_local_module_alias_shadows_a_dependency_only_for_unrooted_paths() {
    let edited = "pub mod local { pub struct Secret; }\nuse self::local as engine;\npub fn safe() -> engine::Secret { unimplemented!() }\n/// A colour.";
    let export = export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS);
    let value: serde_json::Value = serde_json::from_str(&export.surface.to_json()).unwrap();
    let safe = value["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|function| function["id"] == "fn:safe")
        .expect("safe is exported");
    assert_eq!(safe["refs"], serde_json::json!(["type:local::Secret"]));

    let rooted = edited.replace("fn safe() -> engine::", "fn leak() -> ::engine::");
    let error = try_export_fixture(&[(LIB, "/// A colour.", &rooted)], HOOKS)
        .err()
        .expect("the rooted path still names the workspace dependency");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn an_indexed_type_import_can_shadow_a_dependency_of_the_same_name() {
    let edited = "use crate::Text as engine;
        pub fn safe() -> engine { unimplemented!() }
        /// A colour.";
    let export = export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS);
    let value: serde_json::Value = serde_json::from_str(&export.surface.to_json()).unwrap();
    let safe = value["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|function| function["id"] == "fn:safe")
        .expect("safe is exported");
    assert_eq!(safe["refs"], serde_json::json!(["type:Text"]));
}

#[test]
fn a_renamed_workspace_dependency_glob_cannot_hide_a_type() {
    let edited = "mod imported { use engine::*; pub fn leak() -> Secret { unimplemented!() } }\npub use imported::leak;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the aliased glob-imported leak must fail the export");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_workspace_module_binding_imported_by_glob_cannot_hide_a_type() {
    let edited = "mod imported { use engine::*; pub fn leak() -> nested::Hidden { unimplemented!() } }\nuse self::imported as h;\npub use h::*;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the module imported through a workspace glob must resolve to its type");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::nested::Hidden"), "{error}");
}

#[test]
fn primitive_and_prelude_types_after_a_workspace_glob_are_not_leaks() {
    let edited = "mod imported { use engine::*; pub fn safe() -> (u32, Option<String>) { unimplemented!() } }\npub use imported::safe;\n/// A colour.";
    try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("primitive and prelude types are not imported from the workspace glob");
}

#[test]
fn a_local_type_shadows_a_workspace_glob_import() {
    let edited = "use engine::*;\npub struct Secret;\npub fn safe() -> Secret { unimplemented!() }\n/// A colour.";
    try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("the local type shadows the name imported by the workspace glob");
}

#[test]
fn a_local_constant_does_not_shadow_a_workspace_type_imported_by_glob() {
    let edited = "use engine::*;\nconst Secret: u8 = 0;\npub fn leak() -> Secret { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("a value name does not shadow the workspace type in a signature");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_named_value_import_does_not_hide_a_workspace_type_imported_by_glob() {
    let edited = "const LOCAL_SECRET: u8 = 0;\nuse crate::LOCAL_SECRET as Secret;\nuse engine::*;\npub fn leak() -> Secret { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("a value import does not shadow a workspace type in a signature");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_named_static_import_does_not_hide_a_workspace_type_imported_by_glob() {
    let edited = "static LOCAL_SECRET: u8 = 0;\nuse crate::LOCAL_SECRET as Secret;\nuse engine::*;\npub fn leak() -> Secret { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("a static import does not shadow a workspace type in a signature");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_named_type_import_keeps_precedence_over_workspace_type_globs() {
    let edited = "const LOCAL_SECRET: u8 = 0;\nuse crate::LOCAL_SECRET as Secret;\nuse crate::Text as Secret;\nuse engine::*;\npub fn safe() -> Secret { unimplemented!() }\n/// A colour.";
    let export = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("an explicit API type import takes precedence over globbed types");
    let value: serde_json::Value = serde_json::from_str(&export.surface.to_json()).unwrap();
    let function = value["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|function| function["id"] == "fn:safe")
        .expect("the function is exported");
    assert!(function["signature"].as_str().unwrap().contains("Text"));
    assert!(
        function["refs"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("type:Text"))
    );
}

#[test]
fn an_opaque_named_import_is_assumed_to_bind_a_type() {
    // Without core's source the audit cannot tell that PI binds only a value, so it
    // cannot see the workspace type that Rust resolves here.
    let edited = "use core::f32::consts::PI as Secret;\nuse engine::*;\npub fn leak() -> Secret { unimplemented!() }\n/// A colour.";
    try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("an unstaged explicit binding has precedence in the source-only audit");
}

#[test]
fn an_explicit_external_type_shadows_the_same_name_from_a_workspace_glob() {
    let edited = "use core::option::Option as Secret;\nuse engine::*;\npub fn safe() -> Secret<u8> { unimplemented!() }\n/// A colour.";
    try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("the explicit type import takes precedence over the workspace glob");
}

#[test]
fn a_named_type_from_a_renamed_dependency_is_a_leak_without_any_glob() {
    let edited = "use engine::Secret as Imported;\npub fn leak() -> Imported { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the named import must use its Cargo alias to resolve the workspace type");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_named_value_from_a_renamed_dependency_does_not_shadow_a_globbed_type() {
    let edited = "use engine::value as nested;\nuse engine::*;\npub fn leak() -> nested::Hidden { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(
        &[
            (LIB, "/// A colour.", edited),
            (
                "engine-fixture/src/lib.rs",
                "pub struct Secret;",
                "pub struct Secret; pub fn value() {}",
            ),
        ],
        HOOKS,
    )
    .err()
    .expect("an indexed value import does not occupy the type namespace");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::nested::Hidden"), "{error}");
}

#[test]
fn two_globs_in_the_same_namespace_are_ambiguous_at_the_api_owner() {
    let direct = "mod local { pub struct Secret; }";
    let indirect = "mod a { pub struct Clash; }
                    mod b { pub struct Clash; }
                    mod bridge { pub use crate::a::*; pub use crate::b::*; }";
    for (modules, globs, name, definitions) in [
        (
            direct,
            "use engine::*; use local::*;",
            "Secret",
            ["engine_library::Secret", "mod_api::local::Secret"],
        ),
        (
            direct,
            "use local::*; use engine::*;",
            "Secret",
            ["engine_library::Secret", "mod_api::local::Secret"],
        ),
        (
            indirect,
            "use bridge::*;",
            "Clash",
            ["mod_api::a::Clash", "mod_api::b::Clash"],
        ),
    ] {
        let edited = format!(
            "{modules}\n{globs}\npub fn ambiguous() -> {name} {{ unimplemented!() }}\n/// A colour."
        );
        let error = try_export_fixture(&[(LIB, "/// A colour.", &edited)], HOOKS)
            .err()
            .expect("two different types imported by globs are ambiguous");
        assert!(error.contains("mod_api::ambiguous"), "{error}");
        assert!(error.contains(&format!("`{name}`")), "{error}");
        for definition in definitions {
            assert!(error.contains(definition), "{error}");
        }
    }

    let unused = format!("{indirect}\nuse bridge::*;\n/// A colour.");
    try_export_fixture(&[(LIB, "/// A colour.", &unused)], HOOKS)
        .expect("an unused conflict does not make a glob import ambiguous");
}

#[test]
fn a_deep_glob_chain_preserves_references_leaks_and_ambiguity() {
    use std::fmt::Write as _;

    let mut chain = String::new();
    for n in 1..=64 {
        writeln!(chain, "mod m{n} {{ pub use crate::m{}::*; }}", n - 1).unwrap();
    }
    let caller = "mod caller { use crate::m64::*; pub fn exposed() -> Shared { unimplemented!() } }
        pub use caller::exposed;
        /// A colour.";
    let edited = format!("pub struct Shared; mod m0 {{ pub use crate::Shared; }} {chain} {caller}");
    let export = export_fixture(&[(LIB, "/// A colour.", &edited)], HOOKS);
    let value: serde_json::Value = serde_json::from_str(&export.surface.to_json()).unwrap();
    let exposed = value["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|function| function["id"] == "fn:exposed")
        .expect("exposed is exported");
    assert_eq!(exposed["refs"], serde_json::json!(["type:Shared"]));

    let leaking = format!("mod m0 {{ pub type Shared = engine::Secret; }} {chain} {caller}");
    let error = try_export_fixture(&[(LIB, "/// A colour.", &leaking)], HOOKS)
        .err()
        .expect("the deep chain reaches the leaking alias");
    assert!(error.contains("mod_api::exposed"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");

    let conflicting = edited.replace(
        "mod caller {",
        "mod shallow { pub struct Shared; } mod caller { use crate::shallow::*;",
    );
    let error = try_export_fixture(&[(LIB, "/// A colour.", &conflicting)], HOOKS)
        .err()
        .expect("the deep binding conflicts with the shallow one");
    assert!(error.contains("mod_api::exposed"), "{error}");
    assert!(error.contains("mod_api::Shared"), "{error}");
    assert!(error.contains("mod_api::shallow::Shared"), "{error}");
}

#[test]
fn multiple_glob_paths_to_the_same_definition_are_not_ambiguous() {
    use std::fmt::Write as _;

    let mut edited = String::from("pub struct Shared; mod bridge0 { pub use crate::Shared; }");
    for n in 1..=32 {
        writeln!(
            edited,
            "mod a{n} {{ pub use crate::bridge{prev}::*; }}
             mod b{n} {{ pub use crate::bridge{prev}::*; }}
             mod bridge{n} {{ pub use crate::a{n}::*; pub use crate::b{n}::*; }}",
            prev = n - 1,
        )
        .unwrap();
    }
    edited.push_str(
        "mod consumer { use crate::bridge32::*; pub fn safe() -> Shared { unimplemented!() } }
        pub use consumer::safe;
        /// A colour.",
    );
    let export = export_fixture(&[(LIB, "/// A colour.", &edited)], HOOKS);
    let value: serde_json::Value = serde_json::from_str(&export.surface.to_json()).unwrap();
    let safe = value["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|function| function["id"] == "fn:safe")
        .expect("safe is exported");
    assert_eq!(safe["refs"], serde_json::json!(["type:Shared"]));
}

#[test]
fn a_private_named_type_blocks_only_its_own_workspace_glob_binding() {
    let bridge = "mod hidden { pub struct Option; }
        mod fallback { pub struct Option; pub struct Other; }
        pub mod bridge { use crate::hidden::Option; pub use crate::fallback::*; }";
    let engine = ("engine-fixture/src/lib.rs", "pub struct Secret;", bridge);
    let safe = "use engine::bridge::*;
        pub fn safe() -> Option<u8> { unimplemented!() }
        /// A colour.";
    try_export_fixture(&[(LIB, "/// A colour.", safe), engine], HOOKS)
        .expect("the private import shadows fallback::Option, leaving the prelude Option");

    let leak = safe.replace("fn safe() -> Option<u8>", "fn leak() -> Other");
    let error = try_export_fixture(&[(LIB, "/// A colour.", &leak), engine], HOOKS)
        .err()
        .expect("the same glob still imports the unrelated public type");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::fallback::Other"), "{error}");
}

#[test]
fn public_glob_cycles_do_not_hide_exports_from_a_later_alias() {
    for exports in [
        "pub use a::marker; pub use alias::*;",
        "pub use alias::*; pub use a::marker;",
    ] {
        let edited = format!(
            "mod a {{
                pub fn marker() {{}}
                pub fn leak() -> engine::Secret {{ unimplemented!() }}
                pub use crate::b::*;
             }}
             mod b {{ pub use crate::a::*; }}
             use self::b as alias;
             {exports}
             /// A colour."
        );
        let error = try_export_fixture(&[(LIB, "/// A colour.", &edited)], HOOKS)
            .err()
            .expect("the glob alias still exports the leaking function");
        assert!(error.contains("mod_api::leak"), "{error}");
        assert!(error.contains("engine_library::Secret"), "{error}");
    }
}

#[test]
fn a_private_module_shadows_globbed_types_without_hiding_the_prelude() {
    let edited = "mod a { pub struct Option; }
        mod b { pub struct Option; }
        mod bridge { mod Option {} pub use crate::a::*; pub use crate::b::*; }
        use bridge::*;
        pub fn safe() -> Option<u8> { unimplemented!() }
        /// A colour.";
    try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("the hidden module blocks both globs, leaving prelude Option");
}

#[test]
fn a_private_module_alias_shadows_a_workspace_module_without_hiding_the_prelude() {
    let engine = (
        "engine-fixture/src/lib.rs",
        "pub struct Secret;",
        "mod hidden { pub mod Option {} }
         mod fallback { pub mod Option {} }
         pub mod bridge { use crate::hidden::Option; pub use crate::fallback::*; }",
    );
    let safe = "use engine::bridge::*;
        pub fn safe() -> Option<u8> { unimplemented!() }
        /// A colour.";
    try_export_fixture(&[(LIB, "/// A colour.", safe), engine], HOOKS)
        .expect("the hidden alias blocks the workspace module, leaving prelude Option");
}

#[test]
fn distinct_modules_named_through_intermediate_globs_are_ambiguous() {
    let edited = "mod a { pub mod clash { pub struct T; } }
        mod b { pub mod original { pub struct T; } pub use original as clash; }
        mod bridge { pub use crate::a::*; pub use crate::b::*; }
        use bridge::*;
        pub fn ambiguous() -> clash::T { unimplemented!() }
        /// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the two modules occupy the same type namespace");
    assert!(error.contains("mod_api::ambiguous"), "{error}");
    assert!(error.contains("`clash`"), "{error}");
    assert!(error.contains("mod_api::a::clash"), "{error}");
    assert!(error.contains("mod_api::b::original"), "{error}");
}

#[test]
fn a_cfg_disabled_workspace_glob_does_not_shadow_the_prelude() {
    let edited = "#[cfg(feature = \"extra\")] use engine::*;\npub fn safe() -> Result<u32, u8> { unimplemented!() }\n/// A colour.";
    try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("the default build resolves Result from the prelude");
}

#[test]
fn a_target_disabled_workspace_glob_does_not_shadow_the_prelude() {
    let inactive = if cfg!(windows) { "unix" } else { "windows" };
    let edited = format!(
        "#[cfg({inactive})] use engine::*;\npub fn safe() -> Result<u32, u8> {{ unimplemented!() }}\n/// A colour."
    );
    try_export_fixture(&[(LIB, "/// A colour.", &edited)], HOOKS)
        .expect("the target-disabled glob does not shadow Result from the prelude");
}

#[test]
fn a_public_type_through_a_workspace_module_glob_is_still_a_leak() {
    let edited =
        "use engine::nested::*;\npub fn leak() -> Hidden { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the exported type from an imported module must be found");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::nested::Hidden"), "{error}");
}

#[test]
fn workspace_globs_see_transitive_default_features_but_not_optional_ones() {
    let edited = "mod imported {
            use engine::*;
            pub fn result() -> Result<u8, u8> { unimplemented!() }
        }
        pub use imported::result;
        /// A colour.";
    let manifest = "engine-fixture/Cargo.toml";
    let source = (
        "engine-fixture/src/lib.rs",
        "#[cfg(feature = \"extra\")] pub struct Result",
        "#[cfg(all(feature = \"default\", feature = \"extra\"))] pub struct Result",
    );
    let declarations = "[features]\nbridge = [\"extra\"]\nextra = []\n";
    let disabled = format!("name = \"engine_library\"\n{declarations}");
    try_export_fixture(
        &[
            (LIB, "/// A colour.", edited),
            source,
            (manifest, "name = \"engine_library\"", &disabled),
        ],
        HOOKS,
    )
    .expect("the optional feature is off, so Result comes from the prelude");

    let enabled = format!("{disabled}default = [\"bridge\"]\n");
    let error = try_export_fixture(
        &[
            (LIB, "/// A colour.", edited),
            source,
            (manifest, "name = \"engine_library\"", &enabled),
        ],
        HOOKS,
    )
    .err()
    .expect("the default feature transitively adds the workspace Result to the glob");
    assert!(error.contains("mod_api::result"), "{error}");
    assert!(error.contains("engine_library::Result"), "{error}");
}

#[test]
fn unsupported_workspace_dependency_feature_configurations_name_the_edge() {
    let path = "crates/sunrise-mod-vocab/Cargo.toml";
    let dep = "engine = { package = \"engine-package\", path = \"../../engine-fixture\" }";
    for option in [
        "features = [\"extra\"]",
        "default-features = false",
        "optional = true",
    ] {
        let configured = format!("{}, {option} }}", dep.trim_end_matches(" }"));
        let error = try_export_fixture(&[(path, dep, &configured)], HOOKS)
            .err()
            .expect("an unsupported workspace edge must fail the audit");
        assert!(
            error.contains("mod_api") && error.contains("`engine`"),
            "{error}"
        );
        assert!(
            error.contains(option.split(" = ").next().unwrap()),
            "{error}"
        );
    }
    let forwarded = format!("{dep}\n[features]\ndefault = [\"engine/extra\"]");
    let error = try_export_fixture(&[(path, dep, &forwarded)], HOOKS)
        .err()
        .expect("forwarding a Cargo feature must fail the audit");
    assert!(
        error.contains("`engine`") && error.contains("engine/extra"),
        "{error}"
    );
}

#[test]
fn preflight_reads_only_dependencies_for_the_current_target() {
    for (selector, active) in [
        ("cfg(all())", true),
        ("cfg(any())", false),
        (r#"cfg(target_vendor = "sunrise-fixture-vendor")"#, false),
        ("x86_64-sunrise-fixture-os", false),
    ] {
        let table = format!("[target.'{selector}'.dependencies]");
        let result = try_export_fixture(
            &[
                (
                    "crates/sunrise-mod-vocab/Cargo.toml",
                    "[dependencies]",
                    &table,
                ),
                (
                    "engine-fixture/src/lib.rs",
                    "pub struct Secret;",
                    "include!(concat!(env!(\"OUT_DIR\"), \"unknown_generated.rs\"));",
                ),
            ],
            HOOKS,
        );
        if active {
            let error = result.err().expect("the active dependency must be indexed");
            assert!(error.contains("unknown_generated.rs"), "{error}");
        } else {
            result.expect("the inactive dependency's source should not be read");
        }
    }
}

#[test]
fn a_workspace_library_without_source_cannot_be_silently_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"outside\"]\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("outside")).unwrap();
    std::fs::write(
        root.join("outside/Cargo.toml"),
        "[package]\nname = \"outside\"\n[lib]\nname = \"outside\"\npath = \"src/lib.rs\"\n",
    )
    .unwrap();
    let error = OutsideSeam::read(root)
        .err()
        .expect("a missing library source must stop the boundary audit");
    assert!(error.contains("library source"), "{error}");
}

#[test]
fn an_unreadable_workspace_generated_include_stops_the_surface_audit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/sunrise-mod-vocab\", \"outside\"]\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("outside/src")).unwrap();
    let vocabulary = root.join("crates/sunrise-mod-vocab/Cargo.toml");
    std::fs::create_dir_all(vocabulary.parent().unwrap()).unwrap();
    std::fs::write(
        vocabulary,
        "[package]\nname = \"sunrise-mod-vocab\"\n[dependencies]\noutside = { path = \"../../outside\" }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("outside/Cargo.toml"),
        "[package]\nname = \"outside\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("outside/src/lib.rs"),
        "include!(\"generated.rs\");\n",
    )
    .unwrap();

    let outside = OutsideSeam::read(root).unwrap();
    let mut index = Index::new(root, &[], &[]);
    let error = outside
        .add_to_index(&mut index)
        .expect_err("the unreadable generated include must stop the audit");
    assert!(error.contains("include!"), "{error}");
}

#[test]
fn an_unreadable_dev_dependency_does_not_stop_the_surface_audit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/sunrise-mod-vocab\", \"outside\"]\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("outside/src")).unwrap();
    let vocabulary = root.join("crates/sunrise-mod-vocab/Cargo.toml");
    std::fs::create_dir_all(vocabulary.parent().unwrap()).unwrap();
    std::fs::write(
        vocabulary,
        "[package]\nname = \"sunrise-mod-vocab\"\n[dev-dependencies]\noutside = { path = \"../../outside\" }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("outside/Cargo.toml"),
        "[package]\nname = \"outside\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("outside/src/lib.rs"),
        "include!(\"generated.rs\");\n",
    )
    .unwrap();

    let outside = OutsideSeam::read(root).unwrap();
    let mut index = Index::new(root, &[], &[]);
    outside
        .add_to_index(&mut index)
        .expect("dev dependencies are not part of the default API surface");
}

#[test]
fn a_public_reexport_cannot_name_a_workspace_type_outside_the_seam() {
    let edited = "pub use engine::Secret as Leaked;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the re-export must fail the export");
    assert!(error.contains("mod_api::Leaked"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_named_reexport_resolves_the_default_cfg_module_variant() {
    let active = if cfg!(windows) {
        "WindowsActive"
    } else {
        "UnixActive"
    };
    let edited = format!("pub use engine::conditional::{active} as Leaked;\n/// A colour.");
    let error = try_export_fixture(&[(LIB, "/// A colour.", &edited)], HOOKS)
        .err()
        .expect("the public type in the active module variant must be found");
    assert!(error.contains("mod_api::Leaked"), "{error}");
    assert!(
        error.contains(&format!("engine_library::conditional::{active}")),
        "{error}"
    );
}

#[test]
fn a_public_glob_reexport_cannot_hide_a_workspace_crate() {
    let edited = "pub use engine::*;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the glob re-export must fail the export");
    assert!(error.contains("mod_api::*"), "{error}");
    assert!(error.contains("engine_library::*"), "{error}");
}

#[test]
fn a_visible_workspace_glob_after_a_hidden_glob_is_still_checked() {
    let edited = "#[doc(hidden)] pub use engine::*;\npub use engine::*;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the visible glob must not be hidden by a preceding hidden glob");
    assert!(
        error.starts_with("mod_api::*: `engine_library::*`"),
        "{error}"
    );
    assert!(error.contains("engine_library::*"), "{error}");
}

#[test]
fn a_module_alias_reexport_cannot_hide_a_workspace_crate() {
    let edited = "use engine as dependency;\npub use dependency::nested as Leaked;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the aliased re-export must fail the export");
    assert!(error.contains("mod_api::Leaked"), "{error}");
    assert!(error.contains("engine_library::nested"), "{error}");
}

#[test]
fn a_whole_crate_import_alias_cannot_hide_a_workspace_type() {
    let edited = "use engine as dependency;\npub fn leak() -> dependency::Secret { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the aliased signature must fail the export");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_local_module_alias_cannot_hide_a_workspace_type_from_a_public_signature() {
    let edited = "mod hidden { pub use engine::Secret; }\nuse crate::hidden as h;\npub fn leak() -> h::Secret { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the local-module alias must fail the export");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_nested_private_alias_composes_with_a_local_module_alias() {
    let edited = "mod inner { pub(crate) type Alias = engine::Secret; }\nuse crate::inner as h;\npub fn leak() -> h::Alias { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the nested alias must resolve through the imported module alias");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn self_qualified_local_module_aliases_still_expand_private_type_aliases() {
    let edited = "mod inner { pub(crate) type Alias = engine::Secret; }\nuse self::inner as h;\npub fn leak() -> self::h::Alias { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("self-qualified module aliases must reach the private type alias");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_public_glob_reexport_checks_signatures_of_its_module_alias() {
    let edited = "mod inner { pub(crate) type Alias = engine::Secret; pub fn leak() -> Alias { unimplemented!() } }\nuse self::inner as h;\npub use h::*;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the re-exported function's alias must be checked");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_public_named_reexport_checks_signatures_of_its_module_alias() {
    let edited = "mod inner { pub(crate) type Alias = engine::Secret; pub fn leak() -> Alias { unimplemented!() } }\nuse self::inner as h;\npub use h::leak;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the named re-export's alias must be checked");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn an_extern_crate_alias_cannot_hide_a_workspace_type() {
    let edited = "extern crate engine as dependency;\npub fn leak() -> dependency::Secret { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the extern-crate alias must fail the export");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_cfg_disabled_extern_crate_alias_does_not_override_the_active_alias() {
    let edited = "extern crate core as api;\n#[cfg(feature = \"extra\")] extern crate engine as api;\npub fn safe() -> api::option::Option<u32> { unimplemented!() }\n/// A colour.";
    try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("the disabled workspace alias does not override the active core alias");
}

#[test]
fn a_renamed_workspace_dependency_cannot_hide_a_workspace_type() {
    let edited = "pub fn leak() -> engine::Secret { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the renamed dependency signature must fail the export");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_rooted_external_path_wins_over_a_local_module_name() {
    let edited = "mod engine { pub struct Local; }\npub fn leak() -> ::engine::Secret { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the rooted path must resolve through the dependency root");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_facade_self_alias_does_not_hide_its_dependency_glob() {
    let export = export_fixture(
        &[(
            "crates/sunrise-mod-api/src/lib.rs",
            "pub use mod_vocab::*;",
            "pub use mod_vocab::*;\npub struct FacadeType;\npub fn own(value: mod_api::FacadeType) -> mod_api::FacadeType { value }",
        )],
        HOOKS,
    );
    assert!(export.surface.traits.iter().any(|t| t.id == "trait:Hooks"));
    assert!(export.surface.types.iter().any(|t| t.id == "type:Point"));
    let own = export
        .surface
        .functions
        .iter()
        .find(|f| f.id == "fn:own")
        .unwrap();
    assert_eq!(own.contract.refs, ["type:FacadeType"]);
}

#[test]
fn a_rooted_facade_reexport_resolves_an_allowed_workspace_crate() {
    let facade = "pub use ::mod_vocab::*;\npub fn rooted(value: ::mod_vocab::EntityRef) -> ::mod_vocab::EntityRef { value }\n";
    let export = try_export_fixture(
        &[(
            "crates/sunrise-mod-api/src/lib.rs",
            "pub use mod_vocab::*;\n",
            facade,
        )],
        HOOKS,
    )
    .expect("rooted paths into an allowed seam crate resolve to its indexed public items");
    let value: serde_json::Value = serde_json::from_str(&export.surface.to_json()).unwrap();
    let function = value["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|function| function["id"] == "fn:rooted")
        .expect("the rooted signature is part of the exported surface");
    assert!(
        function["signature"]
            .as_str()
            .unwrap()
            .contains("entity::EntityRef")
    );
    assert!(
        function["refs"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("type:entity::EntityRef"))
    );
}

#[test]
fn a_rooted_named_reexport_cannot_expose_an_outside_value_through_a_hidden_glob() {
    for (nested, definition) in [
        (
            "pub mod nested { pub struct Hidden; pub fn outside_fn() {} }",
            "engine_library::nested::outside_fn",
        ),
        (
            "mod internal {
                pub fn outside_fn() {}
                pub mod exports { pub use super::outside_fn; }
            }
            pub use internal::exports as nested;",
            "engine_library::internal::outside_fn",
        ),
    ] {
        let error = try_export_fixture(
            &[
                (
                    LIB,
                    "/// A colour.",
                    "#[doc(hidden)] pub mod bridge { pub use engine::nested::*; }\n/// A colour.",
                ),
                (
                    "crates/sunrise-mod-api/src/lib.rs",
                    "pub use mod_vocab::*;",
                    "pub use mod_vocab::*;\npub use ::mod_vocab::bridge::outside_fn as Leaked;",
                ),
                (
                    "engine-fixture/src/lib.rs",
                    "pub mod nested { pub struct Hidden; }",
                    nested,
                ),
            ],
            HOOKS,
        )
        .err()
        .expect("the re-exported value is defined outside the API's crates");
        assert!(error.contains("mod_api::Leaked"), "{error}");
        assert!(error.contains(definition), "{error}");
    }
}

#[test]
fn a_named_reexport_keeps_its_workspace_crate_when_a_local_module_collides() {
    let edited = "mod engine_library { pub struct Local; }\npub use engine::Secret as Exported;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the canonical engine crate must survive a colliding local module");
    assert!(error.contains("mod_api::Exported"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_public_glob_from_self_module_is_not_the_dependency_glob() {
    let edited = "mod engine { pub struct Local; }\npub use self::engine::*;\n/// A colour.";
    try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("self::engine names the local module despite the dependency alias");
}

#[test]
fn a_private_type_alias_cannot_hide_a_workspace_type_from_a_public_signature() {
    let edited = "type Private = engine::Secret;\npub fn leak() -> Private { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the private alias signature must fail the export");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_private_alias_keeps_references_from_its_public_type_expansion() {
    let edited = "type Private = Text;\npub fn through_alias(value: Private) -> Private { value }\n/// A colour.";
    let export = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("a private alias to an API type is valid");
    let value: serde_json::Value = serde_json::from_str(&export.surface.to_json()).unwrap();
    let function = value["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|function| function["id"] == "fn:through_alias")
        .expect("the function is exported");
    assert!(
        function["refs"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("type:Text"))
    );
}

#[test]
fn a_generic_projection_does_not_resolve_to_a_same_named_workspace_module() {
    let edited = "use engine::*;
        pub trait LocalTrait { type Out; }
        pub fn project<T: LocalTrait>(_: T) -> T::Out { unimplemented!() }
        /// A colour.";
    let engine = (
        "engine-fixture/src/lib.rs",
        "pub struct Secret;",
        "pub mod T { pub struct Out; }",
    );
    let export = export_fixture(&[(LIB, "/// A colour.", edited), engine], HOOKS);
    let project = export
        .surface
        .functions
        .iter()
        .find(|f| f.id == "fn:project")
        .unwrap();
    assert!(
        project.contract.signature.contains("-> T::Out"),
        "{project:?}"
    );
    assert_eq!(project.contract.refs, ["trait:LocalTrait"]);

    let leak = format!("pub fn leak<const T: usize>() -> T::Out {{ unimplemented!() }}\n{edited}");
    let error = try_export_fixture(&[(LIB, "/// A colour.", &leak), engine], HOOKS)
        .err()
        .expect("a const parameter must not shadow the module in a qualified type path");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::T::Out"), "{error}");
}

#[test]
fn an_indexed_dependency_type_preserves_its_published_spelling() {
    let export = export_fixture(
        &[(
            LIB,
            "/// A colour.",
            "use sunrise_mod_runtime::storage as store;
             pub type ViaAlias = store::Query;
             pub type Direct = sunrise_mod_runtime::storage::Query;
             /// A colour.",
        )],
        HOOKS,
    );
    for (id, target) in [("type:ViaAlias", "store::Query"), ("type:Direct", "Query")] {
        let query = export.surface.types.iter().find(|t| t.id == id).unwrap();
        assert_eq!(query.contract.target.as_deref(), Some(target));
    }
}

#[test]
fn a_private_alias_keeps_trait_references_from_a_qualified_associated_type() {
    let edited = "pub trait LocalTrait { type Out; }\nimpl LocalTrait for u32 { type Out = u8; }\ntype Private = <u32 as LocalTrait>::Out;\npub fn through_alias() -> Private { unimplemented!() }\n/// A colour.";
    let export = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("a private alias to a public associated type is valid");
    let value: serde_json::Value = serde_json::from_str(&export.surface.to_json()).unwrap();
    let function = value["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|function| function["id"] == "fn:through_alias")
        .expect("the function is exported");
    assert!(
        function["refs"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("trait:LocalTrait"))
    );
}

#[test]
fn a_private_type_alias_generic_default_cannot_hide_a_workspace_type() {
    let edited = "type Private<T = engine::Secret> = T;\npub fn leak() -> Private { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the private alias default must fail the export");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_private_type_alias_does_not_check_a_default_overridden_by_the_call() {
    let edited = "type Alias<T = engine::Secret> = T;\npub fn safe() -> Alias<u32> { unimplemented!() }\n/// A colour.";
    try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .expect("the explicit argument overrides the outside default");
}

#[test]
fn a_bare_const_generic_argument_does_not_hide_a_type_default() {
    let edited = "type Alias<const K: usize, T = engine::Secret> = [T; K];\npub fn leak<const N: usize>(_: Alias<N>) {}\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the type default must be checked when only the const argument is supplied");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_private_alias_checks_its_default_when_used_again_without_an_argument() {
    let edited = "type Alias<T = engine::Secret> = T;\npub fn leak() -> (Alias<u32>, Alias) { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the later defaulted use must still inspect the outside default");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_qualified_associated_type_checks_its_workspace_trait_path_without_a_bound() {
    let edited = "pub fn leak() -> <u32 as engine::Trait>::Out { unimplemented!() }\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the associated type's workspace trait path must fail the export");
    assert!(error.contains("mod_api::leak"), "{error}");
    assert!(error.contains("engine_library::Trait"), "{error}");
}

#[test]
fn a_public_constant_checks_its_workspace_type() {
    let edited = "pub const LEAK: engine::Secret = engine::Secret;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the public constant must fail the export");
    assert!(error.contains("mod_api::LEAK"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_public_static_checks_its_workspace_type() {
    let edited = "pub static LEAK: Option<engine::Secret> = None;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the public static must fail the export");
    assert!(error.contains("mod_api::LEAK"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_public_constant_checks_a_private_alias_to_a_workspace_type() {
    let edited =
        "type Internal = engine::Secret;\npub const LEAK: Option<Internal> = None;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the constant's private alias must be checked");
    assert!(error.contains("mod_api::LEAK"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_public_static_checks_a_private_alias_to_a_workspace_type() {
    let edited =
        "type Internal = engine::Secret;\npub static LEAK: Option<Internal> = None;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the static's private alias must be checked");
    assert!(error.contains("mod_api::LEAK"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_public_constant_checks_a_hidden_alias_to_a_workspace_type() {
    let edited = "#[doc(hidden)] pub type Internal = engine::Secret;\npub const LEAK: Option<Internal> = None;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the constant's hidden alias must be visited");
    assert!(error.contains("mod_api::Internal"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_public_static_checks_a_hidden_alias_to_a_workspace_type() {
    let edited = "#[doc(hidden)] pub type Internal = engine::Secret;\npub static LEAK: Option<Internal> = None;\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the static's hidden alias must be visited");
    assert!(error.contains("mod_api::Internal"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

#[test]
fn a_wire_layout_cannot_hide_a_workspace_type_behind_private_storage() {
    let edited = "#[derive(Wire)] struct Private(engine::Secret);\n#[derive(Wire)] pub struct Leaked(Private);\n/// A colour.";
    let error = try_export_fixture(&[(LIB, "/// A colour.", edited)], HOOKS)
        .err()
        .expect("the embedded type must fail the export");
    assert!(error.contains("mod_api::Private"), "{error}");
    assert!(error.contains("engine_library::Secret"), "{error}");
}

fn hashes(export: &Export) -> BTreeMap<String, String> {
    export
        .surface
        .entries()
        .map(|(id, hash)| (id.to_owned(), hash.to_owned()))
        .collect()
}

/// The ids whose hash changed, and the ids added, from the base fixture to the edited one.
fn changes(edits: &[(&str, &str, &str)], hooks: &str) -> (Vec<String>, Vec<String>) {
    let before = hashes(&export_fixture(&[], HOOKS));
    let after = hashes(&export_fixture(edits, hooks));
    let changed = before
        .iter()
        .filter(|(id, hash)| after.get(*id).is_some_and(|new| new != *hash))
        .map(|(id, _)| id.clone())
        .collect();
    let added = after
        .keys()
        .filter(|id| !before.contains_key(*id))
        .cloned()
        .collect();
    (changed, added)
}

/// `value` with every `hash` and every present `shape` written as `*`.
fn masked(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, value)| match key.as_str() {
                    "hash" | "shape" | "fields_hash" if !value.is_null() => {
                        (key, serde_json::Value::from("*"))
                    }
                    _ => (key, masked(value)),
                })
                .collect(),
        ),
        serde_json::Value::Array(list) => {
            serde_json::Value::Array(list.into_iter().map(masked).collect())
        }
        value => value,
    }
}

#[test]
fn the_fixture_exports_the_expected_document() {
    let export = export_fixture(&[], HOOKS);
    let written: serde_json::Value = serde_json::from_str(&export.surface.to_json()).unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixture_expected.json")).unwrap();
    assert_eq!(masked(written), expected, "{}", export.surface.to_json());
    assert_eq!(
        export.notes,
        [
            "0 public items are not public at their definition path: their id is their shortest public path, which a shorter re-export added later would change",
            "3 items are reachable only through #[doc(hidden)] items or modules: recorded only where a recorded signature names them",
            "constant LIMIT: no record kind for constants",
            "impl Point in mod_api: compiled out of a default native build (cfg(feature = \"extra\")), not recorded",
            "missing_inner! called in mod_api (inside another macro's expansion): not expanded, the items it writes are not recorded",
            "type:Context: no shape, because Context crosses by a hand-written Wire impl that declares no layout, or does not cross",
            "unknown! called in mod_api: not expanded, the items it writes are not recorded",
        ]
    );
}

#[test]
fn macro_calls_are_expanded_or_noted_and_ids_ignore_routes() {
    let export = export_fixture(&[], HOOKS);
    let handles: Vec<&str> = export
        .surface
        .handles
        .iter()
        .map(|h| h.id.as_str())
        .collect();
    for id in [
        "handle:Point::move_to",
        "handle:Point::move_from",
        "handle:Host::draw",
        "handle:Registrar::draw",
        "handle:Point::outer_to",
        "handle:Point::inner_from",
    ] {
        assert!(handles.contains(&id), "{id} in {handles:?}");
    }
    assert!(!handles.contains(&"handle:Point::extra"));
    let thing = export
        .surface
        .types
        .iter()
        .find(|t| t.id.ends_with("Thing"))
        .unwrap();
    assert_eq!(thing.id, "type:shared::Thing");
    assert!(thing.paths.contains(&"s1::Thing".to_owned()));
}

#[test]
fn a_generated_impl_on_a_glob_reexported_handle_is_recorded() {
    let export = export_fixture_with_includes(
        &[(
            LIB,
            "pub mod entity;",
            r#"
pub mod entity;
pub use loan_views::*;
pub use loan_handles::*;
mod loan_views {
    use crate::LoanRef;
    pub fn loan_exists(_: LoanRef) -> bool { true }
}
mod loan_handles {
    pub struct LoanRef(u32);
}
mod generated_handles {
    use super::*;
    include!(concat!(env!("OUT_DIR"), "/fixture_generated_handles.rs"));
}
"#,
        )],
        HOOKS,
        &[(
            "fixture_generated_handles.rs",
            "impl LoanRef { pub fn serial(self) -> u32 { 23 } }",
        )],
    );
    let exported = export
        .surface
        .handles
        .iter()
        .find(|handle| handle.id == "handle:LoanRef::serial")
        .map(|handle| handle.contract.signature.as_str());
    assert_eq!(exported, Some("fn serial(self) -> u32"));
}

#[test]
fn two_exports_are_byte_identical() {
    let a = export_fixture(&[], HOOKS).surface.to_json();
    let b = export_fixture(&[], HOOKS).surface.to_json();
    assert_eq!(a, b);
    assert!(a.ends_with("}\n"));
}

#[test]
fn each_declaration_aspect_changes_exactly_its_records_hash() {
    let cases: &[(&str, &[(&str, &str, &str)], &str, &[&str])] = &[
        (
            "a method's argument type",
            &[(
                LIB,
                "fn save(&mut self, slot: u8);",
                "fn save(&mut self, slot: u16);",
            )],
            HOOKS,
            &["method:ServerHost::save"],
        ),
        (
            "a lifetime",
            &[(LIB, "-> &'a Text {", "-> &'static Text {")],
            HOOKS,
            &["fn:first"],
        ),
        (
            "a field added to a constructible struct: its fields_hash, not its hash",
            &[(
                LIB,
                "    pub y: i32,\n",
                "    pub y: i32,\n    pub z: i32,\n",
            )],
            HOOKS,
            &[],
        ),
        (
            "a variant added to an exhaustive enum",
            &[(LIB, "    On,\n", "    On,\n    Paused,\n")],
            HOOKS,
            &["type:Mode"],
        ),
        (
            "non_exhaustive toggled",
            &[(LIB, "/// A mode.\n", "/// A mode.\n#[non_exhaustive]\n")],
            HOOKS,
            &["type:Mode"],
        ),
        (
            "a derive added: one more of its impls, outside its hash",
            &[(LIB, "/// A mode.\n", "/// A mode.\n#[derive(Debug)]\n")],
            HOOKS,
            &[],
        ),
        (
            "a required method added to a trait mods implement",
            &[(
                LIB,
                "    fn name(&self)",
                "    fn more(&self);\n    fn name(&self)",
            )],
            HOOKS,
            &["trait:Hooks"],
        ),
        (
            "a required method added to a trait the boundary lists do not name (role none)",
            &[(
                LIB,
                "pub trait BlockPart {}",
                "pub trait BlockPart {\n    fn tick(&self);\n}",
            )],
            HOOKS,
            &["trait:BlockPart"],
        ),
        (
            "a handle method's return type",
            &[(
                "crates/sunrise-mod-vocab/src/entity.rs",
                "pub fn id(self) -> u32",
                "pub fn id(self) -> u64",
            )],
            HOOKS,
            &["handle:entity::EntityRef::id"],
        ),
        (
            "a hook's side",
            &[],
            &HOOKS.replacen("side = \"server\"", "side = \"both\"", 1),
            &["hook:ping"],
        ),
        (
            "an argument added to a hook",
            &[(
                LIB,
                "pub cancelled: bool,",
                "pub cancelled: bool,\n        pub extra: u8,",
            )],
            HOOKS,
            &[],
        ),
        (
            "a hook argument's type",
            &[(LIB, "pub cancelled: bool,", "pub cancelled: u8,")],
            HOOKS,
            &["hook:ping.cancelled", "type:PingEvent.cancelled"],
        ),
        (
            "a supertrait bound on a trait mods call",
            &[(
                LIB,
                "pub trait ServerHost: Host {",
                "pub trait ServerHost: Host + Send {",
            )],
            HOOKS,
            &[],
        ),
        (
            "a supertrait bound on a trait mods implement",
            &[(LIB, "pub trait Hooks {", "pub trait Hooks: Send {")],
            HOOKS,
            &["trait:Hooks"],
        ),
        (
            "a defaulted method becoming required on a trait mods implement",
            &[(
                LIB,
                "fn ping(&self, host: &mut dyn ServerHost, event: &mut PingEvent<'_>) {\n        let _ = (host, event);\n    }",
                "fn ping(&self, host: &mut dyn ServerHost, event: &mut PingEvent<'_>);",
            )],
            HOOKS,
            &["method:Hooks::ping", "trait:Hooks"],
        ),
        (
            "an associated type added to a trait mods implement",
            &[(
                LIB,
                "    /// Must be written.",
                "    /// Its settings.\n    type Settings;\n    /// Must be written.",
            )],
            HOOKS,
            &["trait:Hooks"],
        ),
        (
            "a receiver becoming mutable",
            &[(LIB, "fn time(&self) -> u64;", "fn time(&mut self) -> u64;")],
            HOOKS,
            &["method:Host::time"],
        ),
        (
            "a hook's cancellable flag",
            &[],
            &HOOKS.replacen(
                "neoforge = \"PingEvent\"",
                "neoforge = \"PingEvent\",\n        cancellable = true",
                1,
            ),
            &["hook:ping"],
        ),
        (
            "a trait impl added: outside the hash",
            &[(
                LIB,
                "/// A mode.\npub enum Mode {",
                "impl Default for Mode {\n    fn default() -> Self {\n        Self::On\n    }\n}\n/// A mode.\npub enum Mode {",
            )],
            HOOKS,
            &[],
        ),
        (
            "a handle's impl bounds",
            &[(
                "crates/sunrise-mod-vocab/src/entity.rs",
                "impl EntityRef {",
                "impl EntityRef where Self: Copy {",
            )],
            HOOKS,
            &[
                "handle:entity::EntityRef::id",
                "handle:entity::EntityRef::path",
            ],
        ),
        (
            "trait methods reordered",
            &[(
                LIB,
                "    /// Saves.\n    ///\n    /// It writes\n    /// files.\n    fn save(&mut self, slot: u8);\n    /// Sends a text.\n    fn send(&mut self, text: Text) -> bool;",
                "    /// Sends a text.\n    fn send(&mut self, text: Text) -> bool;\n    /// Saves.\n    ///\n    /// It writes\n    /// files.\n    fn save(&mut self, slot: u8);",
            )],
            HOOKS,
            &[],
        ),
        (
            "a file moved under the same public path",
            &[(
                "crates/sunrise-mod-vocab/src/entity.rs",
                MOVE,
                "crates/sunrise-mod-vocab/src/entity/mod.rs",
            )],
            HOOKS,
            &[],
        ),
        (
            "a new root re-export",
            &[(
                LIB,
                "pub use entity::EntityRef;",
                "pub use entity::EntityRef;\npub use entity::EntityRef as Handle;",
            )],
            HOOKS,
            &[],
        ),
        (
            "a method doc reworded",
            &[(
                LIB,
                "/// It writes\n    /// files.",
                "/// It writes to the disk.",
            )],
            HOOKS,
            &[],
        ),
        (
            "a doc line",
            &[(LIB, "/// A point.", "/// A point on a plane.")],
            HOOKS,
            &[],
        ),
        (
            "a hook's doc, engine target and counterpart",
            &[],
            &HOOKS
                .replace("Something pinged.", "Something was pinged.")
                .replace("Game::ping", "Game::pinged")
                .replace("neoforge = \"PingEvent\"", "neoforge = \"PingedEvent\""),
            &[],
        ),
        (
            "a method added to a trait mods call",
            &[(
                LIB,
                "    fn time(&self) -> u64;",
                "    fn time(&self) -> u64;\n    fn day(&self) -> u64;",
            )],
            HOOKS,
            &[],
        ),
        (
            "a defaulted method added to a trait mods implement",
            &[(
                LIB,
                "    fn name(&self)",
                "    fn more(&self) {}\n    fn name(&self)",
            )],
            HOOKS,
            &[],
        ),
        (
            "a field added to a type mods cannot build",
            &[(
                LIB,
                "    style: u8,\n",
                "    style: u8,\n    pub size: u8,\n",
            )],
            HOOKS,
            &[],
        ),
        (
            "a variant added to a non-exhaustive enum",
            &[(LIB, "    Red,\n", "    Red,\n    Blue,\n")],
            HOOKS,
            &[],
        ),
    ];
    for (aspect, edits, hooks, expected) in cases {
        let (changed, added) = changes(edits, hooks);
        assert_eq!(changed, *expected, "{aspect}");
        if aspect.starts_with("trait methods") || aspect.starts_with("a file moved") {
            assert_eq!(added, Vec::<String>::new(), "{aspect}");
        }
    }
    // A new hook (with its event type) adds records and changes none.
    let pong = HOOKS.replace(
        "fn tick_block();\n}",
        "fn tick_block();\n    /// Ponged.\n    #[hook(side = \"client\", contract = \"listen\", target = \"x::pong\", neoforge = \"none\")]\n    fn pong();\n}",
    );
    let (changed, added) = changes(
        &[(
            LIB,
            "mod events {\n    use super::*;",
            "mod events {\n    use super::*;\n    /// The event of `pong`.\n    pub struct PongEvent { pub n: u8 }",
        )],
        &pong,
    );
    assert_eq!(changed, Vec::<String>::new());
    assert!(added.contains(&"hook:pong".to_owned()), "{added:?}");
}

#[test]
fn shapes_follow_the_wire_layout() {
    let base = export_fixture(&[], HOOKS);
    let shape = |export: &Export, id: &str| {
        export
            .surface
            .types
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .shape
            .clone()
    };
    // Reordering fields changes the shape, not the contract of a type mods cannot build.
    let reordered = export_fixture(
        &[(
            LIB,
            "    pub parts: Vec<Text>,\n    style: u8,",
            "    style: u8,\n    pub parts: Vec<Text>,",
        )],
        HOOKS,
    );
    assert_ne!(shape(&base, "type:Text"), shape(&reordered, "type:Text"));
    // A nested type's change reaches the types that hold it.
    let nested = export_fixture(&[(LIB, "    pub y: i32,\n}", "    pub y: i64,\n}")], HOOKS);
    let holder = |e: &Export| shape(e, "type:Text");
    assert!(holder(&base).is_some());
    let with_point = export_fixture(
        &[(
            LIB,
            "    style: u8,\n}",
            "    style: u8,\n    at: Point,\n}",
        )],
        HOOKS,
    );
    let with_wider_point = export_fixture(
        &[
            (
                LIB,
                "    style: u8,\n}",
                "    style: u8,\n    at: Point,\n}",
            ),
            (LIB, "    pub y: i32,\n}", "    pub y: i64,\n}"),
        ],
        HOOKS,
    );
    assert_ne!(holder(&with_point), holder(&with_wider_point));
    assert_ne!(shape(&base, "type:Point"), shape(&nested, "type:Point"));
    // A hand-written impl: its declared fields are its shape, its Rust fields are not.
    let ident = |e: &Export| shape(e, "type:Ident");
    let cache = export_fixture(
        &[(
            LIB,
            "    cache: u64,\n    pub namespace",
            "    cache: u32,\n    pub namespace",
        )],
        HOOKS,
    );
    assert_eq!(ident(&base), ident(&cache));
    let declared = export_fixture(
        &[(
            LIB,
            "layout_fields!(\"namespace\" => String);",
            "layout_fields!(\"namespace\" => String, \"path\" => String);",
        )],
        HOOKS,
    );
    assert_ne!(ident(&base), ident(&declared));
    let contracted = export_fixture(
        &[(
            LIB,
            "layout_fields!(\"namespace\" => String);",
            "layout_fields!(\"contract:v2\"; \"namespace\" => String);",
        )],
        HOOKS,
    );
    assert_eq!(ident(&base), ident(&contracted));
    // A type that crosses as another has that type's shape inside `via`, and one with no
    // declared layout has none.
    let types: BTreeMap<&str, &Type> = base
        .surface
        .types
        .iter()
        .map(|t| (t.id.as_str(), t))
        .collect();
    let form = |id: &str| types[id].contract.wire_form.clone();
    assert_eq!(form("type:Name").as_deref(), Some("via:String"));
    assert_eq!(form("type:Ident").as_deref(), Some("fields"));
    assert_eq!(form("type:SlotKey").as_deref(), Some("fields"));
    assert_eq!(form("type:Context").as_deref(), Some("manual"));
    assert_eq!(form("type:Mode"), None);
    assert!(types["type:Context"].contract.wire && types["type:Context"].shape.is_none());
    assert!(types["type:Mode"].shape.is_none());
    // Renaming an unrelated type changes no other shape.
    let renamed = export_fixture(&[(LIB, "pub enum Mode {", "pub enum Toggle {")], HOOKS);
    assert_eq!(shape(&base, "type:Text"), shape(&renamed, "type:Text"));
    assert_eq!(shape(&base, "type:Point"), shape(&renamed, "type:Point"));
}

/// The real tree exports with unique ids, every catalog hook, and no CI-only engine records.
#[test]
#[ignore = "daily"]
fn the_real_tree_exports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let export = export(&root).unwrap();
    let text = export.surface.to_json();
    let read = Surface::read(text.as_bytes()).unwrap();
    assert!(read.engine_items.is_empty());
    assert!(!text.contains("\"engine_items\":"));
    let catalog = Catalog::load(&root).unwrap();
    let ids: BTreeSet<&str> = read.hooks.iter().map(|h| h.id.as_str()).collect();
    let part_ids: BTreeMap<&str, &[String]> = read
        .parts
        .iter()
        .map(|part| (part.id.as_str(), part.hooks.as_slice()))
        .collect();
    for hook in &catalog.hooks {
        assert!(
            ids.contains(format!("hook:{}", hook.name).as_str()),
            "{}",
            hook.name
        );
        if let Some(kind) = hook.part {
            let part_id = format!("part:{}", part_kind_snake(kind));
            let hook_id = format!("hook:{}", hook.name);
            assert!(
                part_ids
                    .get(part_id.as_str())
                    .is_some_and(|hooks| hooks.iter().any(|id| id == &hook_id)),
                "{part_id} missing {hook_id} in {part_ids:?}"
            );
        }
    }
    assert!(read.traits.iter().any(|t| t.id == "trait:Hooks"));
    for event in read.types.iter().filter(|ty| {
        ty.id.ends_with("Event")
            && matches!(
                ty.contract.kind,
                TypeKind::Struct | TypeKind::UnitStruct | TypeKind::TupleStruct
            )
    }) {
        assert!(!event.contract.constructible, "{}", event.id);
    }
}

#[test]
fn a_derive_and_a_hand_written_impl_are_the_same_impl() {
    let point = |export: &Export| {
        export
            .surface
            .types
            .iter()
            .find(|t| t.id == "type:Point")
            .cloned()
            .unwrap()
    };
    let before = point(&export_fixture(&[], HOOKS));
    let derived = point(&export_fixture(
        &[
            (
                LIB,
                "#[derive(Clone, Copy, Wire)]\npub struct Point",
                "#[derive(Clone, Copy, Default, Wire)]\npub struct Point",
            ),
            (
                LIB,
                "impl Default for Point {\n    fn default() -> Self {\n        Self { x: 0, y: 0 }\n    }\n}\n",
                "",
            ),
        ],
        HOOKS,
    ));
    assert_eq!(before.impls, ["Clone", "Copy", "Default"]);
    assert_eq!(derived.impls, before.impls);
    assert_eq!(derived.hash, before.hash);
}

#[test]
fn how_a_type_crosses_changes_its_shape_and_not_its_hash() {
    let text = |export: &Export| {
        export
            .surface
            .types
            .iter()
            .find(|t| t.id == "type:Text")
            .cloned()
            .unwrap()
    };
    let before = text(&export_fixture(&[], HOOKS));
    // From a derive to crossing as a string.
    let via = text(&export_fixture(
        &[
            (
                LIB,
                "#[derive(Clone, Debug, Wire)]\npub struct Text",
                "#[derive(Clone, Debug)]\npub struct Text",
            ),
            (
                LIB,
                "/// A name that crosses as a string.",
                "impl Wire for Text {\n    layout_via!(String);\n}\n/// A name that crosses as a string.",
            ),
        ],
        HOOKS,
    ));
    assert_eq!(before.contract.wire_form.as_deref(), Some("derive"));
    assert_eq!(via.contract.wire_form.as_deref(), Some("via:String"));
    assert_eq!(via.hash, before.hash);
    assert!(before.shape.is_some() && via.shape.is_some());
    assert_ne!(via.shape, before.shape);
}

/// A tree from before `layout_via!` declared a forwarding impl by a method returning the other
/// type's layout: it reads as the same `via:` form, and exports without call names, so a
/// comparison across the change never skips the old side.
#[test]
fn a_baseline_forwarding_its_layout_reads_as_the_declared_via_form() {
    let text = |export: &Export| {
        export
            .surface
            .types
            .iter()
            .find(|t| t.id == "type:Text")
            .cloned()
            .unwrap()
    };
    let old = [
        (
            LIB,
            "#[derive(Clone, Debug, Wire)]\npub struct Text",
            "#[derive(Clone, Debug)]\npub struct Text",
        ),
        (
            LIB,
            "/// A name that crosses as a string.",
            "impl Wire for Text {\n    fn layout(visiting: &mut Vec<&'static str>) -> u64 {\n        <String as Wire>::layout(visiting)\n    }\n}\n/// A name that crosses as a string.",
        ),
    ];
    let forwarded = text(&export_fixture(&old, HOOKS));
    assert_eq!(forwarded.contract.wire_form.as_deref(), Some("via:String"));
    let declared = text(&export_fixture(
        &[
            old[0],
            (
                LIB,
                "/// A name that crosses as a string.",
                "impl Wire for Text {\n    layout_via!(String);\n}\n/// A name that crosses as a string.",
            ),
        ],
        HOOKS,
    ));
    assert_eq!(forwarded.hash, declared.hash);
    assert_eq!(forwarded.shape, declared.shape);
}

#[test]
fn a_hidden_public_field_is_part_of_the_field_set() {
    let point = |edits: &[(&str, &str, &str)]| {
        export_fixture(edits, HOOKS)
            .surface
            .types
            .iter()
            .find(|t| t.id == "type:Point")
            .cloned()
            .unwrap()
    };
    let before = point(&[]);
    let hidden = point(&[(
        LIB,
        "    pub y: i32,\n",
        "    pub y: i32,\n    #[doc(hidden)]\n    pub z: i32,\n",
    )]);
    assert_eq!(hidden.fields.len(), before.fields.len());
    assert_eq!(hidden.hash, before.hash);
    assert_ne!(hidden.fields_hash, before.fields_hash);
    // A private field changes neither, and makes the struct unbuildable.
    let private = point(&[(LIB, "    pub y: i32,\n", "    pub y: i32,\n    z: i32,\n")]);
    assert_eq!(private.hash, before.hash);
    assert_eq!(private.fields_hash, before.fields_hash);
    assert!(before.contract.constructible && !private.contract.constructible);
}

#[test]
fn a_listed_boundary_trait_an_older_tree_lacks_is_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for (path, text) in FILES {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let catalog = Catalog::parse(&[("hooks/ping.rs".into(), HOOKS.into())]).unwrap();
    let mut index = crate::index::game(root, &[]);
    let mut listed = boundary();
    listed.roles.push(("FluidsHost".into(), Role::Call));
    let export = build(root, &mut index, &catalog.hooks, &listed).unwrap();
    assert!(
        export.notes.iter().any(|note| note
            == "boundary trait FluidsHost: not found in mod_api, its role is not recorded"),
        "{:?}",
        export.notes
    );
}

#[test]
fn a_hidden_type_named_only_by_an_impl_gets_its_record() {
    let secret = |edits: &[(&str, &str, &str)]| {
        export_fixture(edits, HOOKS)
            .surface
            .types
            .iter()
            .find(|t| t.id.ends_with("Secret"))
            .map(|t| t.id.clone())
    };
    assert_eq!(secret(&[]), None);
    let from = "impl From<internal::Secret> for Point {\n    fn from(_: internal::Secret) -> Self {\n        Self { x: 0, y: 0 }\n    }\n}\n/// A colour.";
    assert_eq!(
        secret(&[(LIB, "/// A colour.", from)]).as_deref(),
        Some("type:internal::Secret")
    );
}
#[test]
fn a_part_hook_is_listed_in_its_part_record() {
    let hooks = HOOKS.replacen(
        "neoforge = \"PingEvent\"",
        "neoforge = \"PingEvent\",\n        part = \"item\"",
        1,
    );
    let export = export_fixture(
        &[(
            LIB,
            "pub trait BlockPart {}",
            "pub trait BlockPart {}\n/// An item's part.\npub trait ItemPart {}",
        )],
        &hooks,
    );
    let part = export
        .surface
        .parts
        .iter()
        .find(|part| part.id == "part:item")
        .expect("item part record");
    assert_eq!(part.hooks, vec!["hook:ping".to_owned()]);
}
