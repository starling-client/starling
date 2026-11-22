//! The exporters' JVMs run without a console window on Windows and keep their output in the log file.
#![cfg(windows)]

use std::{fs, path::Path, path::PathBuf, process::Command, sync::OnceLock};
use sunrise_prepare::java::Jdk;

fn console_probe() -> &'static Path {
    static PROBE: OnceLock<PathBuf> = OnceLock::new();
    PROBE.get_or_init(|| {
        let root = std::env::temp_dir().join(format!(
            "sunrise-prepare-console-probe-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let executable = root.join("console-probe.exe");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/console_probe.rs");
        let output = Command::new("rustc")
            .arg(source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        executable
    })
}

#[test]
#[ignore = "daily"]
fn exporters_write_to_their_log_file_without_a_console() {
    let root = console_probe().parent().unwrap();
    let path = root.join("exporter.log");
    let log = fs::File::create(&path).unwrap();
    let jdk = Jdk {
        java: console_probe().into(),
        javac: console_probe().into(),
    };
    jdk.java(&[], Some(root), &log).unwrap();
    let output = fs::read_to_string(path).unwrap();
    for expected in [
        "console_window_visible=false",
        "stdout is captured",
        "stderr is captured",
    ] {
        assert!(output.contains(expected), "missing {expected}: {output}");
    }
}
