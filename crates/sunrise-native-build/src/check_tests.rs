use super::*;
use std::path::PathBuf;
use sunrise_mod_package::mod_surface::uses::Change;

const OTHER_BUILD: &str = "cccccccccccccccccccccccccccccccccccccccc";

/// A surface holding free functions (id, hash).
fn surface(functions: &[(&str, &str)]) -> mod_surface::Surface {
    let functions: Vec<_> = functions
        .iter()
        .map(|(id, hash)| serde_json::json!({"id": id, "hash": hash, "signature": "fn f()"}))
        .collect();
    serde_json::from_value(serde_json::json!({
        "schema": mod_surface::SCHEMA, "hash": "s", "functions": functions,
    }))
    .unwrap()
}

/// A mod made for its bundle's pin whose table (when `uses` is given) lists those items.
fn project_dir(name: &str, uses: Option<&[(&str, &str)]>, made_for: Option<&str>) -> PathBuf {
    let dir = crate::tests::dir(name);
    crate::tests::write_bundle(&dir, "greeter", "");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname='greeter'\nversion='0.1.0'\n[dependencies]\nsunrise-mod-api={ path = '.sunrise/sdk/api/crates/sunrise-mod-api' }\n",
    )
    .unwrap();
    let pin = pin(&dir);
    if let Some(uses) = uses {
        let entries: Vec<_> = uses
            .iter()
            .map(|(id, hash)| serde_json::json!({"id": id, "hash": hash}))
            .collect();
        let table = serde_json::json!({
            "schema": uses::USES_SCHEMA,
            "made_for": made_for.unwrap_or(&pin),
            "entries": entries,
        });
        std::fs::write(dir.join(uses::USES_FILE), table.to_string()).unwrap();
    }
    dir
}

fn judged(dir: &Path, build: &str, target: &mod_surface::Surface) -> Verdict {
    judged_with(dir, build, target, None)
}

/// As [`judged`], with the build's build data in `data` when the mod declares game code.
fn judged_with(
    dir: &Path,
    build: &str,
    target: &mod_surface::Surface,
    data: Option<&Path>,
) -> Verdict {
    let bundle = crate::bundle_in(dir).unwrap().unwrap();
    let project = api::Project::read(&bundle).unwrap();
    let target = Rc::new(target.clone());
    judge(
        &project,
        build,
        &mut || Ok::<_, String>(target.clone()),
        &mut || match data {
            Some(data) => game_uses::entries(&bundle.manifest, data, build),
            None => Err("no game code declared: never read".to_owned()),
        },
    )
    .unwrap()
}

fn pin(dir: &Path) -> String {
    crate::bundle_in(dir)
        .unwrap()
        .unwrap()
        .native()
        .sunrise
        .clone()
}

