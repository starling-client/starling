use super::*;

fn index(project: &str, release: serde_json::Value, revoked: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": 2, "serial": 1, "revoked": revoked,
        "projects": [{"id": project, "mod": "library", "kind": "native-mod",
            "repository": {"id": 1, "name": "author/library"}, "releases": [release]}]
    }))
    .unwrap()
}

fn release(version: &str, sha256: &str, size: u64) -> serde_json::Value {
    serde_json::json!({"version": version, "workflow": "w",
        "file": {"url": "u", "size": size, "sha256": sha256, "attestation": "a"}})
}

#[test]
fn ids_digests_sizes_and_versions_outside_their_shapes_refuse_the_whole_index() {
    let digest = "a".repeat(64);
    let good = release("1.2.0", &digest, 10);
    let parsed = Index::parse(&index("Ab3dE9xQ", good.clone(), serde_json::json!([]))).unwrap();
    assert_eq!(
        parsed.projects[0].releases[0].version,
        semver::Version::new(1, 2, 0)
    );
    for id in ["../x", "/abs/xyz", "..", "Ab3dE9x", "Ab3dE9x/"] {
        let error = Index::parse(&index(id, good.clone(), serde_json::json!([]))).unwrap_err();
        assert!(error.contains("project ID"), "{id}: {error}");
    }
    for sha in [
        "..",
        "a/b",
        &"a".repeat(63),
        &"A".repeat(64),
        &format!("{}/", "a".repeat(63)),
    ] {
        let error = Index::parse(&index(
            "Ab3dE9xQ",
            release("1.2.0", sha, 10),
            serde_json::json!([]),
        ))
        .unwrap_err();
        assert!(error.contains("Ab3dE9xQ 1.2.0: SHA-256"), "{sha}: {error}");
    }
    for size in [0, MAX_FILE + 1, u64::MAX] {
        let error = Index::parse(&index(
            "Ab3dE9xQ",
            release("1.2.0", &digest, size),
            serde_json::json!([]),
        ))
        .unwrap_err();
        assert!(error.contains("Ab3dE9xQ 1.2.0: size"), "{size}: {error}");
    }
    let error = Index::parse(&index(
        "Ab3dE9xQ",
        release("1.x", &digest, 10),
        serde_json::json!([]),
    ))
    .unwrap_err();
    assert!(error.contains("release version \"1.x\""), "{error}");
    for revoked in [
        serde_json::json!([{"project": "../x", "sha256": null}]),
        serde_json::json!([{"project": "Ab3dE9xQ", "sha256": ".."}]),
    ] {
        let error = Index::parse(&index("Ab3dE9xQ", good.clone(), revoked)).unwrap_err();
        assert!(error.contains("takedown"), "{error}");
    }
}

#[test]
fn a_signed_file_without_four_signature_lines_is_refused_before_any_check() {
    let error = Index::verify(b"untrusted comment\nsig\ntrusted comment\n{}", "key").unwrap_err();
    assert!(error.contains("four signature lines"), "{error}");
}
