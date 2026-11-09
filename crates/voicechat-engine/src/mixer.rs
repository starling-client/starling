//! Incoming voices: one decoder and jitter buffer per channel, mixed to
//! 48 kHz stereo with distance falloff and left/right panning around the
//! listener.
use crate::opus::{Decoder, FRAME};
use crate::proto::Voice;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Samples buffered before a voice starts playing (60 ms), so a late
/// packet doesn't cut it up.
const START_AFTER: usize = FRAME * 3;
/// At most this much is kept per voice (500 ms); older audio is dropped.
const MAX_BUFFER: usize = 48_000 / 2;
/// Gaps up to this many frames are concealed; longer ones start over.
const MAX_CONCEAL: i64 = 4;
/// A channel with no packet for this long is forgotten.
const IDLE: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// At an entity (the channel's UUID, usually the speaking player).
    Entity(u128),
    /// At a fixed position.
    At([f64; 3]),
    /// Everywhere: a group member.
    Everywhere,
}

struct Source {
    decoder: Decoder,
    buffer: VecDeque<f32>,
    last_sequence: Option<i64>,
    playing: bool,
    last_packet: Instant,
    sender: u128,
    place: Place,
    distance: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Listener {
    pub position: [f64; 3],
    /// Minecraft yaw in degrees: 0 faces +Z (south), 90 faces -X (west).
    pub yaw: f32,
}

pub struct Mixer {
    sources: HashMap<u128, Source>,
    pub listener: Listener,
    /// Entity positions by UUID, refreshed each game tick.
    pub positions: HashMap<u128, [f64; 3]>,
    /// 0..=2, 1 = as received.
    pub volume: f32,
    /// Voice chat turned off: everything incoming is dropped.
    pub deafened: bool,
    /// Per-player volume, 0..=2 (missing = 1).
    pub player_volume: HashMap<u128, f32>,
    /// The microphone test: your own voice, straight back.
    local: VecDeque<f32>,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            sources: HashMap::new(),
            listener: Listener::default(),
            positions: HashMap::new(),
            volume: 1.0,
            deafened: false,
            player_volume: HashMap::new(),
            local: VecDeque::new(),
        }
    }
}

