use super::*;
use std::cell::RefCell;

fn manifest(id: &str, version: &str, dependencies: &str) -> Manifest {
    Manifest::parse(&format!(
        "id='{id}'\nname='Test Mod'\nversion='{version}'\napi='0.2'\ncapabilities=['registries']\n\
         [native]\ngame='26.3'\nsunrise='{}'\n[dependencies]\n{dependencies}",
        "a".repeat(40)
    ))
    .unwrap()
}

/// A registry of `(project, mod, versions)` rows whose packages are never downloaded here.
fn registry(projects: &[(&str, &str, &[&str])]) -> Registry {
    let projects: Vec<_> = projects
        .iter()
        .map(|(id, mod_id, versions)| {
            let releases: Vec<_> = versions
                .iter()
                .map(|version| {
                    serde_json::json!({"version": version, "workflow": "w",
                        "file": {"url": "u", "size": 1, "sha256": format!("{id}-{version}"), "attestation": "a"}})
                })
                .collect();
            serde_json::json!({"id": id, "mod": mod_id, "kind": "native-mod",
                "repository": {"id": 1, "name": format!("author/{id}")}, "releases": releases})
        })
        .collect();
    let index = serde_json::from_value(
        serde_json::json!({"schema": 2, "serial": 1, "projects": projects, "revoked": []}),
    )
    .unwrap();
    Registry::fixture(
        index,
        |url, _| panic!("downloaded {url}"),
        |_, _, _, _| panic!("attested"),
    )
}

fn libraries(local: &[Manifest], registry: Option<Registry>) -> Libraries {
    Libraries {
        local: local
            .iter()
            .map(|manifest| {
                let package = Package {
                    manifest: manifest.clone(),
                    icon: None,
                };
                (
                    manifest.id.clone(),
                    LocalSource {
                        path: PathBuf::from(&manifest.id),
                        package,
                    },
                )
            })
            .collect(),
        registry,
        side: Side::Client,
    }
}

/// Accept every release, giving it the dependencies `rows` lists for its mod.
fn accept<'a>(rows: &'a [(&str, &str)]) -> impl FnMut(&'a Project, &'a Release) -> Take<'a> + 'a {
    move |project, release| {
        let dependencies = rows
            .iter()
            .find(|(id, _)| *id == project.mod_id)
            .map_or("", |(_, deps)| deps);
        Ok(Ok(Taken {
            project,
            release,
            archive: PathBuf::new(),
            manifest: manifest(&project.mod_id, &release.version.to_string(), dependencies),
            files: None,
        }))
    }
}

fn selected(chosen: &BTreeMap<String, Source<'_>>) -> Vec<String> {
    chosen
        .iter()
        .map(|(id, source)| match source {
            Source::Local(_) => format!("{id} local"),
            Source::Registry(taken) => {
                format!("{id} {} {}", taken.project.id, taken.release.version)
            }
        })
        .collect()
}

#[test]
fn a_local_library_wins_and_its_version_cannot_be_replaced() {
    let addon = manifest("addon", "1.0.0", "library='^1'");
    for (version, fits) in [("1.1.0", true), ("2.0.0", false)] {
        let libraries = libraries(
            &[manifest("library", version, "")],
            Some(registry(&[("Ab3dE9xQ", "library", &["1.2.0"])])),
        );
        let result = libraries.select(&addon, accept(&[]));
        if fits {
            assert_eq!(selected(&result.unwrap()), ["library local"]);
        } else {
            let error = result.err().unwrap();
            assert!(
                error.contains("addon needs library ^1, but library 2.0.0 was selected"),
                "{error}"
            );
        }
    }
}

#[test]
fn a_missing_library_names_the_mod_range_and_both_sources() {
    let addon = manifest("addon", "1.0.0", "missing_library='=1.2.3'");
    let libraries = libraries(&[], Some(registry(&[])));
    let error = libraries.select(&addon, accept(&[])).err().unwrap();
    for required in [
        "addon needs missing_library =1.2.3",
        "--with <path>",
        "Sunrise mod registry",
    ] {
        assert!(error.contains(required), "{error}");
    }
}

