//! A registry checkout holding the sample index's mod, modpack and shader pack
//! builds the sample index (`tests/index/index-v1.json`) exactly.
use super::*;

const SMOD: &str = "cb6e658445269b2a4d2eb104e83e4b993586bc2e56814fd56454244139f08364";
const SPACK: &str = "ca9a0207d5a9df142962a2dd7563a639eea6774e7f144b6ef239eb6eaf753f58";
const SSHADER: &str = "a23c5fe9fd173243704dd51160f20f35927c6e0060c5541ce1823f25db8c7961";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/index")
}

fn sample() -> Index {
    Index::parse(&std::fs::read_to_string(fixtures().join("index-v1.json")).unwrap()).unwrap()
}

fn hud_entry() -> String {
    let p = sample();
    let sig = p.project("coords_hud").unwrap().versions[0]
        .file
        .signature
        .clone()
        .unwrap();
    format!(
        r#"id = "coords_hud"
kind = "mod"
name = "Coordinates HUD"
summary = "Sample content: a resource translation."
description = "A signed content-only package with a resource translation for registry tests."
authors = ["Sunrise"]
owners = ["ludovic111"]
license = "MIT"
source = "https://github.com/horizon-rust/horizon"
icon = "icons/coords_hud.png"
categories = ["utility"]
key = "RWTVDqAKF4nm3+eZIK4SeLU6R6sSdJyNgrG/8bdFuTPpXlGK0I8+7wmc"

[[versions]]
version = "0.1.0"
published = "2026-10-24"
game = "26.3"
file = "coords_hud-0.1.0.smod"
sha256 = "{SMOD}"
signature = """{sig}"""
"#
    )
}

const COZY: &str = r#"id = "cozy"
kind = "modpack"
name = "Cozy"
summary = "Coordinates and a calm render distance."
description = "Coordinates HUD with a calm render distance, played together as one profile."
categories = ["lightweight"]
icon = "icons/cozy.png"
authors = ["Sunrise"]
owners = ["ludovic111"]
license = "MIT"
source = "https://github.com/horizon-rust/horizon-mods"

[[versions]]
version = "1.0.0"
published = "2026-10-24"
game = "26.3"
file = "cozy-1.0.0.spack"
sha256 = "ca9a0207d5a9df142962a2dd7563a639eea6774e7f144b6ef239eb6eaf753f58"
"#;

const GLOW: &str = r#"id = "glow"
kind = "shader"
name = "Soft Glow"
summary = "A post-processing pass over the world."
description = "A soft bloom drawn over the world after it renders, in Sunrise's own shader format."
categories = ["bloom"]
icon = "icons/glow.png"
authors = ["Sunrise"]
owners = ["ludovic111"]
license = "MIT"
source = "https://github.com/horizon-rust/horizon-mods"

[[versions]]
version = "1.0.0"
published = "2026-10-26"
game = "26.3"
file = "glow-1.0.0.sshader"
sha256 = "a23c5fe9fd173243704dd51160f20f35927c6e0060c5541ce1823f25db8c7961"
"#;

const REVOKED_TOML: &str = r#"[[revoked]]
id = "bad_mod"
sha256 = "1111111111111111111111111111111111111111111111111111111111111111"
reason = "Sent chat messages it didn't declare."
date = "2026-10-24"
"#;

/// The header of a PNG of `w`×`h` pixels: what the registry reads of an image.
pub(super) fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b.extend([8, 6, 0, 0, 0]);
    b
}

/// A registry checkout and its downloaded files, in a scratch folder.
pub(super) struct Scratch(pub(super) PathBuf);

