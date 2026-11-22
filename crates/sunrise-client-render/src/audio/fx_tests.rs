use super::*;

const RATE: u32 = 48_000;

/// `effects` settled on a voice, then `signal` (mono) through it: the direct output.
fn through(effects: SoundEffects, signal: &[f32]) -> Vec<f32> {
    let mut fx = VoiceFx {
        target: effects,
        ..VoiceFx::default()
    };
    let mut glide = fx.begin(1, RATE);
    fx.frame(&mut glide, &mut [0.0; 2], false);
    let mut glide = fx.begin(signal.len(), RATE);
    signal
        .iter()
        .map(|&s| {
            let mut x = [s, 0.0];
            fx.frame(&mut glide, &mut x, false);
            x[0]
        })
        .collect()
}

fn tone(hz: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin())
        .collect()
}

/// The gain of a filter at `hz`, in dB, from the steady part of a long tone.
fn gain_db(effects: SoundEffects, hz: f32) -> f32 {
    let out = through(effects, &tone(hz, RATE as usize / 2));
    let tail = &out[out.len() / 2..];
    let peak = tail.iter().fold(0.0f32, |p, s| p.max(s.abs()));
    20.0 * peak.log10()
}

#[test]
fn the_low_pass_is_a_two_pole_butterworth() {
    let mut e = SoundEffects::NONE;
    e.lowpass = 1000.0;
    assert!(gain_db(e, 100.0).abs() < 0.1);
    // -3 dB at the cutoff, 12 dB an octave above it.
    assert!(
        (gain_db(e, 1000.0) + 3.0).abs() < 0.3,
        "{}",
        gain_db(e, 1000.0)
    );
    assert!(gain_db(e, 8000.0) < -34.0, "{}", gain_db(e, 8000.0));
}

#[test]
fn the_high_pass_is_a_two_pole_butterworth() {
    let mut e = SoundEffects::NONE;
    e.highpass = 1000.0;
    assert!(gain_db(e, 10_000.0).abs() < 0.2);
    assert!(
        (gain_db(e, 1000.0) + 3.0).abs() < 0.3,
        "{}",
        gain_db(e, 1000.0)
    );
    assert!(gain_db(e, 125.0) < -34.0, "{}", gain_db(e, 125.0));
}

#[test]
fn no_effects_pass_the_sound_untouched() {
    let signal = tone(3000.0, 4800);
    assert_eq!(through(SoundEffects::NONE, &signal), signal);
    assert!(VoiceFx::default().bypassed());
}

#[test]
fn a_cutoff_swept_every_frame_stays_bounded() {
    // The state-variable filter is stable while its cutoff moves: sweep it between 20 Hz and
    // 20 kHz every few frames over loud noise and the output never runs away.
    let mut fx = VoiceFx::default();
    let mut seed = 1u32;
    let mut noise = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
    };
    let mut loudest = 0.0f32;
    for block in 0..2000 {
        let mut e = SoundEffects::NONE;
        e.lowpass = if block % 2 == 0 { 20.0 } else { 19_000.0 };
        e.highpass = if block % 3 == 0 { 15_000.0 } else { 0.0 };
        fx.target = e;
        let mut glide = fx.begin(16, RATE);
        for _ in 0..16 {
            let mut x = [noise(), noise()];
            fx.frame(&mut glide, &mut x, true);
            loudest = loudest.max(x[0].abs()).max(x[1].abs());
        }
    }
    assert!(loudest.is_finite() && loudest < 4.0, "{loudest}");
}

