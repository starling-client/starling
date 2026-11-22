//! Keyframe animations mods and packs ship (NeoForge's
//! `assets/<namespace>/neoforge/animations/entity/<path>.json`), read on every resource load into
//! the published render assets, where `RenderAssets::animation` finds them by id. The game's own
//! exported animations keep their reader (`mobs::keyframes`); both play through `mod_api`'s
//! sampling. Custom targets and interpolations mods register (NeoForge's
//! `RegisterJsonAnimationTypesEvent`) are bound once at start and resolved directly onto each
//! parsed channel and keyframe (matching NeoForge's `AnimationParser`).
use mod_api::{
    __private::game::{
        CallbackOwner, JsonAnimationTypeRegistration, bind_animation_interpolation,
        bind_animation_target, convert_animation_target,
    },
    AnimationChannel, AnimationHolder, AnimationTarget, Answered, BoundAnimationTarget,
    BoundInterpolation, Identifier, Interpolation, Keyframe,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

struct BoundAnimations {
    targets: BTreeMap<String, BoundAnimationTarget>,
    interpolations: BTreeMap<String, BoundInterpolation>,
}

static TYPES: OnceLock<Arc<BoundAnimations>> = OnceLock::new();

fn freeze(types: Vec<Answered<JsonAnimationTypeRegistration>>) -> BoundAnimations {
    let mut targets: BTreeMap<String, BoundAnimationTarget> = BTreeMap::new();
    let mut interpolations: BTreeMap<String, BoundInterpolation> = BTreeMap::new();
    for Answered { answer, mod_id, .. } in types {
        match answer {
            JsonAnimationTypeRegistration::Target { id, apply, convert } => {
                let key = id.to_string();
                if matches!(
                    key.as_str(),
                    "minecraft:position" | "minecraft:rotation" | "minecraft:scale"
                ) {
                    log::error!(
                        "Mod {mod_id}: animation target {key} collides with a built-in animation target"
                    );
                } else if let Some(first) = targets.get(&key) {
                    log::error!(
                        "Mod {mod_id}: animation target {key} is already registered by {}; the first registration is kept",
                        first.owner()
                    );
                } else if let Some(owner) = CallbackOwner::new(mod_id) {
                    targets.insert(key, bind_animation_target(id, owner, apply, convert));
                    continue;
                } else {
                    log::error!("Animation target {key} has no installed owner {mod_id}");
                }
                // Keep their destructors apart: both captures may panic independently.
                let owner = CallbackOwner::for_mod(mod_id);
                owner.dispose("animation target", "drop_apply", apply);
                owner.dispose("animation target", "drop_converter", convert);
            }
            JsonAnimationTypeRegistration::Interpolation { id, interpolate } => {
                let key = id.to_string();
                if matches!(key.as_str(), "minecraft:linear" | "minecraft:catmullrom") {
                    log::error!(
                        "Mod {mod_id}: animation interpolation {key} collides with a built-in animation interpolation"
                    );
                } else if let Some(first) = interpolations.get(&key) {
                    log::error!(
                        "Mod {mod_id}: animation interpolation {key} is already registered by {}; the first registration is kept",
                        first.owner()
                    );
                } else if let Some(owner) = CallbackOwner::new(mod_id) {
                    interpolations
                        .insert(key, bind_animation_interpolation(id, owner, interpolate));
                    continue;
                } else {
                    log::error!("Animation interpolation {key} has no installed owner {mod_id}");
                }
                CallbackOwner::for_mod(mod_id).dispose(
                    "animation interpolation",
                    "drop_interpolator",
                    interpolate,
                );
            }
            other => {
                CallbackOwner::for_mod(mod_id).dispose("animation type", "drop_registration", other)
            }
        }
    }
    BoundAnimations {
        targets,
        interpolations,
    }
}

/// Binds the animation targets and interpolations native mods registered once at start, before
/// resources load. Any valid `Identifier` (including `minecraft:` and empty-path ids) is accepted;
/// collisions with the five built-ins (`minecraft:position`, `minecraft:rotation`,
/// `minecraft:scale`, `minecraft:linear`, `minecraft:catmullrom`) and duplicates are logged as
/// registration errors and left out (the first registration wins).
pub fn register(types: Vec<Answered<JsonAnimationTypeRegistration>>) {
    let bound = freeze(types);
    if TYPES.set(Arc::new(bound)).is_err() {
        log::warn!("animation types were registered already; these are left out");
    }
}

/// The folder under each namespace the files are read from.
const FOLDER: &str = "neoforge/animations/entity";

/// The animations of one resource load, by id (`mymod:golem/attack` for
/// `assets/mymod/neoforge/animations/entity/golem/attack.json`).
#[derive(Clone, Debug, Default)]
pub struct Animations(BTreeMap<Identifier, Arc<AnimationHolder>>);

impl Animations {
    /// Every animation file of `source`. A file that doesn't parse is logged with its reason and
    /// left out; the others load.
    pub fn load(source: &crate::resource_source::ResourceSource) -> Self {
        let types = TYPES.get().map(Arc::as_ref);
        let mut out = BTreeMap::new();
        for file in source.list_under("assets", &[FOLDER]) {
            let Some((namespace, path)) = file
                .strip_prefix("assets/")
                .and_then(|rest| rest.split_once(&format!("/{FOLDER}/")))
            else {
                continue;
            };
            let Some(path) = path.strip_suffix(".json") else {
                continue;
            };
            let Ok(id) = Identifier::from_parts(namespace, path) else {
                log::warn!("Sunrise: animation {file}: not a valid id; skipped");
                continue;
            };
            match source
                .read_to_string(&file)
                .map_err(|e| e.to_string())
                .and_then(|text| parse_with_types(id.clone(), &text, types))
            {
                Ok(holder) => {
                    out.insert(id, Arc::new(holder));
                }
                Err(error) => log::warn!("Sunrise: animation {id} not loaded: {error}"),
            }
        }
        Self(out)
    }
    /// The animation `id`, if a file gave it.
    pub fn get(&self, id: &Identifier) -> Option<&AnimationHolder> {
        self.0.get(id).map(Arc::as_ref)
    }
}

/// One animation file: `length` (seconds), optional `loop`, and `animations`, entries of a
/// `bone`, a `target` and its `keyframes` (`timestamp`, `target` or `preTarget`/`postTarget`
/// values, `interpolation`). Values go through vanilla's builder helpers (`KeyframeAnimations`) for
/// built-in targets — a position's y is negated (`posVec`), a rotation is in degrees (`degreeVec`),
/// a scale is a factor (`scaleVec` keeps its offset from 1) — or through a registered custom
/// target's converter during parse (matching NeoForge's `AnimationParser`). A target or
/// interpolation neither built in nor registered fails the file, naming it.
pub fn parse(animation: Identifier, text: &str) -> Result<AnimationHolder, String> {
    parse_with_types(animation, text, TYPES.get().map(Arc::as_ref))
}

fn parse_with_types(
    animation: Identifier,
    text: &str,
    types: Option<&BoundAnimations>,
) -> Result<AnimationHolder, String> {
    let json: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let length = finite(&json["length"]).ok_or("no finite `length`")? as f32;
    if length <= 0.0 {
        return Err(format!("`length` {length} is not positive"));
    }
    let looping = match json.get("loop") {
        None => false,
        Some(value) => value.as_bool().ok_or("`loop` is not a boolean")?,
    };
    let entries = json["animations"]
        .as_array()
        .ok_or("no `animations` list")?;
    let mut bones: Vec<(String, Vec<AnimationChannel>)> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let bone = entry["bone"]
            .as_str()
            .ok_or_else(|| format!("animations[{index}]: no `bone`"))?;
        let target_id = id(&entry["target"]);
        let target = match target_id.as_deref() {
            Some("minecraft:position") => AnimationTarget::Position,
            Some("minecraft:rotation") => AnimationTarget::Rotation,
            Some("minecraft:scale") => AnimationTarget::Scale,
            Some(id) => match types.and_then(|t| t.targets.get(id)) {
                Some(bound) => AnimationTarget::Custom(bound.clone()),
                None => return Err(format!("animations[{index}]: unknown target {id}")),
            },
            None => return Err(format!("animations[{index}]: unknown target (none)")),
        };
        let keyframes = entry["keyframes"]
            .as_array()
            .ok_or_else(|| format!("animations[{index}]: no `keyframes` list"))?
            .iter()
            .enumerate()
            .map(|(at, keyframe)| {
                keyframe_of(keyframe, &target, types)
                    .map_err(|e| format!("animations[{index}].keyframes[{at}]: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let channel = AnimationChannel { target, keyframes };
        match bones.iter_mut().find(|(name, _)| name == bone) {
            Some((_, channels)) => channels.push(channel),
            None => bones.push((bone.to_owned(), vec![channel])),
        }
    }
    Ok(AnimationHolder {
        id: animation,
        length,
        looping,
        bones,
    })
}

/// A number that stays finite as an `f32`: 1e40 doesn't, it would become infinity.
fn finite(value: &Value) -> Option<f64> {
    let value = value.as_f64()?;
    (value as f32).is_finite().then_some(value)
}

/// A namespaced id parsed with `Identifier::parse` (`minecraft:` for bare or leading `:` names).
fn id(value: &Value) -> Option<String> {
    Identifier::parse(value.as_str()?)
        .ok()
        .map(|id| id.to_string())
}

fn parse_vec3(value: &Value, field: &str) -> Result<[f64; 3], String> {
    let values = value
        .as_array()
        .filter(|values| values.len() == 3)
        .ok_or_else(|| format!("`{field}` is not three numbers"))?;
    let mut v = [0.0f64; 3];
    for (slot, value) in v.iter_mut().zip(values) {
        *slot = finite(value).ok_or_else(|| format!("`{field}` is not three finite numbers"))?;
    }
    Ok(v)
}

fn convert_vector(target: &AnimationTarget, v: [f64; 3]) -> Result<[f32; 3], String> {
    match target {
        // KeyframeAnimations.posVec
        AnimationTarget::Position => Ok([v[0] as f32, -(v[1] as f32), v[2] as f32]),
        // KeyframeAnimations.degreeVec
        AnimationTarget::Rotation => Ok(v.map(|d| d as f32 * 0.017_453_292)),
        // KeyframeAnimations.scaleVec
        AnimationTarget::Scale => Ok(v.map(|d| (d - 1.0) as f32)),
        AnimationTarget::Custom(bound) => convert_animation_target(bound, v.map(|d| d as f32)),
    }
}

fn keyframe_of(
    keyframe: &Value,
    target: &AnimationTarget,
    types: Option<&BoundAnimations>,
) -> Result<Keyframe, String> {
    let time = finite(&keyframe["timestamp"]).ok_or("no finite `timestamp`")? as f32;
    let (pre, post) = if keyframe.get("preTarget").is_some() || keyframe.get("postTarget").is_some()
    {
        let pre_raw = parse_vec3(&keyframe["preTarget"], "preTarget")?;
        let post_raw = parse_vec3(&keyframe["postTarget"], "postTarget")?;
        (
            convert_vector(target, pre_raw)?,
            convert_vector(target, post_raw)?,
        )
    } else {
        let raw = parse_vec3(&keyframe["target"], "target")?;
        let value = convert_vector(target, raw)?;
        (value, value)
    };
    let interpolation_id = id(&keyframe["interpolation"]);
    let interpolation = match interpolation_id.as_deref() {
        Some("minecraft:linear") => Interpolation::Linear,
        Some("minecraft:catmullrom") => Interpolation::CatmullRom,
        Some(id) => match types.and_then(|t| t.interpolations.get(id)) {
            Some(bound) => Interpolation::Custom(bound.clone()),
            None => return Err(format!("unknown interpolation {id}")),
        },
        None => return Err("unknown interpolation (none)".to_owned()),
    };
    Ok(Keyframe {
        time,
        pre,
        post,
        interpolation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_source::{Resource, ResourceSource};
    use mod_api::{
        __private::{game::RegistrationContextNew, take_hook_panics},
        AnimationState, ModelPart, PartPose, RegistrationContext,
    };
    use std::{
        f32::consts::{FRAC_PI_2, FRAC_PI_4},
        sync::atomic::{AtomicUsize, Ordering},
    };

    const SWING: &str = r#"{"length": 2.0, "loop": true, "animations": [
        {"bone": "arm", "target": "minecraft:rotation", "keyframes": [
            {"timestamp": 0.0, "target": [0, 0, 0], "interpolation": "minecraft:linear"},
            {"timestamp": 1.0, "target": [90, 0, -45], "interpolation": "linear"}]},
        {"bone": "arm", "target": "minecraft:position", "keyframes": [
            {"timestamp": 0.0, "target": [1, 2, 3], "interpolation": "minecraft:catmullrom"}]},
        {"bone": "body", "target": "minecraft:scale", "keyframes": [
            {"timestamp": 0.0, "target": [2, 1, 0.5], "interpolation": "minecraft:linear"}]}
    ]}"#;

    /// A file of the test animation `test:animation`.
    fn read(text: &str) -> Result<AnimationHolder, String> {
        parse(Identifier::parse("test:animation").unwrap(), text)
    }

    fn key(holder: &AnimationHolder, bone: usize, channel: usize, at: usize) -> &Keyframe {
        &holder.bones[bone].1[channel].keyframes[at]
    }

    /// Values go through vanilla's builder helpers: degrees to radians, y negated, scale as an
    /// offset from 1. 90 and -45 degrees times `degreeVec`'s factor round to the f32 quarter and
    /// eighth turns exactly (worked out apart from this code).
    #[test]
    fn a_file_parses_with_vanillas_units_and_groups_its_bones() {
        let holder = read(SWING).unwrap();
        assert_eq!((holder.length, holder.looping), (2.0, true));
        let bones: Vec<&str> = holder.bones.iter().map(|(b, _)| b.as_str()).collect();
        assert_eq!(
            bones,
            ["arm", "body"],
            "two entries of `arm` share its bone"
        );
        let swing = key(&holder, 0, 0, 1);
        assert_eq!(swing.post, [FRAC_PI_2, 0.0, -FRAC_PI_4]);
        assert_eq!(swing.pre, swing.post);
        assert_eq!(
            swing.interpolation,
            Interpolation::Linear,
            "an unnamespaced id is minecraft's"
        );
        let at = key(&holder, 0, 1, 0);
        assert_eq!(at.post, [1.0, -2.0, 3.0]);
        assert_eq!(at.interpolation, Interpolation::CatmullRom);
        assert_eq!(key(&holder, 1, 0, 0).post, [1.0, 0.0, -0.5]);
        assert!(
            !read(r#"{"length":1,"animations":[]}"#).unwrap().looping,
            "loop defaults to false"
        );
        // Leading colon resolves into minecraft: just like vanilla Identifier.parse.
        let leading_colon = read(
            r#"{"length":1,"animations":[{"bone":"arm","target":":position",
                "keyframes":[{"timestamp":0,"target":[1,2,3],"interpolation":":linear"}]}]}"#,
        )
        .unwrap();
        assert_eq!(
            leading_colon.bones[0].1[0].target,
            AnimationTarget::Position
        );
        assert_eq!(
            key(&leading_colon, 0, 0, 0).interpolation,
            Interpolation::Linear
        );
    }

    #[test]
    fn an_unknown_target_or_interpolation_fails_the_file_by_name() {
        let bad_target =
            r#"{"length":1,"animations":[{"bone":"a","target":"mymod:glow","keyframes":[]}]}"#;
        assert_eq!(
            read(bad_target).unwrap_err(),
            "animations[0]: unknown target mymod:glow"
        );
        let bad_interpolation = r#"{"length":1,"animations":[{"bone":"a","target":"minecraft:scale",
            "keyframes":[{"timestamp":0,"target":[1,1,1],"interpolation":"mymod:bounce"}]}]}"#;
        assert_eq!(
            read(bad_interpolation).unwrap_err(),
            "animations[0].keyframes[0]: unknown interpolation mymod:bounce"
        );
        assert!(read(r#"{"animations":[]}"#).is_err(), "no length");
        assert_eq!(
            read(r#"{"length":0,"loop":true,"animations":[]}"#).unwrap_err(),
            "`length` 0 is not positive"
        );
        assert_eq!(
            read(r#"{"length":1e40,"animations":[]}"#).unwrap_err(),
            "no finite `length`"
        );
        let huge = r#"{"length":1,"animations":[{"bone":"a","target":"minecraft:scale",
            "keyframes":[{"timestamp":0,"target":[1,1e40,1],"interpolation":"minecraft:linear"}]}]}"#;
        assert_eq!(
            read(huge).unwrap_err(),
            "animations[0].keyframes[0]: `target` is not three finite numbers"
        );
        let late = r#"{"length":1,"animations":[{"bone":"a","target":"minecraft:scale",
            "keyframes":[{"timestamp":1e40,"target":[1,1,1],"interpolation":"minecraft:linear"}]}]}"#;
        assert_eq!(
            read(late).unwrap_err(),
            "animations[0].keyframes[0]: no finite `timestamp`"
        );
        let short = r#"{"length":1,"animations":[{"bone":"a","target":"minecraft:scale",
            "keyframes":[{"timestamp":0,"target":[1,1],"interpolation":"minecraft:linear"}]}]}"#;
        assert!(read(short).is_err(), "two values");
    }

    /// Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted; all five
    /// built-in collisions and duplicates are rejected (first registration wins), while a target
    /// and an interpolation with the same id remain separate kinds.
    #[test]
    fn minecraft_namespace_is_accepted_and_builtins_and_duplicates_are_rejected() {
        struct First;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for First {
            fn register_json_animation_types(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterJsonAnimationTypesEvent<'_>,
            ) {
                event.register_target(
                    Identifier::parse("minecraft:custom_target").unwrap(),
                    |part, v| part.offset_pos(v),
                    |v| v.map(|c| c * 2.0),
                );
                event.register_interpolation(
                    Identifier::parse("minecraft:custom_interp").unwrap(),
                    |k, _, prev, _, scale| k[prev].post.map(|c| c * scale),
                );
                event.register_target(
                    Identifier::parse("animreg:glow").unwrap(),
                    |part, v| part.offset_pos(v),
                    |v| v,
                );
                event.register_target(
                    Identifier::parse("animreg:").unwrap(),
                    |part, v| part.offset_pos(v),
                    |v| v,
                );
                event.register_interpolation(
                    Identifier::parse("animreg:").unwrap(),
                    |k, _, prev, _, scale| k[prev].post.map(|c| c * scale),
                );
                // All 5 built-ins must be rejected.
                for builtin in [
                    "minecraft:position",
                    "minecraft:rotation",
                    "minecraft:scale",
                ] {
                    event.register_target(
                        Identifier::parse(builtin).unwrap(),
                        |_, _| panic!("built-in target collision should be rejected"),
                        |_| panic!("built-in target collision should be rejected"),
                    );
                }
                for builtin in ["minecraft:linear", "minecraft:catmullrom"] {
                    event.register_interpolation(
                        Identifier::parse(builtin).unwrap(),
                        |_, _, _, _, _| {
                            panic!("built-in interpolation collision should be rejected")
                        },
                    );
                }
            }
        }

        struct Second;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Second {
            fn register_json_animation_types(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterJsonAnimationTypesEvent<'_>,
            ) {
                // Duplicate target: first registration stands.
                event.register_target(
                    Identifier::parse("animreg:glow").unwrap(),
                    |_, _| panic!("duplicate target should be rejected"),
                    |_| panic!("duplicate target should be rejected"),
                );
                // Same id as an interpolation is a separate kind and is accepted.
                event.register_interpolation(
                    Identifier::parse("animreg:glow").unwrap(),
                    |k, _, _, next, scale| k[next].pre.map(|c| c * scale),
                );
                // Duplicate interpolation: first registration stands.
                event.register_interpolation(
                    Identifier::parse("minecraft:custom_interp").unwrap(),
                    |_, _, _, _, _| panic!("duplicate interpolation should be rejected"),
                );
            }
        }

        let mods: mod_api::Mods = vec![
            mod_api::ModEntry::new("first_anim", Box::new(First)),
            mod_api::ModEntry::new("second_anim", Box::new(Second)),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut types = Vec::new();
            mod_api::dispatch_register_json_animation_types(
                &RegistrationContext::new(),
                &mut types,
            );
            let bound = freeze(types);
            let target_keys: Vec<_> = bound
                .targets
                .iter()
                .map(|(k, v)| (k.as_str(), v.owner()))
                .collect();
            let interp_keys: Vec<_> = bound
                .interpolations
                .iter()
                .map(|(k, v)| (k.as_str(), v.owner()))
                .collect();
            assert_eq!(
                target_keys,
                [
                    ("animreg:", "first_anim"),
                    ("animreg:glow", "first_anim"),
                    ("minecraft:custom_target", "first_anim"),
                ]
            );
            assert_eq!(
                interp_keys,
                [
                    ("animreg:", "first_anim"),
                    ("animreg:glow", "second_anim"),
                    ("minecraft:custom_interp", "first_anim"),
                ]
            );
        });
    }

    /// Registered targets convert keyframe vectors at parse time and bind directly onto channels;
    /// registered interpolations bind directly onto keyframes; `holder.bake(&root)` plays both
    /// without a second lookup.
    #[test]
    fn custom_target_converts_at_parse_and_plays_directly_through_bake() {
        struct GlowMod;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for GlowMod {
            fn register_json_animation_types(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterJsonAnimationTypesEvent<'_>,
            ) {
                event.register_target(
                    Identifier::parse("animfile:pulse").unwrap(),
                    |part, value| part.offset_pos(value),
                    |raw| raw.map(|v| v * 3.0),
                );
                event.register_interpolation(
                    Identifier::parse("animfile:step").unwrap(),
                    |keyframes, alpha, _prev, next, scale| {
                        keyframes[next].pre.map(|v| v * alpha * scale)
                    },
                );
            }
        }

        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("pulser", Box::new(GlowMod))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut types = Vec::new();
            mod_api::dispatch_register_json_animation_types(
                &RegistrationContext::new(),
                &mut types,
            );
            let bound = freeze(types);
            let read = |text: &str| {
                parse_with_types(
                    Identifier::parse("test:animation").unwrap(),
                    text,
                    Some(&bound),
                )
            };

            let holder = read(
                r#"{"length": 1, "animations": [
                {"bone": "head", "target": "animfile:pulse", "keyframes": [
                    {"timestamp": 0, "preTarget": [1, 2, 3], "postTarget": [2, 3, 4], "interpolation": "linear"},
                    {"timestamp": 1, "target": [3, 4, 5], "interpolation": "animfile:step"}]},
                {"bone": "head", "target": "minecraft:position", "keyframes": [
                    {"timestamp": 0, "target": [0, 2, 0], "interpolation": "animfile:step"}]}
            ]}"#,
            )
            .unwrap();

            // Keyframe vectors were converted at parse time, not deferred to bake.
            let first = key(&holder, 0, 0, 0);
            assert_eq!(first.pre, [3.0, 6.0, 9.0]);
            assert_eq!(first.post, [6.0, 9.0, 12.0]);
            let second = key(&holder, 0, 0, 1);
            assert_eq!(second.pre, [9.0, 12.0, 15.0]);
            assert_eq!(second.post, [9.0, 12.0, 15.0]);
            assert!(matches!(
                &holder.bones[0].1[0].target,
                AnimationTarget::Custom(bound) if bound.id().to_string() == "animfile:pulse"
            ));
            assert!(matches!(
                &second.interpolation,
                Interpolation::Custom(bound) if bound.id().to_string() == "animfile:step"
            ));

            // Built-in posVec still applies on built-in target beside custom interpolation.
            let built_in = key(&holder, 0, 1, 0);
            assert_eq!(built_in.post, [0.0, -2.0, 0.0]);

            // Bake and play directly through `holder.bake(&root)`.
            let root = ModelPart::new(
                Vec::new(),
                vec![(
                    "head".to_owned(),
                    ModelPart::new(Vec::new(), Vec::new(), PartPose::ZERO),
                )],
                PartPose::ZERO,
            );
            let baked = holder.bake(&root);
            let mut state = AnimationState::new();
            state.start(0);
            let mut posed = root.clone();
            // 10 ticks = 500 ms = alpha 0.5:
            // channel 0 (custom target + custom interp): [9, 12, 15] * 0.5 = [4.5, 6, 7.5]
            // channel 1 (position, single keyframe at t=0, alpha=0): [0, -2, 0] * 0 = [0, 0, 0]
            baked.apply(&mut posed, &state, 10.0);
            let head = posed.get_child("head").unwrap();
            assert_eq!((head.x, head.y, head.z), (4.5, 6.0, 7.5));

            // Target and interpolation remain separate kinds.
            let as_interpolation = r#"{"length":1,"animations":[{"bone":"head","target":"animfile:pulse",
                "keyframes":[{"timestamp":0,"target":[1,1,1],"interpolation":"animfile:pulse"}]}]}"#;
            assert_eq!(
                read(as_interpolation).unwrap_err(),
                "animations[0].keyframes[0]: unknown interpolation animfile:pulse"
            );

            // Raw non-finite JSON values on custom target are still rejected.
            let huge = r#"{"length":1,"animations":[{"bone":"head","target":"animfile:pulse",
                "keyframes":[{"timestamp":0,"target":[1e40,0,0],"interpolation":"linear"}]}]}"#;
            assert_eq!(
                read(huge).unwrap_err(),
                "animations[0].keyframes[0]: `target` is not three finite numbers"
            );
        });
    }

    /// A converter returning non-finite values fails that file without latching; panics in a
    /// converter, apply callback, or interpolation disable that callback with a panic notice.
    #[test]
    fn converter_and_playback_panics_disable_with_notice_and_non_finite_does_not_latch() {
        let flaky_calls = Arc::new(AtomicUsize::new(0));
        let panic_convert_calls = Arc::new(AtomicUsize::new(0));
        let panic_apply_calls = Arc::new(AtomicUsize::new(0));
        let panic_interp_calls = Arc::new(AtomicUsize::new(0));

        struct FaultyMod {
            flaky_calls: Arc<AtomicUsize>,
            panic_convert_calls: Arc<AtomicUsize>,
            panic_apply_calls: Arc<AtomicUsize>,
            panic_interp_calls: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for FaultyMod {
            fn register_json_animation_types(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterJsonAnimationTypesEvent<'_>,
            ) {
                let flaky = Arc::clone(&self.flaky_calls);
                event.register_target(
                    Identifier::parse("animfault:flaky_convert").unwrap(),
                    |part, v| part.offset_pos(v),
                    move |v| {
                        if flaky.fetch_add(1, Ordering::Relaxed) == 0 {
                            [f32::NAN, 0.0, 0.0]
                        } else {
                            v
                        }
                    },
                );
                let pc = Arc::clone(&self.panic_convert_calls);
                event.register_target(
                    Identifier::parse("animfault:panic_convert").unwrap(),
                    |_, _| {},
                    move |_| {
                        pc.fetch_add(1, Ordering::Relaxed);
                        panic!("convert boom");
                    },
                );
                let pa = Arc::clone(&self.panic_apply_calls);
                event.register_target(
                    Identifier::parse("animfault:panic_apply").unwrap(),
                    move |_, _| {
                        pa.fetch_add(1, Ordering::Relaxed);
                        panic!("apply boom");
                    },
                    |v| v,
                );
                let pi = Arc::clone(&self.panic_interp_calls);
                event.register_interpolation(
                    Identifier::parse("animfault:panic_interp").unwrap(),
                    move |_, _, _, _, _| {
                        pi.fetch_add(1, Ordering::Relaxed);
                        panic!("interp boom");
                    },
                );
            }
        }

        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "anim_panicker",
            Box::new(FaultyMod {
                flaky_calls: Arc::clone(&flaky_calls),
                panic_convert_calls: Arc::clone(&panic_convert_calls),
                panic_apply_calls: Arc::clone(&panic_apply_calls),
                panic_interp_calls: Arc::clone(&panic_interp_calls),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut types = Vec::new();
            mod_api::dispatch_register_json_animation_types(
                &RegistrationContext::new(),
                &mut types,
            );
            let bound = freeze(types);
            let read = |text: &str| {
                parse_with_types(
                    Identifier::parse("test:animation").unwrap(),
                    text,
                    Some(&bound),
                )
            };
            let _ = take_hook_panics();

            // Non-finite converter fails the file cleanly without latching or panic notice.
            let flaky_json = r#"{"length":1,"animations":[{"bone":"head","target":"animfault:flaky_convert",
                "keyframes":[{"timestamp":0,"target":[1,2,3],"interpolation":"linear"}]}]}"#;
            assert!(read(flaky_json).is_err());
            assert_eq!(flaky_calls.load(Ordering::Relaxed), 1);
            assert!(take_hook_panics().is_empty());
            assert!(read(flaky_json).is_ok());
            assert_eq!(flaky_calls.load(Ordering::Relaxed), 2);

            // Panicking converter fails the file, disables the target, and records a panic notice.
            let panic_convert_json = r#"{"length":1,"animations":[{"bone":"head","target":"animfault:panic_convert",
                "keyframes":[{"timestamp":0,"target":[1,2,3],"interpolation":"linear"}]}]}"#;
            assert!(read(panic_convert_json).is_err());
            assert_eq!(panic_convert_calls.load(Ordering::Relaxed), 1);
            let notices = take_hook_panics();
            assert!(
                notices
                    .iter()
                    .any(|notice| notice.mod_id == "anim_panicker"
                        && notice.hook == "convert_target"),
                "expected converter panic notice, got {notices:?}"
            );
            assert!(read(panic_convert_json).is_err());
            assert_eq!(panic_convert_calls.load(Ordering::Relaxed), 1);

            // Panicking playback interpolation skips its channel (never invoking its target),
            // and a separate channel with a panicking target disables on first panic with notices.
            let playback_json = r#"{"length":1,"animations":[
                {"bone":"head","target":"animfault:panic_apply","keyframes":[
                    {"timestamp":0,"target":[0,0,0],"interpolation":"linear"},
                    {"timestamp":1,"target":[1,1,1],"interpolation":"animfault:panic_interp"}
                ]},
                {"bone":"head","target":"animfault:panic_apply","keyframes":[
                    {"timestamp":0,"target":[0,0,0],"interpolation":"linear"},
                    {"timestamp":1,"target":[1,1,1],"interpolation":"linear"}
                ]}
            ]}"#;
            let holder = read(playback_json).unwrap();
            let root = ModelPart::new(
                Vec::new(),
                vec![(
                    "head".to_owned(),
                    ModelPart::new(Vec::new(), Vec::new(), PartPose::ZERO),
                )],
                PartPose::ZERO,
            );
            let baked = holder.bake(&root);
            let mut state = AnimationState::new();
            state.start(0);
            let mut posed = root.clone();
            baked.apply(&mut posed, &state, 10.0);
            assert_eq!(panic_interp_calls.load(Ordering::Relaxed), 1);
            assert_eq!(panic_apply_calls.load(Ordering::Relaxed), 1);
            let notices = take_hook_panics();
            assert!(
                notices
                    .iter()
                    .any(|notice| notice.mod_id == "anim_panicker" && notice.hook == "interpolate"),
                "expected interpolation panic notice, got {notices:?}"
            );
            assert!(
                notices
                    .iter()
                    .any(|notice| notice.mod_id == "anim_panicker" && notice.hook == "apply"),
                "expected target panic notice, got {notices:?}"
            );
            // Second frame does not invoke either disabled callback.
            baked.apply(&mut posed, &state, 10.0);
            assert_eq!(panic_interp_calls.load(Ordering::Relaxed), 1);
            assert_eq!(panic_apply_calls.load(Ordering::Relaxed), 1);
        });
    }

    /// Every load reads the files again: a nested path is part of the id, a broken file is left
    /// out while the others load, and an edited file plays its new pose on a mod's own model.
    #[test]
    fn a_mod_animation_loads_by_id_and_a_reload_reads_the_edited_file() {
        let source = |swing: &str| {
            ResourceSource::indexed(
                [
                    (
                        "assets/golems/neoforge/animations/entity/golem/swing.json",
                        swing.to_owned(),
                    ),
                    (
                        "assets/golems/neoforge/animations/entity/broken.json",
                        "{".to_owned(),
                    ),
                    ("assets/golems/other/swing.json", SWING.to_owned()),
                ]
                .into_iter()
                .map(|(name, text)| (name.to_owned(), Resource::bytes(text.into_bytes())))
                .collect(),
            )
        };
        let id = Identifier::parse("golems:golem/swing").unwrap();
        let model = ModelPart::new(
            Vec::new(),
            vec![(
                "arm".to_owned(),
                ModelPart::new(Vec::new(), Vec::new(), PartPose::ZERO),
            )],
            PartPose::ZERO,
        );
        let arm_x_rot = |animations: &Animations| {
            let holder = animations.get(&id).expect("the file loads by its id");
            let mut posed = model.clone();
            let mut state = AnimationState::new();
            state.start(0);
            // 10 ticks: 500 ms, halfway to the second keyframe.
            holder.bake(&model).apply(&mut posed, &state, 10.0);
            posed.get_child("arm").unwrap().x_rot
        };
        let first = Animations::load(&source(SWING));
        assert_eq!(first.0.len(), 1, "only the good file under the folder");
        assert_eq!(arm_x_rot(&first), FRAC_PI_4);
        let edited = SWING.replace("[90, 0, -45]", "[180, 0, 0]");
        assert_eq!(arm_x_rot(&Animations::load(&source(&edited))), FRAC_PI_2);
    }
}
