#!/usr/bin/env python3
"""Remove the package-reader hint for native items; references resolve at game load."""

from pathlib import Path


def replace(path, pairs):
    path = Path(path)
    source = path.read_text()
    for before, after in pairs:
        assert source.count(before) == 1, (path, before)
        source = source.replace(before, after)
    path.write_text(source)


replace("crates/sunrise-mod-package/src/content.rs", [
    ('''        Self::read_declaring(path, manifest, &[])
    }
    /// [`Content::read`] for a mod whose native code also declares items (`native_items`):
    /// a file's `crafting_remainder` may name one of those `declared` ids, which are not in the
    /// files.
    pub fn read_declaring(
        path: &Path,
        manifest: &Manifest,
        declared: &[String],
    ) -> Result<Content, String> {\n''', ''),
    ("content.check(manifest, declared)?;", "content.check(manifest)?;"),
    ("fn check(&mut self, manifest: &Manifest, declared: &[String])", "fn check(&mut self, manifest: &Manifest)"),
    ("        let mut remainders = Vec::new();\n", ""),
    ('''            if let Some(rest) = &item.crafting_remainder {
                remainders.push((name, rest.clone()));
            }
''', ''),
    ('''        // After every item, so a remainder may be declared in a later file.
        for (name, rest) in remainders {
            if rest == "minecraft:air" {
                return Err(format!(
                    "{name}: crafting_remainder {rest:?} is no remainder (leave it out)"
                ));
            }
            if !vanilla_id(&rest) && !ids.contains(&rest) && !declared.contains(&rest) {
                return Err(format!(
                    "{name}: crafting_remainder {rest:?} is not a vanilla item (minecraft:...) or one of this mod's"
                ));
            }
        }
''', ''),
    ('''    #[test]
    fn a_file_item_may_leave_a_code_declared_item_as_its_remainder() {
        // `native_items` declares ruby:wrench in code; a file item names it as its remainder.
        let files: &[(&str, &[u8])] = &[(
            "data/ruby/sunrise/item/tool.json",
            br#"{"template": "minecraft:iron_pickaxe", "crafting_remainder": "ruby:wrench"}"#,
        )];
        let read = |declared: &[String]| {
            let dir = scratch("declared-remainder");
            let path = dir.join("m.smod");
            std::fs::write(&path, Package::write_files(MANIFEST_TEXT, files).unwrap()).unwrap();
            let package = Package::read(&path).unwrap();
            let content = Content::read_declaring(&path, &package.manifest, declared);
            std::fs::remove_dir_all(&dir).unwrap();
            content
        };
        assert!(read(&[]).is_err(), "a remainder nobody declares is refused");
        let content = read(&["ruby:wrench".to_owned()]).expect("the code-declared item is known");
        assert_eq!(
            content.items[0].crafting_remainder.as_deref(),
            Some("ruby:wrench")
        );
    }

''', ''),
])

replace("crates/sunrise-client/src/game/modding/mods/mod.rs", [
    ("requests: &[(&Path, &sunrise_mod_package::Manifest, Vec<String>)],", "requests: &[(&Path, &sunrise_mod_package::Manifest)],"),
    ("let Some((path, manifest, declared)) = requests.get(index) else {", "let Some((path, manifest)) = requests.get(index) else {"),
    ('''                        let content =
                            sunrise_mod_package::Content::read_declaring(path, manifest, declared);
                        if declared.is_empty()
                            && let Ok(content) = &content
                        {''', '''                        let content = sunrise_mod_package::Content::read(path, manifest);
                        if let Ok(content) = &content {'''),
    ('''            let item_ids =
                super::native_content_items::owned_item_ids(source.manifest, &declared.batches);
            (source.path, source.manifest, item_ids)''', '''            (source.path, source.manifest)'''),
    (".map(|(path, package)| (path.as_path(), &package.manifest, Vec::new()))", ".map(|(path, package)| (path.as_path(), &package.manifest))"),
])

replace("crates/sunrise-client/src/game.rs", [
    ('#[path = "game/modding/native_content_items.rs"]\nmod native_content_items;\n', ''),
])
Path("crates/sunrise-client/src/game/modding/native_content_items.rs").unlink()
