//! Keyframe animations (vanilla's `AnimationDefinition`, `AnimationChannel`, `Keyframe`,
//! `KeyframeAnimation` and `AnimationState`) as plain data and pure functions. The game samples its
//! own mobs' animations through [`AnimationChannel::sample`], and a mod plays the files it ships
//! (`RenderAssets::animation`) on its own `ModelPart` with the same code.
use crate::{
    Identifier, ModelPart, RegisterJsonAnimationTypesEvent,
    callback_owner::{CallbackOwner, CallbackValue},
};
use std::sync::Arc;

#[cfg(test)]
#[path = "keyframe_animation_tests.rs"]
mod tests;

/// Callback that applies a sampled animation value to a model part (NeoForge's
/// `AnimationTarget.channelTarget` / vanilla's `AnimationChannel.Target.apply`).
// no wire: retained process-local animation target callback.
pub type AnimationTargetApply = dyn Fn(&mut ModelPart, [f32; 3]) + Send + Sync;

/// Callback that converts a raw JSON keyframe `[x, y, z]` vector into the target's internal units
/// during animation file parsing (NeoForge's `AnimationTarget.keyframeTarget`).
// no wire: retained process-local animation target vector converter.
pub type AnimationTargetConvert = dyn Fn([f32; 3]) -> [f32; 3] + Send + Sync;

/// Callback that interpolates a channel segment between `keyframes[current_idx]` and
/// `keyframes[next_idx]` at segment progress `alpha` (`0.0..=1.0`, or `0.0` when
/// `current_idx == next_idx`) scaled by `scale` (vanilla's `AnimationChannel.Interpolation.apply`).
// no wire: retained process-local animation interpolation callback.
pub type AnimationInterpolationFn =
    dyn Fn(&[Keyframe], f32, usize, usize, f32) -> [f32; 3] + Send + Sync;

struct BoundAnimationTargetInner {
    id: Identifier,
    apply: CallbackValue<Arc<AnimationTargetApply>>,
    convert: CallbackValue<Arc<AnimationTargetConvert>>,
}

/// A custom animation target bound to its owning mod's callbacks at registration
/// (`register_json_animation_types`).
#[derive(Clone)]
// exhaustive: engine-constructed bound target; private fields prevent downstream construction
// no wire: carries process-local animation target/interpolation callbacks
pub struct BoundAnimationTarget(Arc<BoundAnimationTargetInner>);

impl BoundAnimationTarget {
    /// The target's registered identifier (`mymod:glow`).
    pub fn id(&self) -> &Identifier {
        &self.0.id
    }

    /// The `[mod] id` of the mod that registered this target.
    pub fn owner(&self) -> &'static str {
        self.0.apply.owner().id()
    }
}

impl std::fmt::Debug for BoundAnimationTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundAnimationTarget")
            .field("id", &self.0.id)
            .field("owner", &self.owner())
            .finish()
    }
}

impl PartialEq for BoundAnimationTarget {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for BoundAnimationTarget {}

struct BoundInterpolationInner {
    id: Identifier,
    interpolate: CallbackValue<Arc<AnimationInterpolationFn>>,
}

/// A custom animation interpolation bound to its owning mod's callback at registration
/// (`register_json_animation_types`).
#[derive(Clone)]
// exhaustive: engine-constructed bound interpolation; private fields prevent downstream construction
// no wire: carries process-local animation target/interpolation callbacks
pub struct BoundInterpolation(Arc<BoundInterpolationInner>);

impl BoundInterpolation {
    /// The interpolation's registered identifier (`mymod:step`).
    pub fn id(&self) -> &Identifier {
        &self.0.id
    }

    /// The `[mod] id` of the mod that registered this interpolation.
    pub fn owner(&self) -> &'static str {
        self.0.interpolate.owner().id()
    }
}

impl std::fmt::Debug for BoundInterpolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundInterpolation")
            .field("id", &self.0.id)
            .field("owner", &self.owner())
            .finish()
    }
}

impl PartialEq for BoundInterpolation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for BoundInterpolation {}

/// Bind a registered animation target to its owning mod's callbacks.
// hidden: pairs the staged engine callback with its owner
#[doc(hidden)]
pub fn bind_animation_target(
    id: Identifier,
    owner: CallbackOwner,
    apply: Arc<AnimationTargetApply>,
    convert: Arc<AnimationTargetConvert>,
) -> BoundAnimationTarget {
    BoundAnimationTarget(Arc::new(BoundAnimationTargetInner {
        id,
        apply: CallbackValue::new(owner.clone(), "animation target", "drop_apply", apply),
        convert: CallbackValue::new(owner, "animation target", "drop_converter", convert),
    }))
}