impl Scratch {
    pub(super) fn new(name: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("sunrise-registry-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["repo/mods", "repo/icons", "files"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let s = Scratch(dir);
        s.write("mods/coords_hud.toml", &hud_entry());
        s.write("mods/cozy.toml", COZY);
        s.write("mods/glow.toml", GLOW);
        s.write("revoked.toml", REVOKED_TOML);
        for icon in ["coords_hud", "cozy", "glow"] {
            s.write_bytes(&format!("icons/{icon}.png"), &png(128, 128));
        }
        for (name, hash) in [
            ("coords_hud-0.1.0.smod", SMOD),
            ("cozy-1.0.0.spack", SPACK),
            ("glow-1.0.0.sshader", SSHADER),
        ] {
            std::fs::copy(fixtures().join(name), s.0.join("files").join(hash)).unwrap();
        }
        s
    }

    pub(super) fn write(&self, path: &str, text: &str) {
        std::fs::write(self.0.join("repo").join(path), text).unwrap();
    }

    pub(super) fn write_bytes(&self, path: &str, bytes: &[u8]) {
        let path = self.0.join("repo").join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    pub(super) fn build(&self) -> Result<Index, Vec<String>> {
        let (repo, files) = (self.0.join("repo"), self.0.join("files"));
        build(&Sources {
            registry: &repo,
            files: &files,
            release: super::super::FILE_HOST,
            site: "https://horizon-rust.github.io/horizon-mods/",
            generated: "2026-10-24T18:00:00Z".into(),
            releases: None,
            nightly: None,
            binary_checks: None,
        })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn builds_the_sample_index() {
    let s = Scratch::new("sample");
    let built = s.build().unwrap();
    let sample = sample();
    assert_eq!(built.projects, sample.projects[..3]);
    assert_eq!(built.revoked, sample.revoked);
    assert_eq!(built.generated, sample.generated);
    // What it writes reads back.
    Index::parse(&serde_json::to_string_pretty(&built).unwrap()).unwrap();
}

#[test]
fn a_stable_source_file_has_its_kinds_extension() {
    let mut entry: Entry = toml::from_str(GLOW).unwrap();
    entry.source = "https://github.com/author/glow".into();
    let mut row = entry.versions[0].clone();
    row.sunrise = Some("26.3-0.1.0".into());
    let asset = "https://github.com/author/glow/releases/download/v1.0.0/glow-1.0.0";
    row.file = format!("{asset}.sshader");
    assert_eq!(entry.source_file_name(&row), Ok("glow-1.0.0.sshader"));
    row.file = format!("{asset}.smod");
    assert!(entry.source_file_name(&row).is_err());
    entry.kind = Kind::Other("future-pack".into());
    assert!(
        entry
            .source_file_name(&row)
            .unwrap_err()
            .contains("future-pack")
    );
}

#[test]
fn stable_source_uses_an_author_url_but_the_index_uses_the_registry_mirror() {
    let s = Scratch::new("author-source");
    let releases = parse_releases(
        r#"[{"tag":"26.3-0.1.0","commit":"0123456789abcdef0123456789abcdef01234567"}]"#,
    )
    .unwrap();
    let inputs = Sources {
        registry: &s.0.join("repo"),
        files: &s.0.join("files"),
        release: super::super::FILE_HOST,
        site: "https://example.invalid/",
        generated: String::new(),
        releases: Some(&releases),
        nightly: None,
        binary_checks: None,
    };
    let mut entry: Entry = toml::from_str(&hud_entry()).unwrap();
    entry.source.clear();
    entry.repo = Some("author/Coords".into());
    let mut row = entry.versions[0].clone();
    let legacy = checked_version(&entry, &row, &inputs, &[]).unwrap();
    assert_eq!(legacy.file.name, "coords_hud-0.1.0.smod");
    assert!(
        legacy
            .file
            .url
            .ends_with("/coords_hud-0.1.0/coords_hud-0.1.0.smod")
    );
    assert!(legacy.file.url.starts_with(super::super::FILE_HOST));

    row.sunrise = Some(releases[0].tag.clone());
    assert!(
        checked_version(&entry, &row, &inputs, &[])
            .unwrap_err()
            .contains("GitHub release asset URL")
    );
    for url in [
        "https://github.com/other/coords/releases/download/v0.1.0/coords_hud-0.1.0.smod",
        "https://github.com/author/coords/releases/download/v0.1.0/other-0.1.0.smod",
        "https://github.com/author/coords/releases/download/extra/v0.1.0/coords_hud-0.1.0.smod",
    ] {
        row.file = url.into();
        assert!(
            checked_version(&entry, &row, &inputs, &[]).is_err(),
            "{url}"
        );
    }
    row.file =
        "https://github.com/author/coords/releases/download/v0.1.0/coords_hud-0.1.0.smod".into();
    let stable = checked_version(&entry, &row, &inputs, &[]).unwrap();
    assert_eq!(stable.file, legacy.file);
    entry.versions[0] = row;
    let mut changed = entry.clone();
    changed.versions[0].file = changed.versions[0].file.replace("/v0.1.0/", "/retagged/");
    let review = review(&[entry], &[changed], "author");
    assert!(
        review
            .errors
            .iter()
            .any(|e| e.contains("coords_hud 0.1.0") && e.contains("published file")),
        "{review:?}"
    );
}

#[test]
fn an_entry_with_a_short_version_label_takes_the_file_that_says_it_in_full() {
    let s = Scratch::new("short");
    s.write(
        "mods/coords_hud.toml",
        &hud_entry().replace("version = \"0.1.0\"", "version = \"0.1\""),
    );
    // The sample modpack still needs 0.1.0, which is the only problem left.
    let errors = s.build().unwrap_err();
    assert_eq!(
        errors,
        ["cozy 1.0.0: needs coords_hud 0.1.0, which is not in the index"]
    );
}

#[test]
fn downloads_and_stars_come_from_the_counts_and_a_zero_is_left_out() {
    let s = Scratch::new("counts");
    let mut index = s.build().unwrap();
    let first = &index.projects[0];
    let url = first.versions[0].file.url.clone();
    let repo = first
        .source
        .strip_prefix("https://github.com/")
        .unwrap()
        .to_ascii_uppercase();
    let counts: Counts = serde_json::from_str(&format!(
        r#"{{"downloads": {{"{url}": 7, "{url}?other": 100}}, "stars": {{"{repo}": 3}}}}"#
    ))
    .unwrap();
    count(&mut index, &counts);
    assert_eq!(
        (index.projects[0].downloads, index.projects[0].stars),
        (7, 3)
    );
    // A project the counts don't name counts nothing, and the index leaves both out.
    let second = &index.projects[1];
    assert_eq!((second.downloads, second.stars), (0, 0));
    let json = serde_json::to_value(&index).unwrap();
    assert!(json["projects"][1].get("downloads").is_none());
    assert_eq!(json["projects"][0]["downloads"], 7);
}

#[test]
fn lists_every_problem_of_a_registry() {
    let s = Scratch::new("bad");
    // A file that isn't the one its hash names, an unsigned version of a
    // signed project, a shader pack, and a mod whose file is another mod.
    std::fs::write(s.0.join("files").join(SPACK), b"other bytes").unwrap();
    s.write(
        "mods/coords_hud.toml",
        &hud_entry().replace("signature = \"\"\"", "changelog = \"\"\""),
    );
    s.write(
        "mods/glow.toml",
        &COZY
            .replace("\"cozy\"", "\"glow\"")
            .replace("modpack", "shader"),
    );
    s.write(
        "mods/other.toml",
        &hud_entry()
            .replace("\"coords_hud\"", "\"other\"")
            .replace("key = ", "# key = ")
            .replace("signature = \"\"\"", "changelog = \"\"\""),
    );
    let errors = s.build().unwrap_err();
    let has = |part: &str| errors.iter().any(|e| e.contains(part));
    assert!(
        has("cozy 1.0.0: the file isn't the one its sha256 names"),
        "{errors:?}"
    );
    assert!(has("coords_hud 0.1.0: the project has a key"), "{errors:?}");
    // A shader entry is taken now, and checked like the others.
    assert!(
        has("glow 1.0.0: the file isn't the one its sha256 names"),
        "{errors:?}"
    );
    assert!(
        has("other 0.1.0: the file's id is \"coords_hud\""),
        "{errors:?}"
    );
    assert!(
        has("\"lightweight\" is not a category of a shader"),
        "{errors:?}"
    );
    assert!(has("glow: its icon \"icons/cozy.png\""), "{errors:?}");
    assert!(
        has("other: its icon \"icons/coords_hud.png\""),
        "{errors:?}"
    );
    assert_eq!(errors.len(), 7, "{errors:?}");
}

#[test]
fn an_entry_is_named_by_its_id_and_has_only_known_fields() {
    let s = Scratch::new("names");
    s.write("mods/hud.toml", &hud_entry());
    s.write("mods/notes.txt", "");
    s.write("mods/cozy.toml", &format!("{COZY}homepage = \"x\"\n"));
    let errors = load(&s.0.join("repo")).unwrap_err();
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(errors[0].contains("cozy.toml") && errors[0].contains("homepage"));
    assert!(errors[1].contains("must be mods/coords_hud.toml"));
    assert!(errors[2].contains("only <id>.toml"));
}

#[test]
fn an_entry_written_reads_back() {
    let entry: Entry = toml::from_str(&hud_entry()).unwrap();
    let text = entry.to_toml().unwrap();
    assert_eq!(toml::from_str::<Entry>(&text).unwrap(), entry);
    assert!(text.contains("[[versions]]"), "{text}");
}

#[test]
fn owners_and_owner_ids_must_name_the_same_accounts() {
    let entry = |extra: &str| -> Entry {
        toml::from_str(&hud_entry().replacen("owners = [\"ludovic111\"]", extra, 1)).unwrap()
    };
    let mismatch = |e: &Entry| e.check_fields().iter().any(|m| m.contains("owner_ids"));
    // No IDs yet (older entries), or one ID per login: fine.
    assert!(!mismatch(&entry(r#"owners = ["ludovic111"]"#)));
    assert!(!mismatch(&entry(
        "owner_ids = [136255825, 583231]\nowners = [\"ludovic111\", \"someone\"]"
    )));
    // A co-owner added to `owners` alone would silently not be an owner: refused when it's made.
    assert!(mismatch(&entry(
        "owner_ids = [136255825]\nowners = [\"ludovic111\", \"someone\"]"
    )));
    assert!(mismatch(&entry(
        "owner_ids = [136255825, 583231]\nowners = [\"ludovic111\"]"
    )));
}

#[test]
fn owner_ids_are_optional_and_kept() {
    // Entries from before IDs were recorded still read, and write no `owner_ids`.
    let old: Entry = toml::from_str(&hud_entry()).unwrap();
    assert!(old.owner_ids.is_empty());
    assert!(!old.to_toml().unwrap().contains("owner_ids"));
    let text = hud_entry().replacen("owners = ", "owner_ids = [583231]\nowners = ", 1);
    let new: Entry = toml::from_str(&text).unwrap();
    assert_eq!(new.owner_ids, [583231]);
    assert_eq!(
        toml::from_str::<Entry>(&new.to_toml().unwrap()).unwrap(),
        new
    );
    // A change of IDs is pointed out to reviewers.
    let r = review(
        std::slice::from_ref(&old),
        std::slice::from_ref(&new),
        "ludovic111",
    );
    assert!(
        r.notes
            .iter()
            .any(|n| n.contains("owner IDs (none) -> 583231")),
        "{r:?}"
    );
}

#[test]
fn owner_only_nightly_changes_are_routine() {
    let mut old: Entry = toml::from_str(&hud_entry()).unwrap();
    old.versions[0].sunrise = Some("26.3-0.1.0".into());
    let mut with_source = old.clone();
    with_source.nightly = Some(NightlySource {
        repo: "ludovic111/coords_hud".into(),
        path: None,
    });
    let status = |base: Option<&Entry>, head: &Entry, author| {
        let review = review(
            base.map_or(&[], std::slice::from_ref),
            std::slice::from_ref(head),
            author,
        );
        assert!(review.errors.is_empty(), "{:?}", review.errors);
        assert_eq!(review.nightly_reviews[0].id, head.id);
        review.nightly_reviews[0].status
    };
    assert_eq!(
        status(Some(&old), &with_source, "Ludovic111"),
        ReviewStatus::Routine
    );
    assert_eq!(
        status(Some(&old), &with_source, "someone_else"),
        ReviewStatus::ReviewRequired
    );
    let mut edited = with_source.clone();
    edited.nightly.as_mut().unwrap().path = Some("new-path".into());
    assert_eq!(
        status(Some(&with_source), &edited, "ludovic111"),
        ReviewStatus::Routine
    );
    edited.owners.push("someone_else".into());
    assert_eq!(
        status(Some(&with_source), &edited, "ludovic111"),
        ReviewStatus::ReviewRequired
    );
    assert_eq!(
        status(None, &with_source, "ludovic111"),
        ReviewStatus::ReviewRequired
    );
    with_source.versions.clear();
    assert_eq!(
        status(None, &with_source, "ludovic111"),
        ReviewStatus::Routine
    );
    assert_eq!(
        status(None, &with_source, "someone_else"),
        ReviewStatus::ReviewRequired
    );
}

#[test]
fn the_release_list_refuses_malformed_or_ambiguous_releases() {
    let hex = "ab".repeat(20);
    let row = |tag: &str, commit: &str| serde_json::json!({ "tag": tag, "commit": commit });
    let check = |rows| parse_releases(&serde_json::Value::Array(rows).to_string());
    for tag in [
        "26-0.1.0",
        "26..3-0.1.0",
        "26.3-0.1",
        "26.3-v0.1.0",
        "26.3-0.1.0-rc.1",
        "26.3-0.1.0+x",
    ] {
        assert!(check(vec![row(tag, &hex)]).unwrap_err().contains(tag));
    }
    assert!(
        check(vec![row("26.3-0.1.0", "abc")])
            .unwrap_err()
            .contains("40 hex digits")
    );
    let release = row("26.3-0.1.0", &hex);
    assert!(
        check(vec![release.clone(), release])
            .unwrap_err()
            .contains("listed twice")
    );
}

#[test]
fn a_review_protects_published_files() {
    let base: Entry = toml::from_str(&hud_entry()).unwrap();
    let mut cozy: Entry = toml::from_str(COZY).unwrap();
    cozy.versions[0].sunrise = Some("26.3-0.1.0".into());
    let one = std::slice::from_ref(&base);
    assert_eq!(review(one, one, "someone"), Review::default());

    let mut update = base.clone();
    let mut next = update.versions[0].clone();
    next.version = "0.2.0".into();
    next.sunrise = Some("26.3-0.1.0".into());
    update.versions.insert(0, next);
    let r = review(one, std::slice::from_ref(&update), "ludovic111");
    assert!(r.errors.is_empty(), "{r:?}");
    assert_eq!(r.notes, ["coords_hud: 1 new version(s)"]);
    let r = review(one, &[update], "stranger");
    assert!(r.notes[0].contains("isn't one of its owners"), "{r:?}");

    let mut swapped = base.clone();
    swapped.versions[0].sha256 = SPACK.into();
    swapped.versions[0].sunrise = Some("26.3-0.1.0".into());
    swapped.kind = Kind::ResourcePack;
    swapped.key = None;
    let r = review(&[base.clone(), cozy.clone()], &[swapped], "ludovic111");
    assert_eq!(r.errors.len(), 3, "{r:?}");
    assert!(r.errors.iter().any(|e| e.contains("cozy: removed")));
    assert!(r.errors.iter().any(|e| e.contains("never changes")));
    assert!(r.errors.iter().any(|e| e.contains("kind can't change")));
    assert!(r.notes.iter().any(|n| n.contains("signing key")));

    let r = review(&[], &[cozy], "newcomer");
    assert!(r.errors.is_empty(), "{r:?}");
    assert_eq!(r.notes, ["cozy: a new modpack by ludovic111"]);

    // A row may gain a stable release; once published, the release it names stays.
    let mut stable = base.clone();
    stable.versions[0].sunrise = Some("26.3-0.1.0".into());
    let r = review(one, std::slice::from_ref(&stable), "ludovic111");
    assert!(r.errors.is_empty(), "{r:?}");
    for renamed in [None, Some("26.3-0.2.0".to_owned())] {
        let mut changed = stable.clone();
        changed.versions[0].sunrise = renamed;
        let r = review(std::slice::from_ref(&stable), &[changed], "ludovic111");
        assert_eq!(r.errors.len(), 1, "{r:?}");
        assert!(r.errors[0].contains("coords_hud 0.1.0"), "{r:?}");
    }
}

#[test]
fn review_requires_a_stable_release_on_a_new_row() {
    let base: Entry = toml::from_str(&hud_entry()).unwrap();
    let mut head = base.clone();
    let mut next = base.versions[0].clone();
    next.version = "0.2.0".into();
    head.versions.push(next);
    for (before, version) in [(std::slice::from_ref(&base), "0.2.0"), (&[][..], "0.1.0")] {
        let r = review(before, std::slice::from_ref(&head), "ludovic111");
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains(&format!("coords_hud {version}:"))
                    && e.contains("needs sunrise")),
            "{r:?}"
        );
    }
}

#[test]
fn review_requires_a_stable_release_on_a_changed_row() {
    let base: Entry = toml::from_str(&hud_entry()).unwrap();
    let mut head = base.clone();
    head.versions[0].changelog = "An edited changelog.".into();
    let r = review(&[base], &[head], "ludovic111");
    assert!(
        matches!(&r.errors[..], [e] if e.starts_with("coords_hud 0.1.0:")),
        "{r:?}"
    );
}

#[test]
fn review_refuses_deleting_an_entry_even_with_only_legacy_rows() {
    let base: Entry = toml::from_str(&hud_entry()).unwrap();
    let r = review(&[base], &[], "ludovic111");
    assert!(
        matches!(&r.errors[..], [e] if e.starts_with("coords_hud:")),
        "{r:?}"
    );
}

#[test]
fn a_version_taken_down_by_hash_may_leave_its_entry() {
    // A lost signing key: the next version comes under a new key, and the old one is dropped.
    let mut base: Entry = toml::from_str(&hud_entry()).unwrap();
    base.versions[0].sunrise = Some("26.3-0.1.0".into());
    let old = base.versions[0].clone();
    let mut rotated = base.clone();
    rotated.versions[0].version = "0.2.0".into();
    rotated.versions[0].sha256 = SPACK.into();
    let down = |id: &str, sha256: &str| Revoked {
        id: id.into(),
        sha256: Some(sha256.into()),
        reason: "signed with a lost key".into(),
        date: "2026-11-09".into(),
    };
    let one = std::slice::from_ref(&base);
    let next = std::slice::from_ref(&rotated);

    let r = review_with_revoked(one, next, &[down("coords_hud", &old.sha256)], "ludovic111");
    assert!(r.errors.is_empty(), "{r:?}");
    assert!(
        r.notes
            .iter()
            .any(|n| n == "coords_hud 0.1.0: leaves the entry, taken down in revoked.toml"),
        "{r:?}"
    );
    // Not taken down, another project's take-down, or a take-down of every version
    // without a hash: still refused.
    for revoked in [
        vec![],
        vec![down("other_mod", &old.sha256)],
        vec![Revoked {
            sha256: None,
            ..down("coords_hud", "")
        }],
    ] {
        let r = review_with_revoked(one, next, &revoked, "ludovic111");
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains("coords_hud 0.1.0: removed")),
            "{revoked:?}: {r:?}"
        );
    }
}

#[test]
fn a_takedown_review_names_the_dependents_of_a_removed_version() {
    let mut index = sample();
    let base: Entry = toml::from_str(&hud_entry()).unwrap();
    let mut fireflies = index.projects.remove(0);
    fireflies.id = "fireflies".into();
    fireflies.versions[0].dependencies = serde_json::from_value(serde_json::json!([
        {"id": "coords_hud", "version": "^0.1"}
    ]))
    .unwrap();
    let mut another = fireflies.clone();
    another.id = "another".into();
    another.versions[0].dependencies.as_mut().unwrap()[0].version = "^2".into();
    let mut optional = fireflies.clone();
    optional.id = "optional".into();
    optional.versions[0].dependencies.as_mut().unwrap()[0].optional = true;
    index.projects = vec![fireflies, another, optional];
    index.revoked = vec![Revoked {
        id: "coords_hud".into(),
        sha256: Some(SMOD.into()),
        reason: "test".into(),
        date: String::new(),
    }];
    let notes = takedown_notes(std::slice::from_ref(&base), &[], &index);
    assert_eq!(notes.len(), 1);
    for word in ["fireflies", "0.1.0", "coords_hud", "^0.1"] {
        assert!(notes[0].contains(word), "{notes:?}");
    }
    assert!(takedown_notes(std::slice::from_ref(&base), &index.revoked, &index).is_empty());
    index.revoked[0].sha256 = None;
    assert_eq!(takedown_notes(&[base], &[], &index), notes);
    // `another` requires ^2, which never matched any affected version either way.
}

#[test]
fn a_review_refuses_following_a_repository() {
    let pinned: Entry = toml::from_str(COZY).unwrap();
    let follows: Entry = toml::from_str(COZY_REPO).unwrap();
    // A new entry that follows a repository, and a pinned entry turned into one.
    for base in [vec![], vec![pinned.clone()]] {
        let r = review(&base, std::slice::from_ref(&follows), "acme");
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains("doesn't follow repositories")),
            "{r:?}"
        );
    }
    // An entry from before keeps its repository until it is pinned, and may drop it.
    let r = review(
        std::slice::from_ref(&follows),
        std::slice::from_ref(&follows),
        "acme",
    );
    assert!(r.errors.is_empty(), "{r:?}");
    let r = review(&[follows], &[pinned], "acme");
    assert!(
        !r.errors.iter().any(|e| e.contains("repositories")),
        "{r:?}"
    );
}

#[test]
fn a_project_cannot_change_between_mod_and_native_mod() {
    let content: Entry = toml::from_str(&hud_entry()).unwrap();
    let mut native = content.clone();
    native.kind = Kind::NativeMod;
    for (old, new) in [(&content, &native), (&native, &content)] {
        let r = review(
            std::slice::from_ref(old),
            std::slice::from_ref(new),
            "ludovic111",
        );
        assert!(
            r.errors.iter().any(|e| e.contains("kind can't change")),
            "{r:?}"
        );
    }
}

#[test]
fn native_project_rejects_a_content_only_version() {
    let s = Scratch::new("native-kind");
    s.write(
        "mods/coords_hud.toml",
        &hud_entry().replace("kind = \"mod\"", "kind = \"native-mod\""),
    );
    let errors = s.build().unwrap_err();
    assert!(
        errors.iter().any(|e| e.contains("coords_hud 0.1.0:")),
        "{errors:?}"
    );
}

const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";
const REQUIREMENT_BUILD: &str = "0123456789abcdef0123456789abcdef01234567";

fn package_for_requirement_review(files: &Path, cargo: &str, helper: &str) -> Entry {
    let manifest = format!(
        "id='coords_hud'\nname='Coords'\nversion='0.1.0'\napi='0.2'\n[native]\ngame='26.3'\nsunrise='{REQUIREMENT_BUILD}'\n"
    );
    let bytes = crate::Package::write_files(
        &manifest,
        &[
            ("Cargo.toml", cargo.as_bytes()),
            ("src/lib.rs", b""),
            ("src/helper/Cargo.toml", helper.as_bytes()),
            (
                "Cargo.lock",
                b"version = 4\n[[package]]\nname='serde_json'\nversion='1.0.154'\nsource='registry+https://github.com/rust-lang/crates.io-index'\n",
            ),
        ],
    )
    .unwrap();
    let sha256 = super::super::sha256(&bytes);
    std::fs::write(files.join(&sha256), bytes).unwrap();
    let mut entry: Entry = toml::from_str(&hud_entry()).unwrap();
    entry.kind = Kind::NativeMod;
    entry.versions[0].sha256 = sha256;
    entry.versions[0].sunrise = Some("26.3-0.1.0".into());
    entry
}

fn write_requirement_review_game_lock(sdk_data: &Path) {
    let build = sdk_data.join(REQUIREMENT_BUILD);
    std::fs::create_dir(&build).unwrap();
    let mut lock = String::from("version = 4\n");
    for (name, version) in [
        ("syn", "2.0.119"),
        ("syn", "3.0.5"),
        ("syn", "3.0.6"),
        ("serde_json", "1.0.151"),
        ("log", "0.4.34"),
        ("parser", "3.1.0"),
        ("zero", "0.0.1"),
        ("zero_pre", "0.0.1-alpha.1"),
    ] {
        lock.push_str(&format!(
            "[[package]]\nname='{name}'\nversion='{version}'\nsource='{CRATES_IO}'\n"
        ));
    }
    std::fs::write(build.join("Cargo.lock"), lock).unwrap();
}

#[test]
fn native_requirement_review_warns_on_an_edited_row_and_follows_target_path_dependencies() {
    let files = tempfile::tempdir().unwrap();
    let sdk = tempfile::tempdir().unwrap();
    write_requirement_review_game_lock(sdk.path());
    let old = package_for_requirement_review(
        files.path(),
        "[package]\nname='coords_hud'\nversion='0.1.0'\n[dependencies]\nsyn='3.0.10'\nolder={package='parser',version='=3.0.9'}\nzero_pre='0.0.*'\n\
         [target.'cfg(unix)'.build-dependencies]\nhelper={path='src/helper'}\n",
        "[package]\nname='helper'\nversion='0.1.0'\n[dev-dependencies]\n\
         other_json={package='serde_json',version='=1.0.154'}\nlog='>0.4.34, <0.5'\n",
    );
    let base = std::slice::from_ref(&old);
    let mut review = review(base, base, "ludovic111");
    review.check_native_requirements(base, base, files.path(), Some(sdk.path()), CRATES_IO);
    assert!(
        review.warnings.is_empty() && review.notes.is_empty(),
        "{review:?}"
    );

    let mut edited = old.clone();
    edited.versions[0].changelog = "A note about this version.".into();
    let head = &[edited];
    review.check_native_requirements(base, head, files.path(), Some(sdk.path()), CRATES_IO);
    assert!(
        review.errors.is_empty() && review.notes.is_empty(),
        "{review:?}"
    );
    assert_eq!(review.warnings.len(), 5, "{review:?}");
    for (warning, (manifest, name, req, pins)) in review.warnings.iter().zip([
        // Below the game's pin on its line: a build could only use it by moving the pin back.
        ("Cargo.toml", "parser", "=3.0.9", "3.1.0"),
        ("Cargo.toml", "syn", "3.0.10", "3.0.5, 3.0.6"),
        // A prerelease pin shares its line with its own release.
        ("Cargo.toml", "zero_pre", "0.0.*", "0.0.1-alpha.1"),
        ("src/helper/Cargo.toml", "log", ">0.4.34, <0.5", "0.4.34"),
        ("src/helper/Cargo.toml", "serde_json", "=1.0.154", "1.0.151"),
    ]) {
        for fact in [
            "coords_hud 0.1.0",
            manifest,
            name,
            req,
            pins,
            &REQUIREMENT_BUILD[..12],
        ] {
            assert!(warning.contains(fact), "missing {fact}: {warning}");
        }
    }
    assert!(!review.warnings[1].contains("2.0.119"), "{review:?}");

    // A valid archive at the wrong digest path must not be attributed to the edited row.
    let mut wrong_hash = head[0].clone();
    wrong_hash.versions[0].sha256 = "f".repeat(64);
    std::fs::copy(
        files.path().join(&head[0].versions[0].sha256),
        files.path().join(&wrong_hash.versions[0].sha256),
    )
    .unwrap();
    let mut skipped = Review::default();
    skipped.check_native_requirements(
        base,
        &[wrong_hash],
        files.path(),
        Some(sdk.path()),
        CRATES_IO,
    );
    assert!(skipped.warnings.is_empty(), "{skipped:?}");
}

#[test]
fn native_requirement_review_accepts_game_pins_even_if_the_mod_lock_is_ahead() {
    let files = tempfile::tempdir().unwrap();
    let sdk = tempfile::tempdir().unwrap();
    write_requirement_review_game_lock(sdk.path());
    let entry = package_for_requirement_review(
        files.path(),
        "[package]\nname='coords_hud'\nversion='0.1.0'\n[dependencies]\n\
         serde_json='1'\nsyn='>=4'\nlog='0.5'\nextra='1'\nparser='>=2, <3'\nzero='0.0.2'\n\
         matching_syn={package='syn',version='>=2.0.119, <3.0.5'}\n\
         upstream={package='serde_json',version='1.0.154',git='https://example.invalid/serde'}\n\
         alternate={package='log',version='0.4.35',registry='other'}\n",
        "[package]\nname='unused'\nversion='0.1.0'\n[dependencies]\nsyn='3.0.10'\n",
    );
    let mut review = Review::default();
    review.check_native_requirements(&[], &[entry], files.path(), Some(sdk.path()), CRATES_IO);
    assert_eq!(review, Review::default());
}

#[test]
fn native_requirement_review_notes_a_missing_game_lock_once_per_build() {
    let files = tempfile::tempdir().unwrap();
    let sdk = tempfile::tempdir().unwrap();
    let entry = package_for_requirement_review(
        files.path(),
        "[package]\nname='coords_hud'\nversion='0.1.0'\n[dependencies]\nsyn='3.0.10'\n",
        "",
    );
    let mut head = vec![entry];
    let mut other = head[0].clone();
    other.id = "other".into();
    head.push(other);
    for sdk_data in [None, Some(sdk.path())] {
        let mut review = Review::default();
        review.check_native_requirements(&[], &head, files.path(), sdk_data, CRATES_IO);
        assert!(
            review.warnings.is_empty() && review.errors.is_empty(),
            "{review:?}"
        );
        assert_eq!(review.notes.len(), 1, "{review:?}");
        for fact in [REQUIREMENT_BUILD, "not compared", "Cargo.lock"] {
            assert!(review.notes[0].contains(fact), "missing {fact}: {review:?}");
        }
    }
}

#[test]
fn a_new_icon_is_a_new_address() {
    let s = Scratch::new("icon-address");
    let icon = |s: &Scratch| {
        s.build()
            .unwrap()
            .project("coords_hud")
            .unwrap()
            .icon
            .clone()
            .unwrap()
    };
    let before = icon(&s);
    let (path, version) = before.split_once("?v=").unwrap();
    assert_eq!(
        path,
        "https://horizon-rust.github.io/horizon-mods/icons/coords_hud.png"
    );
    assert_eq!(version.len(), 12, "{before}");
    assert_eq!(icon(&s), before, "the same icon keeps its address");
    s.write_bytes("icons/coords_hud.png", &png(256, 256));
    let after = icon(&s);
    assert!(after.starts_with(path) && after != before, "{after}");
}

#[test]
fn page_fields_and_screenshots_reach_the_index() {
    let s = Scratch::new("page");
    let page = "tags = [\"hud\", \"coordinates\"]\n\
                screenshots = [\"screenshots/coords_hud/1.png\"]\n";
    s.write("mods/coords_hud.toml", &format!("{page}{}", hud_entry()));
    s.write_bytes("screenshots/coords_hud/1.png", &png(1280, 720));
    let built = s.build().unwrap();
    let p = built.project("coords_hud").unwrap();
    assert_eq!(p.categories, ["utility"]);
    assert_eq!(p.tags, ["hud", "coordinates"]);
    assert_eq!(
        p.screenshots,
        ["https://horizon-rust.github.io/horizon-mods/screenshots/coords_hud/1.png"]
    );
    Index::parse(&serde_json::to_string(&built).unwrap()).unwrap();

    // A category of another kind, a screenshot too small, one in another project's folder, one on
    // another host, and an icon that isn't square.
    let bad = "screenshots = [\"screenshots/coords_hud/1.png\", \"screenshots/other/2.png\", \
               \"https://x.test/3.jpg\"]\n";
    let entry = hud_entry().replace("[\"utility\"]", "[\"bloom\"]");
    s.write("mods/coords_hud.toml", &format!("{bad}{entry}"));
    s.write_bytes("screenshots/coords_hud/1.png", &png(100, 100));
    s.write_bytes("icons/coords_hud.png", &png(128, 64));
    let errors = s.build().unwrap_err();
    let has = |part: &str| errors.iter().any(|e| e.contains(part));
    assert!(has("\"bloom\" is not a category of a mod"), "{errors:?}");
    assert!(has("1.png: 100×100 pixels"), "{errors:?}");
    assert!(
        has("\"screenshots/other/2.png\" must be kept in the registry"),
        "{errors:?}"
    );
    assert!(has("the icon is 128×64"), "{errors:?}");
    assert!(
        has("\"https://x.test/3.jpg\" must be kept in the registry"),
        "{errors:?}"
    );
    assert_eq!(errors.len(), 5, "{errors:?}");
}

#[test]
fn only_open_kinds_are_taken() {
    assert!(open(&Kind::Mod).is_ok() && open(&Kind::ResourcePack).is_ok());
    assert!(open(&Kind::Shader).is_ok() && open(&Kind::ShaderPack).is_ok());
    assert!(open(&Kind::DataPack).is_ok());
    assert!(open(&Kind::Other("world".into())).is_err());
    let mut submit: Entry = toml::from_str(COZY).unwrap();
    let dir = std::env::temp_dir();
    let src = Sources {
        registry: &dir,
        files: &dir,
        release: super::super::FILE_HOST,
        site: "https://x.test/",
        generated: String::new(),
        releases: None,
        nightly: None,
        binary_checks: None,
    };
    for page in ["submit", "mine", "account", "policy"] {
        submit.id = page.into();
        assert!(project(&submit, &src).unwrap_err()[0].contains("reserved"));
    }
    assert!(plain_version("1.0.0-beta.2") && !plain_version("../1") && !plain_version("1 0"));
    assert!(plain_date("2026-10-24") && !plain_date("24/10/2026"));
}

const COZY_REPO: &str = r#"id = "cozy"
kind = "modpack"
repo = "acme/cozy-pack"
owners = ["acme"]
description = "Coordinates HUD with a calm render distance, played together as one profile."
categories = ["lightweight"]
icon = "icons/cozy.png"
"#;

#[test]
fn a_project_listed_by_its_repository_takes_its_versions_from_releases() {
    let s = Scratch::new("repo");
    s.write("mods/coords_hud.toml", &hud_entry());
    s.write("mods/cozy.toml", COZY_REPO);
    let spack = fixtures().join("cozy-1.0.0.spack");
    let entry: Entry = toml::from_str(COZY_REPO).unwrap();
    // Listed, nothing released yet: not in the index, and not an error.
    let built = s.build().unwrap();
    assert!(built.project("cozy").is_none());
    // The page file fills what the entry leaves out; the entry's own words win.
    let page = page(
        &entry,
        "name = \"Cozy\"\nsummary = \"Calm.\"\nlicense = \"MIT\"\nauthors = [\"Acme\"]\n",
    )
    .unwrap();
    assert!(
        self::page(&entry, "name = \"Cozy\"")
            .unwrap_err()
            .contains("summary")
    );
    assert!(self::page(&entry, "owners = [\"me\"]").is_err());
    let release = |tag, asset, notes| Release {
        tag,
        published: "2026-10-30",
        notes,
        asset,
        game: "26.3",
        signature: None,
    };
    let v = release_version(
        &entry,
        &release("v1.0.0", "cozy-1.0.0.spack", "Notes."),
        &spack,
    )
    .unwrap();
    assert_eq!(v.version, "1.0.0");
    assert_eq!(v.file.sha256, SPACK);
    assert_eq!(
        v.file.url,
        "https://github.com/acme/cozy-pack/releases/download/v1.0.0/cozy-1.0.0.spack"
    );
    assert_eq!(v.changelog, "Notes.");
    // A name the launcher can't download, a wrong extension, a project with a key but no signature.
    assert!(release_version(&entry, &release("v1/x", "cozy-1.0.0.spack", ""), &spack).is_err());
    assert!(release_version(&entry, &release("v1", "cozy.zip", ""), &spack).is_err());
    let mut keyed = entry.clone();
    keyed.key = Some("RWTVtuGb/GRXcBf3Re5hzM07EzoStr2sLTJjghJia0xX15ULoZRFFZDb".into());
    assert!(
        release_version(&keyed, &release("v1", "cozy-1.0.0.spack", ""), &spack)
            .unwrap_err()
            .contains("sign")
    );
    let cache = Cache {
        page,
        versions: vec![v.clone()],
    };
    std::fs::create_dir_all(s.0.join("repo").join(CACHE)).unwrap();
    s.write(
        "versions/cozy.json",
        &serde_json::to_string(&cache).unwrap(),
    );
    let built = s.build().unwrap();
    let p = built.project("cozy").unwrap();
    assert_eq!(
        (p.name.as_str(), p.authors.clone()),
        ("Cozy", vec!["Acme".to_string()])
    );
    assert_eq!(p.source, "https://github.com/acme/cozy-pack");
    assert_eq!(p.versions, vec![v]);
    // The registry's own words win over the repository's.
    s.write(
        "mods/cozy.toml",
        &format!("{COZY_REPO}name = \"Cozy Pack\"\n"),
    );
    assert_eq!(
        s.build().unwrap().project("cozy").unwrap().name,
        "Cozy Pack"
    );
    // A repository that is not owner/name is refused.
    s.write(
        "mods/cozy.toml",
        &COZY_REPO.replace("acme/cozy-pack", "acme"),
    );
    assert!(
        s.build()
            .unwrap_err()
            .iter()
            .any(|e| e.contains("owner/name"))
    );
}

#[cfg(unix)]
#[test]
fn screenshots_and_caches_refuse_linked_ancestor_directories() {
    use std::{fs, os::unix::fs::symlink};
    let s = Scratch::new("linked-screenshot-ancestor");
    let root = s.0.join("repo");
    let with_screenshot = hud_entry().replace(
        "[[versions]]",
        "screenshots = ['screenshots/coords_hud/view.png']\n\n[[versions]]",
    );
    s.write("mods/coords_hud.toml", &with_screenshot);
    s.write_bytes("screenshots/coords_hud/view.png", &png(640, 360));
    let index = s.build().unwrap();
    assert_eq!(index.project("coords_hud").unwrap().screenshots.len(), 1);
    let outside = s.0.join("outside-screenshots");
    fs::rename(root.join("screenshots"), &outside).unwrap();
    symlink(outside, root.join("screenshots")).unwrap();
    let errors = s.build().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("coords_hud") && e.contains("view.png")),
        "{errors:?}"
    );
    fs::remove_file(root.join("screenshots")).unwrap();
    s.write("mods/coords_hud.toml", &hud_entry());

    s.write(
        "mods/cozy.toml",
        &format!("{COZY_REPO}name='Cozy'\nsummary='Calm.'\nlicense='MIT'\n"),
    );
    let cache = Cache {
        versions: index.project("cozy").unwrap().versions.clone(),
        ..Cache::default()
    };
    s.write_bytes("versions/cozy.json", &serde_json::to_vec(&cache).unwrap());
    assert!(s.build().unwrap().project("cozy").is_some());
    let outside = s.0.join("outside-versions");
    fs::rename(root.join("versions"), &outside).unwrap();
    symlink(outside, root.join("versions")).unwrap();
    let errors = s.build().unwrap_err();
    assert!(errors.iter().any(|e| e.contains("cozy.json")), "{errors:?}");
}

