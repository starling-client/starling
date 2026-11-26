use std::{fs, io::Read, process::Command};

#[test]
#[ignore = "daily"]
fn builds_emit_an_identical_ustar_with_only_page_files() {
    let root = tempfile::tempdir().unwrap();
    let tool = |dir: &std::path::Path, args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    tool(root.path(), &["new", "ruby"]);
    let project = root.path().join("ruby");
    let metadata = b"# keep the author's TOML\nsummary = 'Summary'\ndescription = 'Description'\n[[screenshots]]\npath = 'page/z.png'\n[[screenshots]]\npath = 'page/a.jpg'\n";
    let png = include_bytes!("../../sunrise-mod-package/tests/fixtures/page.png");
    let jpeg = include_bytes!("../../sunrise-mod-package/tests/fixtures/page.jpg");
    fs::write(project.join("page.toml"), metadata).unwrap();
    fs::write(project.join("page/z.png"), png).unwrap();
    fs::write(project.join("page/a.jpg"), jpeg).unwrap();
    let bundle = project.join("target/sunrise-mod/ruby.page.tar");
    tool(&project, &["build"]);
    let before = fs::read(&bundle).unwrap();
    tool(&project, &["build"]);
    assert_eq!(fs::read(&bundle).unwrap(), before);
    let mut tar = tar::Archive::new(&before[..]);
    let mut entries = tar.entries().unwrap();
    for (name, contents) in [
        ("page.toml", metadata.as_slice()),
        ("page/a.jpg", jpeg.as_slice()),
        ("page/z.png", png.as_slice()),
    ] {
        let mut entry = entries.next().unwrap().unwrap();
        assert_eq!(entry.path_bytes(), name.as_bytes());
        let header = entry.header();
        assert!(header.as_ustar().is_some());
        assert!(header.entry_type().is_file());
        assert_eq!(header.uid().unwrap(), 0);
        assert_eq!(header.gid().unwrap(), 0);
        assert_eq!(header.mtime().unwrap(), 0);
        assert_eq!(header.mode().unwrap(), 0o644);
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, contents);
    }
    assert!(entries.next().is_none());
    fs::remove_file(project.join("page.toml")).unwrap();
    tool(&project, &["build"]);
    assert!(!bundle.exists());
}

#[test]
#[ignore = "daily"]
fn git_project_build_checks_only_the_files_it_packages() {
    let root = std::env::temp_dir().join(format!("sunrise-content-cli-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let run = |command: &mut Command| {
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let binary = env!("CARGO_BIN_EXE_cargo-sunrise-mod");
    run(Command::new(binary)
        .current_dir(&root)
        .args(["new", "ruby"]));
    let project = root.join("ruby");
    run(Command::new("git")
        .current_dir(&project)
        .args(["init", "--quiet"]));
    // Arbitrary project files, not a list of special ignored directory names.
    fs::create_dir_all(project.join("work")).unwrap();
    fs::write(project.join("work/cache.bin"), b"\0asm").unwrap();
    run(Command::new(binary).current_dir(&project).arg("build"));
    run(Command::new(binary).current_dir(&project).arg("build"));
    let path = project.join("target/sunrise-mod/ruby.smod");
    let package = sunrise_mod_package::Package::read(&path).unwrap();
    assert_eq!(package.manifest.id, "ruby");
    let zip = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
    assert!(zip.file_names().all(|name| {
        let name = name.unwrap();
        name == "sunrise-mod.toml" || name.starts_with("assets/") || name.starts_with("data/")
    }));
    fs::write(project.join("assets/ruby/renamed.bin"), b"\0asm").unwrap();
    let output = Command::new(binary)
        .current_dir(&project)
        .arg("build")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported executable file"));
    fs::remove_dir_all(root).unwrap();
}
