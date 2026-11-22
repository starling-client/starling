use super::*;
use crate::index::{
    Index,
    registry::{
        self,
        tests::{Scratch, png},
    },
    sha256,
};
use std::io::{Cursor, Write};

const ID: &str = "sunrise_creatures";
const COLLECTION: &str = r#"id = "sunrise_creatures"
name = "Sunrise Creatures"
summary = "Fireflies and butterflies with the library they share."
owners = ["EAnathos"]
icon = "icons/sunrise_creatures.png"
members = [
  { id = "creature" },
  { id = "fireflies" },
  { id = "butterflies", default = false },
]
"#;

fn mentions_id(text: &str, id: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|token| token == id)
}

fn write_mod(s: &Scratch, id: &str, name: &str, owner: &str) {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        crate::manifest::MANIFEST,
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    write!(
        zip,
        "id='{id}'\nname='{name}'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n"
    )
    .unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let hash = sha256(&bytes);
    std::fs::write(s.0.join("files").join(&hash), &bytes).unwrap();
    s.write_bytes(&format!("icons/{id}.png"), &png(128, 128));
    s.write(
        &format!("mods/{id}.toml"),
        &format!(
            r#"
id = "{id}"
kind = "mod"
name = "{name}"
summary = "An example mod for collection validation."
description = "An example content mod, for testing the registry's collection rules."
owners = ["{owner}"]
license = "MIT"
source = "https://example.test/{id}"
icon = "icons/{id}.png"
categories = ["utility"]
[[versions]]
version = "1.0.0"
published = "2026-11-18"
game = "26.3"
file = "{id}.smod"
sha256 = "{hash}"
"#
        ),
    );
}

fn checkout(name: &str) -> Scratch {
    let s = Scratch::new(name);
    write_mod(&s, "creature", "Sunrise Creatures", "EAnathos");
    write_mod(&s, "fireflies", "Fireflies", "EAnathos");
    write_mod(&s, "butterflies", "Butterflies", "EAnathos");
    s.write_bytes("collections/sunrise_creatures.toml", COLLECTION.as_bytes());
    s.write_bytes("icons/sunrise_creatures.png", &png(128, 128));
    s
}

#[test]
fn a_collection_and_project_can_have_the_same_name_but_have_distinct_ids() {
    let s = checkout("collection-format");
    let index = s.build().unwrap();
    let collection = index.collection(ID).unwrap();
    assert_eq!(collection.name, index.project("creature").unwrap().name);
    assert!(collection.icon.contains("icons/sunrise_creatures.png?v="));
    let json = serde_json::to_value(collection).unwrap();
    assert_eq!(
        json["members"],
        serde_json::json!([
            {"id": "creature"}, {"id": "fireflies"}, {"id": "butterflies", "default": false}
        ])
    );
    for absent in ["owner_ids", "file", "versions"] {
        assert!(json.get(absent).is_none());
    }
    let text = serde_json::to_string(&index).unwrap();
    assert_eq!(Index::parse(&text).unwrap(), index);
    let mut index = index;
    index.projects.retain(|p| p.id != "creature");
    let read = Index::parse(&serde_json::to_string(&index).unwrap()).unwrap();
    assert!(read.collection(ID).is_none());
    assert!(read.project("fireflies").is_some());
    assert!(
        read.left_out
            .iter()
            .any(|row| row.id == ID && mentions_id(&row.reason, "creature"))
    );
}

#[test]
fn a_reader_skips_malformed_collection_rows_without_losing_projects() {
    let s = checkout("collection-bad-json");
    let index = s.build().unwrap();
    let mut input = serde_json::to_value(&index).unwrap();
    let rows = input["collections"].as_array_mut().unwrap();
    let mut wrong_type = rows[0].clone();
    wrong_type["id"] = "wrong_type".into();
    wrong_type["members"][0]["default"] = "no".into();
    let mut missing = rows[0].clone();
    missing["id"] = "missing_icon".into();
    missing.as_object_mut().unwrap().remove("icon");
    // A reader ignores a future member field, unlike the checkout reader.
    rows[0]["members"][0]["future"] = true.into();
    rows.extend([wrong_type, missing]);
    let text = serde_json::to_string(&input).unwrap();
    let read = Index::parse(&text).unwrap();
    assert_eq!(read.projects, index.projects);
    assert_eq!(read.collections, index.collections);
    for id in ["wrong_type", "missing_icon"] {
        assert!(
            read.left_out
                .iter()
                .any(|row| row.id == id && row.version.is_none() && !row.reason.is_empty())
        );
    }
    assert!(serde_json::from_str::<Index>(&text).is_err());
}

