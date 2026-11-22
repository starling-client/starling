//! The manifest shared by the game and launcher, and recovery of interrupted game installs.
mod file_claim;

use crate::durable;
use crate::install_lock::lock_home;
use file_claim::{Conflict, FileClaim};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::Path;

pub const MANIFEST: &str = "mod-browser.json";
// There is at most one in-flight install in a home: all browser writers share the home lock.
const PENDING: &str = ".mod-browser.pending.json";
// Old and new can come from different indexes. Each row stores the version label twice,
// so allow two index budgets per row, plus space for fixed fields and the journal envelope.
const MAX_PENDING: u64 = 4 * super::MAX_INDEX + (4 << 10);

/// The managed path for a content mod whose id has been validated by the registry.
pub fn content_file(id: &str) -> String {
    format!("{}/{id}.smod", crate::mod_list::DIR)
}

#[derive(Serialize, Deserialize)]
struct Pending {
    id: String,
    file: String,
    old: Option<Value>,
    new: Value,
}

/// Settle an interrupted game install under the home lock before reading the browser manifest.
/// Mutating callers must still hold their own guard across the entire read/modify/write.
pub fn reconcile(home: &Path) -> Result<(), String> {
    let _lock = lock_home(home).map_err(|e| format!("{}: {e}", home.display()))?;
    reconcile_locked(home)
}

