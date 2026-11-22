//! `status <work>`: what a work folder holds, read from the files the steps wrote (docs/api/port-pack.md).

use crate::{
    datapacks,
    fetch::Fetched,
    import::Pack,
    inventory::Record,
    pipeline::{self, read_json},
    report::{self, Row, Summary},
    resourcepacks::{self, Report},
};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// The replies' `format`: a new field keeps it, a changed or removed one makes the next.
pub const FORMAT: u32 = 1;

/// A work folder, as `status --json` prints it.
#[derive(Debug, Serialize)]
pub struct Status {
    pub format: u32,
    pub work: PathBuf,
    /// `pack.json`, as `import` wrote it.
    pub pack: Pack,
    /// `coverage.json`, as `report` wrote it; empty before it ran.
    pub rows: Vec<Row>,
    pub summary: Summary,
    /// `fetch.json`; empty before `fetch` ran.
    pub fetched: Fetched,
    /// The carried-over mods the game loads together, in load order.
    pub mods: Vec<PathBuf>,
    /// Carried-over mods the game leaves off, each with why.
    pub left_off: Vec<String>,
    /// The mods `port` would get and carry over (content of the data tier, made for 1.21 or newer).
    pub portable: usize,
    /// Resource packs staged for the profile.
    pub resourcepacks: usize,
    /// Resource packs the pack lists that `port` can get.
    pub indexed_resources: usize,
    /// Data packs from jars that hold nothing else, on in the profile's new worlds.
    pub datapacks: usize,
    /// `resourcepacks.json` and `profile-resourcepacks.json`: what happened to each resource pack.
    pub resource_reports: ResourceReports,
}

#[derive(Debug, Serialize)]
pub struct ResourceReports {
    pub staged: Report,
    pub profile: Report,
}

/// The carried-over mods in a work folder (`mods/<id>/carry/` with its manifest), by folder name.
pub fn carried(work: &Path) -> Vec<PathBuf> {
    let mut mods: Vec<PathBuf> = fs::read_dir(work.join("mods"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("carry"))
        .filter(|c| c.join("sunrise-mod.toml").is_file())
        .collect();
    mods.sort();
    mods
}

/// The status of `work`: Err when it holds no imported pack.
pub fn status(work: &Path) -> Result<Status, String> {
    let pack: Pack = read_json(&work.join("pack.json"))?;
    let rows: Vec<Row> = read_json(&work.join("coverage.json")).unwrap_or_default();
    let records: Vec<Record> = read_json(&work.join("inventory.json")).unwrap_or_default();
    let (mods, left_off) = pipeline::loadable(&carried(work));
    let old: BTreeSet<&str> = rows
        .iter()
        .filter(|r| r.old_layout)
        .map(|r| r.key.as_str())
        .collect();
    let portable = records
        .iter()
        .filter(|r| pipeline::portable(r) && !old.contains(r.key.as_str()))
        .count();
    let indexed_resources = pack
        .files
        .iter()
        .filter(|f| f.kind() == "resourcepacks" && f.client != "unsupported")
        .count();
    Ok(Status {
        format: FORMAT,
        work: work.to_path_buf(),
        summary: report::summary(&rows),
        fetched: Fetched::load(work),
        resourcepacks: resourcepacks::available(work).len(),
        datapacks: datapacks::installed(work),
        resource_reports: ResourceReports {
            staged: Report::load(work),
            profile: read_json(&work.join("profile-resourcepacks.json")).unwrap_or_default(),
        },
        pack,
        rows,
        mods,
        left_off,
        portable,
        indexed_resources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_import_reports_its_pack_and_nothing_ported() {
        let work = std::env::temp_dir().join(format!("port-pack-status-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(&work).unwrap();
        assert!(status(&work).is_err(), "no pack.json, no status");
        fs::write(
            work.join("pack.json"),
            r#"{"name": "Cozy", "format": "mrpack", "minecraft": "1.21.1", "loader": "neoforge",
                "loader_version": "21.1.1", "overrides": {},
                "files": [{"path": "resourcepacks/glow.zip", "client": "required", "server": ""},
                          {"path": "resourcepacks/off.zip", "client": "unsupported", "server": ""},
                          {"path": "mods/a.jar", "client": "required", "server": "required"}]}"#,
        )
        .unwrap();
        let reply = serde_json::to_value(status(&work).unwrap()).unwrap();
        assert_eq!(reply["format"], 1);
        assert_eq!(reply["pack"]["name"], "Cozy");
        assert_eq!(reply["pack"]["files"].as_array().unwrap().len(), 3);
        assert_eq!(reply["indexed_resources"], 1);
        assert_eq!(reply["rows"], serde_json::json!([]));
        assert_eq!(reply["fetched"]["files"], serde_json::json!([]));
        assert_eq!(reply["mods"], serde_json::json!([]));
        for count in ["portable", "resourcepacks", "datapacks"] {
            assert_eq!(reply[count], 0, "{count}");
        }
        assert_eq!(
            reply["summary"],
            serde_json::json!({"needing": 0, "carried": 0, "partial": 0, "ready": 0, "blocked": 0, "later": 0})
        );
        fs::remove_dir_all(&work).unwrap();
    }
}
