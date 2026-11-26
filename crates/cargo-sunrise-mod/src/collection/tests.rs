use super::*;
use sunrise_mod_package::index::v2::collection::Member;

/// A registry on `main`: two projects of `example` and one of `other`, and an API that knows
/// `example/ports` (repository 7).
fn registry(url: &str) -> Result<Option<String>, String> {
    let meta = |path: &str| format!("{MOD_REGISTRY_METADATA}/{path}");
    let text = if url == meta("repositories/7.toml") {
        "name = \"example/ports\"\n\n[[package]]\npath = \"doors\"\nproject = \"Ab3dE9xQ\"\n\n[[package]]\npath = \"walls\"\nproject = \"Zz91kLmN\"\n"
    } else if url == meta("repositories/8.toml") {
        "name = \"other/thing\"\n\n[[package]]\npath = \".\"\nproject = \"Ot4erP0j\"\n"
    } else if url == meta("projects/Ab3dE9xQ/project.toml") {
        "mod = \"doors\"\nowners = [\"Example\", \"helper\"]\nrepository = 7\n"
    } else if url == meta("projects/Zz91kLmN/project.toml") {
        "mod = \"walls\"\nowners = [\"example\"]\nrepository = 7\n"
    } else if url == meta("projects/Ot4erP0j/project.toml") {
        "mod = \"thing\"\nowners = [\"other\"]\nrepository = 7\n"
    } else if url == "https://api.github.com/repos/example/ports" {
        r#"{"id":7}"#
    } else {
        return Ok(None);
    };
    Ok(Some(text.into()))
}

fn png(width: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, width);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![0; (width * width * 4) as usize])
            .unwrap();
    }
    bytes
}

