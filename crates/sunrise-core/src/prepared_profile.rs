//! The preparation metadata shared by the preparer and the games that consume its output.

use std::path::Path;

pub const MANIFEST: &str = "sunrise-resources.json";
pub const STAMP: &str = "sunrise-prepare.json";
pub const SCHEMA: i64 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Full,
    Server,
}

impl Profile {
    pub const fn output_version(self) -> u32 {
        match self {
            Self::Full => 9,
            Self::Server => 2,
        }
    }

    pub fn repair(self, root: &Path) -> String {
        let mode = match self {
            Self::Full => "--prepare --server-jar PATH",
            Self::Server => "--server",
        };
        let path = std::path::absolute(root).unwrap_or_else(|_| root.to_owned());
        format!("sunrise-prepare {mode} --output \"{}\"", path.display())
    }
}

#[derive(serde::Deserialize)]
pub struct Manifest {
    profile: Option<String>,
    schema: Option<i64>,
    minecraft: Option<String>,
    pub server_sha1: Option<String>,
    server_files: Option<Vec<String>>,
    pub facts_sha256: Option<String>,
    pub block_tags_sha256: Option<String>,
}

impl Manifest {
    pub fn read(root: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(root.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{MANIFEST} is not valid JSON ({e})"))
    }

    pub fn profile(&self) -> Profile {
        if self.profile.as_deref() == Some("server") {
            Profile::Server
        } else {
            Profile::Full
        }
    }

    /// Check the layout and stamp written by the preparer before opening any game resources.
    pub fn version_problems(&self, root: &Path, minecraft: &str) -> Vec<String> {
        let mut problems = Vec::new();
        if self.schema != Some(SCHEMA) {
            let found = self
                .schema
                .map_or_else(|| "none".to_owned(), |v| v.to_string());
            problems.push(format!(
                "an older layout (schema {found}, this preparer writes {SCHEMA})"
            ));
        }
        if self.minecraft.as_deref() != Some(minecraft) {
            let found = self.minecraft.as_deref().unwrap_or("unknown");
            problems.push(format!(
                "prepared for Minecraft {found}, this build needs {minecraft}"
            ));
        }
        let made_by = std::fs::read(root.join(STAMP)).ok().and_then(|bytes| {
            #[derive(serde::Deserialize)]
            struct Stamp {
                output_version: u32,
            }
            Some(serde_json::from_slice::<Stamp>(&bytes).ok()?.output_version)
        });
        let version = self.profile().output_version();
        if made_by != Some(version) {
            let found = made_by.map_or_else(|| "unknown".to_owned(), |v| v.to_string());
            problems.push(format!(
                "made by output version {found}, this build needs {version}"
            ));
        }
        problems
    }

    /// The server profile has no client extras; require the files its manifest lists instead.
    pub fn server_inventory_problems(&self, root: &Path) -> Vec<String> {
        let Some(files) = self.server_files.as_ref().filter(|files| !files.is_empty()) else {
            return vec!["no server resource inventory".into()];
        };
        let mut problems = Vec::new();
        for name in files {
            if !crate::prepared_files::server_resource(name)
                || name.contains(['\\', ':'])
                || name.split('/').any(|part| matches!(part, "" | "." | ".."))
            {
                problems.push(format!("invalid resource in server inventory: {name}"));
            } else if !root.join(name).metadata().is_ok_and(|meta| {
                meta.is_file() && (meta.len() != 0 || name == "data/.mcassetsroot")
            }) {
                problems.push(format!("{name} is missing or empty"));
            }
        }
        problems
    }
}
