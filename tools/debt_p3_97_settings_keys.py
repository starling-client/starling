#!/usr/bin/env python3
"""Replay the P3-97 settings callers' migration from raw strings to ResourceId."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src"
updates = {}


def rewrite(path, *replacements):
    source = (ROOT / path).read_text(encoding="utf-8")
    for old, new, count in replacements:
        if source.count(old) != count:
            raise SystemExit(f"{path}: expected {count} occurrences of {old!r}")
        source = source.replace(old, new)
    updates[path] = source


rewrite(
    "game/modding/native_book_net.rs",
    ("    settings_codec,", "    placement_sync::ResourceId, settings_codec,", 1),
    ("fn registered() -> BTreeSet<String>", "fn registered() -> BTreeSet<ResourceId>", 1),
    (".map(|key| key.as_str().to_owned())", ".map(|key| ResourceId::from_registered_book(&key))", 1),
    ("key: book.as_str().into(),", "key: ResourceId::from_registered_book(book),", 1),
    ("key.as_str() == id)", "key.as_str() == id.as_str())", 1),
)
rewrite(
    "game/native_hooks/shared/book_screen/lifecycle_tests.rs",
    ("{BookFlags, settings_codec, settings_sync::Message}",
     "{BookFlags, placement_sync::ResourceId, settings_codec, settings_sync::Message}", 1),
    ("key.as_str().into(),", "ResourceId::from_registered_book(&key),", 6),
)
rewrite(
    "game/modding/native_book_apply/tests.rs",
    ('    connection.authority = Some(Authority::negotiate(\n        1,\n        vec!["review:book".into()],\n        &["review:book".into()].into_iter().collect(),\n    ));',
     '''    let book = selection.catalog.book.clone();
    connection.authority = Some(Authority::negotiate(1, vec![book.clone()], &[book].into()));''', 1),
)
rewrite(
    "lan/hub.rs",
    ('g.recipe_book.custom.set("test:cooking", flags).unwrap();',
     '''let book = crate::recipe_book::placement_sync::ResourceId::try_from("test:cooking".to_owned()).unwrap();
            g.recipe_book.custom.set(&book, flags);''', 1),
)
rewrite(
    "recipe_book/settings_connection.rs",
    ('    BookFlags, CustomBooks, settings_codec,',
     '    BookFlags, CustomBooks, placement_sync::ResourceId, settings_codec,', 1),
    ('Option<(String, BookFlags)>', 'Option<(ResourceId, BookFlags)>', 1),
    ('-> BTreeSet<String>', '-> BTreeSet<ResourceId>', 1),
    ('let registered: BTreeSet<String> = ["demo:pot".to_owned(), "demo:oven".to_owned()].into();',
     '''let pot = ResourceId::try_from("demo:pot".to_owned()).unwrap();
        let oven = ResourceId::try_from("demo:oven".to_owned()).unwrap();
        let registered: BTreeSet<_> = [pot.clone(), oven].into();''', 1),
    ('"demo:pot".into()', 'pot.clone()', 4),
)
rewrite(
    "recipe_book/settings_placement_tests.rs",
    ('placement_sync::{Frame, schema}', 'placement_sync::{Frame, ResourceId, schema}', 1),
    ('    for message in [Message::Hello, Message::Offer(vec!["demo:book".into()])] {',
     '''    let book = ResourceId::try_from("demo:book".to_owned()).unwrap();
    for message in [Message::Hello, Message::Offer(vec![book.clone()])] {''', 1),
    ('|| ["demo:book".into()].into()', '|| [book.clone()].into()', 1),
)
rewrite(
    "recipe_book/settings_sync_tests.rs",
    ('''fn registered() -> BTreeSet<String> {
    ["demo:pot", "demo:oven"].map(str::to_owned).into()
}''', '''fn id(key: &str) -> ResourceId {
    ResourceId::try_from(key.to_owned()).unwrap()
}
fn registered() -> BTreeSet<ResourceId> {
    ["demo:pot", "demo:oven"].map(id).into()
}''', 1),
    ('"demo:pot".into()', 'id("demo:pot")', 12),
    ('"demo:oven".into()', 'id("demo:oven")', 1),
    ('"absent:book".into()', 'id("absent:book")', 1),
    ('"demo:pot", flags(', '&id("demo:pot"), flags(', 7),
    ('"unknown:book", flags(', '&id("unknown:book"), flags(', 1),
    ('                "demo:oven",', '                &id("demo:oven"),', 1),
    ('books.set("absent:book", flags(true, false)).unwrap();',
     'books.set(&id("absent:book"), flags(true, false));', 1),
    ('books.set("demo:oven", flags(true, true)).unwrap();',
     'books.set(&id("demo:oven"), flags(true, true));', 1),
    ('imported.set(&id("demo:pot"), flags(true, false)).unwrap();',
     'imported.set(&id("demo:pot"), flags(true, false));', 1),
)

for path, source in updates.items():
    (ROOT / path).write_text(source, encoding="utf-8")
