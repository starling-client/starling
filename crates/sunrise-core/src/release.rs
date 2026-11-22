//! Which Sunrise release this build is (docs/versioning.md). Not Minecraft's version: that is
//! [`crate::version`], generated from the game's own files.

/// The release's version (`0.2.3`, no `v`): release.yml builds every release with `SUNRISE_VERSION`
/// set from its tag, so there are no version-bump commits. None for a build from source.
pub const RELEASE: Option<&str> = match option_env!("SUNRISE_VERSION") {
    Some(version) if !version.is_empty() => Some(version),
    _ => None,
};

/// [`RELEASE`], or this crate's Cargo.toml version for a build from source: what `--version`, user
/// agents and reports say.
pub const VERSION: &str = match RELEASE {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// A nightly's date and commit (`2026-10-28 0c02067`): nightly-build.yml builds the opt-in Nightly
/// channel with `SUNRISE_NIGHTLY` set (docs/release-flow.md), and a Dev build the same way, with
/// `SUNRISE_VERSION` `dev-<date>` ([`kind`]). None for a release or a build from source.
pub const NIGHTLY: Option<&str> = match option_env!("SUNRISE_NIGHTLY") {
    Some(nightly) if !nightly.is_empty() => Some(nightly),
    _ => None,
};

/// How a Nightly or Dev build names itself where players read it, so their reports say which
/// build they come from: `Nightly 2026-10-28 · 0c02067`, `Dev 2026-11-13 · 0c02067`. None for
/// any other build.
pub fn nightly_label() -> Option<String> {
    NIGHTLY.map(|nightly| label(kind(RELEASE), nightly))
}

/// A Nightly or Dev build's version as the launcher installs it (`nightly-2026-10-28-0c02067`,
/// `dev-2026-11-13-0c02067`).
pub fn nightly_version() -> Option<String> {
    NIGHTLY.map(|n| {
        format!(
            "{}-{}",
            kind(RELEASE).to_ascii_lowercase(),
            n.replace(' ', "-")
        )
    })
}

/// Which build of nightly-build.yml this is: `Dev` when it was built as one (release.yml kind
/// dev sets `SUNRISE_VERSION` to `dev-<date>`), else `Nightly`.
fn kind(release: Option<&str>) -> &'static str {
    if release.is_some_and(|version| version.starts_with("dev-")) {
        "Dev"
    } else {
        "Nightly"
    }
}

fn label(kind: &str, nightly: &str) -> String {
    match nightly.split_once(' ') {
        Some((date, commit)) => format!("{kind} {date} · {commit}"),
        None => format!("{kind} {nightly}"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_nightly_names_its_date_and_commit() {
        assert_eq!(
            super::label("Nightly", "2026-10-28 0c02067"),
            "Nightly 2026-10-28 · 0c02067"
        );
        assert_eq!(super::label("Nightly", "2026-10-28"), "Nightly 2026-10-28");
        assert_eq!(super::nightly_label().is_some(), super::NIGHTLY.is_some());
    }

    #[test]
    fn a_dev_build_names_itself_dev() {
        assert_eq!(super::kind(Some("dev-2026-11-13")), "Dev");
        assert_eq!(super::kind(Some("nightly-2026-11-13")), "Nightly");
        assert_eq!(super::kind(None), "Nightly");
        assert_eq!(
            super::label(super::kind(Some("dev-2026-11-13")), "2026-11-13 0c02067"),
            "Dev 2026-11-13 · 0c02067"
        );
    }

    #[test]
    fn the_version_is_the_release_or_the_crate() {
        assert_eq!(
            super::RELEASE,
            option_env!("SUNRISE_VERSION").filter(|v| !v.is_empty())
        );
        assert_eq!(
            super::VERSION,
            super::RELEASE.unwrap_or(env!("CARGO_PKG_VERSION"))
        );
        assert!(!super::VERSION.starts_with('v'));
    }
}
