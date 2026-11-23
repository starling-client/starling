//! What the catalog decides by itself: no network here.
use serde_json::json;
use starling_catalog::browser::{Kind, Project, SourceId};
use starling_catalog::{caution, installed_projects, is_native_mod, is_shader_pack, shader_hits};
use std::io::Write;
use std::path::{Path, PathBuf};

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("starling-catalog-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn zip_with(path: &PathBuf, names: &[&str]) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for name in names {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"x").unwrap();
    }
    zip.finish().unwrap();
}

fn project(kind: Kind, source: SourceId, game_versions: &[&str], loaders: &[&str]) -> Project {
    Project {
        source,
        id: "id".into(),
        slug: "slug".into(),
        title: "Title".into(),
        summary: String::new(),
        author: String::new(),
        downloads: 0,
        page: String::new(),
        icon: None,
        license: String::new(),
        kind,
        game_versions: game_versions.iter().map(|v| v.to_string()).collect(),
        loaders: loaders.iter().map(|l| l.to_string()).collect(),
    }
}

#[test]
fn a_shader_search_answer_becomes_shader_packs() {
    let found = json!({"hits": [
        {
            "project_id": "abc",
            "slug": "complementary",
            "title": "Complementary",
            "description": "Shaders",
            "author": "EminGT",
            "downloads": 12,
            "icon_url": "",
            "versions": ["1.21"],
            "categories": ["iris", "optifine"]
        },
        {"slug": "no-id"}
    ]});
    let hits = shader_hits(&found);
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit.kind, Kind::Shader);
    assert_eq!(hit.source, SourceId::Modrinth);
    assert_eq!(hit.id, "abc");
    assert_eq!(hit.page, "https://modrinth.com/shader/complementary");
    assert_eq!(hit.icon, None);
    assert_eq!(hit.game_versions, ["1.21"]);
}

#[test]
fn an_answer_with_no_hits_is_empty() {
    assert!(shader_hits(&json!({})).is_empty());
}

#[test]
fn a_modrinth_shader_pack_carries_no_caution() {
    let pack = project(Kind::Shader, SourceId::Modrinth, &[], &["iris"]);
    assert_eq!(caution(&pack), None);
}

#[test]
fn an_older_resource_pack_says_so_and_a_current_one_does_not() {
    let old = project(Kind::ResourcePack, SourceId::Modrinth, &["1.20.1"], &[]);
    assert!(caution(&old).is_some_and(|why| why.contains("1.20.1")));
    let current = project(
        Kind::ResourcePack,
        SourceId::Modrinth,
        &[starling_catalog::browser::GAME],
        &[],
    );
    assert_eq!(caution(&current), None);
}

#[test]
fn a_modrinth_project_is_never_a_native_mod() {
    assert!(!is_native_mod(&project(
        Kind::Mod,
        SourceId::Modrinth,
        &[],
        &[]
    )));
}

