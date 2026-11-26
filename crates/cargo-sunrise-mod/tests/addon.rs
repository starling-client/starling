#[path = "support/own_sdk.rs"]
mod own_sdk;

use own_sdk::{OwnSdkScratch, passed};
use std::fs;
use sunrise_mod_package::Package;

#[test]
#[ignore = "daily"]
fn builds_and_checks_a_native_addon_with_local_libraries() {
    // book-quickstart assembles and signs this run's SDK, like the native run smoke test.
    let scratch = OwnSdkScratch::new();
    let root = scratch.root();
    let library = root.join("native");
    let addon = root.join("test_addon");
    let tool = |dir: &std::path::Path, args: &[&str], registry: Option<&str>| {
        scratch.tool_output(dir, args, registry)
    };
    passed(tool(root, &["new", "native", "--template", "native"], None));
    passed(tool(
        root,
        &["new", "test_addon", "--template", "native"],
        None,
    ));
    fs::write(
        library.join("src/lib.rs"),
        r#"
#[derive(Default)]
struct Library;
#[mod_api::sunrise_mod]
impl mod_api::Hooks for Library {}
pub fn magic() -> u32 { 1729 }
#[derive(mod_api::Event)]
#[event(id = "native:reaction", side = server, cancellable)]
#[non_exhaustive]
pub struct Reaction { pub heat: u32, pub cancelled: bool }
"#,
    )
    .unwrap();
    fs::write(
        addon.join("src/lib.rs"),
        r#"
#[derive(Default)]
struct Addon;
#[mod_api::sunrise_mod]
impl mod_api::Hooks for Addon {
    #[subscribe(priority = 5)]
    fn on_reaction(&self, _: &mut dyn mod_api::ServerHost, event: &mut native::Reaction) -> mod_api::Flow {
        if event.heat > 1000 { mod_api::Flow::Cancel } else { mod_api::Flow::Continue }
    }
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        assert_eq!(native::magic(), 1729);
        // The library's event type is shared: the game accepts a listener of it.
        r.add_listener_receive_cancelled(0, Self::after).unwrap();
        println!("addon registered with server library");
        std::process::exit(0);
    }
}
impl Addon {
    fn after(&self, _: &mut dyn mod_api::ServerHost, event: &mut native::Reaction) -> mod_api::Flow {
        event.cancelled = false;
        mod_api::Flow::Continue
    }
}
"#,
    )
    .unwrap();
    // The alias must be the mod ID, not the Cargo package's name.
    let cargo = library.join("Cargo.toml");
    let text = fs::read_to_string(&cargo).unwrap();
    fs::write(&cargo, text.replace("native", "check-library-crate")).unwrap();
    let library_manifest = library.join("sunrise-mod.toml");
    let original = fs::read_to_string(&library_manifest)
        .unwrap()
        .replace("0.1.0", "1.0.0");
    fs::write(&library_manifest, &original).unwrap();
    let addon_manifest = addon.join("sunrise-mod.toml");
    let base_manifest = fs::read_to_string(&addon_manifest).unwrap();
    let dependencies = |rows: &str| {
        fs::write(
            &addon_manifest,
            format!("{base_manifest}\n[dependencies]\n{rows}\n"),
        )
        .unwrap()
    };
    dependencies("native = '^1'");
    let path = library.to_str().unwrap();
    let pin = Package::read(&library)
        .unwrap()
        .manifest
        .native
        .unwrap()
        .sunrise;
    // A missing registry cannot stop a complete local build. Check first: the API route should
    // type-check the linkage without either linking mod libraries or fetching an engine slice.
    let offline = Some("http://127.0.0.1:1");
    passed(tool(&addon, &["check", "--with", path], offline));
    let mut policies = fs::read_dir(root.join("cache/sdk").join(&pin)).unwrap();
    let sdk_cache = policies.next().unwrap().unwrap().path();
    assert!(policies.next().is_none());
    let mut hosts = fs::read_dir(sdk_cache.join("api-slices")).unwrap();
    let host = hosts.next().unwrap().unwrap();
    assert!(hosts.next().is_none());
    let slices = sunrise_native_build::prebuilt::Package::at(&host.path());
    assert!(slices.runtime.join("bin").is_dir());
    assert!(slices.api.join("deps").is_dir());
    assert!(!slices.engine.exists());
    assert!(!sdk_cache.join(host.file_name()).join("engine").exists());
    passed(tool(&addon, &["build", "--with", path], offline));

    let content = root.join("content");
    fs::create_dir(&content).unwrap();
    fs::write(
        content.join("sunrise-mod.toml"),
        "id='content'\nname='Content'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n",
    )
    .unwrap();
    let content_package = root.join("content.smod");
    fs::write(&content_package, Package::pack_directory(&content).unwrap()).unwrap();
    dependencies("native = '^1'\ncontent = '^1'");
    let content_path = content_package.to_str().unwrap();
    for command in ["build", "check"] {
        passed(tool(
            &addon,
            &[command, "--with", path, "--with", content_path],
            offline,
        ));
    }
    dependencies("native = '^1'");
    let source = addon.join("src/lib.rs");
    let code = fs::read_to_string(&source).unwrap();
    fs::write(
        &source,
        format!("{code}\nfn wrong() -> &'static str {{ native::magic() }}"),
    )
    .unwrap();
    let refused = tool(&addon, &["check", "--with", path], None);
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success()
            && error.contains("mismatched types")
            && error.contains(&source.display().to_string()),
        "{error}"
    );
    fs::write(&source, code).unwrap();

    let other_pin = "a".repeat(40);
    fs::write(&library_manifest, original.replace(&pin, &other_pin)).unwrap();
    let refused = tool(&addon, &["check", "--with", path], None);
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success()
            && error.contains("native")
            && error.contains(&other_pin)
            && error.contains("uses list"),
        "{error}"
    );
    fs::write(&library_manifest, original).unwrap();
    let package = root.join("native.smod");
    fs::write(&package, Package::pack_directory(&library).unwrap()).unwrap();
    // A packed library is staged from its archive, as a registry library is after its checks.
    let stdout = passed(tool(
        &addon,
        &["build", "--with", package.to_str().unwrap()],
        offline,
    ));
    assert!(
        stdout.contains("Using library native 1.0.0 from"),
        "{stdout}"
    );
    let output = addon.join("target/sunrise-mod/test_addon.smod");
    assert_eq!(Package::read(&output).unwrap().manifest.id, "test_addon");
    assert_eq!(
        Package::entry(&output, "native/src/lib.rs", 4096).unwrap(),
        None
    );

    // `run` also carries the dependencies beside the SDK's executable. Registration only needs
    // a current full-profile marker and the registries table; it exits before a world opens.
    let resources = scratch.stub_resources();
    let run = addon.join("target/sunrise-mod/run");
    fs::create_dir_all(run.join("game")).unwrap();
    fs::write(run.join("game/eula.txt"), "eula=true\n").unwrap();
    dependencies("native = { version = '^1', side = 'server' }");
    let stdout = passed(tool(
        &addon,
        &[
            "run",
            "--with",
            package.to_str().unwrap(),
            "--resources",
            resources.to_str().unwrap(),
            "--",
            "--dedicated",
        ],
        offline,
    ));
    assert!(
        stdout.contains("addon registered with server library"),
        "{stdout}"
    );
    let records = fs::read_to_string(run.join("app/native-mods.json")).unwrap();
    assert!(
        records.contains("test_addon") && records.contains("native"),
        "{records}"
    );
}
