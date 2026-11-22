#!/usr/bin/env python3
"""Apply p2-71's item and animation fixture scoping to the reviewed sources."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ITEM = "crates/sunrise-client/src/item_render"
ANIMATIONS = "crates/sunrise-client-render/src/mod_animations.rs"


def require_count(source, block, expected, path):
    found = source.count(block)
    if found != expected:
        raise SystemExit(f"{path}: expected {expected} match(es) for {block!r}, found {found}")


def replace(source, old, new, path, count=1):
    require_count(source, old, count, path)
    return source.replace(old, new)


def replace_span(source, first, after, lines, new, path):
    require_count(source, first, 1, path)
    require_count(source, after, 1, path)
    start = source.index(first)
    end = source.index(after)
    if end <= start or source.count("\n", start, end) != lines:
        raise SystemExit(f"{path}: unexpected block between {first!r} and {after!r}")
    return source[:start] + new + source[end:]


def item_providers(source, path):
    return replace_span(
        source,
        "#[cfg(test)]\n#[derive(Default)]\npub(super) struct ProvidersFixtureState {\n",
        "fn is_builtin_tint_source(kind: &str) -> bool {\n",
        113,
        """impl<'a> ItemProviderTables<'a> {
    pub(super) fn installed(special: Option<&'a SpecialModelFactories>) -> Self {
        Self {
            special,
            tint: TINT_FACTORIES.get().map(Arc::as_ref),
            item_model: ITEM_MODEL_FACTORIES.get().map(Arc::as_ref),
            range_property: RANGE_PROPERTY_FACTORIES.get().map(Arc::as_ref),
            select_property: SELECT_PROPERTY_FACTORIES.get().map(Arc::as_ref),
        }
    }
}

""",
        path,
    )


def item_renderer(source, path):
    require_count(source, "Self::load_tables(", 3, path)
    for old, new in (
        ("use registered_providers::ActiveItemProviders;\n", ""),
        (
            "        Self::load_tables(root, library, true, item_data, None, FACTORIES.get())\n",
            """        Self::load_tables(
            root,
            library,
            true,
            item_data,
            None,
            ItemProviderTables::installed(FACTORIES.get()),
        )
""",
        ),
        (
            "        Self::load_tables(root, library, false, item_data, None, None)\n",
            """        Self::load_tables(
            root,
            library,
            false,
            item_data,
            None,
            ItemProviderTables::default(),
        )
""",
        ),
        (
            "            Some(document),\n            FACTORIES.get(),\n",
            "            Some(document),\n            ItemProviderTables::installed(FACTORIES.get()),\n",
        ),
        (
            "        factories: Option<&SpecialModelFactories>,\n",
            "        providers: ItemProviderTables<'_>,\n",
        ),
        (
            "            let active = ActiveItemProviders::current();\n"
            "            let providers = active.tables(factories);\n",
            "",
        ),
        (
            "            let (_fixture, kept) =\n"
            "                registered_providers::ItemProvidersFixture::item_models(registered);\n",
            """            let (factories, kept) = bind_item_model_factories(registered);
            let providers = ItemProviderTables {
                item_model: Some(&factories),
                ..Default::default()
            };
""",
        ),
        (
            "            let models = |full: &str| {\n"
            "                let active = ActiveItemProviders::current();\n"
            "                let providers = active.tables(None);\n",
            "            let models = |full: &str| {\n",
        ),
    ):
        source = replace(source, old, new, path)
    return source


def special_tests(source, path):
    require_count(source, "ItemRenderer::load_tables(", 2, path)
    for value in ("factories", "&factories"):
        source = replace(
            source,
            f"        None,\n        Some({value}),\n    )\n    .unwrap()",
            f"""        None,
        ItemProviderTables {{
            special: Some({value}),
            ..Default::default()
        }},
    )
    .unwrap()""",
            path,
        )
    return source


def tint_tests(source, path):
    for old, new in (
        (
            "fn renderer(root: &std::path::Path) -> ItemRenderer {\n",
            "fn renderer(root: &std::path::Path, providers: ItemProviderTables<'_>) -> ItemRenderer {\n",
        ),
        (
            "    ItemRenderer::load(&ResourceRoot::directory(root.to_path_buf()), &data).unwrap()\n",
            """    let root = ResourceRoot::directory(root.to_path_buf());
    ItemRenderer::load_tables(
        &root,
        crate::model_library::ParsedModels::load(root.source()).into_item_library(),
        true,
        &data,
        None,
        providers,
    )
    .unwrap()
