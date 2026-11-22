use super::*;
use crate::{__private::take_hook_panics, Hooks, ModEntry, PartPose, testing::with_dispatch};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

static PANIC_NOTICE_LOCK: Mutex<()> = Mutex::new(());

struct QuietMod;
#[mod_api::__subscriptions]
impl Hooks for QuietMod {}

fn key(time: f32, value: [f32; 3], interpolation: Interpolation) -> Keyframe {
    Keyframe {
        time,
        pre: value,
        post: value,
        interpolation,
    }
}
fn channel(target: AnimationTarget, keyframes: Vec<Keyframe>) -> AnimationChannel {
    AnimationChannel { target, keyframes }
}
fn animation_id() -> Identifier {
    Identifier::parse("test:animation").unwrap()
}
fn part(children: Vec<(&str, ModelPart)>) -> ModelPart {
    ModelPart::new(
        Vec::new(),
        children
            .into_iter()
            .map(|(name, child)| (name.to_owned(), child))
            .collect(),
        PartPose::ZERO,
    )
}

#[test]
fn linear_keyframes_interpolate_post_to_pre() {
    let c = channel(
        AnimationTarget::Position,
        vec![
            key(0.0, [0.0; 3], Interpolation::Linear),
            key(1.0, [2.0, 4.0, 6.0], Interpolation::Linear),
        ],
    );
    assert_eq!(c.sample(0.5, 1.0), Some([1.0, 2.0, 3.0]));
    // Past the end it holds the last value, times the scale.
    assert_eq!(c.sample(3.0, 0.5), Some([1.0, 2.0, 3.0]));
    // Before the first, the first.
    assert_eq!(c.sample(-1.0, 1.0), Some([0.0; 3]));
    // A linear segment ends at the next keyframe's arriving value, not its leaving one.
    let jump = channel(
        AnimationTarget::Position,
        vec![
            key(0.0, [0.0; 3], Interpolation::Linear),
            Keyframe {
                time: 1.0,
                pre: [2.0; 3],
                post: [10.0; 3],
                interpolation: Interpolation::Linear,
            },
        ],
    );
    assert_eq!(jump.sample(0.5, 1.0), Some([1.0; 3]));
}

#[test]
fn catmullrom_passes_through_keyframes() {
    let c = channel(
        AnimationTarget::Rotation,
        vec![
            key(0.0, [0.0; 3], Interpolation::CatmullRom),
            key(1.0, [1.0; 3], Interpolation::CatmullRom),
            key(2.0, [0.0; 3], Interpolation::CatmullRom),
        ],
    );
    assert!((c.sample(1.0, 1.0).unwrap()[0] - 1.0).abs() < 1e-6);
    // Mth.catmullrom(0.5, 0, 0, 1, 0) = 0.5 * (1 + (0 + 4 - 0) * 0.25 + (0 - 0 - 3 + 0) * 0.125)
    assert_eq!(c.sample(0.5, 1.0).unwrap()[0], 0.5625);
}

/// A bone resolves to the first part of its name, each part's children before its
/// grandchildren; a bone the model lacks is left out, and the rest still plays.
#[test]
fn bake_finds_the_first_part_of_a_name_and_drops_a_missing_bone() {
    let root = part(vec![
        ("body", part(vec![("arm", part(vec![]))])),
        ("arm", part(vec![])),
    ]);
    let holder = AnimationHolder {
        id: animation_id(),
        length: 1.0,
        looping: false,
        bones: vec![
            (
                "arm".into(),
                vec![channel(
                    AnimationTarget::Position,
                    vec![key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear)],
                )],
            ),
            (
                "tail".into(),
                vec![channel(
                    AnimationTarget::Position,
                    vec![key(0.0, [9.0; 3], Interpolation::Linear)],
                )],
            ),
        ],
    };
    let baked = holder.bake(&root);
    assert_eq!(baked.entries.len(), 1, "the missing tail is dropped");
    let mut posed = root.clone();
    baked.apply_static(&mut posed);
    // The root's own `arm` (index 1) is found before body's.
    let arm = posed.get_child("arm").unwrap();
    assert_eq!((arm.x, arm.y, arm.z), (1.0, 2.0, 3.0));
    let inner = posed.get_child("body").unwrap().get_child("arm").unwrap();
    assert_eq!((inner.x, inner.y, inner.z), (0.0, 0.0, 0.0));
}

