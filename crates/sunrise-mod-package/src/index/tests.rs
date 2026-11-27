//! The sample index (`tests/index/index-v1.json`, the one docs/modding/src/reference/registry-index.md
//! shows) and its files: `coords_hud-0.1.0.smod`, signed with the throwaway
//! key `test.pub` (Ed25519/Blake2b via OpenSSL; its private key was deleted),
//! `cozy-1.0.0.spack`, unsigned, and `glow-1.0.0.sshader`, unsigned
//! (`sunrise-shader new --template post`, then `sunrise-shader pack`).
use super::*;
use std::path::PathBuf;

const OTHER_KEY: &str = "RWSe8ZD+cTOAqNd/jnXTmDtoKp0c0HxkGMrBBWcWQeEkPmcHB+XkaE9S";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/index")
}

fn sample() -> Index {
    Index::parse(&std::fs::read_to_string(fixtures().join("index-v1.json")).unwrap()).unwrap()
}

fn check(index: &Index, id: &str, file: &Path) -> Result<Trust, String> {
    let p = index.project(id).unwrap();
    check_file(index, p, &p.versions[0], file)
}

#[test]
fn reads_the_sample_and_keeps_unknown_kinds() {
    let index = sample();
    assert_eq!(index.schema, SCHEMA);
    let kinds: Vec<&str> = index.projects.iter().map(|p| p.kind.name()).collect();
    assert_eq!(kinds, ["mod", "modpack", "shader", "world"]);
    assert_eq!(index.projects[3].kind, Kind::Other("world".into()));
    let hud = index.project("coords_hud").unwrap();
    assert_eq!(
        hud.versions[0].capabilities.known(),
        [Capability::Registries]
    );
    // What CI writes reads back the same.
    let text = serde_json::to_string(&index).unwrap();
    let again = Index::parse(&text).unwrap();
    assert_eq!(again, index);
    // Download counts: the publish job adds them; none is left out.
    assert_eq!(hud.downloads, 0);
    assert!(!text.contains("downloads"));
    let counted = text.replacen("\"versions\"", "\"downloads\":12345,\"versions\"", 1);
    assert_eq!(Index::parse(&counted).unwrap().projects[0].downloads, 12345);
    // GitHub stars: the same, the source repository's.
    assert_eq!(hud.stars, 0);
    assert!(!text.contains("\"stars\""));
    let starred = text.replacen("\"versions\"", "\"stars\":42,\"versions\"", 1);
    assert_eq!(Index::parse(&starred).unwrap().projects[0].stars, 42);
}

#[test]
fn a_capability_this_build_does_not_know_blocks_only_its_version() {
    let text = std::fs::read_to_string(fixtures().join("index-v1.json")).unwrap();
    let old = text.replacen(
        "\"registries\"\n          ],",
        "\"registries\", \"game-read\"\n          ],",
        1,
    );
    assert_ne!(old, text, "the fixture's capability list moved");
    let index = Index::parse(&old).unwrap();
    assert_eq!(index.projects.len(), sample().projects.len());
    let hud = index.project("coords_hud").unwrap();
    let listed = &hud.versions[0].capabilities;
    assert_eq!(listed.known(), [Capability::Registries]);
    assert_eq!(listed.unknown(), ["game-read"]);
    assert!(listed.unsupported().unwrap().contains("game-read"));
    assert!(index.installable(hud, "26.3").is_none());
    let pack = index.project("cozy").unwrap();
    assert!(index.installable(pack, &pack.versions[0].game).is_some());
    // Written back, the unknown name stays.
    let again = Index::parse(&serde_json::to_string(&index).unwrap()).unwrap();
    assert_eq!(again, index);
}