const STABLE_NATIVE: &str = include_str!("../../../tests/index/stable-native/schema-1.json");

#[test]
fn native_archive_records_keep_schema_one_and_exact_lookup() {
    use super::super::NativeBinaryUnavailable;
    let read = Index::parse(STABLE_NATIVE).unwrap();
    assert!(read.left_out.is_empty(), "{:?}", read.left_out);
    read.check().unwrap();
    let v = &read.project("release_probe").unwrap().versions[0];
    let commit = "b".repeat(40);
    let server = v
        .native_binary(&commit, "x86_64-unknown-linux-gnu-server")
        .unwrap();
    assert!(server.file.name.ends_with("-server.tar.xz"));
    assert_eq!(
        v.native_binary(&commit, "aarch64-apple-darwin"),
        Err(NativeBinaryUnavailable::Absent)
    );
    assert!(read.project("independent").is_some());
    assert_eq!(
        Index::parse(&serde_json::to_string(&read).unwrap()).unwrap(),
        read
    );
}

#[test]
fn invalid_optional_binaries_keep_the_source_and_its_dependents() {
    use super::super::NativeBinaryUnavailable;
    use serde_json::Value;
    let original: Value = serde_json::from_str(STABLE_NATIVE).unwrap();
    let path = "/projects/0/versions/0/native_binaries";
    let commit = "b".repeat(40);
    let client = "x86_64-unknown-linux-gnu";
    let server = "x86_64-unknown-linux-gnu-server";
    for kind in [
        "missing signature",
        "invalid signature",
        "unknown field",
        "duplicate",
    ] {
        let mut bad = original.clone();
        let rows = bad.pointer_mut(path).unwrap().as_array_mut().unwrap();
        match kind {
            "missing signature" => {
                rows[0]["file"].as_object_mut().unwrap().remove("signature");
            }
            "invalid signature" => rows[0]["file"]["signature"] = "bad".into(),
            "unknown field" => rows[0]["unexpected"] = "bad".into(),
            "duplicate" => rows.push(rows[0].clone()),
            _ => unreachable!(),
        }
        let text = bad.to_string();
        let read = Index::parse(&text).unwrap();
        let source = read.project("release_probe").unwrap();
        let version = read.installable(source, "26.3").unwrap();
        assert_eq!(version.file.name, "release_probe-1.0.0.smod");
        assert!(
            matches!(version.native_binary(&commit, client), Err(NativeBinaryUnavailable::Refused(reason)) if reason.contains(client))
        );
        assert!(version.native_binary(&commit, server).is_ok());
        assert_eq!(version.verdicts["26.3-0.1.1"].targets, [server]);
        assert!(read.project("dependent").is_some());
        assert!(read.left_out.iter().any(|row| row.reason.contains(client)));
        assert!(
            serde_json::from_str::<Index>(&text)
                .and_then(|index| index.check().map_err(serde::de::Error::custom))
                .is_err()
        );
    }
    let mut bad = original;
    *bad.pointer_mut(path).unwrap() = Value::Bool(false);
    let read = Index::parse(&bad.to_string()).unwrap();
    let source = read.project("release_probe").unwrap();
    let version = read.installable(source, "26.3").unwrap();
    assert_eq!(version.file.name, "release_probe-1.0.0.smod");
    assert!(version.native_binary(&commit, client).is_err());
    assert!(version.native_binary(&commit, server).is_err());
    assert!(
        read.left_out
            .iter()
            .any(|row| row.id == source.id
                && row.version.as_deref() == Some(version.version.as_str()))
    );
}