#[test]
fn a_nightly_only_project_keeps_its_id_from_a_collection() {
    let s = checkout("collection-nightly-id");
    let mut index = s.build().unwrap();
    let mut project = index.project("creature").unwrap().clone();
    project.id = "source_only".into();
    project.versions.clear();
    project.nightly = Some(crate::index::Nightly {
        repo: "author/example".into(),
        path: None,
        metadata: Default::default(),
    });
    index.projects.push(project);
    assert_eq!(index.check(), Ok(()));
    index.projects.last_mut().unwrap().id = ID.into();
    let error = index.check().unwrap_err();
    assert!(error.contains(ID), "{error}");
}

#[test]
fn a_pruned_project_still_reserves_its_id() {
    let s = checkout("collection-pruned-id");
    let mut index = s.build().unwrap();
    let mut project = index.project("cozy").unwrap().clone();
    project.id = ID.into();
    project.versions.clear();
    index.projects.push(project);
    let read = Index::parse(&serde_json::to_string(&index).unwrap()).unwrap();
    assert!(read.project(ID).is_none());
    assert!(read.collection(ID).is_none());
    assert!(read.project("creature").is_some());
    assert_eq!(
        read.left_out
            .iter()
            .filter(|row| row.id == ID && row.version.is_none())
            .count(),
        2
    );
}

#[test]
fn collection_entries_are_bounded_regular_toml_named_by_their_ids() {
    let s = checkout("collection-loader");
    s.write("collections/wrong.toml", COLLECTION);
    let oversized = format!(
        "{}#{}",
        COLLECTION.replace(ID, "large"),
        " ".repeat(registry::MAX_COLLECTION_BYTES as usize)
    );
    s.write("collections/large.toml", &oversized);
    std::fs::create_dir(s.0.join("repo/collections/dir.toml")).unwrap();
    let errors = registry::load(&s.0.join("repo")).unwrap_err();
    for file in ["wrong.toml", "large.toml", "dir.toml"] {
        assert!(errors.iter().any(|e| e.contains(file)), "{errors:?}");
    }
}

#[test]
fn a_checkout_rejects_unknown_member_fields() {
    let s = checkout("collection-member-typo");
    let file = "collections/sunrise_creatures.toml";
    s.write(
        file,
        &COLLECTION.replace("default = false", "defualt = false"),
    );
    let errors = registry::load(&s.0.join("repo")).unwrap_err();
    assert!(errors.iter().any(|e| e.contains(file)), "{errors:?}");
}

#[test]
fn the_publisher_refuses_invalid_collection_links_and_selection() {
    let s = checkout("collection-check");
    let index = s.build().unwrap();
    let valid = index.collection(ID).unwrap();
    let check = |collection: Collection, member: Option<&str>| {
        let mut proposed = index.clone();
        let id = collection.id.clone();
        proposed.collections = vec![collection];
        let error = proposed.check().unwrap_err();
        assert!(
            error.contains(&id) && member.is_none_or(|id| error.contains(id)),
            "{error}"
        );
    };
    for id in ["creature", "submit"] {
        check(
            Collection {
                id: id.into(),
                ..valid.clone()
            },
            None,
        );
    }
    check(
        Collection {
            members: valid.members[..1].to_vec(),
            ..valid.clone()
        },
        None,
    );
    let mut row = valid.clone();
    row.members.push(row.members[0].clone());
    check(row, Some("creature"));
    let mut row = valid.clone();
    row.members[0].id = "cozy".into(); // A modpack, not a member project.
    check(row, Some("cozy"));
    let mut row = valid.clone();
    row.members[0].id = ID.into(); // Collections cannot be members, including self.
    check(row, Some(ID));
    let mut row = valid.clone();
    row.members.iter_mut().for_each(|m| m.default = false);
    check(row, None);

    // The lightweight review detects the removal without package downloads too.
    let root = s.0.join("repo");
    std::fs::remove_file(root.join("mods/creature.toml")).unwrap();
    let errors = registry::load(&root).unwrap().check_collections(&Sources {
        registry: &root,
        files: &root,
        release: crate::index::FILE_HOST,
        site: "https://example.test/",
        generated: String::new(),
        releases: None,
        nightly: None,
        binary_checks: None,
    });
    assert!(
        errors
            .iter()
            .any(|e| e.contains(ID) && e.contains("creature")),
        "{errors:?}"
    );
}