/// `apply` only plays a started state, at `(age - start) * 50` ms; a looping animation wraps
/// by its length; `apply_walk` times by `pos * 50 * speed_factor` and scales by
/// `min(speed * scale_factor, 1)`; every apply adds to the pose.
#[test]
fn apply_times_by_the_state_and_wraps_a_loop() {
    let root = part(vec![("head", part(vec![]))]);
    let holder = AnimationHolder {
        id: animation_id(),
        length: 2.0,
        looping: true,
        bones: vec![(
            "head".into(),
            vec![channel(
                AnimationTarget::Rotation,
                vec![
                    key(0.0, [0.0; 3], Interpolation::Linear),
                    key(2.0, [4.0, 0.0, 0.0], Interpolation::Linear),
                ],
            )],
        )],
    };
    let baked = holder.bake(&root);
    let head_x = |root: &ModelPart| root.get_child("head").unwrap().x_rot;

    // Not looping, any time past the start would hold the last value, 4.0.
    let once = AnimationHolder {
        id: animation_id(),
        looping: false,
        ..holder.clone()
    }
    .bake(&root);
    let mut stopped = root.clone();
    once.apply(&mut stopped, &AnimationState::new(), 30.0);
    assert_eq!(head_x(&stopped), 0.0, "a stopped state plays nothing");

    let mut state = AnimationState::new();
    state.start(10);
    // 10 ticks after the start: 500 ms, a quarter of the way, 1.0.
    let mut posed = root.clone();
    baked.apply(&mut posed, &state, 20.0);
    assert_eq!(head_x(&posed), 1.0);
    // 50 ticks after: 2.5 s, wrapped to 0.5 s.
    let mut wrapped = root.clone();
    baked.apply(&mut wrapped, &state, 60.0);
    assert_eq!(head_x(&wrapped), 1.0);
    // Twice as fast: 1 s, halfway.
    let mut fast = root.clone();
    baked.apply_scaled(&mut fast, &state, 20.0, 2.0);
    assert_eq!(head_x(&fast), 2.0);

    // Walk position 20 at speed factor 1: 1000 ms, halfway; strength min(0.25 * 2, 1).
    let mut walk = root.clone();
    baked.apply_walk(&mut walk, 20.0, 0.25, 1.0, 2.0);
    assert_eq!(head_x(&walk), 1.0);
    // Applying again adds on top, as offsetRotation does.
    baked.apply_walk(&mut walk, 20.0, 0.25, 1.0, 2.0);
    assert_eq!(head_x(&walk), 2.0);
    // A strength over 1 is held at 1: min(1.5 * 2, 1) at halfway is 2.0.
    let mut strong = root.clone();
    baked.apply_walk(&mut strong, 20.0, 1.5, 1.0, 2.0);
    assert_eq!(head_x(&strong), 2.0);
}

/// Vanilla's transitions: `STOPPED` is `Integer.MIN_VALUE`, `animateWhen` starts once and stops,
/// `fastForward` moves the start back by `(int)(ticks * speed)` and wraps as Java's int does.
#[test]
fn animation_state_follows_vanillas_transitions() {
    let mut state = AnimationState::new();
    assert!(!state.is_started());
    state.animate_when(true, 5);
    state.animate_when(true, 9);
    assert_eq!(state.time_in_millis(7.0), 100, "started once, at 5");
    state.fast_forward(4, 0.5);
    assert_eq!(state.time_in_millis(7.0), 200, "now as if started at 3");
    state.animate_when(false, 9);
    assert!(!state.is_started());
    state.fast_forward(100, 1.0);
    assert!(!state.is_started(), "a stopped state doesn't move");
    state.start(i32::MIN + 1);
    state.fast_forward(2, 1.0);
    assert_eq!(
        state,
        AnimationState {
            start_tick: i32::MAX
        },
        "wraps, never panics"
    );
}

#[test]
fn oracle_root_bone_targets_actual_root_without_children() {
    let mut root = part(vec![]);
    let holder = AnimationHolder {
        id: animation_id(),
        length: 1.0,
        looping: false,
        bones: vec![(
            "root".into(),
            vec![channel(
                AnimationTarget::Position,
                vec![key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear)],
            )],
        )],
    };
    holder.bake(&root).apply_static(&mut root);
    assert_eq!((root.x, root.y, root.z), (1.0, 2.0, 3.0));
}

