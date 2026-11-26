#!/usr/bin/env python3
"""Replace the generated provider enums with a native typed binding, leaving traversal alone.

Run once on the pre-migration tree. Every replacement checks its anchor before
any files are written. Pass --root to replay on another copy of the base tree.
"""

import argparse
from pathlib import Path
import subprocess


def migrate(root: Path) -> None:
    vocab = "crates/sunrise-mod-vocab/src/"
    api = "crates/sunrise-mod-api/src/"
    gen = "crates/sunrise-mod-api-gen/src/"
    tokens = vocab + "parts/installation_tokens/"
    updates: dict[str, str | None] = {}

    def replace(path: str, old: str, new: str, count: int = 1) -> None:
        source = updates[path] if path in updates else (root / path).read_text()
        if source is None or source.count(old) != count:
            raise ValueError(f"{path}: expected {count} occurrence(s) of {old!r}")
        updates[path] = source.replace(old, new)

    binding = "NativeCapabilityProvider"
    capabilities = vocab + "parts/capabilities.rs"
    replace(
        capabilities,
        "/// A capability provider a mod registered: a [`BlockProviderOf`], [`ItemProviderOf`] or\n"
        "/// [`EntityProviderOf`].",
        "/// A capability provider a mod registered, with its dispatch metadata.",
    )
    replace(
        capabilities,
        """/// A capability provider whose handler type is a mod's own (a key a library's `[api]` crate
/// publishes), erased: only mods linked with that crate know its handler type.
// hidden: the registrar's erased provider, not part of the mod surface
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: transfers the handler name and a retained provider resource id, reporting unsupported handlers rather than serializing the native Any object.
#[doc(hidden)]
#[derive(Clone)]
pub struct OwnProvider {
    /// The `Arc` of the provider trait object, which a lookup of that handler type downcasts.
    pub(crate) provider: Arc<dyn Any + Send + Sync>,
    pub(crate) handler: Arc<str>,
}""",
        """/// Opaque native provider bound by the typed capability methods on `dyn Registrar`.
// hidden: the registrar's erased provider, not a mod construction surface
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: it retains an in-process typed callback.
#[doc(hidden)]
#[derive(Clone)]
pub struct NativeCapabilityProvider {
    provider: Arc<dyn Any + Send + Sync>,
    pub(crate) handler: &'static str,
}

impl NativeCapabilityProvider {
    pub(crate) fn of<T: ?Sized + 'static, P: Send + Sync + 'static>(provider: P) -> Self {
        Self {
            provider: Arc::new(provider),
            handler: type_name::<T>(),
        }
    }

    pub(crate) fn get<P: 'static>(&self) -> Option<&P> {
        self.provider.downcast_ref::<P>()
    }
}""",
    )
    replace(
        capabilities,
        """/// A provider by its handler type: one case for each handler type `mod_api`'s keys name
/// (generated from them), and [`OwnProvider`] for a mod's own.
#[allow(missing_docs)]
pub(crate) mod providers_of {
    use super::*;
    // The keys' handler types and contexts, as the generated cases name them.
    #[allow(unused_imports)]
    use crate::*;
    include!(concat!(
        env!("OUT_DIR"),
        "/capability_providers_generated.rs"
    ));
}""",
        'include!(concat!(env!("OUT_DIR"), "/builtin_capability_keys.rs"));',
    )
    replace(
        capabilities,
        "/// providers and the game's); the first to answer wins. `get` takes a provider of this\n"
        "/// capability's handler type out of its case: a provider of a mod's own type the asker isn't\n"
        "/// linked with is passed over.",
        "/// providers and the game's); the first to answer wins. `get` borrows the bound provider\n"
        "/// if it matches the capability's handler type and context.",
    )
    replace(
        capabilities,
        "providers_of::builtin_block_capabilities()",
        "builtin_block_capabilities()",
    )
    replace(
        vocab + "exports/parts_capabilities.rs",
        "pub use parts::{BlockProviderOf, EntityProviderOf, ItemProviderOf, OwnProvider};",
        f"pub use parts::{binding};",
    )

    block_entities = vocab + "parts/block_entities.rs"
    blocks = vocab + "parts/blocks.rs"
    items = vocab + "parts/items.rs"
    entities = vocab + "parts/entities.rs"
    replace(
        block_entities,
        "pub use super::capabilities::providers_of::{BlockProviderOf, EntityProviderOf, ItemProviderOf};\n\n",
        "",
    )
    replace(
        blocks,
        "BlockProviderOf::of(Arc::new(move |host: &mut dyn ServerHost, at: BlockSide| {",
        "let provider: Arc<BlockProvider<T>> = Arc::new(move |host: &mut dyn ServerHost, at: BlockSide| {",
    )
    replace(
        blocks,
        "    }) as Arc<dyn BlockCapabilityProvider<T>>)\n}",
        "    });\n    NativeCapabilityProvider::of::<T, _>(provider)\n}",
    )

    for path, stem, method, params in [
        (block_entities, "Block", "provide_block_entity_capability", "T"),
        (items, "Item", "provide_item_capability", "T"),
        (entities, "Entity", "provide_entity_capability", "T, C"),
    ]:
        replace(
            path,
            f"        self.{method}(",
            f"        let provider: Arc<{stem}Provider<{params}>> = Arc::new(provider);\n"
            f"        self.{method}(",
            count=2,
        )
        replace(
            path,
            f"{stem}ProviderOf::of(Arc::new(provider))",
            f"{binding}::of::<T, _>(provider)",
            count=2,
        )

    for path, prefix, stem, params in [
        (block_entities, "", "Block", "T"),
        (entities, "", "Entity", "T, C"),
        (vocab + "item/capability.rs", "crate::", "Block", "T"),
        (vocab + "block/item_place.rs", "crate::", "Item", "T"),
        (vocab + "entity/entity_capability.rs", "crate::", "Entity", "T, C"),
    ]:
        arc = "std::sync::Arc" if prefix else "Arc"
        replace(
            path,
            f"{prefix}{stem}ProviderOf::get::<{params}>",
            f"{prefix}{binding}::get::<{arc}<{prefix}{stem}Provider<{params}>>>",
        )

    for path, stem, count in [
        (block_entities, "Block", 3),
        (blocks, "Block", 4),
        (items, "Item", 3),
        (entities, "Entity", 4),
        (vocab + "parts/shared.rs", "Block", 2),
        (vocab + "parts/shared.rs", "Item", 1),
        (vocab + "parts/shared.rs", "Entity", 1),
    ]:
        replace(path, f"{stem}ProviderOf", binding, count)

    for path in (capabilities, blocks, items, entities):
        replace(
            path,
            "// hidden: per-kind lookup implementation, exposed for generated provider bindings",
            "// hidden: per-kind lookup implementation for native provider bindings",
        )

    for name, stem in [
        ("block_entities", "Block"),
        ("blocks", "Block"),
        ("items", "Item"),
        ("entities", "Entity"),
    ]:
        path = api + f"parts/{name}.rs"
        replace(path, f"provider: {stem}ProviderOf,", f"provider: {binding},")
        replace(
            path,
            f"::{stem.lower()}_provider_handler(&provider)",
            "::capability_provider_handler(&provider)",
        )

    for stem in ("block", "item", "entity"):
        for suffix in ("", "_handler"):
            path = tokens + f"{stem}_provider{suffix}.rs"
            if f"pub fn {stem}_provider{suffix}" not in (root / path).read_text():
                raise ValueError(f"{path}: expected the old provider installation token")
            updates[path] = None
    updates[tokens + "capability_provider_handler.rs"] = """//! Erased provider metadata retained behind the native binding.
/// Its validated handler name for registration collision checks.
pub fn capability_provider_handler(provider: &crate::NativeCapabilityProvider) -> &'static str {
    provider.handler
}
"""
    replace(
        tokens + "builtin_capabilities.rs",
        "crate::parts::capabilities::providers_of::builtin_block_capabilities()",
        "crate::parts::capabilities::builtin_block_capabilities()",
    )
    replace(gen + "parts/installation.rs", '                                        | "OwnProvider"\n', "")

    old = gen + "capability_providers.rs"
    new = gen + "capability_keys.rs"
    source = (root / old).read_text()
    start = source.index("    /// Its case in the provider enum")
    end = source.index("/// Every name a `use` tree brings in.", start)
    updates[new] = source[:start] + "}\n\n" + source[end:]
    updates[old] = None
    replace(
        new,
        "//! The capability provider enums `mod_api` includes, derived from its capability keys.",
        "//! Provider roles and built-in block capability metadata derived from the public keys.",
    )
    replace(new, "#[derive(Clone, PartialEq)]", "#[derive(PartialEq)]")
    replace(
        new,
        "/// The provider enums, and the source files read to derive them.\npub fn generate(",
        'pub(crate) const OUTPUT: &str = "builtin_capability_keys.rs";\n\n'
        "/// The built-in block capability metadata, and the source files read to derive it.\n"
        "pub(crate) fn generate(",
    )
    replace(
        new,
        """    let mut shapes = Vec::new();
    for (_, shape) in &declarations {
        if !shapes.contains(shape) {
            shapes.push(shape.clone());
        }
    }
    let mut providers = providers_code(&shapes);
""",
        "",
    )
    replace(
        new,
        '''    providers.push_str(&format!(
        "pub(crate) fn builtin_block_capabilities() -> Vec<(&'static str, &'static str)> {{ vec![{block_keys}] }}\\n"
    ));
    Ok((providers, index.read.take()))''',
        '''    let keys = format!(
        "// @generated by sunrise-mod-api-gen from mod_api's capability keys; do not edit.\\n\\
         pub(crate) fn builtin_block_capabilities() -> Vec<(&'static str, &'static str)> {{\\n\\
         #[allow(unused_imports)] use crate::*;\\n\\
         vec![{block_keys}]\\n}}\\n"
    );
    Ok((keys, index.read.take()))''',
    )
    replace(gen + "lib.rs", "mod capability_providers;", "mod capability_keys;")
    replace(
        gen + "build.rs",
        "let (providers, paths) = super::capability_providers::generate(root, &included)?;",
        "let (keys, paths) = super::capability_keys::generate(root, &included)?;",
    )
    replace(
        gen + "build.rs",
        '("capability_providers_generated.rs", providers),',
        "(super::capability_keys::OUTPUT, keys),",
    )
    replace(
        gen + "surface/mod.rs",
        "crate::capability_providers::key_shapes(index)?",
        "crate::capability_keys::key_shapes(index)?",
    )
    replace(
        gen + "surface/mod.rs",
        '''    let providers = crate::capability_providers::generate(root, &included)?.0;
    let mut with_providers = included.to_vec();
    with_providers.push(("capability_providers_generated.rs", providers.as_str()));
    let mut index = crate::index::game(root, &with_providers);''',
        """    let (keys, _) = crate::capability_keys::generate(root, &included)?;
    let mut with_keys = included.to_vec();
    with_keys.push((crate::capability_keys::OUTPUT, keys.as_str()));
    let mut index = crate::index::game(root, &with_keys);""",
    )
    replace(
        "tools/check_mod_vocabulary_state.py",
        "'capability_providers_generated.rs'",
        "'builtin_capability_keys.rs'",
    )

    doc = "docs/modding/src/concepts/capabilities.md"
    replace(
        doc,
        "This registration corresponds to NeoForge's `RegisterCapabilitiesEvent`.",
        "This registration corresponds to NeoForge's `RegisterCapabilitiesEvent`. Use the typed\n"
        "methods on `dyn Registrar` for built-in and [mod-defined capabilities](capabilities/own-capability.md);\n"
        "both take native callbacks typed by their keys.",
    )

    note = "crates/sunrise-mod-vocab/changes/debt-p3-31-register-open-native-capability-provider.toml"
    updates[note] = '''ids = [
    "type:BlockProviderOf",
    "type:ItemProviderOf",
    "type:EntityProviderOf",
    "type:OwnProvider",
    "type:NativeCapabilityProvider",
    "method:BlocksRegistrar::provide_block_capability",
    "method:BlockEntitiesRegistrar::provide_block_entity_capability",
    "method:ItemsRegistrar::provide_item_capability",
    "method:EntitiesRegistrar::provide_entity_capability",
]
why = """
Built-in and mod-defined capability keys now both bind typed native callbacks instead of carrying
separate provider enum cases inherited from the guest runtime. This follows NeoForge's typed
RegisterCapabilitiesEvent and BlockCapability.getCapability, ItemCapability.getCapability and
EntityCapability.getCapability. Lookup precedence and the vanilla container path
(HopperBlockEntity.getBlockContainer) are unchanged.
"""
migrate = """
Do not construct or match BlockProviderOf, ItemProviderOf, EntityProviderOf or OwnProvider.
Replace direct calls with the typed registrar helpers:
- provide_block_capability -> r.block_capability(&key, &block, callback)
- provide_block_entity_capability -> r.block_entity_capability(&key, &kind, callback)
- provide_item_capability -> r.item_capability(&key, item_id, callback)
- provide_entity_capability -> r.entity_capability(&key, kind_id, callback)
Use the matching _before_game helper when the old before_game argument was true. Pass the typed
key instead of key.name. Existing calls to these typed helpers are unchanged. Mock or forwarding
Registrar implementations replace the provider argument type of each raw provide_*_capability
method with NativeCapabilityProvider and may pass that opaque value through unchanged.
"""
'''

    for path in (tokens + "capability_provider_handler.rs", new, note):
        if (root / path).exists():
            raise ValueError(f"{path}: new file already exists")

    for path, source in updates.items():
        dest = root / path
        if source is None:
            dest.unlink()
        else:
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(source)
    subprocess.run(
        ["rustfmt", "--edition", "2024", "--config", "skip_children=true"]
        + [
            path
            for path, source in sorted(updates.items())
            if path.endswith(".rs") and source is not None
        ],
        cwd=root,
        check=True,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    migrate(parser.parse_args().root)
