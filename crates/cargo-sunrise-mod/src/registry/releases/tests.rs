use super::*;
use serde_json::json;
use sunrise_mod_package::index::registry::parse_releases;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
const D: &str = "dddddddddddddddddddddddddddddddddddddddd";
const STABLE: &str = "26.3-0.1.0";

fn release(id: u64, tag: &str) -> serde_json::Value {
    json!({
        "id": id, "tag_name": tag, "draft": false, "prerelease": false,
        "published_at": "2026-11-21T03:04:05Z",
        "target_commitish": "main"
    })
}

fn body(value: serde_json::Value) -> Response {
    Response {
        body: serde_json::to_vec(&value).unwrap(),
        links: Vec::new(),
    }
}

fn full_page() -> Vec<serde_json::Value> {
    (1000..1000 + PAGE_SIZE as u64)
        .map(|id| release(id, "nightly"))
        .collect()
}

fn page(rows: Vec<serde_json::Value>, links: &[&str]) -> Response {
    let mut response = body(rows.into());
    response.links = links.iter().map(|link| (*link).to_owned()).collect();
    response
}

fn object(kind: &str, sha: &str) -> Response {
    body(json!({"object": {"type": kind, "sha": sha}}))
}

#[test]
fn stable_releases_are_filtered_sorted_and_peeled_across_pages() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("releases.json");
    let mut requests = Vec::new();
    write_list(&out, |url| {
        requests.push(url.to_owned());
        if url == format!("{SDK_RELEASE_LIST}&page=1") {
            let mut draft = release(3, "26.3-0.1.2");
            draft["draft"] = true.into();
            let mut prerelease = release(4, "26.3-0.1.3");
            prerelease["prerelease"] = true.into();
            let mut unpublished = release(5, "26.3-0.2.0");
            unpublished["published_at"] = serde_json::Value::Null;
            let mut rows = full_page();
            let fixtures = [
                release(2, "26.3-0.1.1"), draft, prerelease, unpublished,
                release(7, "26.3-01.0.0"), release(8, "26.3-0.1.0-rc.1"),
                release(9, "26.3-0.1.0+build"), release(10, "26.3-0.1.18446744073709551616"),
                release(11, "nightly"), release(12, "sdk-abcdef"),
            ];
            rows.splice(..fixtures.len(), fixtures);
            Ok(page(rows, &[r#"<https://api.github.com/repositories/42/releases?per_page=100&page=2>; rel="next", <https://api.github.com/repositories/42/releases?per_page=100&page=2>; rel="last""#]))
        } else if url == format!("{SDK_RELEASE_LIST}&page=2") {
            Ok(page(vec![release(1, STABLE)], &[r#"<https://api.github.com/repositories/42/releases?page=1>; rel="prev", <https://api.github.com/repositories/42/releases?page=1>; rel="first""#]))
        } else if url == format!("{GAME_GIT_API}/ref/tags/{STABLE}") {
            Ok(object("commit", A))
        } else if url == format!("{GAME_GIT_API}/ref/tags/26.3-0.1.1") {
            Ok(object("tag", B))
        } else if url == format!("{GAME_GIT_API}/tags/{B}") {
            Ok(object("tag", C))
        } else if url == format!("{GAME_GIT_API}/tags/{C}") {
            Ok(object("commit", D))
        } else {
            panic!("unexpected request: {url}");
        }
    }).unwrap();
    assert_eq!(requests.len(), 6);
    let written = std::fs::read_to_string(out).unwrap();
    let releases = parse_releases(&written).unwrap();
    assert_eq!(
        releases
            .iter()
            .map(|release| (release.tag.as_str(), release.commit.as_str()))
            .collect::<Vec<_>>(),
        [(STABLE, A), ("26.3-0.1.1", D)],
    );
}

#[test]
fn an_incomplete_listing_does_not_replace_a_previous_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("releases.json");
    std::fs::write(&out, "keep the previous result").unwrap();
    for second in [
        Err("HTTP 503".into()),
        Ok(Response {
            body: b"[{".to_vec(),
            links: Vec::new(),
        }),
        Ok(body(json!([]))),
        Ok(page(vec![release(2, "26.3-0.1.1")], &[])),
        Ok(page(
            {
                let mut rows = full_page();
                rows[0] = release(1, STABLE);
                for (offset, row) in rows[1..].iter_mut().enumerate() {
                    *row = release(2000 + offset as u64, "nightly");
                }
                rows
            },
            &[r#"<https://api.github.com/repositories/42/releases?page=3>; rel="next""#],
        )),
    ] {
        let mut second = Some(second);
        let error = write_list(&out, |url| {
            if url == format!("{SDK_RELEASE_LIST}&page=1") {
                let mut first = full_page();
                first[0] = release(1, STABLE);
                Ok(page(first, &[r#"<https://api.github.com/repositories/42/releases?page=2>; rel="next", <https://api.github.com/repositories/42/releases?page=3>; rel="last""#]))
            } else {
                assert_eq!(url, format!("{SDK_RELEASE_LIST}&page=2"));
                second.take().expect("only one second-page request")
            }
        }).unwrap_err();
        assert!(error.contains("page 2"), "{error}");
        assert_eq!(std::fs::read(&out).unwrap(), b"keep the previous result");
    }
}

#[test]
fn an_ambiguous_first_page_keeps_the_previous_release_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("releases.json");
    let previous = b"unchanged result\r\n";
    std::fs::write(&out, previous).unwrap();
    let short = || vec![release(1, "nightly")];
    let link = |query: &str, relation: &str| {
        format!(r#"<https://api.github.com/repositories/42/releases?{query}>; rel="{relation}""#)
    };
    for (rows, links) in [
        (short(), vec![link("page=2", "next")]),
        (short(), vec!["unrecognised link".into()]),
        (short(), vec![link("page=1", "unknown")]),
        (short(), vec![link("page=1", "next last")]),
        (short(), vec![link("per_page=100", "last")]),
        (short(), vec![link("page=1&page=2", "last")]),
        (
            short(),
            vec![link("page=1", "last"), link("page=1", "last")],
        ),
        (
            short(),
            vec![format!(r#"{}; rel="next""#, link("page=1", "last"))],
        ),
    ] {
        let mut response = Some(page(
            rows,
            &links.iter().map(String::as_str).collect::<Vec<_>>(),
        ));
        let error = write_list(&out, |url| {
            assert_eq!(url, format!("{SDK_RELEASE_LIST}&page=1"));
            Ok(response.take().expect("one first-page request"))
        })
        .unwrap_err();
        assert!(error.contains("page 1"), "{error}");
        assert_eq!(std::fs::read(&out).unwrap(), previous);
    }
}

#[test]
fn a_full_page_explicitly_marked_last_can_finish() {
    let rows = discover(|url| {
        assert_eq!(url, format!("{SDK_RELEASE_LIST}&page=1"));
        Ok(page(full_page(), &[r#"<https://api.github.com/repos/sunrise-org/sunrise/releases?per_page=100&page=1>; rel="last""#]))
    })
    .unwrap();
    assert!(rows.is_empty());
}

#[test]
fn a_full_page_without_a_known_last_finishes_when_its_probe_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("releases.json");
    let mut first = full_page();
    first[0] = release(1, STABLE);
    let mut fixtures = [
        (format!("{SDK_RELEASE_LIST}&page=1"), page(first, &[])),
        (format!("{SDK_RELEASE_LIST}&page=2"), body(json!([]))),
        (
            format!("{GAME_GIT_API}/ref/tags/{STABLE}"),
            object("commit", A),
        ),
    ]
    .into_iter();
    write_list(&out, |url| {
        let (expected, response) = fixtures.next().expect("planned request");
        assert_eq!(url, expected);
        Ok(response)
    })
    .unwrap();
    assert!(fixtures.next().is_none());
    let releases = parse_releases(&std::fs::read_to_string(out).unwrap()).unwrap();
    assert_eq!(releases.len(), 1);
    assert_eq!((&*releases[0].tag, &*releases[0].commit), (STABLE, A));
}

#[test]
fn a_full_pages_probe_must_be_empty_and_terminal_or_the_previous_file_stays() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("releases.json");
    let previous = b"unchanged release listing\r\n";
    std::fs::write(&out, previous).unwrap();
    for probe in [
        Ok(page(vec![release(1, STABLE)], &[])),
        Ok(page(
            vec![],
            &[r#"<https://api.github.com/repositories/42/releases?page=3>; rel="next""#],
        )),
        Err("HTTP 503".into()),
    ] {
        let mut probe = Some(probe);
        let error = write_list(&out, |url| {
            if url == format!("{SDK_RELEASE_LIST}&page=1") {
                Ok(page(full_page(), &[]))
            } else {
                assert_eq!(url, format!("{SDK_RELEASE_LIST}&page=2"));
                probe.take().expect("one probe")
            }
        })
        .unwrap_err();
        assert!(probe.is_none());
        assert!(error.contains("page 2"), "{error}");
        assert_eq!(std::fs::read(&out).unwrap(), previous);
    }
}

#[test]
fn an_empty_probe_cannot_erase_an_already_read_page() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("releases.json");
    let previous = b"unchanged release listing\r\n";
    std::fs::write(&out, previous).unwrap();
    let mut responses = [
        page(
            full_page(),
            &[r#"<https://api.github.com/repositories/42/releases?page=2>; rel="next""#],
        ),
        page(
            (2000..2000 + PAGE_SIZE as u64)
                .map(|id| release(id, "nightly"))
                .collect(),
            &[],
        ),
        page(
            vec![],
            &[r#"<https://api.github.com/repositories/42/releases?page=1>; rel="last""#],
        ),
    ]
    .into_iter()
    .enumerate();
    let error = write_list(&out, |url| {
        let (offset, response) = responses.next().expect("planned request");
        assert_eq!(url, format!("{SDK_RELEASE_LIST}&page={}", offset + 1));
        Ok(response)
    })
    .unwrap_err();
    assert!(responses.next().is_none());
    assert!(error.contains("page 3"), "{error}");
    assert_eq!(std::fs::read(&out).unwrap(), previous);
}

#[test]
fn malformed_publication_metadata_keeps_the_previous_release_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("releases.json");
    let previous = b"keep this result\r\n";
    std::fs::write(&out, previous).unwrap();
    for timestamp in [
        json!("2026-11-21"),
        json!("2026-02-30T03:04:05Z"),
        json!(42),
    ] {
        let mut row = release(1, STABLE);
        row["published_at"] = timestamp;
        let error = write_list(&out, |url| {
            assert_eq!(url, format!("{SDK_RELEASE_LIST}&page=1"));
            Ok(body(json!([row])))
        })
        .unwrap_err();
        assert!(error.contains("page 1"), "{error}");
        assert_eq!(std::fs::read(&out).unwrap(), previous);
    }
}

#[test]
fn a_tag_must_end_at_a_full_commit_and_cannot_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("releases.json");
    for (kind, sha, lookups) in [("tag", B, 2), ("tree", A, 1), ("commit", "main", 1)] {
        let mut requests = Vec::new();
        let error = write_list(&out, |url| {
            if url == format!("{SDK_RELEASE_LIST}&page=1") {
                Ok(body(json!([release(1, STABLE)])))
            } else {
                requests.push(url.to_owned());
                Ok(object(kind, sha))
            }
        })
        .unwrap_err();
        let expected = [
            format!("{GAME_GIT_API}/ref/tags/{STABLE}"),
            format!("{GAME_GIT_API}/tags/{B}"),
        ];
        assert_eq!(requests, expected[..lookups]);
        assert!(error.contains(STABLE), "{error}");
        assert!(!out.exists());
    }
}