""",
        ),
        (
            "        let (_fixture, kept) = registered_providers::ItemProvidersFixture::tint(sources);\n"
            "        assert_eq!(kept, [\"tintdraw:charge\"]);\n"
            "        let renderer = renderer(&root);\n",
            """        let (factories, kept) = bind_item_tint_factories(sources);
        assert_eq!(kept, ["tintdraw:charge"]);
        let renderer = renderer(
            &root,
            ItemProviderTables {
                tint: Some(&factories),
                ..Default::default()
            },
        );
""",
        ),
    ):
        source = replace(source, old, new, path)
    return source


def animations(source, path):
    for first, after, lines in (
        ("#[cfg(test)]\nstatic TYPES_FIXTURE:", "fn freeze(", 6),
        ("fn active_types()", "/// The folder under each namespace", 12),
        ("    struct TypesFixture;\n", "    const SWING: &str", 28),
    ):
        source = replace_span(source, first, after, lines, "", path)
    for old, new, count in (
        (
            "    pub fn load(source: &crate::resource_source::ResourceSource) -> Self {\n",
            "    pub fn load(source: &crate::resource_source::ResourceSource) -> Self {\n"
            "        let types = TYPES.get().map(Arc::as_ref);\n",
            1,
        ),
        (
            ".and_then(|text| parse(id.clone(), &text))\n",
            ".and_then(|text| parse_with_types(id.clone(), &text, types))\n",
            1,
        ),
        (
            "pub fn parse(animation: Identifier, text: &str) -> Result<AnimationHolder, String> {\n",
            """pub fn parse(animation: Identifier, text: &str) -> Result<AnimationHolder, String> {
    parse_with_types(animation, text, TYPES.get().map(Arc::as_ref))
}

fn parse_with_types(
    animation: Identifier,
    text: &str,
    types: Option<&BoundAnimations>,
) -> Result<AnimationHolder, String> {
""",
            1,
        ),
        ("    let active = active_types();\n", "", 1),
        ("active.as_ref().and_then(|t| t.targets.get(id))", "types.and_then(|t| t.targets.get(id))", 1),
        ("keyframe_of(keyframe, &target, active.as_deref())", "keyframe_of(keyframe, &target, types)", 1),
        ("    active: Option<&BoundAnimations>,", "    types: Option<&BoundAnimations>,", 1),
        ("active.and_then(|t| t.interpolations.get(id))", "types.and_then(|t| t.interpolations.get(id))", 1),
        (
            "            let _fixture = TypesFixture::from_registrations(types);\n",
            """            let bound = freeze(types);
            let read = |text: &str| {
                parse_with_types(
                    Identifier::parse("test:animation").unwrap(),
                    text,
                    Some(&bound),
                )
            };
""",
            2,
        ),
    ):
        source = replace(source, old, new, path, count)
    return source


def main():
    changes = {}
    for relative, transform in (
        (f"{ITEM}/registered_providers.rs", item_providers),
        (f"{ITEM}.rs", item_renderer),
        (f"{ITEM}/special_parts_tests.rs", special_tests),
        (f"{ITEM}/tint_tests.rs", tint_tests),
        (ANIMATIONS, animations),
    ):
        path = ROOT / relative
        changes[path] = transform(path.read_text(), relative)
    for path, source in changes.items():
        path.write_text(source)


if __name__ == "__main__":
    main()