#[test]
fn a_mod_fits_a_build_where_what_it_uses_is_unchanged() {
    let dir = project_dir("fit-fits", Some(&[("fn:give", "h1")]), None);
    // Something it doesn't use changed, and something new came: it still fits.
    let target = surface(&[("fn:give", "h1"), ("fn:other", "h9"), ("fn:new", "n")]);
    assert_eq!(
        judged(&dir, OTHER_BUILD, &target),
        Verdict::Fits {
            made_for: pin(&dir)
        }
    );
    // On the build it was made for, no test runs.
    assert_eq!(judged(&dir, &pin(&dir), &surface(&[])), Verdict::MadeFor);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_list_that_holds_no_version_of_a_hook_cannot_show_the_mod_fits() {
    let dir = project_dir("fit-old-list", None, None);
    let made_for = pin(&dir);
    let list = |schema: u32, hash: &str, version: Option<u64>| {
        let table = serde_json::json!({
            "schema": schema, "made_for": made_for,
            "entries": [{"id": "hook:joined", "hash": hash, "version": version}],
        });
        std::fs::write(dir.join(uses::USES_FILE), table.to_string()).unwrap();
    };
    let target = |version: u64| -> mod_surface::Surface {
        serde_json::from_value(serde_json::json!({
            "schema": mod_surface::SCHEMA, "hash": "s",
            "hooks": [{
                "id": "hook:joined", "hash": "h1", "engine": {"target": "t"}, "call": "c",
                "version": version, "side": "server", "contract": "listen", "cancellable": false, "arguments": [],
            }],
        }))
        .unwrap()
    };
    let unversioned = Verdict::NoList {
        made_for: made_for.clone(),
        why: Unlisted::HookVersions,
        hooks: vec!["hook:joined".into()],
    };
    // The mod may have been made for a later version than the first: the target's own version
    // says nothing about it, whichever it is.
    for schema in [1, uses::USES_SCHEMA] {
        list(schema, "h1", None);
        assert_eq!(judged(&dir, OTHER_BUILD, &target(1)), unversioned);
        assert_eq!(judged(&dir, OTHER_BUILD, &target(2)), unversioned);
    }
    // What such a list can still judge is named.
    list(1, "h0", None);
    assert_eq!(
        judged(&dir, OTHER_BUILD, &target(1)),
        Verdict::DoesNotFit {
            made_for: made_for.clone(),
            misfits: vec![Misfit {
                id: "hook:joined".into(),
                change: Change::Changed
            }],
        }
    );
    list(uses::USES_SCHEMA, "h1", Some(1));
    assert_eq!(
        judged(&dir, OTHER_BUILD, &target(1)),
        Verdict::Fits {
            made_for: made_for.clone()
        }
    );
    // A list a newer tool wrote is named as that, for a newer game build to read.
    list(uses::USES_SCHEMA + 1, "h1", Some(1));
    assert_eq!(
        judged(&dir, OTHER_BUILD, &target(1)),
        Verdict::NoList {
            made_for,
            why: Unlisted::NewerSchema,
            hooks: Vec::new(),
        }
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_copy_a_build_makes_is_judged_by_the_same_table() {
    let dir = project_dir("fit-copied", Some(&[("fn:give", "h1")]), None);
    let bundle = crate::bundle_in(&dir).unwrap().unwrap();
    let copy = crate::tests::dir("fit-copied-to");
    crate::package::copy(&bundle, &copy).unwrap();
    let target = surface(&[("fn:give", "h1")]);
    assert_eq!(
        judged(&copy, OTHER_BUILD, &target),
        Verdict::Fits {
            made_for: pin(&dir)
        }
    );
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(copy).unwrap();
}

#[test]
fn a_changed_or_removed_item_it_uses_is_named() {
    let dir = project_dir(
        "fit-misfit",
        Some(&[("fn:give", "h1"), ("hook:player_logged_in", "h2")]),
        None,
    );
    let target = surface(&[("fn:give", "h1-changed")]);
    let Verdict::DoesNotFit { made_for, misfits } = judged(&dir, OTHER_BUILD, &target) else {
        panic!("a changed item fits");
    };
    assert_eq!(made_for, pin(&dir));
    let said: Vec<(&str, &Change)> = misfits.iter().map(|m| (m.id.as_str(), &m.change)).collect();
    assert_eq!(
        said,
        [
            ("fn:give", &Change::Changed),
            ("hook:player_logged_in", &Change::Removed)
        ]
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn without_a_list_for_its_pin_a_mod_works_on_its_own_build_only() {
    let none = project_dir("fit-no-list", None, None);
    let target = surface(&[("fn:give", "h1")]);
    assert_eq!(
        judged(&none, OTHER_BUILD, &target),
        Verdict::NoList {
            made_for: pin(&none),
            why: Unlisted::Missing,
            hooks: Vec::new(),
        }
    );
    // A list made for another build than the mod's pin says nothing about this one.
    let stale = project_dir(
        "fit-stale-list",
        Some(&[("fn:give", "h1")]),
        Some("dddddddddddddddddddddddddddddddddddddddd"),
    );
    assert_eq!(
        judged(&stale, OTHER_BUILD, &target),
        Verdict::NoList {
            made_for: pin(&stale),
            why: Unlisted::OtherBuild,
            hooks: Vec::new(),
        }
    );
    // A list that does not read is no list: never a panic, never "fits".
    std::fs::write(
        none.join(uses::USES_FILE),
        b"{\"schema\": 1, \"made_for\": 3}",
    )
    .unwrap();
    assert_eq!(
        judged(&none, OTHER_BUILD, &target),
        Verdict::NoList {
            made_for: pin(&none),
            why: Unlisted::Unreadable,
            hooks: Vec::new(),
        }
    );
    std::fs::remove_dir_all(none).unwrap();
    std::fs::remove_dir_all(stale).unwrap();
}

const RUN_WITH: &str = "sunrise_client::game::run_with";
const INDEX_REASON: &str = "sentinel from functions.json: private::provider";

fn refused_run_with(name: &str, build: &str) -> (PathBuf, PathBuf) {
    let id = format!("game:fn:{RUN_WITH}");
    let dir = project_dir(name, Some(&[(&id, "sig:body")]), None);
    let manifest = dir.join(crate::NATIVE_MANIFEST);
    let text = std::fs::read_to_string(&manifest)
        .unwrap()
        .replace(crate::tests::B, crate::MIXINS_REV)
        + &format!(
            "[[native.mixin]]\nid = \"entry_probe\"\ntarget = \"{RUN_WITH}\"\nkind = \"HEAD\"\nhelper = \"src/m.rs\"\n"
        );
    std::fs::write(&manifest, text).unwrap();
    std::fs::write(dir.join("src/m.rs"), "{}").unwrap();
    let data = dir.join(".sunrise/build-data");
    std::fs::create_dir_all(&data).unwrap();
    let file = serde_json::json!({
        "schema": 1, "build": {"commit": build}, "crates": {}, "functions": {},
        "refused": { RUN_WITH: { "code": "E_MIXIN_NOT_REPLACEABLE", "why": INDEX_REASON } },
    });
    std::fs::write(data.join("functions.json"), file.to_string()).unwrap();
    (dir, data)
}

#[test]
fn native_check_needs_the_index_and_names_the_refused_same_build_target() {
    let (dir, data) = refused_run_with("refused-same-build", crate::tests::A);
    let mut plan = Plan {
        contract: crate::contract::CONTRACT,
        mods: dir.clone(),
        build: crate::tests::A.into(),
        surface: None,
        build_data: None,
    };
    let needs = check(&plan).unwrap();
    assert_eq!(needs.mods[0].needs, Some("build-data"));
    assert_eq!(needs.route, None);
    plan.build_data = Some(data);
    let checked = check(&plan).unwrap();
    assert_eq!(checked.route, None);
    assert_eq!(checked.mods[0].verdict, None);
    assert_eq!(checked.mods[0].needs, None);
    assert_eq!(checked.mods[0].id, "greeter");
    let refusal = checked.mods[0].refusal.as_deref().unwrap();
    assert!(refusal.contains("E_MIXIN_NOT_REPLACEABLE"), "{refusal}");
    assert!(refusal.contains("greeter"), "{refusal}");
    assert!(refusal.contains("entry_probe"), "{refusal}");
    assert!(refusal.contains(RUN_WITH), "{refusal}");
    assert!(refusal.contains(INDEX_REASON), "{refusal}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cross_build_fit_distinguishes_a_refused_target_from_a_removed_one() {
    let (dir, data) = refused_run_with("refused-cross-build", OTHER_BUILD);
    assert_eq!(
        judged_with(&dir, OTHER_BUILD, &surface(&[]), Some(&data)),
        Verdict::DoesNotFit {
            made_for: pin(&dir),
            misfits: vec![Misfit {
                id: format!("game:fn:{RUN_WITH}"),
                change: Change::Refused {
                    code: "E_MIXIN_NOT_REPLACEABLE",
                    why: INDEX_REASON.into(),
                }
            }],
        },
    );
    let surface_file = dir.join(".sunrise/surface.json");
    std::fs::write(&surface_file, serde_json::to_vec(&surface(&[])).unwrap()).unwrap();
    let checked = check(&Plan {
        contract: crate::contract::CONTRACT,
        mods: dir.clone(),
        build: OTHER_BUILD.into(),
        surface: Some(surface_file),
        build_data: Some(data),
    })
    .unwrap();
    let reply = serde_json::to_value(checked).unwrap();
    assert_eq!(reply["contract"], 2);
    assert_eq!(reply["route"], serde_json::Value::Null);
    let row = &reply["mods"][0];
    assert_eq!(row["id"], "greeter");
    assert_eq!(row["refusal"], serde_json::Value::Null);
    let verdict = &row["verdict"];
    assert_eq!(verdict["kind"], "does-not-fit");
    assert_eq!(verdict["made_for"], pin(&dir));
    let [misfit] = verdict["misfits"].as_array().unwrap().as_slice() else {
        panic!("expected one misfit: {verdict}");
    };
    assert_eq!(misfit["change"], "changed");
    let diagnostic = misfit["id"].as_str().unwrap();
    assert!(
        !diagnostic.starts_with("game:"),
        "the launcher shortens game ids"
    );
    assert!(diagnostic.contains(RUN_WITH), "{diagnostic}");
    assert!(diagnostic.contains(INDEX_REASON), "{diagnostic}");
    assert!(
        diagnostic.contains("E_MIXIN_NOT_REPLACEABLE"),
        "{diagnostic}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_mod_with_a_mixin_fits_while_its_target_is_what_its_list_records() {
    const TARGET: &str = "sunrise_client::game::f";
    let dir = project_dir("fit-mixin", None, None);
    let manifest = dir.join(crate::NATIVE_MANIFEST);
    let text = std::fs::read_to_string(&manifest)
        .unwrap()
        .replace(crate::tests::B, crate::MIXINS_REV)
        + &format!(
            "[[native.mixin]]\nid = \"m\"\ntarget = \"{TARGET}\"\nkind = \"HEAD\"\nhelper = \"src/m.rs\"\n"
        );
    std::fs::write(&manifest, text).unwrap();
    std::fs::write(dir.join("src/m.rs"), "{}").unwrap();
    let made_for = pin(&dir);
    let list = |entries: serde_json::Value| {
        let table = serde_json::json!({
            "schema": uses::USES_SCHEMA, "made_for": made_for, "entries": entries,
        });
        std::fs::write(dir.join(uses::USES_FILE), table.to_string()).unwrap();
    };
    // The installed build's function index: the target with this body, or without the target.
    let data = crate::tests::dir("fit-mixin-data");
    std::fs::create_dir_all(&data).unwrap();
    let index = |body: Option<&str>| {
        let functions = match body {
            Some(body) => serde_json::json!({TARGET: {
                "module": "sunrise_client::game", "file": "crates/sunrise-client/src/game.rs",
                "span": [0, 1], "signature_hash": "sig", "sha256": body,
            }}),
            None => serde_json::json!({}),
        };
        let file = serde_json::json!({
            "schema": 1, "build": {"commit": OTHER_BUILD},
            "crates": {}, "functions": functions, "refused": {},
        });
        std::fs::write(data.join("functions.json"), file.to_string()).unwrap();
    };
    let id = format!("game:fn:{TARGET}");
    let judged = || judged_with(&dir, OTHER_BUILD, &surface(&[]), Some(&data));
    let misfit = |change| Verdict::DoesNotFit {
        made_for: made_for.clone(),
        misfits: vec![Misfit {
            id: id.clone(),
            change,
        }],
    };
    list(serde_json::json!([{"id": id, "hash": "sig:body"}]));
    index(Some("body"));
    assert_eq!(
        judged(),
        Verdict::Fits {
            made_for: made_for.clone()
        }
    );
    // A replacement written for the old body could undo what the game changed in it.
    index(Some("another body"));
    assert_eq!(judged(), misfit(Change::Changed));
    index(None);
    assert_eq!(judged(), misfit(Change::Removed));
    // The mod pins a horizon-mixins revision this tool does not build with.
    index(Some("body"));
    let pinned = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        pinned.replace(crate::MIXINS_REV, crate::tests::B),
    )
    .unwrap();
    assert_eq!(
        judged(),
        Verdict::DoesNotFit {
            made_for: made_for.clone(),
            misfits: vec![Misfit {
                id: "game:tool:horizon-mixins".into(),
                change: Change::Changed
            }],
        }
    );
    std::fs::write(&manifest, pinned).unwrap();
    // A list that does not record the target cannot show it unchanged.
    list(serde_json::json!([]));
    index(Some("body"));
    assert_eq!(
        judged(),
        Verdict::NoList {
            made_for: made_for.clone(),
            why: Unlisted::GameCode,
            hooks: Vec::new(),
        }
    );
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(data).unwrap();
}

/// Build data for `build` in a fresh folder: the function index and the internals given.
fn build_data(
    name: &str,
    build: &str,
    functions: serde_json::Value,
    internals: serde_json::Value,
) -> PathBuf {
    let data = crate::tests::dir(name);
    std::fs::create_dir_all(&data).unwrap();
    let header = serde_json::json!({"schema": 1, "build": {"commit": build}});
    let write = |file: &str, body: serde_json::Value| {
        let mut document = header.clone();
        document
            .as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        std::fs::write(data.join(file), document.to_string()).unwrap();
    };
    write(
        "functions.json",
        serde_json::json!({"crates": {}, "functions": functions, "refused": {}}),
    );
    write(
        "internals.json",
        serde_json::json!({"internals": internals}),
    );
    data
}

fn function(body: &str) -> serde_json::Value {
    serde_json::json!({
        "module": "sunrise_client::game", "file": "crates/sunrise-client/src/game.rs",
        "span": [0, 1], "signature_hash": "sig", "sha256": body,
    })
}

#[test]
fn a_target_that_is_also_a_declared_internal_is_two_entries_of_one_readable_list() {
    const PATH: &str = "sunrise_client::game::f";
    let dir = project_dir("fit-both", None, None);
    let manifest = dir.join(crate::NATIVE_MANIFEST);
    let text = std::fs::read_to_string(&manifest)
        .unwrap()
        .replace(crate::tests::B, crate::MIXINS_REV)
        + &format!(
            "[[native.mixin]]\nid = \"m\"\ntarget = \"{PATH}\"\nkind = \"HEAD\"\nhelper = \"src/m.rs\"\n[[internals]]\npath = \"{PATH}\"\nkind = \"fn\"\n"
        );
    std::fs::write(&manifest, text).unwrap();
    std::fs::write(dir.join("src/m.rs"), "{}").unwrap();
    let internal = |path: &str| serde_json::json!({"path": path, "kind": "fn", "signature": "fn f()", "hash": "isig"});
    let data = build_data(
        "fit-both-data",
        OTHER_BUILD,
        serde_json::json!({PATH: function("body")}),
        serde_json::json!([internal(PATH)]),
    );
    let bundle = crate::bundle_in(&dir).unwrap().unwrap();
    let now = game_uses::entries(&bundle.manifest, &data, OTHER_BUILD).unwrap();
    // As the packer writes them, the list reads back.
    let table = UsesTable {
        schema: uses::USES_SCHEMA,
        made_for: pin(&dir),
        entries: now
            .into_iter()
            .filter_map(|(id, now)| now.into_entry(id))
            .collect(),
    };
    assert_eq!(
        table.entries,
        [
            uses::Entry::plain(format!("game:fn:{PATH}"), "sig:body".into()),
            uses::Entry::plain(
                format!("game:internal:fn:{PATH}"),
                "fn:isig:sig:body".into()
            ),
        ]
    );
    std::fs::write(dir.join(uses::USES_FILE), table.to_json()).unwrap();
    assert_eq!(UsesTable::read(table.to_json().as_bytes()), Ok(table));
    let judged = |data: &Path| judged_with(&dir, OTHER_BUILD, &surface(&[]), Some(data));
    assert_eq!(
        judged(&data),
        Verdict::Fits {
            made_for: pin(&dir)
        }
    );
    // A function the mod calls whose body the build data does not hold is never shown unchanged.
    let unindexed = build_data(
        "fit-both-unindexed",
        OTHER_BUILD,
        serde_json::json!({PATH: function("body")}),
        serde_json::json!([internal(PATH), internal("sunrise_client::entry::g")]),
    );
    let more = std::fs::read_to_string(&manifest).unwrap()
        + "[[internals]]\npath = \"sunrise_client::entry::g\"\nkind = \"fn\"\n";
    std::fs::write(&manifest, more).unwrap();
    assert_eq!(
        judged(&unindexed),
        Verdict::Exact {
            made_for: pin(&dir),
            why: "internals"
        }
    );
    for folder in [dir, data, unindexed] {
        std::fs::remove_dir_all(folder).unwrap();
    }
}

#[test]
fn a_changed_function_declared_as_target_and_internal_is_reported_once() {
    const PATH: &str = "sunrise_client::game::f";
    let dir = project_dir("fit-both-changed", None, None);
    let manifest = dir.join(crate::NATIVE_MANIFEST);
    let text = std::fs::read_to_string(&manifest)
        .unwrap()
        .replace(crate::tests::B, crate::MIXINS_REV)
        + &format!(
            "[[native.mixin]]\nid = \"m\"\ntarget = \"{PATH}\"\nkind = \"HEAD\"\nhelper = \"src/m.rs\"\n[[internals]]\npath = \"{PATH}\"\nkind = \"fn\"\n"
        );
    std::fs::write(&manifest, text).unwrap();
    std::fs::write(dir.join("src/m.rs"), "{}").unwrap();
    let data = |name: &str, body: &str| {
        let mut function = function(body);
        function["digests"] = serde_json::json!({"declaration": "declaration", "body": body});
        build_data(
            name,
            OTHER_BUILD,
            serde_json::json!({PATH: function}),
            serde_json::json!([{"path": PATH, "kind": "fn", "signature": "fn f()", "hash": "isig"}]),
        )
    };
    let packed = data("fit-both-changed-packed", "body");
    let bundle = crate::bundle_in(&dir).unwrap().unwrap();
    let table = UsesTable {
        schema: uses::USES_SCHEMA,
        made_for: pin(&dir),
        entries: game_uses::entries(&bundle.manifest, &packed, OTHER_BUILD)
            .unwrap()
            .into_iter()
            .filter_map(|(id, now)| now.into_entry(id))
            .collect(),
    };
    assert_eq!(table.entries.len(), 2);
    std::fs::write(dir.join(uses::USES_FILE), table.to_json()).unwrap();
    let surface_file = dir.join("surface.json");
    std::fs::write(&surface_file, serde_json::to_vec(&surface(&[])).unwrap()).unwrap();
    let edited = data("fit-both-changed-edited", "edited");
    let checked = check(&Plan {
        contract: crate::contract::CONTRACT,
        mods: dir.clone(),
        build: OTHER_BUILD.into(),
        surface: Some(surface_file),
        build_data: Some(edited.clone()),
    })
    .unwrap();
    assert_eq!(
        serde_json::to_value(checked).unwrap()["mods"][0]["verdict"]["misfits"],
        serde_json::json!([{
            "id": format!("the game's function `{PATH}` (body changed)"),
            "change": "changed",
        }])
    );
    for folder in [dir, packed, edited] {
        std::fs::remove_dir_all(folder).unwrap();
    }
}

#[test]
fn an_internal_declared_without_a_kind_is_named_when_it_becomes_another_kind() {
    const PATH: &str = "sunrise_client::game::Limit";
    let dir = project_dir("fit-kind", None, None);
    let manifest = dir.join(crate::NATIVE_MANIFEST);
    let text = std::fs::read_to_string(&manifest).unwrap()
        + &format!("[[internals]]\npath = \"{PATH}\"\n");
    std::fs::write(&manifest, text).unwrap();
    let data = |name: &str, kind: &str| {
        build_data(
            name,
            OTHER_BUILD,
            serde_json::json!({}),
            serde_json::json!([{"path": PATH, "kind": kind, "signature": "s", "hash": "h"}]),
        )
    };
    let (as_const, as_static) = (
        data("fit-kind-const", "const"),
        data("fit-kind-static", "static"),
    );
    let id = format!("game:internal:any:{PATH}");
    let table = serde_json::json!({
        "schema": uses::USES_SCHEMA, "made_for": pin(&dir),
        "entries": [{"id": id, "hash": "const:h"}],
    });
    std::fs::write(dir.join(uses::USES_FILE), table.to_string()).unwrap();
    let judged = |data: &Path| judged_with(&dir, OTHER_BUILD, &surface(&[]), Some(data));
    assert_eq!(
        judged(&as_const),
        Verdict::Fits {
            made_for: pin(&dir)
        }
    );
    assert_eq!(
        judged(&as_static),
        Verdict::DoesNotFit {
            made_for: pin(&dir),
            misfits: vec![Misfit {
                id,
                change: Change::Changed
            }],
        }
    );
    for folder in [dir, as_const, as_static] {
        std::fs::remove_dir_all(folder).unwrap();
    }
}

#[test]
fn generated_loops_compare_each_function_and_keep_the_slot_contract() {
    use sunrise_platform::mod_build::api_gen::{functions, internals, loops};
    const GAME: &str = r#"
struct Game;
trait Slot { fn update(&mut self); }
impl Game {
    #[sunrise_mod_api_macros::sunrise_loop(name = "ticks", side = "server")]
    fn tick(&mut self) { self.tick_with(&mut ()) }
    fn tick_with<S: Slot>(&mut self, slot: &mut S) { slot.update(); }
}
fn inspect() -> u32 { 1 }
"#;
    const INTERNAL: &str = "sunrise_client::game::inspect";
    const GENERIC: &str = "sunrise_client::game::Game::tick_with";
    let source = tempfile::tempdir().unwrap();
    for (path, text) in [
        (
            "crates/sunrise-client/Cargo.toml",
            "[package]\nname='sunrise-client'\n",
        ),
        ("crates/sunrise-client/src/lib.rs", "pub mod game;"),
        ("crates/sunrise-client/src/entry.rs", ""),
        ("crates/sunrise-mod-api-gen/internals.txt", ""),
    ] {
        let path = source.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let dir = project_dir("fit-generated-loop", None, None);
    let manifest = dir.join(crate::NATIVE_MANIFEST);
    let text = std::fs::read_to_string(&manifest).unwrap()
        + &format!(
            "[[native.loop]]\nid='loop'\nloop='ticks'\nhelper='src/l.rs'\nwith='Slot'\n\
         [[internals]]\npath='{INTERNAL}'\nkind='fn'\n"
        );
    std::fs::write(&manifest, text).unwrap();
    std::fs::write(dir.join("src/l.rs"), "pub struct Slot;").unwrap();
    let bundle = crate::bundle_in(&dir).unwrap().unwrap();
    let data = tempfile::tempdir().unwrap();
    let generate = |text| {
        let source = source.path();
        std::fs::write(source.join("crates/sunrise-client/src/game.rs"), text).unwrap();
        let loops = loops::collect(source).unwrap();
        let generic = loops
            .iter()
            .map(|row| functions::key(&row.generic_target()))
            .collect();
        let functions = functions::index(source, &generic).unwrap();
        let internals = internals::catalog(source).unwrap();
        let build = sunrise_mod_package::build_data::Build {
            commit: OTHER_BUILD.into(),
        };
        for (file, body) in [
            (functions::FILE, serde_json::to_value(functions).unwrap()),
            (loops::FILE, serde_json::json!({"loops": loops})),
            (
                sunrise_mod_package::build_data::INTERNALS,
                serde_json::json!({"internals": internals}),
            ),
        ] {
            let document = sunrise_mod_package::build_data::Document::new(&build, body);
            std::fs::write(
                data.path().join(file),
                serde_json::to_vec(&document).unwrap(),
            )
            .unwrap();
        }
        game_uses::entries(&bundle.manifest, data.path(), OTHER_BUILD).unwrap()
    };
    let entries: Vec<_> = generate(GAME)
        .into_iter()
        .map(|(id, now)| now.into_entry(id).unwrap())
        .collect();
    let entries: Vec<_> = entries.iter().collect();
    let compare = |text| game_uses::compare(&entries, generate(text)).misfits;
    let edited = GAME
        .replace("slot.update();", "slot.update(); slot.update();")
        .replace("{ 1 }", "{ 2 }");
    let changes = [
        Misfit {
            id: format!("game:internal:fn:{INTERNAL}"),
            change: Change::BodyChanged {
                function: INTERNAL.into(),
            },
        },
        Misfit {
            id: "game:loop:ticks".into(),
            change: Change::BodyChanged {
                function: GENERIC.into(),
            },
        },
    ];
    assert_eq!(compare(&edited), changes);
    let new_slot = GAME.replace("fn update(&mut self);", "fn update(&mut self, n: usize);");
    assert_eq!(
        compare(&new_slot),
        [Misfit {
            id: "game:loop:ticks".into(),
            change: Change::Changed
        }]
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_loop_whose_entry_is_no_path_is_refused_by_name() {
    let dir = project_dir("fit-loop-entry", None, None);
    let manifest = dir.join(crate::NATIVE_MANIFEST);
    let text = std::fs::read_to_string(&manifest).unwrap()
        + "[[native.loop]]\nid = \"l\"\nloop = \"ticks\"\nhelper = \"src/l.rs\"\nwith = \"Slot\"\n";
    std::fs::write(&manifest, text).unwrap();
    std::fs::write(dir.join("src/l.rs"), "pub struct Slot;").unwrap();
    let data = build_data(
        "fit-loop-entry-data",
        OTHER_BUILD,
        serde_json::json!({}),
        serde_json::json!([]),
    );
    std::fs::write(
        data.join("loops.json"),
        serde_json::json!({
            "schema": 1, "build": {"commit": OTHER_BUILD},
            "loops": [{"name": "ticks", "entry": "tick", "generic": "tick_with", "slot": "Slot", "side": "server"}],
        })
        .to_string(),
    )
    .unwrap();
    let bundle = crate::bundle_in(&dir).unwrap().unwrap();
    let error = game_uses::entries(&bundle.manifest, &data, OTHER_BUILD).unwrap_err();
    assert!(
        error.contains("`ticks`") && error.contains("`tick`"),
        "{error}"
    );
    for folder in [dir, data] {
        std::fs::remove_dir_all(folder).unwrap();
    }
}

#[test]
fn build_data_of_another_build_is_refused_by_the_check() {
    const PATH: &str = "sunrise_client::game::f";
    let dir = project_dir("fit-other-data", None, None);
    let manifest = dir.join(crate::NATIVE_MANIFEST);
    let text = std::fs::read_to_string(&manifest)
        .unwrap()
        .replace(crate::tests::B, crate::MIXINS_REV)
        + &format!(
            "[[native.mixin]]\nid = \"m\"\ntarget = \"{PATH}\"\nkind = \"HEAD\"\nhelper = \"src/m.rs\"\n"
        );
    std::fs::write(&manifest, text).unwrap();
    std::fs::write(dir.join("src/m.rs"), "{}").unwrap();
    let table = serde_json::json!({
        "schema": uses::USES_SCHEMA, "made_for": pin(&dir),
        "entries": [{"id": format!("game:fn:{PATH}"), "hash": "sig:body"}],
    });
    std::fs::write(dir.join(uses::USES_FILE), table.to_string()).unwrap();
    let surface_file = dir.join("target-surface.json");
    std::fs::write(
        &surface_file,
        serde_json::json!({"schema": mod_surface::SCHEMA, "hash": "s"}).to_string(),
    )
    .unwrap();
    let functions = serde_json::json!({PATH: function("body")});
    let plan = |data: &Path| Plan {
        contract: crate::contract::CONTRACT,
        mods: dir.clone(),
        build: OTHER_BUILD.into(),
        surface: Some(surface_file.clone()),
        build_data: Some(data.to_owned()),
    };
    // The pinned build's own data would compare that build with itself.
    let stale = build_data(
        "fit-other-data-stale",
        &pin(&dir),
        functions.clone(),
        serde_json::json!([]),
    );
    let error = check(&plan(&stale)).unwrap_err();
    assert!(
        error.contains("functions.json") && error.contains(OTHER_BUILD),
        "{error}"
    );
    let right = build_data(
        "fit-other-data-right",
        OTHER_BUILD,
        functions,
        serde_json::json!([]),
    );
    assert_eq!(
        check(&plan(&right)).unwrap().mods[0].verdict,
        Some(Verdict::Fits {
            made_for: pin(&dir)
        })
    );
    for folder in [dir, stale, right] {
        std::fs::remove_dir_all(folder).unwrap();
    }
}

#[test]
fn a_mod_with_rows_no_list_can_judge_is_exact_whatever_its_list_says() {
    let dir = project_dir("fit-exact", Some(&[("fn:give", "h1")]), None);
    let manifest = dir.join(crate::NATIVE_MANIFEST);
    let text = std::fs::read_to_string(&manifest).unwrap()
        + "[[native.module]]\nid = \"m\"\nfile = \"game.rs\"\nname = \"m\"\nsource = \"src/m.rs\"\n";
    std::fs::write(&manifest, text).unwrap();
    std::fs::write(dir.join("src/m.rs"), "pub fn m() {}").unwrap();
    let target = surface(&[("fn:give", "h1")]);
    assert_eq!(
        judged(&dir, OTHER_BUILD, &target),
        Verdict::Exact {
            made_for: pin(&dir),
            why: "[[native.module]]"
        }
    );
    std::fs::remove_dir_all(dir).unwrap();
}