fn reconcile_locked(home: &Path) -> Result<(), String> {
    let path = home.join(PENDING);
    let error = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    match fs::symlink_metadata(&path) {
        Ok(info) if !info.is_file() => return Err(error(&"not a regular file")),
        Ok(info) if info.len() > MAX_PENDING => return Err(error(&"pending entry too large")),
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(error(&e)),
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|file| file.take(MAX_PENDING + 1).read_to_end(&mut bytes))
        .map_err(|e| error(&e))?;
    if bytes.len() as u64 > MAX_PENDING {
        return Err(error(&"pending entry too large"));
    }
    let pending: Pending = serde_json::from_slice(&bytes).map_err(|e| error(&e))?;
    let sha256 = pending.new["checksum"]
        .as_str()
        .and_then(|hash| hash.strip_prefix("SHA-256:"))
        .ok_or_else(|| error(&"missing SHA-256"))?;
    let valid_row = |row: &Value| {
        row["source"] == "sunrise" && row["project"] == pending.id && row["file"] == pending.file
    };
    if !super::valid_id(&pending.id)
        || pending.file != content_file(&pending.id)
        || !valid_row(&pending.new)
        || !pending.new["version"].is_string()
        || !pending.old.as_ref().is_none_or(valid_row)
    {
        return Err(error(&"invalid pending entry"));
    }
    super::hex64(sha256).map_err(|e| error(&e))?;
    let mut list = Installed::read(home)?;
    // Check before reading the archive or discarding any intent, even if the archive
    // has not been created or the manifest already contains the new row.
    list.check_file_claim(home, &pending.file, &pending.id)
        .map_err(|e| error(&e))?;
    let target = home.join(&pending.file);
    let published = match fs::symlink_metadata(&target) {
        Ok(info) if info.is_file() && info.len() <= super::MAX_FILE => {
            super::sha256_file(&target)? == sha256
        }
        Ok(_) => false,
        Err(e) if e.kind() == ErrorKind::NotFound => false,
        Err(e) => return Err(format!("{}: {e}", target.display())),
    };
    if published {
        // The journal settles only the project's single row, or none. It never overwrites
        // a row edited since the install began or drops a duplicate row.
        let current = list.single(&pending.id)?.map(|i| &list.entries[i]);
        // Compare parsed rows so formatting changes don't matter but metadata edits do.
        if current == Some(&pending.new) {
            // The row may be visible before its rename is durable. This also handles a
            // completed Windows install whose sidecar deletion did not survive power loss.
            let manifest = home.join(MANIFEST);
            durable::sync_parent(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
        } else if current == pending.old.as_ref() {
            // The process could have died after renaming but before syncing this directory.
            durable::sync_parent(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            list.record(pending.new);
            list.save(home)?;
        } else {
            return Err(error(
                &"the project's entry changed since the install began",
            ));
        }
    }
    // With the old bytes, or unrelated bytes, keep the old metadata. The usual checks
    // still decide whether the bytes actually match it. This also clears completed installs.
    durable::remove(&path).map_err(|e| error(&e))
}

/// Whether `entry` is the registry's row for project `id`.
fn owned_by(entry: &Value, id: &str) -> bool {
    entry["source"] == "sunrise" && entry["project"] == id
}

/// `mod-browser.json`, kept as JSON values so the launcher's entries this
/// game doesn't model (Modrinth resource packs, ported Java mods) are written
/// back as they were.
#[derive(Debug, Default)]
pub struct Installed {
    pub entries: Vec<Value>,
}

impl Installed {
    /// Loads a consistent snapshot, settling an interrupted install first.
    pub fn load(home: &Path) -> Result<Self, String> {
        let _lock = lock_home(home).map_err(|e| format!("{}: {e}", home.display()))?;
        reconcile_locked(home)?;
        Self::read(home)
    }

    fn read(home: &Path) -> Result<Self, String> {
        let path = home.join(MANIFEST);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let value: Value =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        let entries = value["entries"]
            .as_array()
            .ok_or_else(|| format!("{}: entries must be an array", path.display()))?
            .clone();
        Ok(Self { entries })
    }

    /// Saves the list. Hold the home lock from before loading through this write.
    pub fn save(&self, home: &Path) -> Result<(), String> {
        let path = home.join(MANIFEST);
        let text = serde_json::to_vec_pretty(&serde_json::json!({ "entries": self.entries }))
            .map_err(|e| e.to_string())?;
        durable::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Publishes a checked, already-synced package and its browser row. The caller holds
    /// the home lock from before loading through publication and checks that an existing
    /// target belongs to the browser, not the player.
    pub fn publish(
        &mut self,
        home: &Path,
        id: &str,
        download: &Path,
        entry: Value,
    ) -> Result<(), String> {
        let pending = Pending {
            id: id.to_owned(),
            file: content_file(id),
            old: self.single(id)?.map(|i| self.entries[i].clone()),
            new: entry,
        };
        // Recheck the current filesystem after the download, before writing intent.
        self.check_file_claim(home, &pending.file, id)?;
        let target = home.join(&pending.file);
        let path = home.join(PENDING);
        let bytes = serde_json::to_vec(&pending).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_PENDING {
            return Err(format!("{}: pending entry too large", path.display()));
        }
        durable::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        durable::replace(download, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        self.record(pending.new);
        self.save(home)?;
        durable::remove(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The registry's entry for project `id`.
    pub fn position(&self, id: &str) -> Option<usize> {
        self.entries.iter().position(|e| owned_by(e, id))
    }

    /// The registry's one entry for project `id`, if any. Several are refused: which one holds
    /// later edits is unknown, and replacing them would lose the others.
    pub fn single(&self, id: &str) -> Result<Option<usize>, String> {
        let mut rows = (0..self.entries.len()).filter(|&i| owned_by(&self.entries[i], id));
        match (rows.next(), rows.next()) {
            (_, Some(_)) => Err(format!("several entries in {MANIFEST} name {id}")),
            (first, None) => Ok(first),
        }
    }

    /// Checks that `file` can be used under this home and no other entry can claim it.
    /// Directory links are followed; a link at the destination file is refused. An ambiguous
    /// claim stops installation or recovery without changing the archive, manifest or journal.
    /// Hold the home lock while checking and using the result.
    pub fn check_file_claim(&self, home: &Path, file: &str, id: &str) -> Result<(), String> {
        let target = FileClaim::target(home, file).map_err(|e| format!("{file}: {e}"))?;
        for entry in &self.entries {
            if owned_by(entry, id) {
                continue;
            }
            let Some(other) = entry["file"].as_str() else {
                continue;
            };
            let title = entry["title"].as_str().unwrap_or("another mod");
            let claim = FileClaim::other(home, other).map_err(|e| {
                format!("cannot compare {file} with {title}'s file claim {other:?}: {e}")
            })?;
            if let Some(conflict) = target.conflict_with(&claim) {
                let reason = match conflict {
                    Conflict::Confirmed => "conflicts with",
                    Conflict::Possible => "may conflict with",
                };
                return Err(format!("{file} {reason} {title}'s file claim {other:?}"));
            }
        }
        Ok(())
    }

    fn record(&mut self, entry: Value) {
        let id = entry["project"].clone();
        self.entries
            .retain(|e| !(e["source"] == "sunrise" && e["project"] == id));
        self.entries.push(entry);
    }

    /// The version of `id` installed from the registry whose file is still
    /// there; for a `sunrise-mods/<id>.smod` put there another way, its
    /// manifest's version.
    pub fn version_of(&self, home: &Path, id: &str) -> Option<String> {
        if let Some(e) = self.position(id).map(|i| &self.entries[i])
            && let (Some(version), Some(file)) = (e["version"].as_str(), e["file"].as_str())
            && home.join(file).exists()
        {
            return Some(version.to_owned());
        }
        let path = home.join(content_file(id));
        crate::Package::read(&path)
            .ok()
            .filter(|p| p.manifest.id == id)
            .map(|p| p.manifest.version.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_publication_recovers_the_row_only_for_the_published_bytes() {
        let entry = |version: &str, bytes: &[u8]| {
            serde_json::json!({
                "source": "sunrise", "project": "ruby", "file": content_file("ruby"),
                "version": version, "checksum": format!("SHA-256:{}", super::super::sha256(bytes)),
            })
        };
        let old = entry("1", b"old");
        let new = entry("2", b"new");
        for (bytes, expected) in [(b"new", &new), (b"old", &old), (b"bad", &old)] {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            let guard = lock_home(home).unwrap();
            let manifest = home.join(MANIFEST);
            let backup = home.join("backup");
            let target = home.join(content_file("ruby"));
            let download = home.join("download.part");
            fs::create_dir(home.join(crate::mod_list::DIR)).unwrap();
            durable::write(&target, b"old").unwrap();
            durable::write(&download, b"new").unwrap();
            let mut list = Installed {
                entries: vec![old.clone()],
            };
            list.save(home).unwrap();

            // Make saving the manifest fail after the package rename has succeeded.
            fs::rename(&manifest, &backup).unwrap();
            fs::create_dir(&manifest).unwrap();
            assert!(list.publish(home, "ruby", &download, new.clone()).is_err());
            assert_eq!(fs::read(&target).unwrap(), b"new");
            fs::remove_dir(&manifest).unwrap();
            fs::rename(&backup, &manifest).unwrap();
            if bytes != b"new" {
                // Also model a rename that did not reach disk, and bytes neither row expects.
                durable::write(&target, bytes).unwrap();
            }
            drop(guard);

            let expected_rows = std::slice::from_ref(expected);
            assert_eq!(Installed::load(home).unwrap().entries, expected_rows);
            assert_eq!(Installed::read(home).unwrap().entries, expected_rows);
            assert_eq!(fs::read(&target).unwrap(), bytes);
            assert!(!home.join(PENDING).exists());
        }
    }

    #[test]
    fn an_ambiguous_manifest_keeps_the_journal_and_refuses() {
        let row = |version: &str, bytes: &[u8], file: &str| {
            serde_json::json!({
                "source": "sunrise", "project": "ruby", "file": file,
                "version": version, "checksum": format!("SHA-256:{}", super::super::sha256(bytes)),
            })
        };
        let file = content_file("ruby");
        let old = row("1", b"old", &file);
        let new = row("2", b"new", &file);
        let mut edited = old.clone();
        edited["dependency"] = Value::Bool(true);
        let mut duplicate = old.clone();
        duplicate["title"] = "Ruby, renamed later".into();
        let claim = |file: &str| {
            let mut other = row("1", b"other", file);
            other["project"] = "gem".into();
            other
        };
        for manifest_rows in [
            // The project's row was edited after the install began.
            vec![edited.clone()],
            // Two rows name the project; the second carries a later edit.
            vec![old.clone(), duplicate.clone()],
            // Another project claims the same file under another spelling, or by a path only
            // the filesystem can place.
            vec![old.clone(), claim("./sunrise-mods//ruby.smod")],
            vec![old.clone(), claim("sunrise-mods/../sunrise-mods/ruby.smod")],
            // Even a completed manifest update cannot clear another project's claim.
            vec![new.clone(), claim("./sunrise-mods//ruby.smod")],
        ] {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            fs::create_dir(home.join(crate::mod_list::DIR)).unwrap();
            fs::write(home.join(&file), b"new").unwrap();
            let journal = serde_json::to_vec(&Pending {
                id: "ruby".into(),
                file: file.clone(),
                old: Some(old.clone()),
                new: new.clone(),
            })
            .unwrap();
            fs::write(home.join(PENDING), &journal).unwrap();
            Installed {
                entries: manifest_rows.clone(),
            }
            .save(home)
            .unwrap();
            let before = fs::read(home.join(MANIFEST)).unwrap();
            assert!(reconcile(home).is_err(), "{manifest_rows:?}");
            assert_eq!(fs::read(home.join(PENDING)).unwrap(), journal);
            assert_eq!(fs::read(home.join(MANIFEST)).unwrap(), before);
            assert_eq!(fs::read(home.join(&file)).unwrap(), b"new");
        }
    }

    // Exercise both mutations, not just the comparison: neither may rewrite the rows,
    // replace an archive, clear intent, or consume the staged download on refusal.
    fn assert_alias_refused(home: &Path, alias: &str, target_exists: bool) {
        let file = content_file("ruby");
        let target = home.join(&file);
        let row = |version, bytes: &[u8]| {
            serde_json::json!({
                "source": "sunrise", "project": "ruby", "file": file, "version": version,
                "checksum": format!("SHA-256:{}", super::super::sha256(bytes)),
            })
        };
        let old = row("1", b"old");
        let new = row("2", b"new");
        let mut list = Installed {
            entries: vec![
                old.clone(),
                serde_json::json!({ "source": "modrinth", "file": alias, "title": "Gem" }),
            ],
        };
        let _guard = lock_home(home).unwrap();
        let manifest = home.join(MANIFEST);
        list.save(home).unwrap();
        let before = fs::read(&manifest).unwrap();
        let download = home.join("download.part");
        let journal = home.join(PENDING);
        fs::write(&download, b"new").unwrap();

        assert!(
            list.publish(home, "ruby", &download, new.clone()).is_err(),
            "publish: {alias}"
        );
        assert_eq!(fs::read(&manifest).unwrap(), before);
        assert!(!journal.try_exists().unwrap());
        assert_eq!(fs::read(&download).unwrap(), b"new");
        if target_exists {
            assert_eq!(fs::read(&target).unwrap(), b"old");
            // Now simulate a previously interrupted install, e.g. from a version that
            // didn't check ownership with the same rules. The archive is already new.
            fs::write(&target, b"new").unwrap();
        } else {
            assert!(!target.try_exists().unwrap());
        }
        let intent = serde_json::to_vec(&Pending {
            id: "ruby".into(),
            file,
            old: Some(old),
            new,
        })
        .unwrap();
        fs::write(&journal, &intent).unwrap();
        assert!(reconcile(home).is_err(), "reconcile: {alias}");
        assert_eq!(fs::read(&manifest).unwrap(), before);
        assert_eq!(fs::read(&journal).unwrap(), intent);
        assert_eq!(fs::read(&download).unwrap(), b"new");
        if target_exists {
            assert_eq!(fs::read(&target).unwrap(), b"new");
        } else {
            assert!(!target.try_exists().unwrap());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_cannot_claim_the_same_archive() {
        for target_exists in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            fs::create_dir(home.join("sunrise-mods")).unwrap();
            if target_exists {
                fs::write(home.join(content_file("ruby")), b"old").unwrap();
            }
            std::os::unix::fs::symlink("sunrise-mods", home.join("alias")).unwrap();
            assert_alias_refused(home, "alias/ruby.smod", target_exists);
            assert_eq!(
                fs::read_link(home.join("alias")).unwrap(),
                Path::new("sunrise-mods")
            );
        }
    }

    #[test]
    fn ambiguous_spellings_and_invalid_names_are_refused_before_mutation() {
        for (alias, target_exists) in [
            ("sunrise-mods/RUBY.smod", true),
            ("SUNRISE-MODS/RUBY.smod", false),
            ("sunrise-mods/../sunrise-mods/ruby.smod", true),
            ("sunrise-mods/ruby.smod.", true),
            ("sunrise-mods/ruby.smod ", true),
            ("sunrise-mods/CON.txt", true),
            ("sunrise-mods/rubý.smod", true),
            ("sunrise-mods/RUBY~1.SMO", true),
            ("sunrise-mods/ruby.smod/other.smod", false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            if target_exists {
                fs::create_dir(home.join("sunrise-mods")).unwrap();
                fs::write(home.join(content_file("ruby")), b"old").unwrap();
            }
            assert_alias_refused(home, alias, target_exists);
        }
    }

    #[test]
    fn case_folded_missing_targets_conflict_at_different_lookup_depths() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        fs::create_dir(home.join("sunrise-mods")).unwrap();
        assert_alias_refused(home, "SUNRISE-MODS/RUBY.smod", false);
    }

    #[test]
    fn a_hard_link_at_a_different_path_cannot_claim_the_same_archive() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        fs::create_dir(home.join("sunrise-mods")).unwrap();
        let target = home.join(content_file("ruby"));
        let link = home.join("gem.smod");
        fs::write(&target, b"old").unwrap();
        fs::hard_link(&target, &link).unwrap();
        assert_alias_refused(home, "gem.smod", true);
        assert_eq!(fs::read(link).unwrap(), b"new");
    }

    #[test]
    fn publication_accepts_unrelated_claims() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let _guard = lock_home(home).unwrap();
        fs::create_dir(home.join("sunrise-mods")).unwrap();
        let download = home.join("download.part");
        fs::write(&download, b"new").unwrap();
        fs::create_dir(home.join("resourcepacks")).unwrap();
        let unrelated = [
            serde_json::json!({ "file": "./sunrise-mods/../sunrise-mods/gem.smod" }),
            serde_json::json!({ "file": "resourcepacks/déjà.zip" }),
            serde_json::json!({ "file": "resourcepacks/missing.zip. " }),
        ];
        let mut list = Installed {
            entries: unrelated.to_vec(),
        };
        let row = serde_json::json!({ "source": "sunrise", "project": "ruby", "file": content_file("ruby") });
        list.publish(home, "ruby", &download, row.clone()).unwrap();
        assert_eq!(
            Installed::read(home).unwrap().entries,
            [unrelated.to_vec(), vec![row]].concat()
        );
        assert_eq!(fs::read(home.join(content_file("ruby"))).unwrap(), b"new");
        assert!(!home.join(PENDING).exists());
    }

    #[cfg(unix)]
    #[test]
    fn publication_keeps_a_distinct_archive_reached_through_a_link_and_parents() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let _guard = lock_home(home).unwrap();
        fs::create_dir(home.join("sunrise-mods")).unwrap();
        fs::create_dir_all(home.join("elsewhere/target/nested")).unwrap();
        std::os::unix::fs::symlink("../elsewhere/target", home.join("sunrise-mods/alias")).unwrap();
        let target = content_file("ruby");
        let other = "sunrise-mods/alias/nested/../../ruby.smod";
        let download = home.join("download.part");
        fs::write(home.join(&target), b"old").unwrap();
        fs::write(home.join("elsewhere/ruby.smod"), b"distinct").unwrap();
        fs::write(&download, b"new").unwrap();
        let other_row = serde_json::json!({ "file": other });
        let mut list = Installed {
            entries: vec![other_row.clone()],
        };
        let row = serde_json::json!({ "source": "sunrise", "project": "ruby", "file": target });
        list.publish(home, "ruby", &download, row.clone()).unwrap();
        assert_eq!(Installed::read(home).unwrap().entries, [other_row, row]);
        assert_eq!(fs::read(home.join(target)).unwrap(), b"new");
        assert_eq!(fs::read(home.join(other)).unwrap(), b"distinct");
    }

    #[test]
    fn recovery_can_clear_unpublished_intent_with_a_distinct_uncertain_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let file = content_file("ruby");
        let list = Installed {
            entries: vec![serde_json::json!({ "file": "resourcepacks/déjà.zip" })],
        };
        list.save(home).unwrap();
        let before = fs::read(home.join(MANIFEST)).unwrap();
        fs::write(
            home.join(PENDING),
            serde_json::to_vec(&Pending {
                id: "ruby".into(),
                file: file.clone(),
                old: None,
                new: serde_json::json!({
                    "source": "sunrise", "project": "ruby", "file": file, "version": "1",
                    "checksum": format!("SHA-256:{}", super::super::sha256(b"new")),
                }),
            })
            .unwrap(),
        )
        .unwrap();

        reconcile(home).unwrap();
        assert_eq!(fs::read(home.join(MANIFEST)).unwrap(), before);
        assert!(!home.join(PENDING).try_exists().unwrap());
        assert!(!home.join("sunrise-mods").try_exists().unwrap());
        assert!(!home.join("resourcepacks").try_exists().unwrap());
    }

    #[test]
    fn publication_refuses_several_rows_of_the_project_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let _guard = lock_home(home).unwrap();
        let file = content_file("ruby");
        fs::create_dir(home.join(crate::mod_list::DIR)).unwrap();
        fs::write(home.join(&file), b"old").unwrap();
        let download = home.join("download.part");
        fs::write(&download, b"new").unwrap();
        let row = |title: &str| serde_json::json!({ "source": "sunrise", "project": "ruby", "file": file, "title": title });
        let mut list = Installed {
            entries: vec![row("Ruby"), row("Ruby, renamed later")],
        };
        list.save(home).unwrap();
        let before = fs::read(home.join(MANIFEST)).unwrap();
        assert!(
            list.publish(home, "ruby", &download, row("Ruby 2"))
                .is_err()
        );
        assert_eq!(fs::read(home.join(MANIFEST)).unwrap(), before);
        assert_eq!(fs::read(home.join(&file)).unwrap(), b"old");
        assert!(!home.join(PENDING).exists());
    }

    #[test]
    fn interrupted_publication_recovers_a_large_registry_title() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let _guard = lock_home(home).unwrap();
        let target = home.join(content_file("ruby"));
        fs::create_dir(home.join(crate::mod_list::DIR)).unwrap();
        let download = home.join("download.part");
        durable::write(&download, b"new").unwrap();
        let row = serde_json::json!({
            "source": "sunrise", "project": "ruby", "file": content_file("ruby"),
            "version": "1", "checksum": format!("SHA-256:{}", super::super::sha256(b"new")),
            "title": "an allowed title ".repeat(20_000),
        });
        let manifest = home.join(MANIFEST);
        fs::create_dir(&manifest).unwrap();
        assert!(
            Installed::default()
                .publish(home, "ruby", &download, row.clone())
                .is_err()
        );
        assert_eq!(fs::read(target).unwrap(), b"new");
        fs::remove_dir(&manifest).unwrap();
        assert_eq!(Installed::load(home).unwrap().entries, [row]);
        assert!(!home.join(PENDING).exists());
    }

    #[test]
    fn malformed_or_oversized_sidecars_cannot_change_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let manifest = home.join(MANIFEST);
        let state = br#"{"entries": []}"#;
        fs::write(&manifest, state).unwrap();
        let path = home.join(PENDING);
        let file = content_file("ruby");
        fs::create_dir(home.join(crate::mod_list::DIR)).unwrap();
        fs::write(home.join(&file), b"new").unwrap();
        let pending = Pending {
            id: "ruby".into(),
            file: file.clone(),
            old: None,
            new: serde_json::json!({
                "source": "sunrise", "project": "ruby", "file": file,
                "version": "1", "checksum": format!("SHA-256:{}", super::super::sha256(b"new")),
            }),
        };
        let mut oversized = serde_json::to_vec(&pending).unwrap();
        // Trailing JSON whitespace changes the size without making the intent invalid.
        oversized.resize(MAX_PENDING as usize + 1, b' ');
        for bytes in [b"not json".to_vec(), oversized] {
            fs::write(&path, bytes).unwrap();
            assert!(reconcile(home).is_err());
            assert!(path.is_file());
            assert_eq!(fs::read(&manifest).unwrap(), state);
        }
    }
}