#[test]
fn duplicate_fields_refuse_only_the_affected_binary() {
    // Add a second `size` to the first binary, leaving the server binary unchanged.
    let original: serde_json::Value = serde_json::from_str(STABLE_NATIVE).unwrap();
    let size = &original["projects"][0]["versions"][0]["native_binaries"][0]["file"]["size"];
    let text = STABLE_NATIVE.replacen(
        &format!("\"size\": {size},"),
        &format!("\"size\": {size}, \"size\": {size},"),
        1,
    );
    assert_ne!(text, STABLE_NATIVE);
    let read = Index::parse(&text).unwrap();
    let source = read.project("release_probe").unwrap();
    let version = read.installable(source, "26.3").unwrap();
    let commit = "b".repeat(40);
    let client = "x86_64-unknown-linux-gnu";
    let server = "x86_64-unknown-linux-gnu-server";
    assert!(matches!(
        version.native_binary(&commit, client),
        Err(super::super::NativeBinaryUnavailable::Refused(_))
    ));
    assert!(version.native_binary(&commit, server).is_ok());
    assert_eq!(version.verdicts["26.3-0.1.1"].targets, [server]);
}

#[test]
fn reading_an_index_drops_the_taken_down_host_but_keeps_the_source() {
    let commit = "b".repeat(40);
    let client = "x86_64-unknown-linux-gnu";
    let server = "x86_64-unknown-linux-gnu-server";
    let mut old: Index = serde_json::from_str(STABLE_NATIVE).unwrap();
    let original = &old.project("release_probe").unwrap().versions[0];
    old.revoked.push(Revoked {
        id: "release_probe".into(),
        sha256: Some(
            original
                .native_binary(&commit, client)
                .unwrap()
                .file
                .sha256
                .clone(),
        ),
        reason: "Withdrawn test archive.".into(),
        date: String::new(),
    });
    let read = Index::parse(&serde_json::to_string(&old).unwrap()).unwrap();
    let source = read.project("release_probe").unwrap();
    let version = read.installable(source, "26.3").unwrap();
    assert_eq!(
        version.file,
        old.project(&source.id).unwrap().versions[0].file
    );
    assert!(version.native_binary(&commit, client).is_err());
    assert!(version.native_binary(&commit, server).is_ok());
    assert_eq!(version.verdicts["26.3-0.1.1"].targets, [server]);
}

