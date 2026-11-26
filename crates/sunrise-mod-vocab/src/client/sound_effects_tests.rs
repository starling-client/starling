use super::SoundEffects;

/// `NONE` with one field set by `set`.
fn with(set: impl FnOnce(&mut SoundEffects)) -> SoundEffects {
    let mut effects = SoundEffects::NONE;
    set(&mut effects);
    effects
}

/// Each field's setter, its lowest and highest valid value, by name.
fn fields() -> [(&'static str, fn(&mut SoundEffects, f32), f32, f32); 8] {
    [
        ("gain", |e, v| e.gain = v, 0.0, 1.0),
        ("direct", |e, v| e.direct = v, 0.0, 1.0),
        ("lowpass", |e, v| e.lowpass = v, 20.0, 20_000.0),
        ("highpass", |e, v| e.highpass = v, 0.0, 20_000.0),
        ("room", |e, v| e.room = v, 0.0, 1.0),
        ("hall", |e, v| e.hall = v, 0.0, 1.0),
        ("cavern", |e, v| e.cavern = v, 0.0, 1.0),
        (
            "reverb_lowpass",
            |e, v| e.reverb_lowpass = v,
            20.0,
            20_000.0,
        ),
    ]
}

#[test]
fn none_is_the_default_and_valid() {
    assert_eq!(SoundEffects::default(), SoundEffects::NONE);
    assert!(SoundEffects::NONE.valid());
}

#[test]
fn every_field_takes_both_ends_of_its_range() {
    for (name, set, lo, hi) in fields() {
        for v in [lo, hi, (lo + hi) * 0.5] {
            assert!(with(|e| set(e, v)).valid(), "{name} = {v}");
        }
    }
}

#[test]
fn every_field_refuses_values_past_its_range_nan_and_infinities() {
    for (name, set, lo, hi) in fields() {
        let below = if lo == 0.0 {
            -f32::MIN_POSITIVE
        } else {
            lo.next_down()
        };
        for v in [
            below,
            hi.next_up(),
            -1.0,
            hi * 2.0,
            f32::NAN,
            -f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ] {
            let effects = with(|e| set(e, v));
            assert!(!effects.valid(), "{name} = {v} must be refused");
            // Refused, not clamped: the value is left as it was given.
            assert!(
                [
                    effects.gain,
                    effects.direct,
                    effects.lowpass,
                    effects.highpass,
                    effects.room,
                    effects.hall,
                    effects.cavern,
                    effects.reverb_lowpass,
                ]
                .iter()
                .any(|f| f.to_bits() == v.to_bits()),
                "{name} = {v}"
            );
        }
    }
}

#[test]
fn the_cutoffs_ranges_are_in_hertz_whatever_the_output_rate() {
    // 20 kHz is above a 32 kHz output's Nyquist frequency and still valid: the mixer limits it
    // there itself, which is not a refusal.
    assert!(with(|e| e.lowpass = 20_000.0).valid());
    assert!(with(|e| e.highpass = 20_000.0).valid());
    assert!(!with(|e| e.lowpass = 19.999).valid());
    assert!(!with(|e| e.reverb_lowpass = 24_000.0).valid());
}