#[test]
fn releases_are_tried_newest_first_and_only_as_far_as_the_range_needs() {
    let addon = manifest("addon", "1.0.0", "library='^1'");
    let libraries = libraries(
        &[],
        Some(registry(&[(
            "Ab3dE9xQ",
            "library",
            &["1.0.0", "2.0.0", "1.1.0"],
        )])),
    );
    for (unfit, expected, probes) in [
        (
            Some("1.1.0"),
            "library Ab3dE9xQ 1.0.0",
            &["1.1.0", "1.0.0"][..],
        ),
        (None, "library Ab3dE9xQ 1.1.0", &["1.1.0"][..]),
    ] {
        let calls = RefCell::new(Vec::new());
        let mut take = accept(&[]);
        let chosen = libraries
            .select(&addon, |project, release| {
                calls.borrow_mut().push(release.version.to_string());
                if Some(release.version.to_string().as_str()) == unfit {
                    return Ok(Err("does not fit".into()));
                }
                take(project, release)
            })
            .unwrap();
        assert_eq!(selected(&chosen), [expected]);
        assert_eq!(calls.into_inner(), probes);
    }
    let error = libraries
        .select(&addon, |_, _| Ok(Err("does not fit".into())))
        .err()
        .unwrap();
    assert!(
        error.contains("no release of registry project Ab3dE9xQ")
            && error.contains("library 1.0.0: does not fit"),
        "{error}"
    );
}

#[test]
fn a_local_content_package_satisfies_a_required_edge_without_the_registry() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("content");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join(sunrise_mod_package::manifest::MANIFEST),
        "id='content'\nname='Content'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n",
    )
    .unwrap();
    let path = root.path().join("content.smod");
    std::fs::write(&path, Package::pack_directory(&dir).unwrap()).unwrap();
    let addon = manifest("addon", "1.0.0", "content='^1'");
    let cache = root.path().join("cache");
    let libraries = Libraries::read(&addon, &[&path], Side::Client, &cache).unwrap();
    assert!(libraries.registry.is_none() && !cache.exists());
    let chosen = libraries
        .select(&addon, |_, _| panic!("no registry release"))
        .unwrap();
    assert_eq!(selected(&chosen), ["content local"]);
}

#[test]
fn required_edges_reach_through_registry_libraries_and_optional_edges_add_nothing() {
    let addon = manifest(
        "addon",
        "1.0.0",
        "local='^1'\nmissing_optional={version='^1', optional=true}",
    );
    let local = manifest(
        "local",
        "1.1.0",
        "bridge='^1'\nserver_only={version='^1', side='server'}",
    );
    let rows = [("bridge", "base='^1'\nextra={version='^1', optional=true}")];
    let registry = || {
        Some(registry(&[
            ("BridgeP1", "bridge", &["1.0.0"]),
            ("BaseP001", "base", &["1.0.0"]),
            ("ExtraP01", "extra", &["1.0.0"]),
        ]))
    };
    let chosen = libraries(std::slice::from_ref(&local), registry());
    assert_eq!(
        selected(&chosen.select(&addon, accept(&rows)).unwrap()),
        [
            "base BaseP001 1.0.0",
            "bridge BridgeP1 1.0.0",
            "local local"
        ]
    );
    // A library given for an optional edge takes part, and the edge's range still applies.
    for (version, fits) in [("1.1.0", true), ("2.0.0", false)] {
        let with_extra = libraries(&[local.clone(), manifest("extra", version, "")], registry());
        let result = with_extra.select(&addon, accept(&rows));
        if fits {
            assert_eq!(
                selected(&result.unwrap()),
                [
                    "base BaseP001 1.0.0",
                    "bridge BridgeP1 1.0.0",
                    "extra local",
                    "local local"
                ]
            );
        } else {
            let error = result.err().unwrap();
            assert!(error.contains("bridge needs extra ^1"), "{error}");
        }
    }
}

#[test]
fn a_slot_with_several_providers_is_named_and_a_project_taken_down_is_not_one() {
    let addon = manifest("addon", "1.0.0", "library='^1'");
    let mut two = registry(&[
        ("Ab3dE9xQ", "library", &["1.0.0"]),
        ("Zz91kLmN", "library", &["1.0.0"]),
    ]);
    let error = libraries(&[], None)
        .select(&addon, accept(&[]))
        .err()
        .unwrap();
    assert!(error.contains("--with <path>"), "{error}");
    let both = Libraries {
        registry: Some(two),
        ..libraries(&[], None)
    };
    let error = both.select(&addon, accept(&[])).err().unwrap();
    assert!(
        error.contains("Ab3dE9xQ from author/Ab3dE9xQ, Zz91kLmN from author/Zz91kLmN"),
        "{error}"
    );
    two = both.registry.unwrap();
    two.index.revoked =
        serde_json::from_value(serde_json::json!([{"project": "Zz91kLmN", "sha256": null}]))
            .unwrap();
    let one = libraries(&[], Some(two));
    assert_eq!(
        selected(&one.select(&addon, accept(&[])).unwrap()),
        ["library Ab3dE9xQ 1.0.0"]
    );
}
