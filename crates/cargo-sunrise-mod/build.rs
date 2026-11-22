//! The commit `--version` names: `SUNRISE_TOOL_COMMIT` when a release build sets it, else `local`.
fn main() {
    println!("cargo:rerun-if-env-changed=SUNRISE_TOOL_COMMIT");
    let commit = tool_build_id(std::env::var("SUNRISE_TOOL_COMMIT").ok());
    println!("cargo:rustc-env=SUNRISE_TOOL_BUILT_FROM={commit}");
}

/// Release and SDK builds name their commit; any other build is `local`, never a git call whose answer goes stale.
fn tool_build_id(named: Option<String>) -> String {
    named
        .filter(|commit| !commit.is_empty())
        .unwrap_or_else(|| "local".to_owned())
}

#[cfg(test)]
mod tests {
    use super::tool_build_id;

    #[test]
    fn a_build_that_names_no_commit_is_local() {
        assert_eq!(tool_build_id(None), "local");
        assert_eq!(tool_build_id(Some(String::new())), "local");
        assert_eq!(tool_build_id(Some("abc123".into())), "abc123");
    }
}