#[test]
fn a_shader_pack_has_a_shaders_folder_at_its_top_or_in_its_one_folder() {
    let dir = folder("shader-zip");
    let zip = dir.join("pack.zip");
    zip_with(&zip, &["shaders/final.fsh"]);
    assert_eq!(is_shader_pack(&zip), Ok(true));
    zip_with(&zip, &["BSL/shaders/final.fsh"]);
    assert_eq!(is_shader_pack(&zip), Ok(true));
    zip_with(&zip, &["assets/minecraft/shaders/core.json"]);
    assert_eq!(is_shader_pack(&zip), Ok(false));
    zip_with(&zip, &["pack.mcmeta"]);
    assert_eq!(is_shader_pack(&zip), Ok(false));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn nothing_installed_is_an_empty_list() {
    let dir = folder("empty");
    assert!(installed_projects(&dir).is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

fn listed(home: &Path, file: &str) {
    use starling_catalog::browser::installed::{Entry, Installed};
    let mut list = Installed::load(home);
    list.entries.push(Entry {
        source: SourceId::Modrinth,
        project: file.into(),
        slug: file.into(),
        title: file.into(),
        kind: Kind::ResourcePack,
        version: "v".into(),
        number: "1".into(),
        file: format!("resourcepacks/{file}.zip"),
        checksum: "SHA-512:00".into(),
        page: String::new(),
        dependency: false,
        verdict: None,
        skipped: Vec::new(),
        namespaces: Vec::new(),
    });
    list.save(home).unwrap();
}

#[test]
fn installed_means_the_file_is_there_not_that_it_was_once() {
    let home = folder("installed-honest");
    std::fs::create_dir_all(home.join("resourcepacks")).unwrap();
    listed(&home, "kept");
    listed(&home, "gone");
    std::fs::write(home.join("resourcepacks/kept.zip"), b"x").unwrap();
    let now = installed_projects(&home);
    assert!(now.contains("kept") && !now.contains("gone"));
    // Deleted by hand later: not installed any more.
    std::fs::remove_file(home.join("resourcepacks/kept.zip")).unwrap();
    assert!(installed_projects(&home).is_empty());
}

#[test]
fn forgetting_the_missing_leaves_the_ones_that_are_there() {
    let home = folder("forget");
    std::fs::create_dir_all(home.join("resourcepacks")).unwrap();
    listed(&home, "kept");
    listed(&home, "gone");
    std::fs::write(home.join("resourcepacks/kept.zip"), b"x").unwrap();
    starling_catalog::forget_missing(&home);
    let list = starling_catalog::browser::installed::Installed::load(&home);
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].project, "kept");
}

#[test]
fn removing_deletes_the_file_and_the_row_goes_back_to_installable() {
    let home = folder("remove");
    std::fs::create_dir_all(home.join("resourcepacks")).unwrap();
    listed(&home, "pack");
    std::fs::write(home.join("resourcepacks/pack.zip"), b"x").unwrap();
    let mut project = project(Kind::ResourcePack, SourceId::Modrinth, &[], &[]);
    project.id = "pack".into();
    let entry = starling_catalog::remove(&home, &project).unwrap();
    assert_eq!(entry.file, "resourcepacks/pack.zip");
    assert!(!home.join("resourcepacks/pack.zip").exists());
    assert!(installed_projects(&home).is_empty());
}

#[test]
fn a_registry_mod_already_there_under_another_name_is_installed() {
    let home = folder("present");
    std::fs::create_dir_all(home.join("sunrise-mods")).unwrap();
    // The launcher's catalog names a download `<id>-<version>.smod`; this one's install was
    // `sunrise-mods/<id>.smod` beside it, the same mod twice (2026-11-20).
    let smod = home.join("sunrise-mods/betterwitchhuts-0.2.0.smod");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&smod).unwrap());
    zip.start_file("sunrise-mod.toml", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"id = \"betterwitchhuts\"\nname = \"YUNG's Better Witch Huts\"\n\n[native]\nid = \"other\"\n")
        .unwrap();
    zip.finish().unwrap();
    assert!(installed_projects(&home).contains("betterwitchhuts"));
    let mut project = project(Kind::Mod, SourceId::Sunrise, &[], &[]);
    project.id = "betterwitchhuts".into();
    let refused = starling_catalog::install(&project, false, &home, &|_, _| {}).unwrap_err();
    assert!(refused.contains("already in the game folder"), "{refused}");
    let entry = starling_catalog::remove(&home, &project).unwrap();
    assert_eq!(entry.file, "sunrise-mods/betterwitchhuts-0.2.0.smod");
    assert_eq!(
        starling_catalog::mod_id(&entry).as_deref(),
        Some("betterwitchhuts")
    );
    assert!(!smod.exists());
    assert!(installed_projects(&home).is_empty());
    std::fs::remove_dir_all(home).unwrap();
}
