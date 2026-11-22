/// `[missing_mods]`: whether the dedicated server may open a world whose saved mods are missing.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MissingMods {
    /// Equivalent to starting the game with `--allowMissingMods`. Off by default.
    pub allow: bool,
}

impl MissingMods {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