#[test]
fn oracle_root_bone_takes_precedence_over_child_named_root() {
    let mut root = part(vec![("root", part(vec![]))]);
    let holder = AnimationHolder {
        id: animation_id(),
        length: 1.0,
        looping: false,
        bones: vec![(
            "root".into(),
            vec![channel(
                AnimationTarget::Position,
                vec![key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear)],
            )],
        )],
    };
    holder.bake(&root).apply_static(&mut root);
    let child = root.get_child("root").unwrap();
    assert_eq!((child.x, child.y, child.z), (0.0, 0.0, 0.0));
    assert_eq!((root.x, root.y, root.z), (1.0, 2.0, 3.0));
}

#[test]
fn oracle_walk_nan_strength_propagates_like_java_math_min() {
    let mut root = part(vec![("head", part(vec![]))]);
    let holder = AnimationHolder {
        id: animation_id(),
        length: 1.0,
        looping: false,
        bones: vec![(
            "head".into(),
            vec![channel(
                AnimationTarget::Position,
                vec![key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear)],
            )],
        )],
    };
    holder
        .bake(&root)
        .apply_walk(&mut root, 0.0, f32::NAN, 1.0, 1.0);
    let head = root.get_child("head").unwrap();
    assert!(
        head.x.is_nan() && head.y.is_nan() && head.z.is_nan(),
        "Java Math.min(NaN, 1) propagates NaN, got {:?}",
        (head.x, head.y, head.z)
    );
}

/// Bound custom targets and interpolations play directly through `AnimationHolder::bake(&root)`
/// and `AnimationChannel::sample`, with no second bake API or lookup per sample.
#[test]
fn bound_custom_target_and_interpolation_play_directly_via_bake() {
    let calls: Arc<Mutex<Vec<(f32, usize, usize, f32)>>> = Arc::new(Mutex::new(Vec::new()));
    let calls_ref = Arc::clone(&calls);

    with_dispatch(
        vec![ModEntry::new("glow_mod", Box::new(QuietMod))].into(),
        || {
            let target = bind_animation_target(
                Identifier::from_parts("glow", "pulse").unwrap(),
                CallbackOwner::new("glow_mod").unwrap(),
                Arc::new(|part: &mut ModelPart, value: [f32; 3]| part.offset_pos(value)),
                Arc::new(|raw: [f32; 3]| raw.map(|v| v * 3.0)),
            );
            let interp = bind_animation_interpolation(
                Identifier::from_parts("glow", "step").unwrap(),
                CallbackOwner::new("glow_mod").unwrap(),
                Arc::new(move |keyframes, alpha, prev, next, scale| {
                    calls_ref.lock().unwrap().push((alpha, prev, next, scale));
                    keyframes[next].pre.map(|v| v * alpha * scale)
                }),
            );
            assert_eq!(target.id().to_string(), "glow:pulse");
            assert_eq!(target.owner(), "glow_mod");
            assert_eq!(interp.id().to_string(), "glow:step");
            assert_eq!(interp.owner(), "glow_mod");
            assert_eq!(target, target.clone());
            assert_eq!(interp, interp.clone());

            let v0 = convert_animation_target(&target, [1.0, 2.0, 3.0]).unwrap();
            let v1 = convert_animation_target(&target, [3.0, 4.0, 5.0]).unwrap();
            assert_eq!(v0, [3.0, 6.0, 9.0]);
            assert_eq!(v1, [9.0, 12.0, 15.0]);

            let holder = AnimationHolder {
                id: animation_id(),
                length: 1.0,
                looping: false,
                bones: vec![(
                    "head".into(),
                    vec![
                        channel(
                            AnimationTarget::Custom(target),
                            vec![
                                key(0.0, v0, Interpolation::Linear),
                                key(1.0, v1, Interpolation::Custom(interp)),
                            ],
                        ),
                        channel(
                            AnimationTarget::Rotation,
                            vec![
                                key(0.0, [0.0; 3], Interpolation::Linear),
                                key(1.0, [2.0, 0.0, 0.0], Interpolation::Linear),
                            ],
                        ),
                    ],
                )],
            };

            // Direct channel sampling invokes the bound interpolation.
            assert_eq!(holder.bones[0].1[0].sample(0.5, 1.0), Some([4.5, 6.0, 7.5]));

            let root = part(vec![("head", part(vec![]))]);
            let baked = holder.bake(&root);
            let mut state = AnimationState::new();
            state.start(0);
            let mut posed = root.clone();
            baked.apply(&mut posed, &state, 10.0);
            let head = posed.get_child("head").unwrap();
            assert_eq!((head.x, head.y, head.z), (4.5, 6.0, 7.5));
            assert_eq!(head.x_rot, 1.0);
            assert_eq!(*calls.lock().unwrap(), [(0.5, 0, 1, 1.0), (0.5, 0, 1, 1.0)]);
        },
    );
}

