//! Whether a listed registry release satisfies a mod's dependency.
//!
//! The registry bot (which publishes the releases whose dependencies cannot be met on a build) and
//! the launcher's install planner (which picks the releases to install) must agree on this, so both
//! call this one function. It takes plain values and does no I/O. Whether a release fits a game
//! build is the caller's question, as are which releases are listed at all and reading a version
//! from text: a caller that cannot read a release's version as semver has no offer to make.

use semver::{Version, VersionReq};

/// What a mod asks of a dependency: a row of its release's `dependencies`.
pub struct Requirement<'a> {
    /// The in-game ID of the mod it needs.
    pub mod_id: &'a str,
    /// The range the provider's version must be in.
    pub range: &'a VersionReq,
    /// The one registry project that may provide it, when the row pins one.
    pub project: Option<&'a str>,
}

/// A listed release that might provide a dependency.
pub struct Offer<'a> {
    /// The registry project the release belongs to.
    pub project: &'a str,
    /// The in-game ID that project provides.
    pub mod_id: &'a str,
    pub version: &'a Version,
    /// The release file's SHA-256, in hexadecimal.
    pub sha256: &'a str,
}

/// A row of the registry's revocation list: one file of a project, or every release of it.
pub struct Revoked<'a> {
    pub project: &'a str,
    /// The revoked file's SHA-256, in hexadecimal; `None` revokes the whole project.
    pub sha256: Option<&'a str>,
}

/// Whether `offer` satisfies `requirement`: it provides the same in-game ID, belongs to the pinned
/// project when one is pinned, its version is inside the range, and neither its project nor its
/// file is revoked.
pub fn satisfies(
    requirement: &Requirement<'_>,
    offer: &Offer<'_>,
    revoked: &[Revoked<'_>],
) -> bool {
    offer.mod_id == requirement.mod_id
        && requirement
            .project
            .is_none_or(|pinned| pinned == offer.project)
        && requirement.range.matches(offer.version)
        && !revoked.iter().any(|row| {
            row.project == offer.project
                && row
                    .sha256
                    .is_none_or(|sha256| sha256.eq_ignore_ascii_case(offer.sha256))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asks(range: &str, project: Option<&str>, version: &str, revoked: &[Revoked<'_>]) -> bool {
        let range = VersionReq::parse(range).unwrap();
        let requirement = Requirement {
            mod_id: "wayfinder",
            range: &range,
            project,
        };
        let offer = Offer {
            project: "Ab3dE9xQ",
            mod_id: "wayfinder",
            version: &Version::parse(version).unwrap(),
            sha256: "aa11",
        };
        satisfies(&requirement, &offer, revoked)
    }

    #[test]
    fn a_release_inside_the_range_satisfies_and_one_outside_does_not() {
        assert!(asks("^1.2", None, "1.4.2", &[]));
        assert!(asks("^1.4", None, "1.4.0", &[]));
        assert!(!asks("^2", None, "1.4.2", &[]));
        assert!(!asks("=1.4.1", None, "1.4.2", &[]));
    }

    #[test]
    fn another_mod_or_another_project_than_the_pinned_one_does_not_satisfy() {
        let other_mod = Requirement {
            mod_id: "compass",
            range: &VersionReq::STAR,
            project: None,
        };
        let offer = Offer {
            project: "Ab3dE9xQ",
            mod_id: "wayfinder",
            version: &Version::new(1, 4, 2),
            sha256: "aa11",
        };
        assert!(!satisfies(&other_mod, &offer, &[]));
        assert!(asks("*", Some("Ab3dE9xQ"), "1.4.2", &[]));
        assert!(!asks("*", Some("Zz91kLmN"), "1.4.2", &[]));
    }

    #[test]
    fn a_revoked_project_or_file_does_not_satisfy_and_another_file_of_it_still_does() {
        let whole = Revoked {
            project: "Ab3dE9xQ",
            sha256: None,
        };
        let this_file = Revoked {
            project: "Ab3dE9xQ",
            sha256: Some("AA11"),
        };
        let another_file = Revoked {
            project: "Ab3dE9xQ",
            sha256: Some("bb22"),
        };
        let another_project = Revoked {
            project: "Zz91kLmN",
            sha256: None,
        };
        assert!(!asks("*", None, "1.4.2", &[whole]));
        assert!(!asks("*", None, "1.4.2", &[this_file]));
        assert!(asks("*", None, "1.4.2", &[another_file, another_project]));
    }
}
