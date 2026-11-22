//! Run a daily that changes process-wide fixtures on its own.

/// True inside the fresh test process. The original test waits for that process and returns.
pub(crate) fn isolated(test: &str) -> bool {
    const CHILD: &str = "SUNRISE_RENDER_ISOLATED_TEST";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        return true;
    }
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", test])
        .env(CHILD, test)
        .status()
        .unwrap();
    assert!(result.success(), "isolated test {test}: {result}");
    false
}
