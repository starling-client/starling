use super::{add, tests::Scratch};
use std::{
    fs,
    path::{Path, PathBuf},
};
use sunrise_mod_package::{
    Manifest,
    manifest::{MANIFEST, Native},
};

const METHOD: &str = "sunrise_client::game::ported_level::PortedGame::tick";
const HASH: &str = "c53f233eacb5389b";

struct Fixture(Scratch);

impl Fixture {
    fn new(modules: bool) -> Self {
        let root = Scratch::new("module-discovery");
        let mut text = format!(
            "id='example'\nname='Example'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        if modules {
            text.push_str("[[native.module]]\nid='ported'\nfile='game.rs'\nname='ported_level'\nsource='src/ported.rs'\n");
        }
        fs::write(root.join(MANIFEST), text).unwrap();
        let catalog = root.join(if modules {
            "module-catalog".to_owned()
        } else {
            format!(".sunrise/cache/{}/build-data", "a".repeat(40))
        });
        fs::create_dir_all(&catalog).unwrap();
        if !modules {
            crate::sdk_fetch::CacheStamp::new(&"a".repeat(40), &crate::sdk_fetch::keys())
                .write(catalog.parent().unwrap())
                .unwrap();
        }
        let build = serde_json::json!({"commit": "a".repeat(40)});
        fs::write(catalog.join("internals.json"), serde_json::json!({"schema": 1, "build": build, "internals": [{
            "path": METHOD, "kind": "method", "signature": "impl PortedGame { fn tick(&mut self) -> u32 }", "hash": HASH,
        }]}).to_string()).unwrap();
        Self(root)
    }

    fn manifest(&self) -> Manifest {
        Manifest::parse(&fs::read_to_string(self.0.join(MANIFEST)).unwrap()).unwrap()
    }
}

fn discover(dir: &Path, native: &Native) -> Result<PathBuf, String> {
    assert_eq!(native.module[0].id, "ported");
    assert_eq!(native.sunrise, "a".repeat(40));
    Ok(dir.join("module-catalog"))
}

#[test]
fn internals_add_uses_the_module_catalog_hash() {
    let fixture = Fixture::new(true);
    add(&fixture.0, &[METHOD], Some("method"), discover).unwrap();
    let declared = fixture.manifest();
    assert_eq!(declared.internals.len(), 1);
    assert_eq!(declared.internals[0].path, METHOD);
    assert_eq!(declared.internals[0].hash.as_deref(), Some(HASH));
}

#[test]
fn module_free_add_uses_the_prebuilt_catalog_without_discovery() {
    let fixture = Fixture::new(false);
    let unexpected = |_: &Path, _: &Native| panic!("started discovery for a module-free mod");
    add(&fixture.0, &[METHOD], Some("method"), unexpected).unwrap();
    let declared = fixture.manifest();
    assert_eq!(declared.internals[0].hash.as_deref(), Some(HASH));
}