#[test]
fn a_collection_cannot_exceed_the_limit_even_with_distinct_listed_members() {
    use crate::index::MAX_COLLECTION_MEMBERS;
    let s = checkout("collection-member-limit");
    let mut index = s.build().unwrap();
    let model = index.project("creature").unwrap().clone();
    index.collections[0].members.clear();
    for n in 0..=MAX_COLLECTION_MEMBERS {
        let id = format!("creature_{n}");
        index.projects.push(crate::index::Project {
            id: id.clone(),
            ..model.clone()
        });
        index.collections[0]
            .members
            .push(Member { id, default: true });
    }
    let error = index.check().unwrap_err();
    assert!(error.contains(ID), "{error}");
    index.collections[0].members.pop();
    assert_eq!(index.check(), Ok(()));
}

#[test]
fn a_takedown_names_the_membership_even_when_its_hash_matches_no_release() {
    let s = checkout("collection-takedown");
    let mut index = s.build().unwrap();
    let base = registry::load(&s.0.join("repo")).unwrap();
    let listed_hash = index.project("creature").unwrap().versions[0]
        .file
        .sha256
        .clone();
    for hash in [listed_hash, "0".repeat(64)] {
        index.revoked.push(crate::index::Revoked {
            id: "creature".into(),
            sha256: Some(hash),
            reason: "test".into(),
            date: String::new(),
        });
        let notes = registry::takedown_notes(&base.entries, &base.revoked, &index);
        assert_eq!(notes.len(), 1);
        let note = &notes[0];
        assert!(
            mentions_id(note, ID) && mentions_id(note, "creature"),
            "{note}"
        );
        index.revoked.pop();
    }
}

#[test]
fn only_owners_membership_edits_for_their_projects_are_routine() {
    let s = checkout("collection-owner");
    write_mod(&s, "cyclops", "Cyclops", "EAnathos");
    write_mod(&s, "vulture", "Vulture", "someone_else");
    let root = s.0.join("repo");
    let base = registry::load(&root).unwrap();
    let owner = "eanathos";
    let review = |edit: &dyn Fn(&mut CollectionEntry), who: &str| {
        let mut head = registry::load(&root).unwrap();
        edit(&mut head.collections[0]);
        let changes = review_collections(&base, &head, who);
        assert_eq!(changes.len(), 1);
        changes.into_iter().next().unwrap()
    };
    let add = |id: &str| {
        let id = id.to_owned();
        move |c: &mut CollectionEntry| {
            c.members.push(Member {
                id: id.clone(),
                default: false,
            })
        }
    };
    let remove = |c: &mut CollectionEntry| {
        c.members.retain(|m| m.id != "butterflies");
    };
    assert_eq!(review(&add("cyclops"), owner).status, ReviewStatus::Routine);
    assert_eq!(review(&remove, owner).status, ReviewStatus::Routine);
    let foreign = review(&add("vulture"), owner);
    assert_eq!(foreign.status, ReviewStatus::ReviewRequired);
    assert!(foreign.reason.contains("vulture"));
    let non_owner = review(&add("cyclops"), "someone_else");
    assert_eq!(non_owner.status, ReviewStatus::ReviewRequired);
    assert!(non_owner.reason.contains("someone_else"));
    for edit in [
        (|c: &mut CollectionEntry| c.members[0].default = false) as fn(&mut CollectionEntry),
        |c: &mut CollectionEntry| c.name = "Other title".into(),
        |c: &mut CollectionEntry| c.owners.push("new_owner".into()),
        |c: &mut CollectionEntry| c.members.swap(0, 1),
    ] {
        assert_eq!(review(&edit, owner).status, ReviewStatus::ReviewRequired);
    }
}

#[test]
fn replacing_an_icon_at_the_same_path_requires_review() {
    let base = checkout("collection-icon-old");
    let head = checkout("collection-icon-new");
    head.write_bytes("icons/sunrise_creatures.png", &png(256, 256));
    let base = registry::load(&base.0.join("repo")).unwrap();
    let head = registry::load(&head.0.join("repo")).unwrap();
    let changes = review_collections(&base, &head, "EAnathos");
    assert_eq!(changes[0].status, ReviewStatus::ReviewRequired);
    assert_eq!(changes[0].id, ID);
}
