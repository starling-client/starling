#!/usr/bin/env python3
"""Replay the P3-97 recipe-book identifier grammar rewrite on its pre-change sources."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
API = Path("crates/sunrise-mod-api/src/parts")
BOOK = Path("crates/sunrise-client/src/recipe_book")


def rewrite(path: Path, *replacements: tuple[str, str]) -> str:
    source = (ROOT / path).read_text(encoding="utf-8")
    for old, new in replacements:
        if source.count(old) != 1:
            raise SystemExit(f"{path}: expected exactly one occurrence of {old!r}")
        source = source.replace(old, new, 1)
    return source


def main() -> None:
    # Check every anchor before changing any files.
    changed = {
        API / "recipe_books.rs": rewrite(
            API / "recipe_books.rs",
            (
                "use super::*;\n\nimpl RecipeBooksRegistrar",
                "use super::*;\n\nconst MAX_ID_BYTES: usize = 256;\n\nimpl RecipeBooksRegistrar",
            ),
            (
                '''                resource_id(sprite).map_err(|reason| format!("sprite {sprite:?}: {reason}"))?;
''',
                '''                if sprite.len() > MAX_ID_BYTES {
                    return Err(format!(
                        "sprite {sprite:?}: id exceeds {MAX_ID_BYTES} bytes"
                    ));
                }
                if crate::Identifier::parse_namespaced(sprite).is_none() {
                    return Err(format!(
                        "sprite {sprite:?}: expected a namespaced identifier"
                    ));
                }
''',
            ),
            (
                '''/// Split a lowercase `namespace:path` id of at most 256 bytes.
fn resource_id(id: &str) -> Result<(&str, &str), String> {
    let (namespace, path) = id.split_once(':').ok_or("explicit namespace required")?;
    if id.len() > 256 {
        return Err("id exceeds 256 bytes".into());
    }
    if namespace.is_empty()
        || path.is_empty()
        || !namespace
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
        || !path
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-/".contains(&b))
    {
        return Err("nonempty lowercase resource-id namespace and path required".into());
    }
    Ok((namespace, path))
}

''',
                "",
            ),
            (
                "            let (namespace, _) = resource_id(id)?;\n",
                '''            if id.len() > MAX_ID_BYTES {
                return Err(format!("id exceeds {MAX_ID_BYTES} bytes"));
            }
            let identifier = crate::Identifier::parse_namespaced(id)
                .ok_or("expected a namespaced identifier")?;
            let namespace = identifier.namespace();
''',
            ),
            (
                '''        let key = registrar.recipe_book_type("demo:cooking_pot").unwrap();
        assert_eq!(key.as_str(), "demo:cooking_pot");
        assert!(
            registrar
                .recipe_book_type("demo:cooking_pot")
''',
                '''        let key = registrar
            .recipe_book_type("demo:kitchen/cooking_pot")
            .unwrap();
        assert_eq!(key.as_str(), "demo:kitchen/cooking_pot");
        assert!(
            registrar
                .recipe_book_type("demo:kitchen/cooking_pot")
''',
            ),
            ('            "demo:Pot",\n', '            "demo:Pot",\n            "..:pot",\n'),
            (
                "        assert_eq!(registrar.refused.len(), 9);\n",
                "        assert_eq!(registrar.refused.len(), 10);\n",
            ),
            (
                '''            crate::RecipeBookFilterLook {
                disabled_highlighted: "demo:Upper".into(),
''',
                '''            crate::RecipeBookFilterLook {
                enabled: "..:pot".into(),
                ..look()
            },
            crate::RecipeBookFilterLook {
                disabled_highlighted: "demo:Upper".into(),
''',
            ),
            (
                '        assert_eq!(r.refused.len(), 6, "each refusal also refuses installation");\n',
                '        assert_eq!(r.refused.len(), 7, "each refusal also refuses installation");\n',
            ),
        ),
        API / "menus.rs": rewrite(
            API / "menus.rs",
            (
                '''            let menu_id =
                crate::Identifier::parse(menu).map_err(|e| format!("invalid menu: {e:?}"))?;
            if !menu.contains(':') || !owns_namespace(self.mod_id, menu_id.namespace()) {
''',
                '''            let menu_id = crate::Identifier::parse_namespaced(menu)
                .ok_or("menu must be a namespaced identifier")?;
            if !owns_namespace(self.mod_id, menu_id.namespace()) {
''',
            ),
            (
                '''            crate::Identifier::parse(recipe_type)
                .map_err(|e| format!("invalid recipe type: {e:?}"))?;
            if !recipe_type.contains(':') {
                return Err("explicit recipe type namespace required".into());
            }
''',
                '''            crate::Identifier::parse_namespaced(recipe_type)
                .ok_or("recipe type must be a namespaced identifier")?;
''',
            ),
        ),
        API / "menus/book_tests.rs": rewrite(
            API / "menus/book_tests.rs",
            (
                '''    assert_eq!(registrar.menu_recipe_books.len(), 1);
    assert_eq!(registrar.refused.len(), 4);
''',
                '''    for id in ["pot", ":pot", "demo:", "..:pot"] {
        assert!(
            registrar
                .menu_recipe_book(id, &book, "demo:cooking", roles())
                .is_err(),
            "{id:?}"
        );
        assert!(
            registrar
                .menu_recipe_book("demo:other", &book, id, roles())
                .is_err(),
            "{id:?}"
        );
    }
    assert_eq!(registrar.menu_recipe_books.len(), 1);
    assert_eq!(registrar.refused.len(), 12);
''',
            ),
        ),
        BOOK / "custom_books.rs": rewrite(
            BOOK / "custom_books.rs",
            (
                '''/// Whether `id` is a namespaced resource id a book can have: `namespace:path`, both nonempty,
/// the namespace of `[a-z0-9_.-]` and the path of `[a-z0-9_.-/]`, at most [`MAX_ID_BYTES`] bytes.
pub fn valid_book_id(id: &str) -> bool {
    id.len() <= MAX_ID_BYTES && super::resource_id::valid(id)
}
''',
                '''/// Whether `id` is an explicit `namespace:path` accepted by
/// [`mod_api::Identifier::parse_namespaced`] and at most [`MAX_ID_BYTES`] bytes.
pub fn valid_book_id(id: &str) -> bool {
    id.len() <= MAX_ID_BYTES && mod_api::Identifier::parse_namespaced(id).is_some()
}
''',
            ),
        ),
        BOOK / "custom_books_tests.rs": rewrite(
            BOOK / "custom_books_tests.rs",
            ('        "de/mo:pot",\n', '        "de/mo:pot",\n        "..:pot",\n'),
        ),
        BOOK / "placement_sync.rs": rewrite(
            BOOK / "placement_sync.rs",
            (
                "        if value.len() <= RESOURCE_ID_BYTES && super::resource_id::valid(&value) {\n",
                '''        if value.len() <= RESOURCE_ID_BYTES
            && mod_api::Identifier::parse_namespaced(&value).is_some()
        {
''',
            ),
        ),
        BOOK / "placement_sync_tests.rs": rewrite(
            BOOK / "placement_sync_tests.rs",
            (
                '    assert!(ResourceId::try_from("demo:bad path".to_owned()).is_err());\n',
                '''    let slash_path = ResourceId::try_from("demo:kitchen/pot".to_owned()).unwrap();
    assert_eq!(slash_path.as_str(), "demo:kitchen/pot");
    for id in ["..:pot", "pot", ":pot", "demo:", "demo:bad path"] {
        assert!(ResourceId::try_from(id.to_owned()).is_err(), "{id:?}");
    }
''',
            ),
        ),
        BOOK.with_suffix(".rs"): rewrite(
            BOOK.with_suffix(".rs"),
            ("pub mod placement_sync;\nmod resource_id;\n", "pub mod placement_sync;\n"),
        ),
    }
    obsolete = ROOT / BOOK / "resource_id.rs"
    if obsolete.read_text(encoding="utf-8") != '''//! Resource-location syntax shared by book keys and placement identities.
pub(super) fn valid(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    let allowed = |byte: u8, slash| {
        matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-') || slash && byte == b'/'
    };
    !namespace.is_empty()
        && !path.is_empty()
        && namespace.bytes().all(|byte| allowed(byte, false))
        && path.bytes().all(|byte| allowed(byte, true))
}
''':
        raise SystemExit(f"{obsolete}: unexpected source; not deleting it")
    for path, source in changed.items():
        (ROOT / path).write_text(source, encoding="utf-8")
    obsolete.unlink()


if __name__ == "__main__":
    main()