#[test]
fn native_binary_build_limits() {
    let mut index = Index::parse(STABLE_NATIVE).unwrap();
    let version = &mut index.projects[0].versions[0];
    version.native_binaries[0].build = "a".repeat(128);
    assert!(index.check().is_ok());
    for build in ["a".repeat(129), "build\nid".into()] {
        let mut bad = index.clone();
        bad.projects[0].versions[0].native_binaries[0].build = build;
        assert!(bad.check().unwrap_err().contains("invalid native build"));
    }
}

#[test]
fn an_owner_can_append_a_build_but_not_change_signed_source_or_published_builds() {
    let mut before: Entry = toml::from_str(&hud_entry()).unwrap();
    before.versions[0].sunrise = Some("26.3-0.1.0".into());
    let row = |release: &str| builds::BuildRow {
        sunrise: release.into(),
        binaries: format!(
            "https://github.com/author/project/releases/download/v0.1.0/coords_hud-0.1.0+{release}.binaries.json"
        ),
        sha256: "a".repeat(64),
        signature: String::new(),
    };
    before.versions[0].builds.push(row("26.3-0.1.0"));
    let before = [before];
    let mut after = before.clone();
    after[0].versions[0].builds.push(row("26.3-0.1.1"));
    let result = review(&before, &after, "Ludovic111");
    assert!(result.errors.is_empty());
    assert_eq!(result.build_reviews[0].status, ReviewStatus::Routine);
    assert_eq!(
        review(&before, &after, "stranger").build_reviews[0].status,
        ReviewStatus::ReviewRequired
    );
    let mut mixed = after.clone();
    mixed[0].summary.push_str(" Edited.");
    assert_eq!(
        review(&before, &mixed, "Ludovic111").build_reviews[0].status,
        ReviewStatus::ReviewRequired
    );
    let mut replaced = after.clone();
    replaced[0].versions[0].builds.remove(0);
    assert!(review(&before, &replaced, "Ludovic111").errors[0].contains("append-only"));
    after[0].versions[0].signature = Some("changed".into());
    assert!(review(&before, &after, "Ludovic111").errors[0].contains("never changes"));
}