impl Mixer {
    /// A sound packet from the server.
    pub fn push(&mut self, packet: Voice) {
        if self.deafened {
            return;
        }
        let (channel, sender, data, sequence, place, distance) = match packet {
            Voice::PlayerSound {
                channel,
                sender,
                data,
                sequence,
                distance,
                ..
            } => (
                channel,
                sender,
                data,
                sequence,
                Place::Entity(channel),
                distance,
            ),
            Voice::GroupSound {
                channel,
                sender,
                data,
                sequence,
                ..
            } => (channel, sender, data, sequence, Place::Everywhere, 0.0),
            Voice::LocationSound {
                channel,
                sender,
                location,
                data,
                sequence,
                distance,
                ..
            } => (
                channel,
                sender,
                data,
                sequence,
                Place::At(location),
                distance,
            ),
            _ => return,
        };
        let now = Instant::now();
        self.sources
            .retain(|_, s| now.duration_since(s.last_packet) < IDLE);
        let source = match self.sources.entry(channel) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let Some(decoder) = Decoder::new() else {
                    return;
                };
                e.insert(Source {
                    decoder,
                    buffer: VecDeque::new(),
                    last_sequence: None,
                    playing: false,
                    last_packet: now,
                    sender,
                    place,
                    distance,
                })
            }
        };
        source.last_packet = now;
        source.sender = sender;
        source.place = place;
        source.distance = distance;
        if let Some(last) = source.last_sequence {
            if sequence <= last && last - sequence < 100 {
                return; // late or repeated
            }
            let gap = sequence - last - 1;
            if (1..=MAX_CONCEAL).contains(&gap) && !data.is_empty() {
                for _ in 0..gap {
                    let pcm = source.decoder.decode(None);
                    source
                        .buffer
                        .extend(pcm.iter().map(|&s| s as f32 / 32768.0));
                }
            } else if gap > MAX_CONCEAL {
                source.decoder.reset();
            }
        }
        source.last_sequence = Some(sequence);
        if data.is_empty() {
            // The speaker stopped: play out what's left, then start fresh.
            source.decoder.reset();
            source.last_sequence = None;
            source.playing = !source.buffer.is_empty();
            return;
        }
        let pcm = source.decoder.decode(Some(&data));
        source
            .buffer
            .extend(pcm.iter().map(|&s| s as f32 / 32768.0));
        while source.buffer.len() > MAX_BUFFER {
            source.buffer.pop_front();
        }
    }

    /// Who is audible right now (senders), for the HUD.
    pub fn talking(&self) -> Vec<u128> {
        let mut v: Vec<u128> = self
            .sources
            .values()
            .filter(|s| s.playing)
            .map(|s| s.sender)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Entities whose positions the next frames need.
    pub fn wanted_positions(&self) -> Vec<u128> {
        self.sources
            .values()
            .filter_map(|s| match s.place {
                Place::Entity(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    pub fn clear(&mut self) {
        self.sources.clear();
        self.local.clear();
    }

    /// 48 kHz mono samples of the microphone test, played in the middle.
    pub fn push_local(&mut self, samples: &[f32]) {
        self.local.extend(samples);
        // Keep the delay short: at most 100 ms queued.
        while self.local.len() > FRAME * 5 {
            self.local.pop_front();
        }
    }

    /// `frames` stereo frames at 48 kHz into `out` (interleaved L R, added).
    pub fn render(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        for source in self.sources.values_mut() {
            if !source.playing {
                if source.buffer.len() >= START_AFTER {
                    source.playing = true;
                } else {
                    continue;
                }
            }
            let personal = self
                .player_volume
                .get(&source.sender)
                .copied()
                .unwrap_or(1.0);
            let (l, r) = gains(
                source.place,
                source.distance,
                &self.listener,
                &self.positions,
            );
            let (l, r) = (l * self.volume * personal, r * self.volume * personal);
            for i in 0..frames {
                let Some(s) = source.buffer.pop_front() else {
                    source.playing = false;
                    break;
                };
                out[2 * i] += s * l;
                out[2 * i + 1] += s * r;
            }
        }
        crate::mic_filter::push_speaker_reference(out);
        let local = self.volume * 0.9;
        for i in 0..frames {
            let Some(s) = self.local.pop_front() else {
                break;
            };
            out[2 * i] += s * local;
            out[2 * i + 1] += s * local;
        }
        for s in out.iter_mut() {
            // Soft limit so several loud voices don't clip harshly.
            *s = s.tanh();
        }
    }
}

/// Left and right gain for a voice at `place`.
pub fn gains(
    place: Place,
    max_distance: f32,
    listener: &Listener,
    positions: &HashMap<u128, [f64; 3]>,
) -> (f32, f32) {
    let at = match place {
        Place::Everywhere => return (0.75, 0.75),
        Place::At(p) => p,
        Place::Entity(e) => match positions.get(&e) {
            Some(p) => *p,
            // Not loaded here, though the server says it's in range.
            None => return (0.5, 0.5),
        },
    };
    let d = [
        at[0] - listener.position[0],
        at[1] - listener.position[1],
        at[2] - listener.position[2],
    ];
    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32;
    let max = if max_distance > 0.0 {
        max_distance
    } else {
        48.0
    };
    let volume = (1.0 - dist / max).clamp(0.0, 1.0);
    if volume == 0.0 {
        return (0.0, 0.0);
    }
    // The listener's right in world space: yaw 0 faces +Z, so right is -X.
    let yaw = listener.yaw.to_radians() as f64;
    let right = [-yaw.cos(), -yaw.sin()];
    let horizontal = (d[0] * d[0] + d[2] * d[2]).sqrt();
    let pan = if horizontal < 1e-3 {
        0.0
    } else {
        ((d[0] * right[0] + d[2] * right[1]) / horizontal) as f32
    };
    // Constant power, never fully one-sided.
    let pan = pan * 0.8;
    let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    let norm = std::f32::consts::SQRT_2;
    (volume * angle.cos() * norm, volume * angle.sin() * norm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opus::Encoder;

    fn tone(frame: usize) -> Vec<i16> {
        (0..FRAME)
            .map(|i| (((frame * FRAME + i) as f32 * 0.06).sin() * 9000.0) as i16)
            .collect()
    }

    #[test]
    fn panning_follows_the_listener() {
        let listener = Listener {
            position: [0.0; 3],
            yaw: 0.0,
        };
        let positions = HashMap::new();
        // Facing south (+Z): west (-X) is on the right.
        let (l, r) = gains(Place::At([-5.0, 0.0, 0.0]), 48.0, &listener, &positions);
        assert!(r > l * 2.0, "{l} {r}");
        let (l, r) = gains(Place::At([5.0, 0.0, 0.0]), 48.0, &listener, &positions);
        assert!(l > r * 2.0);
        let (l, r) = gains(Place::At([0.0, 0.0, 5.0]), 48.0, &listener, &positions);
        assert!((l - r).abs() < 1e-4);
        // Turned to face west (yaw 90): west is ahead.
        let west = Listener {
            position: [0.0; 3],
            yaw: 90.0,
        };
        let (l, r) = gains(Place::At([-5.0, 0.0, 0.0]), 48.0, &west, &positions);
        assert!((l - r).abs() < 1e-3);
        assert_eq!(
            gains(Place::At([100.0, 0.0, 0.0]), 48.0, &listener, &positions),
            (0.0, 0.0)
        );
        let near = gains(Place::At([0.0, 0.0, 2.0]), 48.0, &listener, &positions).0;
        let far = gains(Place::At([0.0, 0.0, 30.0]), 48.0, &listener, &positions).0;
        assert!(near > far);
    }

    #[test]
    fn a_voice_plays_after_the_jitter_buffer_and_stops_cleanly() {
        let mut enc = Encoder::new(2048).unwrap();
        let mut mixer = Mixer::default();
        mixer.positions.insert(7, [0.0, 0.0, 3.0]);
        let packet = |data: Vec<u8>, sequence| Voice::PlayerSound {
            channel: 7,
            sender: 7,
            data,
            sequence,
            distance: 48.0,
            whispering: false,
            category: None,
        };
        mixer.push(packet(enc.encode(&tone(0)).unwrap(), 0));
        mixer.push(packet(enc.encode(&tone(1)).unwrap(), 1));
        let mut out = vec![0.0; FRAME * 2];
        mixer.render(&mut out);
        assert!(out.iter().all(|&s| s == 0.0), "still buffering");
        // Sequence 3: frame 2 was lost and is concealed.
        mixer.push(packet(enc.encode(&tone(3)).unwrap(), 3));
        assert_eq!(mixer.sources[&7].buffer.len(), FRAME * 4);
        mixer.render(&mut out);
        assert!(out.iter().any(|&s| s.abs() > 0.01), "audible");
        assert_eq!(mixer.talking(), vec![7]);
        // A late duplicate is ignored; the stop packet lets it drain.
        mixer.push(packet(enc.encode(&tone(1)).unwrap(), 1));
        mixer.push(packet(Vec::new(), 4));
        for _ in 0..4 {
            let mut out = vec![0.0; FRAME * 2];
            mixer.render(&mut out);
        }
        assert!(mixer.talking().is_empty());
    }

    #[test]
    fn the_microphone_test_plays_in_the_middle() {
        let mut mixer = Mixer::default();
        mixer.push_local(&vec![0.5; FRAME]);
        let mut out = vec![0.0; FRAME * 2];
        mixer.render(&mut out);
        assert!(out[0] > 0.3 && (out[0] - out[1]).abs() < 1e-6);
        mixer.push_local(&vec![0.5; FRAME * 20]);
        assert_eq!(mixer.local.len(), FRAME * 5, "capped");
    }

    #[test]
    fn deafened_drops_everything() {
        let mut enc = Encoder::new(2048).unwrap();
        let mut mixer = Mixer {
            deafened: true,
            ..Default::default()
        };
        mixer.push(Voice::GroupSound {
            channel: 1,
            sender: 1,
            data: enc.encode(&tone(0)).unwrap(),
            sequence: 0,
            category: None,
        });
        assert!(mixer.sources.is_empty());
    }
}