#[test]
fn a_glide_lands_on_its_target_and_then_bypasses() {
    // A voice already playing: its first buffer had no effects.
    let mut fx = VoiceFx::default();
    fx.begin(1, RATE);
    let mut e = SoundEffects::NONE;
    e.gain = 0.25;
    e.lowpass = 2000.0;
    e.room = 0.5;
    fx.target = e;
    let mut glide = fx.begin(100, RATE);
    let mut x = [1.0, 0.0];
    let first = fx.frame(&mut glide, &mut x, false);
    assert_eq!(first, [0.0; 3], "sends start from silence");
    for _ in 1..100 {
        fx.frame(&mut glide, &mut [1.0, 0.0], false);
    }
    assert!((glide.at.direct - 0.25).abs() < 1e-6);
    assert!((glide.at.send[0] - 0.125).abs() < 1e-6);
    fx.target = SoundEffects::NONE;
    let mut glide = fx.begin(100, RATE);
    assert!(
        glide.sends && glide.lowpass,
        "the filter and send fade out first"
    );
    for _ in 0..100 {
        fx.frame(&mut glide, &mut [1.0, 0.0], false);
    }
    assert!(fx.bypassed());
}

#[test]
fn a_voice_starts_on_its_effects_from_its_first_frame() {
    // Started at gain 0: silent from frame 0, sends included.
    let mut silent = SoundEffects::NONE;
    silent.gain = 0.0;
    silent.room = 1.0;
    let mut fx = VoiceFx {
        target: silent,
        ..VoiceFx::default()
    };
    let mut glide = fx.begin(480, RATE);
    for s in tone(1000.0, 480) {
        let mut x = [s, s];
        let send = fx.frame(&mut glide, &mut x, true);
        assert_eq!((x, send), ([0.0; 2], [0.0; 3]));
    }
    // Started behind a wall: filtered from frame 0, exactly as a voice that settled there.
    let mut muffled = SoundEffects::NONE;
    muffled.lowpass = 500.0;
    let signal = tone(6000.0, 480);
    let mut fx = VoiceFx {
        target: muffled,
        ..VoiceFx::default()
    };
    let mut glide = fx.begin(signal.len(), RATE);
    let first: Vec<f32> = signal
        .iter()
        .map(|&s| {
            let mut x = [s, 0.0];
            fx.frame(&mut glide, &mut x, false);
            x[0]
        })
        .collect();
    assert_eq!(first, through(muffled, &signal));
}

#[test]
fn a_send_reaches_only_the_buses_it_is_sent_to() {
    let mut e = SoundEffects::NONE;
    e.room = 0.5;
    let mut fx = VoiceFx {
        target: e,
        ..VoiceFx::default()
    };
    assert_eq!(fx.begin(480, RATE).sent, [true, false, false]);
    // Gliding from the room to the cavern, both are fed for that buffer.
    e.room = 0.0;
    e.cavern = 0.5;
    fx.target = e;
    assert_eq!(fx.begin(480, RATE).sent, [true, false, true]);
    assert_eq!(fx.begin(480, RATE).sent, [false, false, true]);
}

#[test]
fn filter_and_send_state_rings_down_to_zero_not_to_subnormals() {
    let mut e = SoundEffects::NONE;
    e.lowpass = 1000.0;
    e.highpass = 40.0;
    e.room = 1.0;
    e.reverb_lowpass = 200.0;
    let mut fx = VoiceFx {
        target: e,
        ..VoiceFx::default()
    };
    // A click, then silence, a 10 ms buffer at a time.
    let mut click = 1.0;
    for _ in 0..200 {
        let mut glide = fx.begin(480, RATE);
        let state = [
            fx.lowpass[0].ic1,
            fx.lowpass[0].ic2,
            fx.highpass[0].ic1,
            fx.highpass[0].ic2,
            fx.send_lp,
        ];
        assert!(
            state.iter().all(|s| *s == 0.0 || s.abs() >= TINY),
            "{state:?}"
        );
        for _ in 0..480 {
            fx.frame(&mut glide, &mut [click, click], true);
            click = 0.0;
        }
    }
    fx.begin(480, RATE);
    assert_eq!(
        (fx.lowpass[0], fx.highpass[0], fx.send_lp),
        (Svf::default(), Svf::default(), 0.0)
    );
}
