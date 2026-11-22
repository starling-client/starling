//! Opt-in crash points for the dedicated-server harnesses. Ordinary saves never wait here.
use std::sync::OnceLock;

use sunrise_core::position::ChunkPos;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CrashFixture {
    Panic(&'static str),
    SaveLevel,
    SavePlayer,
    SaveRegion(ChunkPos),
}

pub fn selected() -> Option<CrashFixture> {
    static FIXTURE: OnceLock<Option<CrashFixture>> = OnceLock::new();
    *FIXTURE.get_or_init(
        || match std::env::var("SUNRISE_CRASH_FIXTURE").ok()?.as_str() {
            "guest" => Some(CrashFixture::Panic("guest")),
            "hub" => Some(CrashFixture::Panic("hub")),
            "save-level" => Some(CrashFixture::SaveLevel),
            "save-player" => Some(CrashFixture::SavePlayer),
            point => {
                let (x, z) = point.strip_prefix("save-region:")?.split_once(',')?;
                Some(CrashFixture::SaveRegion(ChunkPos::new(
                    x.parse().ok()?,
                    z.parse().ok()?,
                )))
            }
        },
    )
}

/// The harness arms this point after startup, just before `save-all`.
pub fn await_save_kill(point: &str) {
    if std::path::Path::new(".sunrise-save-fault-armed").is_file() {
        eprintln!("save crash fixture reached {point}");
        loop {
            std::thread::park();
        }
    }
}
