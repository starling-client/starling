//! The trust boundary for a mod-owned registry lock. Never resolve missing pins online.
use std::collections::BTreeMap;
use sunrise_mod_package::native_route::game_registry_pin_conflicts;

const CRATES_IO: &str = sunrise_addresses::CARGO_CRATES_IO_SOURCE;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Identity {
    pub name: String,
    pub version: semver::Version,
    pub source: String,
}

#[derive(Clone, Debug)]
pub struct Record {
    checksum: Option<String>,
    raw: toml::Table,
}

#[derive(Clone, Debug, Default)]
pub struct RegistryLock {
    pub packages: BTreeMap<Identity, Record>,
    local: BTreeMap<Identity, Record>,
}

impl RegistryLock {
    /// Parse untrusted lock records once; consumers use typed identities.
    pub fn parse(text: &str) -> Result<Self, String> {
        let lock: toml::Table = text.parse().map_err(|e| format!("Cargo.lock: {e}"))?;
        let records = lock
            .get("package")
            .and_then(toml::Value::as_array)
            .ok_or("Cargo.lock: missing packages")?;
        let mut parsed = Self::default();
        for record in records {
            let table = record.as_table().ok_or("Cargo.lock: invalid package")?;
            let field = |key| {
                table
                    .get(key)
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| format!("Cargo.lock: package missing {key}"))
            };
            let name = field("name")?.to_owned();
            let version = semver::Version::parse(field("version")?)
                .map_err(|e| format!("Cargo.lock: {name}: {e}"))?;
            let source = match table.get("source") {
                None => String::new(),
                Some(value) if value.as_str() == Some(CRATES_IO) => CRATES_IO.to_owned(),
                Some(_) => {
                    return Err(format!(
                        "Cargo.lock: {name} {version}: only crates.io sources are allowed"
                    ));
                }
            };
            let checksum = if source.is_empty() {
                None
            } else {
                let checksum = field("checksum")?;
                if checksum.len() != 64
                    || !checksum
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                {
                    return Err(format!(
                        "Cargo.lock: {name} {version}: invalid crates.io checksum"
                    ));
                }
                Some(checksum.to_owned())
            };
            let identity = Identity {
                name,
                version,
                source,
            };
            let records = if identity.source.is_empty() {
                &mut parsed.local
            } else {
                &mut parsed.packages
            };
            if records
                .insert(
                    identity.clone(),
                    Record {
                        checksum,
                        raw: table.clone(),
                    },
                )
                .is_some()
            {
                return Err(format!(
                    "Cargo.lock: duplicate package {} {}",
                    identity.name, identity.version
                ));
            }
        }
        Ok(parsed)
    }

    pub fn extend(&mut self, other: &Self) -> Result<(), String> {
        for (identity, record) in &other.packages {
            if let Some(previous) = self.packages.get(identity) {
                if previous.checksum != record.checksum {
                    return Err(format!(
                        "Cargo.lock: conflicting pins for {} {}",
                        identity.name, identity.version
                    ));
                }
            } else {
                self.packages.insert(identity.clone(), record.clone());
            }
        }
        Ok(())
    }

    /// Returns the registry pins in `resolved` that moved a version this lock already supplies.
    /// SDK-local identities are a separate source, checked by `check_pins`.
    pub(super) fn moved_pins<'game, 'resolved>(
        &'game self,
        resolved: &'resolved Self,
    ) -> impl Iterator<Item = (&'game Identity, &'resolved Identity)> {
        resolved.packages.keys().filter_map(move |id| {
            (!self.packages.contains_key(id))
                .then(|| {
                    self.packages.keys().find(|game| {
                        game.name == id.name
                            && game.source == id.source
                            && game_registry_pin_conflicts(&game.version, &id.version)
                    })
                })
                .flatten()
                .map(|game| (game, id))
        })
    }

    /// Withholds the registry identity of each crate the SDK builds from its own source (`names`,
    /// local in the game's lock), so a mod's pin cannot authorize a registry copy of it at the
    /// SDK's version.
    pub fn withhold_sdk_patched(
        &mut self,
        game: &Self,
        names: &std::collections::BTreeSet<String>,
    ) {
        self.packages.retain(|id, _| {
            !names.contains(&id.name)
                || !game
                    .local
                    .keys()
                    .any(|sdk| sdk.name == id.name && sdk.version == id.version)
        });
    }

    /// Checks a resolved build graph against these pins (the game's and the mods'): each registry
    /// package in it is pinned here by version and checksum, and a package the SDK supplies itself
    /// keeps the SDK's version.
    ///
    /// Dependency edges are not compared. A registry package in a mod's graph is compiled for the
    /// mod and linked into the mod's library; the prebuilt game's libraries are never rebuilt or
    /// relinked, so the features a mod turns on in its own copy of a package the game also uses
    /// cannot reach the game.
    pub fn check_pins(&self, resolved: &Self) -> Result<(), String> {
        let versions = |ids: &BTreeMap<Identity, Record>, name: &str| {
            ids.keys()
                .filter(|id| id.name == name)
                .map(|id| id.version.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        for id in resolved.local.keys() {
            let sdk = versions(&self.local, &id.name);
            if !sdk.is_empty() && !self.local.contains_key(id) {
                return Err(format!(
                    "Cargo.lock: the SDK supplies {} {sdk}, and the build resolved {} {}. A mod cannot replace a package of the game.",
                    id.name, id.name, id.version
                ));
            }
        }
        for (id, record) in &resolved.packages {
            let Some(pin) = self.packages.get(id) else {
                let sdk = versions(&self.local, &id.name);
                if !sdk.is_empty() {
                    return Err(format!(
                        "Cargo.lock: the SDK supplies {} {sdk} from its own source, and the build resolved {} {} from crates.io. A mod cannot replace a package of the game.",
                        id.name, id.name, id.version
                    ));
                }
                let pinned = versions(&self.packages, &id.name);
                return Err(format!(
                    "Cargo.lock: {} {} is not pinned: the game's and the mods' locks pin [{pinned}]. A mod's Cargo.lock is older than its Cargo.toml; run native-lock with update on that mod.",
                    id.name, id.version
                ));
            };
            if pin.checksum != record.checksum {
                return Err(format!(
                    "Cargo.lock: the checksum of {} {} is not the pinned one. Run native-lock with update on the mod that brings it, and compare the new lock before trusting it.",
                    id.name, id.version
                ));
            }
        }
        Ok(())
    }

    pub fn game_package_names(&self) -> std::collections::BTreeSet<String> {
        self.local.keys().map(|id| id.name.clone()).collect()
    }

    pub fn seed(&self, text: &str) -> Result<String, String> {
        let mut lock: toml::Table = text.parse().map_err(|e| format!("Cargo.lock: {e}"))?;
        let records = lock
            .get_mut("package")
            .and_then(toml::Value::as_array_mut)
            .ok_or("Cargo.lock: missing packages")?;
        records.retain(|record| record.get("source").is_none());
        records.extend(
            self.packages
                .values()
                .map(|record| toml::Value::Table(record.raw.clone())),
        );
        Ok(lock.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lock(source: &str, checksum: &str) -> String {
        format!(
            "version = 4\n[[package]]\nname = 'itoa'\nversion = '1.0.15'\nsource = '{source}'\nchecksum = '{checksum}'\n"
        )
    }
    /// A crates.io record of `name` at `version` depending on `dependencies`, with a checksum of `sum`s.
    fn package(name: &str, version: &str, sum: char, dependencies: &[&str]) -> String {
        format!(
            "[[package]]\nname='{name}'\nversion='{version}'\nsource='{CRATES_IO}'\nchecksum='{}'\ndependencies={dependencies:?}\n",
            sum.to_string().repeat(64)
        )
    }
    fn parsed(records: &[String]) -> RegistryLock {
        RegistryLock::parse(&format!("version=4\n{}", records.concat())).unwrap()
    }
    /// The game's pins and one mod's, as a build plan joins them.
    fn allowed(game: &RegistryLock, pins: &RegistryLock) -> RegistryLock {
        let mut allowed = game.clone();
        allowed.extend(pins).unwrap();
        allowed
    }

    #[test]
    fn separate_zero_zero_patch_versions_do_not_move_game_pins() {
        let game = parsed(&[package("early-crate", "0.0.1", 'a', &[])]);
        let resolved = parsed(&[
            package("early-crate", "0.0.1", 'a', &[]),
            package("early-crate", "0.0.2", 'b', &[]),
        ]);
        assert!(game.moved_pins(&resolved).next().is_none());
    }

    #[test]
    fn a_mods_copy_of_a_package_the_game_locks_takes_the_mods_features_and_dependencies() {
        let game = parsed(&[
            package("arbitrary", "1.5.0", 'a', &[]),
            package("getrandom", "0.4.3", 'b', &["libc"]),
            package("libc", "0.2.190", 'c', &[]),
        ]);
        // zip turns on arbitrary's `derive`, and the mod's lock moved libc under the game's getrandom.
        let pins = parsed(&[
            package("zip", "7.0.0", 'd', &["arbitrary", "getrandom"]),
            package("arbitrary", "1.5.0", 'a', &["derive_arbitrary"]),
            package("derive_arbitrary", "1.5.0", 'e', &[]),
            package("getrandom", "0.4.3", 'b', &["libc"]),
            package("libc", "0.2.191", 'f', &[]),
        ]);
        allowed(&game, &pins).check_pins(&pins).unwrap();
    }

    #[test]
    fn a_package_no_lock_pins_is_refused_by_name_with_the_way_out() {
        let game = parsed(&[package("libc", "0.2.190", 'c', &[])]);
        let pins = parsed(&[
            package("zip", "9.1.3", 'd', &["libc"]),
            package("libc", "0.2.190", 'c', &[]),
        ]);
        let resolved = parsed(&[
            package("zip", "9.1.3", 'd', &["libc"]),
            package("libc", "0.2.191", 'f', &[]),
        ]);
        let error = allowed(&game, &pins).check_pins(&resolved).unwrap_err();
        for detail in ["libc 0.2.191", "0.2.190", "native-lock"] {
            assert!(error.contains(detail), "{error}");
        }
    }

    #[test]
    fn a_pinned_package_with_another_checksum_is_refused() {
        let game = parsed(&[package("libc", "0.2.190", 'c', &[])]);
        let resolved = parsed(&[package("libc", "0.2.190", 'f', &[])]);
        let error = game.check_pins(&resolved).unwrap_err();
        assert!(error.contains("checksum of libc 0.2.190"), "{error}");
    }

    #[test]
    fn a_package_the_sdk_supplies_keeps_the_sdks_version_and_source() {
        let sdk = |version: &str| format!("[[package]]\nname='naga'\nversion='{version}'\n");
        let game = parsed(&[sdk("30.0.1")]);
        let names = std::collections::BTreeSet::from(["naga".to_owned()]);
        // A mod's own lock is made without the SDK's patches, so it pins the registry's naga.
        let pins = parsed(&[
            package("mod-parent", "1.0.0", 'a', &["naga 30.0.1"]),
            package("naga", "30.0.1", 'b', &[]),
            package("older", "1.0.0", 'c', &["naga 29.0.0"]),
            package("naga", "29.0.0", 'd', &[]),
        ]);
        let mut allowed = allowed(&game, &pins);
        allowed.withhold_sdk_patched(&game, &names);
        let built = |naga: String| {
            parsed(&[
                package("mod-parent", "1.0.0", 'a', &["naga 30.0.1"]),
                naga,
                package("older", "1.0.0", 'c', &["naga 29.0.0"]),
                package("naga", "29.0.0", 'd', &[]),
            ])
        };
        // The SDK's naga, beside another version that is the mod's own.
        allowed.check_pins(&built(sdk("30.0.1"))).unwrap();
        let resourced = RegistryLock::parse(&format!(
            "version=4\n{}{}",
            package("mod-parent", "1.0.0", 'a', &["naga"]),
            package("naga", "30.0.1", 'b', &[])
        ))
        .unwrap();
        let moved = RegistryLock::parse(&format!(
            "version=4\n{}{}",
            package("mod-parent", "1.0.0", 'a', &["naga"]),
            sdk("30.0.2")
        ))
        .unwrap();
        for refused in [resourced, moved] {
            let error = allowed.check_pins(&refused).unwrap_err();
            assert!(error.contains("the SDK supplies naga 30.0.1"), "{error}");
        }
    }

    #[test]
    fn mods_that_pin_different_versions_of_one_package_build_on_either_pin() {
        let game = RegistryLock::default();
        let first = parsed(&[package("dtoa", "1.0.9", 'a', &[])]);
        let second = parsed(&[package("dtoa", "1.0.11", 'b', &[])]);
        let mut allowed = allowed(&game, &first);
        allowed.extend(&second).unwrap();
        // One workspace resolves one of them for both mods.
        allowed.check_pins(&first).unwrap();
        allowed.check_pins(&second).unwrap();
        let neither = parsed(&[package("dtoa", "1.0.10", 'c', &[])]);
        let error = allowed.check_pins(&neither).unwrap_err();
        assert!(
            error.contains("dtoa 1.0.10") && error.contains("1.0.9, 1.0.11"),
            "{error}"
        );
    }

    #[test]
    fn source_and_checksum_boundary() {
        let checksum = "a".repeat(64);
        assert_eq!(
            RegistryLock::parse(&lock(CRATES_IO, &checksum))
                .unwrap()
                .packages
                .len(),
            1
        );
        for source in [
            "git+https://example.invalid/repo",
            "registry+https://example.invalid/index",
            "sparse+https://index.crates.io/",
        ] {
            assert!(
                RegistryLock::parse(&lock(source, &checksum))
                    .unwrap_err()
                    .contains("only crates.io")
            );
        }
        for checksum in ["", "no checksum", &"A".repeat(64), &"a".repeat(63)] {
            assert!(
                RegistryLock::parse(&lock(CRATES_IO, checksum))
                    .unwrap_err()
                    .contains("checksum")
            );
        }
        assert!(RegistryLock::parse("version = 4").is_err());
    }
    #[test]
    fn union_preserves_pins_and_refuses_conflicting_checksums() {
        let mut a = RegistryLock::parse(&lock(CRATES_IO, &"a".repeat(64))).unwrap();
        let same = a.clone();
        a.extend(&same).unwrap();
        assert_eq!(a.packages.len(), 1);
        let different = RegistryLock::parse(&lock(CRATES_IO, &"b".repeat(64))).unwrap();
        assert!(a.extend(&different).unwrap_err().contains("itoa 1.0.15"));
    }
}