/// Bind a registered animation interpolation to its owning mod's callback.
// hidden: pairs the staged engine callback with its owner
#[doc(hidden)]
pub fn bind_animation_interpolation(
    id: Identifier,
    owner: CallbackOwner,
    interpolate: Arc<AnimationInterpolationFn>,
) -> BoundInterpolation {
    BoundInterpolation(Arc::new(BoundInterpolationInner {
        id,
        interpolate: CallbackValue::new(
            owner,
            "animation interpolation",
            "drop_interpolator",
            interpolate,
        ),
    }))
}

/// Invoke a bound animation target's parse-time vector converter with panic containment.
// hidden: converts a decoded animation channel for the game
#[doc(hidden)]
pub fn convert_animation_target(
    target: &BoundAnimationTarget,
    value: [f32; 3],
) -> Result<[f32; 3], String> {
    let convert = &target.0.convert;
    let converted = convert
        .owner()
        .call("animation target", "convert_target", || {
            (convert.value())(value)
        })?;
    if !converted.iter().all(|v| v.is_finite()) {
        return Err(format!(
            "mod {}: animation target {} converter returned non-finite vector {converted:?}",
            target.owner(),
            target.0.id
        ));
    }
    Ok(converted)
}

/// What a channel moves (`AnimationChannel.Targets`): its sampled value is applied to a
/// [`ModelPart`] (`offsetPos`, `offsetRotation`, `offsetScale`, or a custom target bound at
/// registration).
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the three built-in targets and a bound custom target
// no wire: carries process-local animation target/interpolation callbacks
pub enum AnimationTarget {
    /// The part's position, in texels (`minecraft:position`).
    Position,
    /// The part's rotation, in radians (`minecraft:rotation`).
    Rotation,
    /// The part's scale, as an offset from 1 (`minecraft:scale`).
    Scale,
    /// A custom target bound at `register_json_animation_types`.
    Custom(BoundAnimationTarget),
}

impl AnimationTarget {
    /// Apply a sampled `value` to `part` (vanilla's `AnimationChannel.Target.apply`).
    ///
    /// For a custom target, invokes the bound callback through its owner's panic containment;
    /// if the callback panics, it is disabled for the session and a panic notice is recorded.
    pub fn apply(&self, part: &mut ModelPart, value: [f32; 3]) {
        match self {
            Self::Position => part.offset_pos(value),
            Self::Rotation => part.offset_rotation(value),
            Self::Scale => part.offset_scale(value),
            Self::Custom(bound) => {
                let apply = &bound.0.apply;
                let owner = apply.owner();
                if !owner.is_enabled() {
                    return;
                }
                let _ = owner.call("animation target", "apply", || (apply.value())(part, value));
            }
        }
    }
}

/// How a channel reaches a keyframe from the one before (`AnimationChannel.Interpolations`).
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the two built-in interpolations and a bound custom interpolation
// no wire: carries process-local animation target/interpolation callbacks
pub enum Interpolation {
    /// A straight line from the previous keyframe (`minecraft:linear`).
    Linear,
    /// A Catmull-Rom spline through the neighbouring keyframes (`minecraft:catmullrom`).
    CatmullRom,
    /// A custom interpolation bound at `register_json_animation_types`.
    Custom(BoundInterpolation),
}

/// One keyframe (`Keyframe`): at `time` seconds, the value arriving (`pre`) and the value leaving
/// (`post`), and how the channel moves into it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: plain data a parser builds and the sampler reads
// no wire: carries process-local animation target/interpolation callbacks
pub struct Keyframe {
    /// When, in seconds from the animation's start.
    pub time: f32,
    /// The value the channel arrives at.
    pub pre: [f32; 3],
    /// The value it leaves from.
    pub post: [f32; 3],
    /// How the channel moves into this keyframe from the one before.
    pub interpolation: Interpolation,
}

/// One bone's keyframes for one target (`AnimationChannel`), in time order.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: plain data a parser builds and the sampler reads
// no wire: carries process-local animation target/interpolation callbacks
pub struct AnimationChannel {
    /// What it moves.
    pub target: AnimationTarget,
    /// Its keyframes, in time order.
    pub keyframes: Vec<Keyframe>,
}

