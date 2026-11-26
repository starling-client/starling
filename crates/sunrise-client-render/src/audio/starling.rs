//! Starling's part of audio.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Audio {
    /// The app went to the background (Android): the output stream stops until
    /// `resume_output`, which plays on where it was.
    pub fn suspend_output(&mut self) {
        #[cfg(feature = "audio")]
        if let Some(stream) = &self._stream {
            use cpal::traits::StreamTrait;
            if let Err(e) = stream.pause() {
                log::warn!("Sunrise: pausing the sound output: {e}");
            }
        }
    }

    pub fn resume_output(&mut self) {
        #[cfg(feature = "audio")]
        if let Some(stream) = &self._stream {
            use cpal::traits::StreamTrait;
            if let Err(e) = stream.play() {
                log::warn!("Sunrise: resuming the sound output: {e}");
            }
        }
    }

    /// Sound the game made itself, mono `samples` at `rate` Hz played once at `gain`, at the
    /// listener: the setup's fishing, before the game's own sounds exist. Their channels go once
    /// they have played.
    pub fn play_samples(&mut self, samples: Arc<Vec<f32>>, rate: u32, gain: f32) {
        /// Made sounds' channels, far above the engine's own.
        const MADE: ChannelId = 1 << 48;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(MADE);
        if !self.backend.live {
            return;
        }
        let Ok(mut m) = self.backend.mixer.lock() else {
            return;
        };
        m.channels
            .retain(|&id, c| id < MADE || c.state != State::Stopped);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        m.channels.insert(
            id,
            Channel {
                fx: fx::VoiceFx::default(),
                samples: Some((samples, rate, 1)),
                position: 0.0,
                state: State::Playing,
                gain: gain * self.master,
                pitch: 1.0,
                linear: None,
                looping: false,
                at: [0.0; 3],
                relative: true,
                owner: 0,
                world: 0,
                started: None,
                unmuffled: false,
            },
        );
    }
}