/// Panicking callbacks disable themselves for the session and queue a panic notice; non-finite
/// values from an interpolation or target converter are rejected without latching.
#[test]
fn custom_callback_panic_disables_it_and_non_finite_does_not_latch() {
    let _notice_lock = PANIC_NOTICE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let interp_panic_calls = Arc::new(AtomicUsize::new(0));
    let target_panic_calls = Arc::new(AtomicUsize::new(0));
    let flaky_interp_calls = Arc::new(AtomicUsize::new(0));
    let flaky_convert_calls = Arc::new(AtomicUsize::new(0));

    with_dispatch(
        vec![ModEntry::new("faulty", Box::new(QuietMod))].into(),
        || {
            let _ = take_hook_panics();

            let ip = Arc::clone(&interp_panic_calls);
            let bad_interp = bind_animation_interpolation(
                Identifier::from_parts("faulty", "panic_interp").unwrap(),
                CallbackOwner::new("faulty").unwrap(),
                Arc::new(move |_, _, _, _, _| {
                    ip.fetch_add(1, Ordering::Relaxed);
                    panic!("interp boom");
                }),
            );
            let ch = channel(
                AnimationTarget::Position,
                vec![
                    key(0.0, [0.0; 3], Interpolation::Linear),
                    key(1.0, [1.0; 3], Interpolation::Custom(bad_interp)),
                ],
            );
            assert_eq!(ch.sample(0.5, 1.0), None);
            assert_eq!(interp_panic_calls.load(Ordering::Relaxed), 1);
            let notices = take_hook_panics();
            assert!(
                notices
                    .iter()
                    .any(|notice| notice.mod_id == "faulty" && notice.hook == "interpolate"),
                "expected interpolation panic notice, got {notices:?}"
            );
            // Disabled for the session: second sample does not re-enter the panicked callback.
            assert_eq!(ch.sample(0.5, 1.0), None);
            assert_eq!(interp_panic_calls.load(Ordering::Relaxed), 1);

            let tp = Arc::clone(&target_panic_calls);
            let bad_target = bind_animation_target(
                Identifier::from_parts("faulty", "panic_target").unwrap(),
                CallbackOwner::new("faulty").unwrap(),
                Arc::new(move |_, _| {
                    tp.fetch_add(1, Ordering::Relaxed);
                    panic!("target boom");
                }),
                Arc::new(|v| v),
            );
            let mut root = part(vec![]);
            let target_enum = AnimationTarget::Custom(bad_target.clone());
            target_enum.apply(&mut root, [1.0, 2.0, 3.0]);
            assert_eq!(target_panic_calls.load(Ordering::Relaxed), 1);
            let notices = take_hook_panics();
            assert!(
                notices
                    .iter()
                    .any(|notice| notice.mod_id == "faulty" && notice.hook == "apply"),
                "expected target panic notice, got {notices:?}"
            );
            // Clones share the same CallbackOwner poison flag.
            AnimationTarget::Custom(bad_target).apply(&mut root, [4.0, 5.0, 6.0]);
            assert_eq!(target_panic_calls.load(Ordering::Relaxed), 1);

            // Non-finite interpolation output returns None without latching.
            let fi = Arc::clone(&flaky_interp_calls);
            let flaky_interp = bind_animation_interpolation(
                Identifier::from_parts("faulty", "flaky_interp").unwrap(),
                CallbackOwner::new("faulty").unwrap(),
                Arc::new(move |_, _, _, _, _| {
                    if fi.fetch_add(1, Ordering::Relaxed) == 0 {
                        [f32::NAN, 0.0, 0.0]
                    } else {
                        [1.0, 2.0, 3.0]
                    }
                }),
            );
            let flaky_ch = channel(
                AnimationTarget::Position,
                vec![
                    key(0.0, [0.0; 3], Interpolation::Linear),
                    key(1.0, [1.0; 3], Interpolation::Custom(flaky_interp)),
                ],
            );
            assert_eq!(flaky_ch.sample(0.5, 1.0), None);
            assert!(take_hook_panics().is_empty());
            assert_eq!(flaky_ch.sample(0.5, 1.0), Some([1.0, 2.0, 3.0]));
            assert_eq!(flaky_interp_calls.load(Ordering::Relaxed), 2);

            // Non-finite converter output returns Err without latching.
            let fc = Arc::clone(&flaky_convert_calls);
            let flaky_target = bind_animation_target(
                Identifier::from_parts("faulty", "flaky_convert").unwrap(),
                CallbackOwner::new("faulty").unwrap(),
                Arc::new(|_, _| {}),
                Arc::new(move |v| {
                    if fc.fetch_add(1, Ordering::Relaxed) == 0 {
                        [f32::INFINITY, 0.0, 0.0]
                    } else {
                        v
                    }
                }),
            );
            assert!(convert_animation_target(&flaky_target, [1.0, 2.0, 3.0]).is_err());
            assert!(take_hook_panics().is_empty());
            assert_eq!(
                convert_animation_target(&flaky_target, [1.0, 2.0, 3.0]).unwrap(),
                [1.0, 2.0, 3.0]
            );
            assert_eq!(flaky_convert_calls.load(Ordering::Relaxed), 2);
        },
    );
}