impl AnimationChannel {
    /// `KeyframeAnimation.Entry.apply`'s interpolation: the channel's value at `seconds`, times
    /// `scale`. Before the first keyframe it holds the first one, after the last the last.
    ///
    /// For a custom interpolation, invokes the bound callback through its owner's panic
    /// containment; if the callback is disabled, panics, or returns a non-finite vector,
    /// returns `None` so [`KeyframeAnimation`] skips only this channel.
    pub fn sample(&self, seconds: f32, scale: f32) -> Option<[f32; 3]> {
        let k = &self.keyframes;
        if k.is_empty() {
            return Some([0.0; 3]);
        }
        // Mth.binarySearch(0, len, i -> seconds <= k[i].time) - 1
        let first = k.iter().position(|f| seconds <= f.time).unwrap_or(k.len());
        let prev = first.saturating_sub(1);
        let next = (prev + 1).min(k.len() - 1);
        let delta = seconds - k[prev].time;
        let alpha = if next != prev {
            (delta / (k[next].time - k[prev].time)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let value = match &k[next].interpolation {
            Interpolation::CatmullRom => {
                let p0 = k[prev.saturating_sub(1)].post;
                let p1 = k[prev].post;
                let p2 = k[next].post;
                let p3 = k[(next + 1).min(k.len() - 1)].post;
                std::array::from_fn(|i| catmullrom(alpha, p0[i], p1[i], p2[i], p3[i]) * scale)
            }
            Interpolation::Linear => {
                let a = k[prev].post;
                let b = k[next].pre;
                std::array::from_fn(|i| (a[i] + (b[i] - a[i]) * alpha) * scale)
            }
            Interpolation::Custom(bound) => {
                let interpolate = &bound.0.interpolate;
                let owner = interpolate.owner();
                if !owner.is_enabled() {
                    return None;
                }
                match owner.call("animation interpolation", "interpolate", || {
                    (interpolate.value())(k, alpha, prev, next, scale)
                }) {
                    Ok(v) if v.iter().all(|c| c.is_finite()) => v,
                    Ok(v) => {
                        log::warn!(
                            "Sunrise: mod {}: animation interpolation {} returned non-finite vector {v:?}",
                            owner.id(),
                            bound.0.id
                        );
                        return None;
                    }
                    Err(_) => return None,
                }
            }
        };
        Some(value)
    }
}

/// `Mth.catmullrom`.
fn catmullrom(t: f32, p0: f32, p1: f32, p2: f32, p3: f32) -> f32 {
    0.5 * (2.0 * p1
        + (p2 - p0) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t)
}

/// An animation as its file gives it, bones by name (vanilla's `AnimationDefinition`; NeoForge's
/// `AnimationHolder`): what `RenderAssets::animation` answers. Custom targets and interpolations
/// are bound directly onto channels and keyframes during resource load, with custom target vectors
/// already converted. Bake it against a model with [`AnimationHolder::bake`] before playing it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: plain data a parser builds
// no wire: carries process-local animation target/interpolation callbacks
pub struct AnimationHolder {
    /// The animation's id (`mymod:golem/attack`).
    pub id: Identifier,
    /// The animation's length in seconds.
    pub length: f32,
    /// Whether it starts over at the end (`loop`).
    pub looping: bool,
    /// Each bone's channels, by the part's name, in file order.
    pub bones: Vec<(String, Vec<AnimationChannel>)>,
}

impl AnimationHolder {
    /// `AnimationDefinition.bake(root)`: each bone resolved once through the model's part lookup
    /// (`ModelPart.createPartLookup`). The name `root` always selects the actual root; other names
    /// select the first match, visiting each part's immediate children before their descendants.
    /// Bake once, as a renderer does in its constructor, and keep the result: a bone the model
    /// lacks is logged here and left out. Custom targets and interpolations bound on the channels
    /// and keyframes play directly with no lookup per sample.
    pub fn bake(&self, root: &ModelPart) -> KeyframeAnimation {
        let mut entries = Vec::new();
        for (bone, channels) in &self.bones {
            let Some(path) = root.descendant_path(bone) else {
                log::warn!("keyframe animation: the model has no part {bone:?}; skipped");
                continue;
            };
            for channel in channels {
                entries.push((path.clone(), channel.clone()));
            }
        }
        KeyframeAnimation {
            length: self.length,
            looping: self.looping,
            entries,
        }
    }
}

/// An animation baked against a model (`KeyframeAnimation`): each channel holds the path to its
/// part and any bound custom target or interpolation callbacks, so playing it walks no names and
/// performs no registry lookup per sample.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: built by `AnimationHolder::bake` only; its fields are private
// no wire: carries process-local animation target/interpolation callbacks
pub struct KeyframeAnimation {
    pub(crate) length: f32,
    pub(crate) looping: bool,
    pub(crate) entries: Vec<(Vec<u32>, AnimationChannel)>,
}

impl KeyframeAnimation {
    /// `KeyframeAnimation.apply(state, ageInTicks)`: while `state` is started, the pose
    /// `age_in_ticks` gives, added to `root`'s parts.
    pub fn apply(&self, root: &mut ModelPart, state: &AnimationState, age_in_ticks: f32) {
        self.apply_scaled(root, state, age_in_ticks, 1.0);
    }
    /// `KeyframeAnimation.apply(state, ageInTicks, speedFactor)`: as [`Self::apply`], played
    /// `speed_factor` times as fast.
    pub fn apply_scaled(
        &self,
        root: &mut ModelPart,
        state: &AnimationState,
        age_in_ticks: f32,
        speed_factor: f32,
    ) {
        if let Some(millis) = state.millis_at(age_in_ticks, speed_factor) {
            self.apply_at(root, millis, 1.0);
        }
    }
    /// `KeyframeAnimation.applyWalk(walkAnimationPos, walkAnimationSpeed, speedFactor,
    /// scaleFactor)`: the animation timed by a walk cycle, its strength by the walk speed.
    pub fn apply_walk(
        &self,
        root: &mut ModelPart,
        walk_pos: f32,
        walk_speed: f32,
        speed_factor: f32,
        scale_factor: f32,
    ) {
        let (millis, scale) = walk_timing(walk_pos, walk_speed, speed_factor, scale_factor);
        self.apply_at(root, millis, scale);
    }
    /// `KeyframeAnimation.applyStatic()`: the first frame only.
    pub fn apply_static(&self, root: &mut ModelPart) {
        self.apply_at(root, 0, 1.0);
    }
    /// `KeyframeAnimation.apply(millis, scale)`.
    fn apply_at(&self, root: &mut ModelPart, millis: i64, scale: f32) {
        // getElapsedSeconds: a looping animation wraps by its length.
        let mut seconds = millis as f32 / 1000.0;
        if self.looping {
            seconds %= self.length;
        }
        for (path, channel) in &self.entries {
            let Some(part) = root.descendant_mut(path) else {
                continue;
            };
            let Some(value) = channel.sample(seconds, scale) else {
                continue;
            };
            channel.target.apply(part, value);
        }
    }
}

/// `applyWalk`'s timing: `walkAnimationPos * 50 * speedFactor` ms, at
/// `min(walkAnimationSpeed * scaleFactor, 1)` strength.
fn walk_timing(walk_pos: f32, walk_speed: f32, speed_factor: f32, scale_factor: f32) -> (i64, f32) {
    let millis = (walk_pos * 50.0 * speed_factor) as i64;
    let strength = walk_speed * scale_factor;
    // Java Math.min propagates NaN; Rust's f32::min would turn it into full strength.
    let scale = if strength.is_nan() {
        strength
    } else {
        strength.min(1.0)
    };
    (millis, scale)
}

/// When an animation started (vanilla's `AnimationState`): a start tick, or stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: its field is private; it changes through vanilla's transitions
#[derive(sunrise_mod_api_macros::Wire)]
pub struct AnimationState {
    pub(crate) start_tick: i32,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self {
            start_tick: Self::STOPPED,
        }
    }
}

impl AnimationState {
    /// `AnimationState.STOPPED`.
    const STOPPED: i32 = i32::MIN;
    /// `new AnimationState()`: stopped.
    pub fn new() -> Self {
        Self::default()
    }
    /// `AnimationState.start(tickCount)`.
    pub fn start(&mut self, tick_count: i32) {
        self.start_tick = tick_count;
    }
    /// `AnimationState.startIfStopped(tickCount)`.
    pub fn start_if_stopped(&mut self, tick_count: i32) {
        if !self.is_started() {
            self.start(tick_count);
        }
    }
    /// `AnimationState.animateWhen(condition, tickCount)`: started while `condition` holds,
    /// stopped otherwise.
    pub fn animate_when(&mut self, condition: bool, tick_count: i32) {
        if condition {
            self.start_if_stopped(tick_count);
        } else {
            self.stop();
        }
    }
    /// `AnimationState.stop()`.
    pub fn stop(&mut self) {
        self.start_tick = Self::STOPPED;
    }
    /// `AnimationState.fastForward(ticks, speed)`: a started animation is `ticks * speed` further.
    pub fn fast_forward(&mut self, ticks: i32, speed: f32) {
        if self.is_started() {
            // Java's int subtraction wraps; so does this, and it never panics.
            self.start_tick = self.start_tick.wrapping_sub((ticks as f32 * speed) as i32);
        }
    }
    /// `AnimationState.getTimeInMillis(ageInTicks)`: how long it has run, 50 ms a tick.
    pub fn time_in_millis(&self, age_in_ticks: f32) -> i64 {
        ((age_in_ticks - self.start_tick as f32) * 50.0) as i64
    }
    /// `AnimationState.isStarted()`.
    pub fn is_started(&self) -> bool {
        self.start_tick != Self::STOPPED
    }
    /// `KeyframeAnimation.apply(state, ageInTicks, speedFactor)`'s time: while started,
    /// `(long)(getTimeInMillis(ageInTicks) * speedFactor)`.
    fn millis_at(&self, age_in_ticks: f32, speed_factor: f32) -> Option<i64> {
        self.is_started()
            .then(|| (self.time_in_millis(age_in_ticks) as f32 * speed_factor) as i64)
    }
}

/// A staged custom animation target or interpolation callback; dispatch attributes it to the
/// registering mod.
// no wire: retained process-local animation target/interpolation callback.
// hidden: retained registration staging belongs to the game
#[doc(hidden)]
#[non_exhaustive]
pub enum JsonAnimationTypeRegistration {
    /// A custom animation channel target (`RegisterJsonAnimationTypesEvent.registerTarget`).
    Target {
        /// The target identifier.
        id: Identifier,
        /// Callback applying a sampled vector to a model part.
        apply: Arc<AnimationTargetApply>,
        /// Callback converting raw JSON keyframe vectors at parse time.
        convert: Arc<AnimationTargetConvert>,
    },
    /// A custom keyframe interpolation (`RegisterJsonAnimationTypesEvent.registerInterpolation`).
    Interpolation {
        /// The interpolation identifier.
        id: Identifier,
        /// Callback computing a segment's interpolated vector.
        interpolate: Arc<AnimationInterpolationFn>,
    },
}

impl RegisterJsonAnimationTypesEvent<'_> {
    /// Register a custom animation target under `id` (NeoForge's
    /// `RegisterJsonAnimationTypesEvent.registerTarget`).
    ///
    /// Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted; duplicates
    /// and collisions with the built-in targets (`minecraft:position`, `minecraft:rotation`,
    /// `minecraft:scale`) are rejected as registration errors. `convert_target` transforms each
    /// JSON keyframe `[x, y, z]` vector once when animation files are parsed; `apply` applies the
    /// sampled vector to a [`ModelPart`] during playback.
    pub fn register_target<A, C>(&mut self, id: Identifier, apply: A, convert_target: C)
    where
        A: Fn(&mut ModelPart, [f32; 3]) + Send + Sync + 'static,
        C: Fn([f32; 3]) -> [f32; 3] + Send + Sync + 'static,
    {
        self.types.push(JsonAnimationTypeRegistration::Target {
            id,
            apply: Arc::new(apply),
            convert: Arc::new(convert_target),
        });
    }

    /// Register a custom keyframe interpolation under `id` (NeoForge's
    /// `RegisterJsonAnimationTypesEvent.registerInterpolation`).
    ///
    /// Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted; duplicates
    /// and collisions with the built-in interpolations (`minecraft:linear`, `minecraft:catmullrom`)
    /// are rejected as registration errors. `interpolate` receives
    /// `(keyframes, alpha, current_idx, next_idx, scale)` during playback.
    pub fn register_interpolation<F>(&mut self, id: Identifier, interpolate: F)
    where
        F: Fn(&[Keyframe], f32, usize, usize, f32) -> [f32; 3] + Send + Sync + 'static,
    {
        self.types
            .push(JsonAnimationTypeRegistration::Interpolation {
                id,
                interpolate: Arc::new(interpolate),
            });
    }
}