#[test]
fn files_pass_against_their_entries() {
    let index = sample();
    let smod = fixtures().join("coords_hud-0.1.0.smod");
    assert_eq!(check(&index, "coords_hud", &smod), Ok(Trust::Verified));
    let spack = fixtures().join("cozy-1.0.0.spack");
    assert_eq!(check(&index, "cozy", &spack), Ok(Trust::Unsigned));
    let sshader = fixtures().join("glow-1.0.0.sshader");
    assert_eq!(check(&index, "glow", &sshader), Ok(Trust::Unsigned));
    let world = index.project("skyblock_start").unwrap();
    let e = check_file(&index, world, &world.versions[0], &spack).unwrap_err();
    assert!(e.contains("bytes"), "{e}");
}

#[test]
fn install_verification_compares_mod_versions_as_semver() {
    let mut index = sample();
    let smod = fixtures().join("coords_hud-0.1.0.smod");
    index.projects[0].versions[0].version = "0.1".into();
    assert_eq!(check(&index, "coords_hud", &smod), Ok(Trust::Verified));
    index.projects[0].versions[0].version = "0.2".into();
    let e = check(&index, "coords_hud", &smod).unwrap_err();
    assert!(e.contains("isn't what the index lists"), "{e}");
}

#[test]
fn a_changed_file_key_or_manifest_is_refused() {
    let index = sample();
    let dir = std::env::temp_dir().join(format!("sunrise-index-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let smod = fixtures().join("coords_hud-0.1.0.smod");
    let mut bytes = std::fs::read(&smod).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let tampered = dir.join("coords_hud-0.1.0.smod");
    std::fs::write(&tampered, &bytes).unwrap();
    let e = check(&index, "coords_hud", &tampered).unwrap_err();
    assert!(e.contains("SHA-256"), "{e}");

    let mut other_key = index.clone();
    other_key.projects[0].key = Some(OTHER_KEY.into());
    let e = check(&other_key, "coords_hud", &smod).unwrap_err();
    assert!(e.contains("key"), "{e}");

    let mut more_caps = index.clone();
    more_caps.projects[0].versions[0].capabilities = Default::default();
    let e = check(&more_caps, "coords_hud", &smod).unwrap_err();
    assert!(e.contains("isn't what the index lists"), "{e}");

    let mut revoked = index.clone();
    revoked.revoked.push(Revoked {
        id: "coords_hud".into(),
        sha256: None,
        reason: "Test.".into(),
        date: String::new(),
    });
    let e = check(&revoked, "coords_hud", &smod).unwrap_err();
    assert!(e.contains("taken down: Test."), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_content_mod_lists_manifest_dependencies_and_checks_index_claims() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coords_hud.smod");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    zip.start_file(
        crate::manifest::MANIFEST,
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    write!(zip, "id='coords_hud'\nname='Coordinates'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[dependencies]\njei={{version='^0.5', optional=true, side='client'}}\ncreature='^0.1'\n").unwrap();
    zip.finish().unwrap();
    let facts = inspect(&Kind::Mod, &path).unwrap();
    assert_eq!(
        serde_json::to_value(&facts.dependencies).unwrap(),
        serde_json::json!([
            {"id": "creature", "version": "^0.1"},
            {"id": "jei", "version": "^0.5", "optional": true, "side": "client"}
        ])
    );

    let index = sample();
    let p = index.project("coords_hud").unwrap();
    let mut version = p.versions[0].clone();
    version.side = facts.side;
    assert!(
        serde_json::to_value(&version)
            .unwrap()
            .get("dependencies")
            .is_none()
    );
    assert!(facts.matches(p, &version).is_ok());
    version.dependencies = Some(facts.dependencies.clone());
    assert!(facts.matches(p, &version).is_ok());
    version.dependencies = Some(Vec::new());
    let serialized = serde_json::to_value(&version).unwrap();
    assert_eq!(serialized["dependencies"], serde_json::json!([]));
    let version = serde_json::from_value(serialized).unwrap();
    let error = facts.matches(p, &version).unwrap_err();
    for word in ["coords_hud", "0.1.0", "dependencies"] {
        assert!(error.contains(word), "{error}");
    }
}

fn indexed_mod(id: &str, version: &str, dependencies: serde_json::Value) -> Project {
    let mut p = sample().projects.remove(0);
    p.id = id.into();
    p.versions[0].version = version.into();
    p.versions[0].dependencies = serde_json::from_value(dependencies).unwrap();
    p
}

#[test]
fn the_live_registry_dependency_shape_is_valid() {
    let mut projects = [
        "butterflies",
        "cyclops",
        "fairies",
        "fireflies",
        "scorpions",
        "vanilla_bestiary",
        "vulture",
        "worms",
    ]
    .map(|id| {
        indexed_mod(
            id,
            "0.1.0",
            serde_json::json!([{"id": "creature", "version": "^0.1"}]),
        )
    })
    .to_vec();
    projects.extend([
        indexed_mod("creature", "0.1.1", serde_json::Value::Null),
        indexed_mod(
            "ironfurnaces",
            "1.0.0",
            serde_json::json!([
                {"id": "ae2", "version": "^15.4.13", "optional": true},
                {"id": "guideme", "version": "^0.1", "optional": true},
                {"id": "jei", "version": "^0.5", "optional": true},
                {"id": "machine_work", "version": "^0.1", "optional": true}
            ]),
        ),
        indexed_mod(
            "jei",
            "0.5.0",
            serde_json::json!([
                {"id": "guideme", "version": "^0.1", "optional": true}
            ]),
        ),
    ]);
    let index = Index {
        schema: SCHEMA,
        projects,
        ..Index::default()
    };
    assert_eq!(index.check(), Ok(()));
}

#[test]
fn a_required_dependency_needs_a_listed_matching_mod() {
    let mod_for = |range| {
        indexed_mod(
            "fireflies",
            "0.1.0",
            serde_json::json!([
                {"id": "creature", "version": range}
            ]),
        )
    };
    let mut index = Index {
        schema: SCHEMA,
        projects: vec![mod_for("^3.6")],
        ..Index::default()
    };
    let assert_error = |index: &Index, range: &str, words: &[&str]| {
        assert_eq!(index.check(), Ok(()));
        let errors = resolve::uninstallable_versions(index, |_| true);
        assert_eq!(errors.len(), 2, "{errors:?}");
        let e = &errors[0];
        for word in ["fireflies", "0.1.0", "creature", range]
            .into_iter()
            .chain(words.iter().copied())
        {
            assert!(e.contains(word), "missing {word:?} in {e:?}");
        }
    };
    assert_error(&index, "^3.6", &["not listed as a mod"]);
    index.projects.push(indexed_mod(
        "creature",
        "3.6.30-sunrise.1",
        serde_json::Value::Null,
    ));
    assert_error(&index, "^3.6", &["prerelease", "=3.6.30-sunrise.1"]);
    index.projects[0] = mod_for("^4.0");
    assert_error(&index, "^4.0", &["no listed version satisfies"]);
    index.projects[0] = mod_for("=3.6.30-sunrise.1");
    assert!(resolve::uninstallable_versions(&index, |_| true).is_empty());
}

#[test]
fn only_required_cycles_on_the_same_installation_are_refused() {
    let mut index = Index {
        schema: SCHEMA,
        projects: vec![
            indexed_mod(
                "fireflies",
                "0.1.0",
                serde_json::json!([
                    {"id": "creature", "version": "^0.1", "side": "client"}
                ]),
            ),
            indexed_mod(
                "creature",
                "0.1.1",
                serde_json::json!([
                    {"id": "fireflies", "version": "^0.1", "side": "server"}
                ]),
            ),
        ],
        ..Index::default()
    };
    assert!(resolve::uninstallable_versions(&index, |_| true).is_empty());
    index.projects[1].versions[0].dependencies.as_mut().unwrap()[0].side = None;
    assert_eq!(index.check(), Ok(()));
    let errors = resolve::uninstallable_versions(&index, |_| true);
    assert_eq!(errors.len(), 2, "{errors:?}");
    for error in errors {
        for word in ["cycle", "fireflies 0.1.0", "creature 0.1.1", "client"] {
            assert!(error.contains(word), "{error}");
        }
    }
    index.projects[1].versions[0].dependencies.as_mut().unwrap()[0].optional = true;
    assert!(resolve::uninstallable_versions(&index, |_| true).is_empty());
}

#[test]
fn malformed_dependency_rows_fail_publication_and_are_left_out_by_readers() {
    for rows in [
        serde_json::json!([{"id": "another", "version": "not semver"}]),
        serde_json::json!([{"id": "another", "version": "^1"}, {"id": "another", "version": "^2"}]),
        serde_json::json!([{"id": "NotValid", "version": "^1"}]),
    ] {
        let id = rows[0]["id"].as_str().unwrap().to_owned();
        let index = Index {
            schema: SCHEMA,
            projects: vec![indexed_mod("fireflies", "0.1.0", rows)],
            ..Index::default()
        };
        let error = index.check().unwrap_err();
        for word in ["fireflies", "0.1.0", &id] {
            assert!(error.contains(word), "{error}");
        }
        let read = Index::parse(&serde_json::to_string(&index).unwrap()).unwrap();
        assert!(read.projects.is_empty());
        assert!(read.left_out.iter().any(|row| row.id == "fireflies"
            && row.version.as_deref() == Some("0.1.0")
            && row.reason == error));
    }
}

#[test]
fn revocations_match_by_hash_or_by_id() {
    let index = sample();
    let ones = "1".repeat(64);
    assert!(index.revoked("bad_mod", &ones).is_some());
    assert!(index.revoked("bad_mod", &"2".repeat(64)).is_none());
    assert!(index.revoked("coords_hud", &ones).is_none());
}

#[test]
fn refuses_broken_indexes() {
    let text = std::fs::read_to_string(fixtures().join("index-v1.json")).unwrap();
    let newer = text.replacen("\"schema\": 1", "\"schema\": 2", 1);
    assert!(Index::parse(&newer).unwrap_err().contains("schema 2"));
    let broken = |edit: &dyn Fn(&mut Index)| {
        let mut index = sample();
        edit(&mut index);
        index.check().unwrap_err()
    };
    let e = broken(&|i| {
        let twin = i.projects[0].clone();
        i.projects.push(twin)
    });
    assert!(e.contains("listed twice"), "{e}");
    let e = broken(&|i| i.projects[0].versions[0].file.url = "http://example.org/x.smod".into());
    assert!(e.contains("https"), "{e}");
    // A local test registry may list files on disk; the published one may not.
    let local = text.replace(
        "https://github.com/sunrise-org/sunrise-mods/releases/download/coords_hud-0.1.0/",
        "file:///tmp/",
    );
    let published = Index::parse(&local).unwrap();
    assert!(published.project("coords_hud").is_none());
    assert!(
        published
            .left_out
            .iter()
            .any(|l| l.reason.contains("https"))
    );
    assert!(Index::parse_local(&local).unwrap().left_out.is_empty());
    let e = broken(&|i| i.projects[0].versions[0].file.name = "../coords_hud.smod".into());
    assert!(e.contains("plain file name"), "{e}");
    let e = broken(&|i| i.projects[0].versions[0].file.name = "coords_hud.zip".into());
    assert!(e.contains("plain file name"), "{e}");
    let e = broken(&|i| i.projects[0].versions[0].file.signature = None);
    assert!(e.contains("unsigned"), "{e}");
    let e = broken(&|i| i.projects[0].key = Some("not a key".into()));
    assert!(e.contains("key"), "{e}");
    let e = broken(&|i| i.projects[1].versions[0].requires[0].version = "9.9".into());
    assert!(e.contains("not in the index"), "{e}");
    let e = broken(&|i| i.projects[1].versions[0].requires[0].sha256 = "2".repeat(64));
    assert!(e.contains("another file"), "{e}");
    let e = broken(&|i| i.projects[0].id = "Coords".into());
    assert!(e.contains("is not"), "{e}");
}

/// `index` as the registry would publish it, read back.
fn reread(index: &Index) -> Index {
    Index::parse(&serde_json::to_string(index).unwrap()).unwrap()
}

/// The sample with a native mod published under the retired `.hnmod` extension, as the live
/// registry still lists some.
fn with_hnmod() -> Index {
    let mut index = sample();
    let mut native = index.projects[0].clone();
    native.id = "accurate_block_placement".into();
    native.kind = Kind::NativeMod;
    native.versions[0].file.name = "accurate_block_placement-1.0.0.hnmod".into();
    index.projects.push(native);
    index
}

#[test]
fn a_project_with_no_usable_version_is_left_out_and_the_rest_loads() {
    let index = reread(&with_hnmod());
    let ids: Vec<&str> = index.projects.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["coords_hud", "cozy", "glow", "skyblock_start"]);
    let reasons: Vec<(&str, Option<&str>)> = index
        .left_out
        .iter()
        .map(|l| (l.id.as_str(), l.version.as_deref()))
        .collect();
    assert_eq!(
        reasons,
        [
            ("accurate_block_placement", Some("0.1.0")),
            ("accurate_block_placement", None)
        ]
    );
    assert!(index.left_out[0].reason.contains("plain file name"));
}

#[test]
fn a_version_naming_a_release_the_index_does_not_list_is_left_out() {
    let mut index = sample();
    let listed = "26.3-0.1.0";
    index.releases = Some(vec![SunriseRelease {
        tag: listed.into(),
        commit: "0123456789abcdef0123456789abcdef01234567".into(),
        line: "26.3-0".into(),
    }]);
    let project = &mut index.projects[0];
    let id = project.id.clone();
    let game = project.versions[0].game.clone();
    let tag = |release: &str| Some(listed.replace("0.1.0", release).replace("26.3", &game));
    let mut versions = Vec::new();
    for (version, release) in [
        ("9.0.0", Some("0.2.0")),
        ("9.0.1", Some("0.1.0")),
        ("9.0.2", None),
    ] {
        let mut v = project.versions[0].clone();
        v.version = version.into();
        v.sunrise = release.and_then(tag);
        versions.push(v);
    }
    project.versions = versions;
    let error = index.check().unwrap_err();
    assert!(error.contains(&id) && error.contains("9.0.0"), "{error}");
    let read = reread(&index);
    let kept: Vec<&str> = read
        .project(&id)
        .unwrap()
        .versions
        .iter()
        .map(|v| v.version.as_str())
        .collect();
    assert_eq!(kept, ["9.0.1", "9.0.2"]);
    let left_out: Vec<Option<&str>> = read
        .left_out
        .iter()
        .filter(|l| l.id == id)
        .map(|l| l.version.as_deref())
        .collect();
    assert_eq!(left_out, [Some("9.0.0")]);

    // An index that lists no releases makes no claim about the tag.
    index.releases = None;
    assert_eq!(reread(&index).project(&id).unwrap().versions.len(), 3);
}

#[test]
fn a_takedown_of_the_whole_project_drops_its_nightly_source_for_a_reader() {
    let mut index = sample();
    index.projects[0].nightly = Some(Nightly {
        repo: "author/example".into(),
        path: None,
        metadata: Default::default(),
    });
    let id = index.projects[0].id.clone();
    let mut source_only = index.projects[0].clone();
    source_only.id = "source_only".into();
    source_only.versions.clear();
    index.projects.push(source_only);
    let takedown = |id: &str, sha256: Option<String>| Revoked {
        id: id.into(),
        sha256,
        reason: "Withdrawn test project.".into(),
        date: String::new(),
    };

    // By hash: the named package alone.
    let hash = index.projects[0].versions[0].file.sha256.clone();
    index.revoked = vec![takedown(&id, Some(hash))];
    assert_eq!(index.check(), Ok(()));
    let read = reread(&index);
    assert!(read.project(&id).unwrap().nightly.is_some());
    assert!(read.project("source_only").is_some());

    // By id: the source goes too, and a project left with nothing is left out.
    index.revoked = vec![takedown(&id, None), takedown("source_only", None)];
    let error = index.check().unwrap_err();
    assert!(error.contains(&id), "{error}");
    let read = reread(&index);
    let project = read.project(&id).unwrap();
    assert!(project.nightly.is_none() && !project.versions.is_empty());
    assert!(read.project("source_only").is_none());
    assert!(read.left_out.iter().any(|l| l.id == "source_only"));
}

#[test]
fn a_bad_version_is_left_out_and_its_project_keeps_the_others() {
    let mut index = with_hnmod();
    let native = index.projects.last_mut().unwrap();
    let mut good = native.versions[0].clone();
    good.version = "0.0.9".into();
    good.file.name = "accurate_block_placement-0.0.9.smod".into();
    native.versions.push(good);
    let index = reread(&index);
    let native = index.project("accurate_block_placement").unwrap();
    let kept: Vec<&str> = native.versions.iter().map(|v| v.version.as_str()).collect();
    assert_eq!(kept, ["0.0.9"]);
    assert_eq!(index.left_out.len(), 1);
    assert_eq!(index.left_out[0].version.as_deref(), Some("0.1.0"));
}

#[test]
fn a_modpack_whose_mod_was_left_out_is_left_out_too() {
    let mut index = sample();
    index.projects[0].versions[0].file.url = "http://example.org/coords_hud.smod".into();
    let index = reread(&index);
    assert!(index.project("coords_hud").is_none());
    assert!(index.project("cozy").is_none());
    assert!(index.project("glow").is_some());
    let cozy = index.left_out.iter().find(|l| l.id == "cozy").unwrap();
    assert!(cozy.reason.contains("not in the index"), "{}", cozy.reason);
}

#[test]
fn an_id_listed_twice_leaves_out_both() {
    let mut index = sample();
    let twin = index.projects[2].clone();
    index.projects.push(twin);
    let index = reread(&index);
    assert!(index.project("glow").is_none());
    let twice = index
        .left_out
        .iter()
        .filter(|l| l.reason.contains("listed twice"));
    assert_eq!(twice.count(), 2);
}

#[test]
fn only_text_that_is_not_an_index_fails_to_load() {
    assert!(Index::parse("{\"schema\": 1, \"projects\": [{}]}").is_err());
    assert!(Index::parse("not json").is_err());
    // Odd values a mod's author controls never panic the reader.
    let mut index = with_hnmod();
    index.projects[0].versions[0].version = String::new();
    index.projects[0].id = "Ünïcode ✓".into();
    index.projects[2].versions[0].file.name = String::new();
    let index = reread(&index);
    assert!(index.left_out.len() >= 3);
}

#[test]
fn resource_packs_need_a_pack_mcmeta() {
    use std::io::Write;
    let dir = std::env::temp_dir().join(format!("sunrise-index-rp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pack = |files: &[(&str, &str)]| {
        let path = dir.join("look.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name, text) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    };
    let meta = r#"{"pack": {"description": "Look", "min_format": 75, "max_format": 75}}"#;
    assert_eq!(
        check_resource_pack(&pack(&[("pack.mcmeta", meta)])),
        Ok("75".into())
    );
    let e = check_resource_pack(&pack(&[("assets/x.png", "")])).unwrap_err();
    assert!(e.contains("missing"), "{e}");
    let e = check_resource_pack(&pack(&[("pack.mcmeta", "{}")])).unwrap_err();
    assert!(e.contains("\"pack\""), "{e}");
    let e = check_resource_pack(&pack(&[("pack.mcmeta", meta), ("../x", "")])).unwrap_err();
    assert!(e.contains("plain path"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn data_packs_and_shader_folder_packs_are_read_by_their_content() {
    use std::io::Write;
    let dir = std::env::temp_dir().join(format!("sunrise-index-dp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut n = 0;
    let mut pack = |files: &[&str]| {
        n += 1;
        let path = dir.join(format!("{n}.zip"));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for name in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            let text = if *name == "pack.mcmeta" {
                r#"{"pack": {"description": "D", "pack_format": 94}}"#
            } else {
                ""
            };
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    };
    let data = pack(&["pack.mcmeta", "data/x/function/a.mcfunction"]);
    assert_eq!(inspect(&Kind::DataPack, &data).unwrap().api, "94");
    assert_eq!(zip_kind(&data), Kind::DataPack);
    let looks = pack(&["pack.mcmeta", "assets/x/textures/a.png"]);
    let e = inspect(&Kind::DataPack, &looks).unwrap_err();
    assert!(e.contains("data/"), "{e}");
    assert_eq!(zip_kind(&looks), Kind::ResourcePack);
    let no_meta = pack(&["data/x/function/a.mcfunction"]);
    assert!(
        inspect(&Kind::DataPack, &no_meta)
            .unwrap_err()
            .contains("missing")
    );
    let shaders = pack(&["shaders/composite.fsh", "shaders/lang/en_us.lang"]);
    assert_eq!(inspect(&Kind::ShaderPack, &shaders).unwrap().api, "");
    assert_eq!(zip_kind(&shaders), Kind::ShaderPack);
    let e = inspect(&Kind::ShaderPack, &looks).unwrap_err();
    assert!(e.contains("shaders/"), "{e}");
    let sneaky = pack(&["shaders/a.fsh", "../../x"]);
    assert!(
        inspect(&Kind::ShaderPack, &sneaky)
            .unwrap_err()
            .contains("plain path")
    );
    // Both are zips, and a reader from before them keeps them as unknown kinds.
    for kind in [Kind::DataPack, Kind::ShaderPack] {
        assert_eq!(kind.extension(), Some("zip"));
        assert!(open(&kind).is_ok());
        assert_eq!(Kind::from(kind.name().to_owned()), kind);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_newest_unrevoked_version_for_this_game_is_installed() {
    let mut index = sample();
    let mut newer = index.projects[0].versions[0].clone();
    newer.version = "0.2.0".into();
    newer.file.sha256 = "2".repeat(64);
    index.projects[0].versions.insert(0, newer);
    let p = index.project("coords_hud").unwrap();
    assert_eq!(index.installable(p, "26.3").unwrap().version, "0.2.0");
    assert_eq!(index.installable(p, "26.4"), None);
    index.revoked.push(Revoked {
        id: "coords_hud".into(),
        sha256: Some("2".repeat(64)),
        reason: "Test.".into(),
        date: String::new(),
    });
    let p = index.project("coords_hud").unwrap();
    assert_eq!(index.installable(p, "26.3").unwrap().version, "0.1.0");
    index.revoked.last_mut().unwrap().sha256 = None;
    let p = index.project("coords_hud").unwrap();
    assert_eq!(index.installable(p, "26.3"), None);
}

#[test]
fn shader_packs_pass_the_game_s_own_check() {
    let manifest = "schema = 1\nid = \"glow\"\nname = \"Glow\"\nversion = \"1.0.0\"\napi = \"0.1\"\n\
                    [[post]]\nfile = \"post.wgsl\"\n";
    let pack = |post: &str| {
        let files = [
            (sunrise_shader_api::MANIFEST, manifest),
            ("post.wgsl", post),
        ]
        .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
        .into();
        sunrise_shader_api::Pack::from_files(files)
            .unwrap()
            .package()
            .unwrap()
    };
    let dir = std::env::temp_dir().join(format!("sunrise-mods-shader-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("glow-1.0.0.sshader");
    std::fs::write(
        &good,
        pack("fn post(input: PostInput) -> vec4<f32> { return input.color; }\n"),
    )
    .unwrap();
    let bad = dir.join("bad.sshader");
    std::fs::write(
        &bad,
        pack("fn post(input: PostInput) -> vec4<f32> { return input.color }\n"),
    )
    .unwrap();
    let experimental = dir.join("glow-0.2.sshader");
    let files = [
        (
            sunrise_shader_api::MANIFEST,
            manifest.replace("api = \"0.1\"", "api = \"0.2\""),
        ),
        (
            "post.wgsl",
            "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n".into(),
        ),
    ]
    .map(|(n, t)| (n.to_owned(), t.into_bytes()))
    .into();
    let bytes = sunrise_shader_api::Pack::from_files(files)
        .unwrap()
        .package()
        .unwrap();
    std::fs::write(&experimental, bytes).unwrap();
    let experimental = inspect(&Kind::Shader, &experimental).unwrap_err();
    let facts = inspect(&Kind::Shader, &good);
    let refused = inspect(&Kind::Shader, &bad).unwrap_err();
    let not_a_pack = inspect(&Kind::Shader, &fixtures().join("cozy-1.0.0.spack")).unwrap_err();
    std::fs::remove_dir_all(&dir).unwrap();
    let facts = facts.unwrap();
    assert_eq!(
        (
            facts.id.as_deref(),
            facts.version.as_deref(),
            facts.api.as_str()
        ),
        (Some("glow"), Some("1.0.0"), "0.1")
    );
    assert!(refused.contains("post.wgsl:1:"), "{refused}");
    assert!(
        experimental.contains("shader API 0.2, which is experimental"),
        "{experimental}"
    );
    assert!(
        not_a_pack.contains(sunrise_shader_api::MANIFEST),
        "{not_a_pack}"
    );
    assert_eq!(Kind::Shader.extension(), Some("sshader"));
    assert!(open(&Kind::Shader).is_ok());
}

/// A native source package with one `runtime/` file.
fn native_package(name: &str, nav: &[u8]) -> PathBuf {
    use std::io::Write;
    let manifest = format!(
        "id='ruby'\nname='Ruby'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n\
         [native]\ngame='26.3'\nsunrise='{0}'\nmixins='{0}'\n[runtime]\n\"nav.bin\"='{1}'\n",
        "a".repeat(40),
        crate::manifest::sha512_of(b"tables"),
    );
    let path = std::env::temp_dir().join(format!("{name}-{}.smod", std::process::id()));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (entry, bytes) in [
        (crate::manifest::MANIFEST, manifest.as_bytes()),
        (
            "Cargo.toml",
            b"[package]\nname='ruby'\nversion='0.1.0'\n".as_slice(),
        ),
        ("src/lib.rs", b"pub fn hooks() {}".as_slice()),
        ("runtime/nav.bin", nav),
    ] {
        zip.start_file(entry, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    path
}

#[test]
fn installing_a_native_mod_checks_its_runtime_files_against_the_manifest() {
    let good = native_package("runtime-native-ok", b"tables");
    let facts = inspect(&Kind::NativeMod, &good);
    // The same name and length, different bytes.
    let edited = native_package("runtime-native-edited", b"tablez");
    let refused = inspect(&Kind::NativeMod, &edited);
    std::fs::remove_file(good).unwrap();
    std::fs::remove_file(edited).unwrap();
    assert_eq!(facts.unwrap().id.as_deref(), Some("ruby"));
    let error = refused.unwrap_err();
    assert!(error.contains("runtime/nav.bin"), "{error}");
}

#[test]
fn a_stable_tag_is_only_its_canonical_form() {
    assert_eq!(stable_tag("26.3-0.1.0"), Some(("26.3", 0)));
    assert_eq!(stable_tag("26.3.1-1.2.0"), Some(("26.3.1", 1)));
    for tag in [
        "026.3-0.1.0",
        "26.03-0.1.0",
        "26.3.1.2-0.1.0",
        "26-0.1.0",
        "26.3-0.1.0-rc.1",
        "26.3-0.1",
        "26.3-0.1.0+build",
    ] {
        assert_eq!(stable_tag(tag), None, "{tag}");
    }
}
