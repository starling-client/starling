//! Immutable physical-process information.

/// The kind of process running this mod, independent of a hook's logical side.
// no wire: describes the current process, not a property negotiated with a peer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Dist {
    /// A client process, including its integrated server.
    #[default]
    Client,
    /// A dedicated server process.
    DedicatedServer,
}

/// Process information captured by the engine before registration and read locally.
// no wire: this snapshot describes only the local process at startup.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: fields are private; only the engine constructor supplies a snapshot.
pub struct RuntimeEnvironment {
    dist: Dist,
    game_version: std::sync::Arc<str>,
    sunrise_version: std::sync::Arc<str>,
    game_tests_enabled: bool,
}

impl RuntimeEnvironment {
    /// Capture the engine's fixed startup values.
    pub fn new(
        dist: Dist,
        game_version: String,
        sunrise_version: String,
        game_tests_enabled: bool,
    ) -> Self {
        Self {
            dist,
            game_version: game_version.into(),
            sunrise_version: sunrise_version.into(),
            game_tests_enabled,
        }
    }
    /// The physical process; an integrated server remains a client.
    pub fn dist(&self) -> Dist {
        self.dist
    }
    /// The exact supported Minecraft version.
    pub fn game_version(&self) -> &str {
        &self.game_version
    }
    /// The version printed in Sunrise crash reports.
    pub fn sunrise_version(&self) -> &str {
        &self.sunrise_version
    }
    /// Whether test-only content was enabled at process startup.
    pub fn game_tests_enabled(&self) -> bool {
        self.game_tests_enabled
    }
}