#[test]
fn a_new_collection_lists_this_repositorys_projects_and_waits_for_a_name_and_an_id() {
    let projects = registered_from(&registry, "git@github.com:example/ports.git").unwrap();
    assert_eq!(
        projects,
        [
            ("Ab3dE9xQ".to_owned(), "doors".to_owned()),
            ("Zz91kLmN".to_owned(), "walls".to_owned())
        ]
    );
    // Only a repository the registry has no file for is unregistered; a failed read says so.
    let unlisted = |url: &str| -> Result<Option<String>, String> {
        match url {
            "https://api.github.com/repos/example/ports" => Ok(Some(r#"{"id":9}"#.into())),
            _ => Ok(None),
        }
    };
    assert_eq!(
        registered_from(&unlisted, "git@github.com:example/ports.git").unwrap_err(),
        NOT_REGISTERED
    );
    let down = |url: &str| -> Result<Option<String>, String> {
        match url {
            "https://api.github.com/repos/example/ports" => Ok(Some(r#"{"id":7}"#.into())),
            _ => Err(format!("{url}: 503")),
        }
    };
    assert_eq!(
        registered_from(&down, "git@github.com:example/ports.git").unwrap_err(),
        format!("{MOD_REGISTRY_METADATA}/repositories/7.toml: 503")
    );
    let text = scaffold(&projects);
    // The ID is added on the next run, keeping the author's text and comments, and never replaced.
    let (text, id) = with_id(&text, &|| Ok("Qm4tZ81c".into())).unwrap().unwrap();
    assert_eq!(id, "Qm4tZ81c");
    assert!(
        text.starts_with("id = \"Qm4tZ81c\"\nname = \"\"\n"),
        "{text}"
    );
    assert!(text.contains("project = \"Zz91kLmN\"  # walls"), "{text}");
    assert!(
        with_id(&text, &|| panic!("an ID exists"))
            .unwrap()
            .is_none()
    );
    let collection: Collection = toml::from_str(&text).unwrap();
    assert!(collection.member.iter().all(|member| member.default));
    // Its blank name and summary are refused until the author fills them in.
    let error = check(&registry, &collection, Some(&png(16))).unwrap_err();
    assert_eq!(
        error,
        "collection.toml: name must not be blank\ncollection.toml: summary must not be blank"
    );
}

#[test]
fn a_collection_names_who_may_open_it_and_is_refused_when_nobody_owns_every_member() {
    let mut collection = Collection {
        id: "Qm4tZ81c".into(),
        name: "Ports".into(),
        summary: "Doors and walls.".into(),
        description: String::new(),
        member: ["Ab3dE9xQ", "Zz91kLmN"]
            .map(|project| Member {
                project: project.into(),
                default: true,
            })
            .into(),
    };
    assert_eq!(
        check(&registry, &collection, Some(&png(16))),
        Ok(vec!["Example".to_owned()])
    );
    assert_eq!(
        check(&registry, &collection, None).unwrap_err(),
        "icon.png: expected the collection's PNG icon"
    );
    collection.member[1].project = "Ot4erP0j".into();
    assert_eq!(
        check(&registry, &collection, Some(&png(16))).unwrap_err(),
        "no GitHub account is in owners of every member, so nobody can open the collection's pull request"
    );
    collection.member[1].project = "Nope0000".into();
    assert_eq!(
        check(&registry, &collection, Some(&png(16))).unwrap_err(),
        "collection.toml: member Nope0000 is not a registered project"
    );
}

#[cfg(unix)]
mod git_fixtures {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> String {
        Git::default().text(dir, args).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn a_proposal_is_one_signed_off_commit_on_main_pushed_to_the_forks_branch() {
        let temp = tempfile::tempdir().unwrap();
        let upstream = temp.path().join("upstream");
        fs::create_dir_all(upstream.join("registry/collections/Qm4tZ81c")).unwrap();
        fs::write(
            upstream.join("registry/collections/Qm4tZ81c/old.txt"),
            "old",
        )
        .unwrap();
        git(&upstream, &["init", "--quiet", "--initial-branch", "main"]);
        for (key, value) in [
            ("user.name", "Registry"),
            ("user.email", "registry@example.com"),
            ("commit.gpgsign", "false"),
        ] {
            git(&upstream, &["config", key, value]);
        }
        git(&upstream, &["add", "--all"]);
        git(&upstream, &["commit", "--quiet", "--message", "registry"]);
        let fork = temp.path().join("fork.git");
        git(
            temp.path(),
            &[
                "clone",
                "--quiet",
                "--bare",
                upstream.to_str().unwrap(),
                fork.to_str().unwrap(),
            ],
        );
        // The author's identity comes from their own Git configuration.
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join(".gitconfig"),
            "[user]\n\tname = Example\n\temail = example@example.com\n[commit]\n\tgpgsign = false\n",
        )
        .unwrap();
        let author = Git {
            env: vec![
                ("HOME", home.clone().into()),
                ("GIT_CONFIG_NOSYSTEM", "1".into()),
            ],
        };
        let upstream_url = format!("file://{}", upstream.display());
        let propose_once = |text: &str| {
            propose(
                &author,
                &upstream_url,
                fork.to_str().unwrap(),
                "collection-Qm4tZ81c",
                "registry/collections/Qm4tZ81c",
                &[(TOML, text.as_bytes().to_vec()), (ICON, png(16))],
                "Update the Ports collection",
            )
        };
        propose_once("first").unwrap();
        // Running again replaces the branch rather than stacking on it.
        propose_once("second").unwrap();
        let branch = "collection-Qm4tZ81c";
        assert_eq!(
            git(&fork, &["rev-list", "--count", branch]),
            "2",
            "one commit on top of main"
        );
        assert_eq!(
            git(
                &fork,
                &[
                    "ls-tree",
                    "--name-only",
                    &format!("{branch}:registry/collections/Qm4tZ81c")
                ]
            ),
            "collection.toml\nicon.png"
        );
        assert_eq!(
            git(
                &fork,
                &[
                    "show",
                    &format!("{branch}:registry/collections/Qm4tZ81c/collection.toml")
                ]
            ),
            "second"
        );
        let body = git(&fork, &["log", "-1", "--format=%an%n%B", branch]);
        assert!(
            body.starts_with("Example\nUpdate the Ports collection"),
            "{body}"
        );
        assert!(
            body.contains("Signed-off-by: Example <example@example.com>"),
            "{body}"
        );
    }
}