#[test]
fn unmet_dependencies_name_the_versions_listed_for_the_game() {
    let hud: Entry = toml::from_str(&hud_entry()).unwrap();
    let cozy: Entry = toml::from_str(COZY).unwrap();
    let mut checkout = Checkout {
        directory: PathBuf::new(),
        entries: vec![hud, cozy],
        collections: Vec::new(),
        revoked: Vec::new(),
        caches: BTreeMap::new(),
    };
    let required = |id: &str, version: &str| super::super::Dependency {
        id: id.into(),
        version: version.into(),
        optional: false,
        side: None,
    };
    let dependencies = [
        required("coords_hud", "^0.1"),
        required("cozy", "^1"),
        required("absent", "^1"),
        super::super::Dependency {
            optional: true,
            ..required("maybe", "^1")
        },
    ];
    let unmet = |checkout: &Checkout, game: &str, dependencies: &[super::super::Dependency]| {
        checkout
            .unmet_dependencies(game, dependencies)
            .into_iter()
            .map(|u| (u.id, u.listed))
            .collect::<Vec<_>>()
    };
    let none = Vec::<String>::new;
    // A modpack is no mod to depend on; an optional dependency is never required.
    assert_eq!(
        unmet(&checkout, "26.3", &dependencies),
        [("cozy".into(), none()), ("absent".into(), none())]
    );
    // Only versions for the same game count.
    assert!(unmet(&checkout, "26.4", &dependencies).contains(&("coords_hud".into(), none())));
    // A version outside the range is named, so the author sees what is there.
    assert_eq!(
        unmet(&checkout, "26.3", &[required("coords_hud", "^0.2")]),
        [("coords_hud".into(), vec!["0.1.0".to_owned()])]
    );
    // A short label is padded as the resolver pads it.
    checkout.entries[0].versions[0].version = "0.1".into();
    assert!(unmet(&checkout, "26.3", &[required("coords_hud", "^0.1")]).is_empty());
    // A version taken down by hash is not listed.
    checkout.revoked.push(Revoked {
        id: "coords_hud".into(),
        sha256: Some(SMOD.into()),
        reason: "lost key".into(),
        date: String::new(),
    });
    assert_eq!(
        unmet(&checkout, "26.3", &[required("coords_hud", "^0.1")]),
        [("coords_hud".into(), none())]
    );
}

