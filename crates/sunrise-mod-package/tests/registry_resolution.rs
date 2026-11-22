#![cfg(feature = "index")]

use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};
use sunrise_mod_package::{
    index::{
        Index,
        resolve::{self, InstallPlan, InstalledMod, ModVersion, Request, RequestResult},
    },
    manifest::Side,
};

#[derive(Deserialize)]
struct Fixture {
    index: serde_json::Value,
    game: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    requests: Vec<Request>,
    installed: Vec<InstalledMod>,
    side: Side,
    native_fits: BTreeMap<String, BTreeMap<String, bool>>,
    expected: InstallPlan,
    refusals: BTreeMap<String, Vec<String>>,
    removal: Option<Removal>,
}

#[derive(Deserialize)]
struct Removal {
    id: String,
    expected: Vec<ModVersion>,
}

#[test]
fn registry_resolution_fixtures() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/registry-resolution");
    let mut fixtures: Vec<_> = std::fs::read_dir(folder)
        .unwrap()
        .map(|file| file.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    fixtures.sort();
    assert!(!fixtures.is_empty());
    for file in fixtures {
        let Fixture { index, game, cases } =
            serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        let index = Index::parse(&index.to_string()).unwrap();
        assert!(index.left_out.is_empty(), "{:?}", index.left_out);
        for Case {
            name,
            requests,
            installed,
            side,
            native_fits,
            expected,
            refusals,
            removal,
        } in cases
        {
            let install = |requests: &[Request]| {
                resolve::install(&index, requests, &installed, &game, side, |p, v| {
                    native_fits[&p.id][&v.version]
                })
            };
            let mut plan = install(&requests);
            // Reversing and rotating covers every order for the three-request recovery cases.
            let mut reordered = requests;
            for _ in 0..2 {
                reordered.reverse();
                for _ in 0..reordered.len() {
                    assert_eq!(install(&reordered), plan, "{name}: request order");
                    reordered.rotate_left(1);
                }
            }
            for (id, words) in refusals {
                let Some(RequestResult::Refused { reason }) = plan.requested.remove(&id) else {
                    panic!("{name}: expected a refusal for {id}: {plan:?}");
                };
                for word in words {
                    assert!(
                        reason.contains(&word),
                        "{name}: missing {word:?} in {reason:?}"
                    );
                }
            }
            assert_eq!(plan, expected, "{name}");
            if let Some(removal) = removal {
                assert_eq!(
                    resolve::removal_dependents(&index, &installed, &removal.id, side).unwrap(),
                    removal.expected,
                    "{name}",
                );
            }
        }
    }
}
