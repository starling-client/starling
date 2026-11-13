//! Capability `lyrics`'s Android side (docs/mods.md): the media session as
//! the `HorizonNowPlayingService` notification listener publishes it, and the
//! lrclib.net lookup of one track's lyrics. The lookup sends only the
//! title, artist, album and length, and only when a new track starts.
use horizon_mods::{LyricLine, LyricsFetch, LyricsPlatform, NowPlaying};
use std::sync::{Arc, Mutex};

/// The platform for Android builds: the notification listener for what
/// plays, a background thread for the fetch.
pub(super) fn platform() -> Arc<dyn LyricsPlatform> {
    Arc::new(AndroidLyrics::default())
}

#[derive(Default)]
struct AndroidLyrics {
    state: Arc<Mutex<Shared>>,
}

#[derive(Default)]
struct Shared {
    /// Bumped on every `start_fetch`; a fetch lands only if it is the newest.
    generation: u64,
    result: Option<Result<LyricsFetch, String>>,
    /// Cached media-session reads (the mod asks every frame).
    cached: Option<(std::time::Instant, Option<NowPlaying>)>,
    cached_access: Option<(std::time::Instant, bool)>,
}

const CACHE: std::time::Duration = std::time::Duration::from_millis(250);

impl LyricsPlatform for AndroidLyrics {
    fn access(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, granted)) = &state.cached_access
            && at.elapsed() < CACHE
        {
            return *granted;
        }
        let granted = crate::platform::notification_access();
        state.cached_access = Some((std::time::Instant::now(), granted));
        granted
    }

    fn request(&self) {
        crate::platform::request_notification_listener();
    }

    fn now_playing(&self) -> Option<NowPlaying> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, cached)) = &state.cached
            && at.elapsed() < CACHE
        {
            return cached.clone();
        }
        let fresh = crate::platform::now_playing_json().and_then(|json| parse_now_playing(&json));
        state.cached = Some((std::time::Instant::now(), fresh.clone()));
        fresh
    }

    fn start_fetch(&self, title: &str, artist: &str, album: &str, duration_seconds: f64) {
        let generation = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.generation += 1;
            // A newer fetch replaces a running one: the older result is
            // dropped when it lands (its generation won't match).
            state.result = None;
            state.generation
        };
        let shared = Arc::clone(&self.state);
        let title = title.to_owned();
        let artist = artist.to_owned();
        let album = album.to_owned();
        // One HTTPS GET, on its own thread so the frame it was asked on is
        // not stalled; the mod reads the result with `poll-fetch`.
        std::thread::spawn(move || {
            let result =
                horizon_client::lyrics::fetch_lrclib(&title, &artist, &album, duration_seconds)
                    .map(|found| LyricsFetch {
                        source: "lrclib".to_owned(),
                        synced: !found.instrumental,
                        instrumental: found.instrumental,
                        lines: found
                            .lines
                            .into_iter()
                            .map(|(start_ms, text)| LyricLine { start_ms, text })
                            .collect(),
                    });
            let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
            if state.generation == generation {
                state.result = Some(result);
            }
        });
    }

    fn poll_fetch(&self) -> Option<Result<LyricsFetch, String>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.result.take()
    }
}

/// `{"title":..,"artist":..,"album":..,"duration_ms":n,"position_ms":n,
/// "position_at_ms":n,"now_ms":n,"playing":bool,"art_color":n|null}` as
/// `HorizonNowPlayingService.currentSnapshotJson` writes it.
fn parse_now_playing(json: &str) -> Option<NowPlaying> {
    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(value) => value,
        Err(error) => {
            log::warn!("NowPlaying: the listener's JSON did not parse: {error}");
            return None;
        }
    };
    let text = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned()
    };
    let number = |key: &str| value.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
    Some(NowPlaying {
        title: text("title"),
        artist: text("artist"),
        album: text("album"),
        duration_ms: number("duration_ms"),
        position_ms: number("position_ms"),
        position_at_ms: number("position_at_ms"),
        now_ms: number("now_ms"),
        playing: value
            .get("playing")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        app: text("app"),
        art_color: value
            .get("art_color")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32),
    })
}