#[test]
fn a_cached_native_release_meets_a_dependency_until_its_project_is_taken_down() {
    let mut lib: Entry = toml::from_str(&hud_entry()).unwrap();
    lib.id = "lib".into();
    lib.kind = Kind::NativeMod;
    lib.repo = Some("ada/lib".into());
    lib.versions.clear();
    let mut release = sample().projects.remove(0).versions.remove(0);
    release.version = "2.1.0".into();
    release.game = "26.3".into();
    let mut checkout = Checkout {
        directory: PathBuf::new(),
        entries: vec![lib],
        collections: Vec::new(),
        revoked: Vec::new(),
        caches: BTreeMap::from([(
            "lib".to_owned(),
            Cache {
                page: Page::default(),
                versions: vec![release],
            },
        )]),
    };
    let dependency = [super::super::Dependency {
        id: "lib".into(),
        version: "^2".into(),
        optional: false,
        side: None,
    }];
    assert!(checkout.unmet_dependencies("26.3", &dependency).is_empty());
    checkout.revoked.push(Revoked {
        id: "lib".into(),
        sha256: None,
        reason: "malware".into(),
        date: String::new(),
    });
    let unmet = checkout.unmet_dependencies("26.3", &dependency);
    assert_eq!(unmet.len(), 1);
    assert_eq!((unmet[0].id.as_str(), unmet[0].listed.len()), ("lib", 0));
}

#[test]
fn the_screenshot_rule_covers_new_projects_new_versions_and_exempts_libraries() {
    let base: Entry = toml::from_str(&hud_entry()).unwrap();
    assert!(screenshot_required(None, &base));
    assert!(!screenshot_required(Some(&base), &base));
    let mut update = base.clone();
    let mut next = update.versions[0].clone();
    next.version = "0.2.0".into();
    update.versions.insert(0, next);
    assert!(screenshot_required(Some(&base), &update));
    // Only a mod or native-mod in the `library` category is exempt.
    for kind in [Kind::Mod, Kind::NativeMod] {
        update.kind = kind;
        assert!(screenshot_required(None, &update));
        assert!(screenshot_required(Some(&base), &update));
        update.categories.push("library".into());
        assert!(!screenshot_required(None, &update));
        assert!(!screenshot_required(Some(&base), &update));
        update.categories.pop();
    }
    update.categories = vec!["library".into()];
    update.kind = Kind::Modpack;
    assert!(screenshot_required(None, &update));
}