/// A panicking, disabled, or non-finite custom interpolation returns `None` so `apply_at` skips
/// only that channel and never calls its target with a fabricated zero vector, while other
/// channels on the same bone still apply.
#[test]
fn failed_custom_interpolation_skips_only_that_channel_and_never_calls_target() {
    let _notice_lock = PANIC_NOTICE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let target_calls = Arc::new(AtomicUsize::new(0));

    with_dispatch(
        vec![ModEntry::new("faulty", Box::new(QuietMod))].into(),
        || {
            let _ = take_hook_panics();
            let calls_ref = Arc::clone(&target_calls);
            // Distinguishes an applied zero from no call: writes a non-zero marker even for [0; 3].
            let overwriting_target = bind_animation_target(
                Identifier::from_parts("faulty", "overwrite").unwrap(),
                CallbackOwner::new("faulty").unwrap(),
                Arc::new(move |part: &mut ModelPart, value: [f32; 3]| {
                    calls_ref.fetch_add(1, Ordering::Relaxed);
                    part.x = value[0] + 100.0;
                }),
                Arc::new(|v| v),
            );
            let panicking_interp = bind_animation_interpolation(
                Identifier::from_parts("faulty", "panic_interp").unwrap(),
                CallbackOwner::new("faulty").unwrap(),
                Arc::new(|_, _, _, _, _| panic!("boom")),
            );
            let nan_interp = bind_animation_interpolation(
                Identifier::from_parts("faulty", "nan_interp").unwrap(),
                CallbackOwner::new("faulty").unwrap(),
                Arc::new(|_, _, _, _, _| [f32::NAN, 0.0, 0.0]),
            );

            let holder = AnimationHolder {
                id: animation_id(),
                length: 1.0,
                looping: false,
                bones: vec![(
                    "head".into(),
                    vec![
                        channel(
                            AnimationTarget::Custom(overwriting_target.clone()),
                            vec![
                                key(0.0, [0.0; 3], Interpolation::Linear),
                                key(1.0, [1.0; 3], Interpolation::Custom(panicking_interp)),
                            ],
                        ),
                        channel(
                            AnimationTarget::Custom(overwriting_target),
                            vec![
                                key(0.0, [0.0; 3], Interpolation::Linear),
                                key(1.0, [1.0; 3], Interpolation::Custom(nan_interp)),
                            ],
                        ),
                        channel(
                            AnimationTarget::Rotation,
                            vec![
                                key(0.0, [0.0; 3], Interpolation::Linear),
                                key(1.0, [2.0, 0.0, 0.0], Interpolation::Linear),
                            ],
                        ),
                    ],
                )],
            };

            let root = part(vec![("head", part(vec![]))]);
            let baked = holder.bake(&root);
            let mut state = AnimationState::new();
            state.start(0);

            // First apply: panicking channel panics (disabled), NaN channel fails, rotation applies.
            let mut posed = root.clone();
            baked.apply(&mut posed, &state, 10.0);
            assert_eq!(target_calls.load(Ordering::Relaxed), 0);
            let head = posed.get_child("head").unwrap();
            assert_eq!(
                head.x, 0.0,
                "failed channels must not call overwriting target"
            );
            assert_eq!(head.x_rot, 1.0, "healthy channel still applies");
            assert!(
                take_hook_panics()
                    .iter()
                    .any(|notice| notice.mod_id == "faulty" && notice.hook == "interpolate")
            );

            // Second apply: disabled channel skips without calling target either.
            let mut posed2 = root.clone();
            baked.apply(&mut posed2, &state, 10.0);
            assert_eq!(target_calls.load(Ordering::Relaxed), 0);
            let head2 = posed2.get_child("head").unwrap();
            assert_eq!(head2.x, 0.0);
            assert_eq!(head2.x_rot, 1.0);
        },
    );
}
