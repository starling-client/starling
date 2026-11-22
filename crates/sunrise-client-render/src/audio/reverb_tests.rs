use super::*;

const RATE: u32 = 48_000;

/// The reverb's stereo response to a single click, `seconds` long.
fn impulse_response(room: Room, seconds: f32) -> Vec<f32> {
    let mut reverb = Reverb::new(room);
    let frames = (seconds * RATE as f32) as usize;
    let mut input = vec![0.0; frames];
    input[0] = 1.0;
    let mut out = vec![0.0; frames * 2];
    reverb.run(&input, true, &mut out, 2, RATE);
    out
}

/// Energy of `out` in 50 ms windows, in dB.
fn energy_db(out: &[f32]) -> Vec<f32> {
    out.chunks(2 * RATE as usize / 20)
        .map(|w| 10.0 * (w.iter().map(|s| s * s).sum::<f32>() + 1e-30).log10())
        .collect()
}

#[test]
fn hadamard_keeps_energy() {
    let mut v = [0.3, -1.0, 0.5, 2.0, 0.0, -0.7, 1.1, 0.25];
    let before: f32 = v.iter().map(|x| x * x).sum();
    hadamard(&mut v);
    let after: f32 = v.iter().map(|x| x * x).sum();
    assert!((before - after).abs() < 1e-5);
    // Its own inverse: twice is the identity.
    hadamard(&mut v);
    assert!((v[3] - 2.0).abs() < 1e-5);
}

#[test]
fn every_room_decays_at_its_rate_and_never_grows() {
    for room in ROOMS {
        let seconds = room.decay * 1.5;
        let db = energy_db(&impulse_response(room, seconds));
        let peak = db.iter().copied().fold(f32::MIN, f32::max);
        let at_decay = db[(room.decay * 20.0) as usize];
        // About 60 dB down at the decay time (the damping makes highs fall faster still).
        assert!(
            (-75.0..=-45.0).contains(&(at_decay - peak)),
            "{room:?}: {} dB",
            at_decay - peak
        );
        // From its loudest window on, the tail only falls, a quarter second to the next (the
        // modes of a feedback network beat, so 50 ms windows wobble a little on the way down).
        let loudest = db.iter().position(|d| *d == peak).unwrap();
        for (a, b) in db[loudest..].iter().zip(&db[loudest + 5..]) {
            assert!(b < a, "{room:?} grew: {a} then {b}");
        }
    }
}

#[test]
fn the_two_sides_hear_different_tails() {
    let out = impulse_response(ROOMS[1], 0.5);
    let (mut lr, mut ll, mut rr) = (0.0f64, 0.0f64, 0.0f64);
    for f in out.chunks(2) {
        let (l, r) = (f64::from(f[0]), f64::from(f[1]));
        lr += l * r;
        ll += l * l;
        rr += r * r;
    }
    let correlation = lr / (ll * rr).sqrt();
    assert!(correlation.abs() < 0.3, "{correlation}");
}

#[test]
fn a_silent_reverb_goes_idle_and_costs_nothing() {
    let mut reverb = Reverb::new(ROOMS[0]);
    let mut out = vec![0.0; 960];
    reverb.run(&[0.0; 480], false, &mut out, 2, RATE);
    assert!(reverb.idle());
    let mut input = [0.0; 480];
    input[0] = 1.0;
    reverb.run(&input, true, &mut out, 2, RATE);
    assert!(!reverb.idle());
    // The small room's tail is gone well within 3 seconds.
    for _ in 0..300 {
        out.fill(0.0);
        reverb.run(&[0.0; 480], false, &mut out, 2, RATE);
    }
    assert!(reverb.idle());
    out.fill(0.0);
    reverb.run(&[0.0; 480], false, &mut out, 2, RATE);
    assert!(out.iter().all(|s| *s == 0.0));
}

#[test]
fn the_lines_fit_the_memory_at_every_rate() {
    for room in ROOMS {
        let mut reverb = Reverb::new(room);
        for rate in [8_000, 22_050, 44_100, 48_000, 96_000, 192_000, 384_000] {
            reverb.set_rate(rate);
            let end = reverb.start[LINES - 1] + reverb.len[LINES - 1];
            assert!(end <= reverb.memory.len(), "{room:?} at {rate}");
        }
    }
}

#[test]
fn the_delay_lines_are_allocated_once_at_creation_for_192_khz() {
    // Each line holds ceil(ms * size * 192) + 64 samples; the eight lines of a bus are one block.
    let sizes: Vec<usize> = ROOMS
        .iter()
        .map(|&room| Reverb::new(room).memory.len())
        .collect();
    assert_eq!(sizes, [36_596, 78_690, 120_785]);
    let bytes: usize = sizes.iter().sum::<usize>() * std::mem::size_of::<f32>();
    assert_eq!(bytes, 944_284);
    // Every rate up to 192 kHz fits in that block, and setting one never reallocates.
    for room in ROOMS {
        let mut reverb = Reverb::new(room);
        let (pointer, capacity) = (reverb.memory.as_ptr(), reverb.memory.capacity());
        for rate in [8_000, 44_100, 48_000, 96_000, 192_000, 384_000] {
            reverb.set_rate(rate);
            assert!(reverb.start[LINES - 1] + reverb.len[LINES - 1] <= reverb.memory.len());
        }
        assert_eq!(
            (reverb.memory.as_ptr(), reverb.memory.capacity()),
            (pointer, capacity)
        );
    }
}

#[test]
fn a_send_of_silence_lets_the_tail_die_away() {
    // Small buffers leave several silent callbacks before the first reflection.
    let mut reverb = Reverb::new(ROOMS[0]);
    let mut input = [0.0; 128];
    input[0] = 1.0;
    let mut out = [0.0; 256];
    reverb.run(&input, true, &mut out, 2, RATE);
    assert!(out.iter().all(|s| *s == 0.0));
    input[0] = 0.0;
    let mut heard_tail = false;
    for _ in 0..(3 * RATE as usize / input.len()) {
        out.fill(0.0);
        reverb.run(&input, true, &mut out, 2, RATE);
        heard_tail |= out.iter().any(|s| *s != 0.0);
        if reverb.idle() {
            break;
        }
    }
    assert!(heard_tail);
    assert!(reverb.idle());
}

#[test]
fn the_loop_low_pass_rings_down_to_zero_not_to_subnormals() {
    let mut reverb = Reverb::new(ROOMS[2]);
    let mut out = [0.0; 8];
    reverb.run(&[0.0; 4], true, &mut out, 2, RATE);
    // Loop state ringing down far below hearing while a sound still sends, all but silent.
    reverb.lp = [1e-20; LINES];
    reverb.run(&[1e-30, 0.0, 0.0, 0.0], true, &mut out, 2, RATE);
    assert_eq!(reverb.lp, [0.0; LINES]);
}
