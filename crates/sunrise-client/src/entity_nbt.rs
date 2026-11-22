//! Vanilla 26.3 entity NBT (`Entity.saveWithoutId` and the subclasses'
//! `addAdditionalSaveData`) for the entities Sunrise simulates: mobs, item
//! entities, experience orbs, arrows and falling blocks. Encoding starts from
//! the compound the entity was loaded with, when there is one, so fields
//! Sunrise does not model (variants, attributes, equipment, custom names)
//! survive a Sunrise save.
use crate::{
    arrows::Pickup,
    entities::{ArmorStand, Cushion, ExperienceOrb, ItemEntity, ItemFrame, Painting},
    mobs::{Mob, MobType, Who},
    nbt_path::int_value,
    projectiles::{Projectile, ProjectileKind},
    save::{
        doubles, float, floats, item_from_nbt, item_to_nbt, number, put, read_doubles, text,
        uuid_from_tag, uuid_tag,
    },
};
use std::{collections::HashMap, sync::Arc};
use sunrise_core::{geometry::Vec3, nbt::JavaString};
mod boat_load;
pub use boat_load::boat_from_nbt;
mod projectile_fields;
use projectile_fields::{load_projectile_entity_fields, put_projectile_entity_fields};
mod ai_pause;
pub use ai_pause::write_ai_paused;
mod mob_death;
mod refs;
mod saved_attributes;
pub use refs::who_by_uuid;

use {
    sunrise_core::nbt::{Compound, Tag},
    sunrise_world::{
        chunk::BlockState,
        registry::{BlockRegistry, BlockStateId},
    },
};

/// Saved data carried by `ConversionType.convertCommon` outside the modelled
/// mob fields. The converted entity has its own identity and type-specific data;
/// only its command tags are inherited from the old entity's raw compound.
pub fn conversion_tags(raw: Option<&Tag>) -> Option<Tag> {
    let Tag::List(tags) = raw?.get("Tags")? else {
        return None;
    };
    (!tags.is_empty()).then(|| Tag::compound([("Tags", Tag::List(tags.clone()))]))
}

fn string(s: &str) -> Tag {
    Tag::String(s.into())
}

fn hidden_effect_nbt(effect: &crate::effects::EffectInstance) -> Tag {
    let mut fields = vec![("duration", Tag::Int(effect.duration))];
    if effect.amplifier != 0 {
        fields.push((
            "amplifier",
            Tag::Byte(effect.amplifier.clamp(0, 255) as u8 as i8),
        ));
    }
    if effect.ambient {
        fields.push(("ambient", Tag::Byte(1)));
    }
    if !effect.visible {
        fields.push(("show_particles", Tag::Byte(0)));
    }
    fields.push(("show_icon", Tag::Byte(i8::from(effect.show_icon))));
    if let Some(hidden) = &effect.hidden {
        fields.push(("hidden_effect", hidden_effect_nbt(hidden)));
    }
    Tag::compound(fields)
}

fn hidden_effect_from_nbt(effect: &'static str, tag: &Tag) -> crate::effects::EffectInstance {
    let mut instance = crate::effects::EffectInstance::new(
        effect,
        number(tag.get("duration")).unwrap_or(0) as i32,
        number(tag.get("amplifier")).unwrap_or(0) as i32,
    );
    instance.ambient = number(tag.get("ambient")).is_some_and(|v| v != 0);
    instance.visible = number(tag.get("show_particles")) != Some(0);
    instance.show_icon = number(tag.get("show_icon")).map_or(instance.visible, |v| v != 0);
    instance.hidden = tag
        .get("hidden_effect")
        .map(|hidden| Box::new(hidden_effect_from_nbt(effect, hidden)));
    instance
}
/// Entity type ID (`id`).
pub fn kind(tag: &Tag) -> Option<String> {
    sunrise_client_core::identifier::normalize_id(&text(tag.get("id"))?)
}
pub fn position(tag: &Tag) -> Option<Vec3> {
    read_doubles::<3>(tag.get("Pos")).map(|p| Vec3::new(p[0], p[1], p[2]))
}
pub fn uuid(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("UUID"))
}
pub fn motion(tag: &Tag) -> Vec3 {
    read_doubles::<3>(tag.get("Motion")).map_or(Vec3::ZERO, |m| Vec3::new(m[0], m[1], m[2]))
}
/// A saved Motion as `Entity.load` takes it: a component faster than 10 blocks
/// a tick loads as 0.
pub fn loaded_motion(tag: &Tag) -> Vec3 {
    let m = motion(tag);
    let clamp = |v: f64| if v.abs() > 10.0 { 0.0 } else { v };
    Vec3::new(clamp(m.x), clamp(m.y), clamp(m.z))
}
pub fn rotation(tag: &Tag) -> [f32; 2] {
    read_doubles::<2>(tag.get("Rotation")).map_or([0.0; 2], |r| [r[0] as f32, r[1] as f32])
}
/// `TagValueInput.getBooleanOr`: a numeric tag's byteValue is nonzero, with a false default.
pub fn flag(tag: &Tag, key: &str) -> bool {
    flag_or(tag, key, false)
}
/// `getBooleanOr(key, default)`: any numeric tag's nonzero test, else `default`.
pub fn flag_or(tag: &Tag, key: &str, default: bool) -> bool {
    // NBT floats floor before int narrowing; the final byte keeps the low bits.
    tag.get(key)
        .and_then(int_value)
        .map_or(default, |value| value as i8 != 0)
}

/// `Entity.saveWithoutId`: write the movement flags and the flags retained
/// from `Entity.load` as booleans. Only OnGround and Invulnerable write false.
pub fn write_saved_flags(tag: &mut Tag, on_ground: bool, no_gravity: bool) {
    let invulnerable = flag(tag, "Invulnerable");
    let optional = [
        ("NoGravity", no_gravity),
        ("CustomNameVisible", flag(tag, "CustomNameVisible")),
        ("Silent", flag(tag, "Silent")),
        ("Glowing", flag(tag, "Glowing")),
        ("HasVisualFire", flag(tag, "HasVisualFire")),
    ];
    let Tag::Compound(map) = tag else {
        return;
    };
    map.insert("OnGround".into(), Tag::Byte(i8::from(on_ground)));
    map.insert("Invulnerable".into(), Tag::Byte(i8::from(invulnerable)));
    for (key, value) in optional {
        let key = key.into();
        if value {
            map.insert(key, Tag::Byte(1));
        } else {
            map.remove(&key);
        }
    }
}

/// One entity save in progress. The loaded compound supplies still-unmodelled
/// values, but is not the output's map: its insertion and resize history is
/// not `Entity.saveWithoutId`'s. Each writer consumes the fields it owns.
pub struct EntityTagWriter {
    saved: Compound,
    fields: Compound,
}

impl EntityTagWriter {
    pub fn new(raw: Option<&Tag>) -> Self {
        Self::from_saved(raw.cloned().unwrap_or(Tag::End))
    }

    pub fn from_saved(raw: Tag) -> Self {
        Self {
            saved: match raw {
                Tag::Compound(fields) => fields,
                _ => Compound::new(),
            },
            fields: Compound::new(),
        }
    }

    pub fn insert(&mut self, key: JavaString, value: Tag) -> Option<Tag> {
        let saved = self.saved.remove(&key);
        self.fields.insert(key, value).or(saved)
    }

    pub fn get(&self, key: &JavaString) -> Option<&Tag> {
        self.fields.get(key).or_else(|| self.saved.get(key))
    }

    pub fn remove(&mut self, key: &JavaString) -> Option<Tag> {
        let saved = self.saved.remove(key);
        self.fields.remove(key).or(saved)
    }

    pub fn entry(&mut self, key: JavaString) -> EntityField<'_> {
        EntityField { writer: self, key }
    }

    pub fn finish(mut self) -> Tag {
        // Recursive persistence appends the live riders after this writer.
        let passengers = self.saved.remove(&"Passengers".into());
        self.fields.extend(self.saved);
        if let Some(passengers) = passengers {
            self.fields.insert("Passengers".into(), passengers);
        }
        Tag::Compound(self.fields)
    }

    pub fn retain(&mut self, key: &str) {
        let key = key.into();
        if let Some(value) = self.saved.remove(&key) {
            self.fields.insert(key, value);
        }
    }

    fn retain_or(&mut self, key: &str, default: impl FnOnce() -> Tag) {
        let key = key.into();
        let value = self.saved.remove(&key).unwrap_or_else(default);
        self.fields.insert(key, value);
    }

    fn flag(&mut self, key: &str, required: bool) {
        let key = key.into();
        let on = self
            .saved
            .remove(&key)
            .as_ref()
            .and_then(int_value)
            .is_some_and(|value| value as i8 != 0);
        if on || required {
            self.fields.insert(key, Tag::Byte(i8::from(on)));
        }
    }
}

pub struct EntityField<'a> {
    writer: &'a mut EntityTagWriter,
    key: JavaString,
}

impl<'a> EntityField<'a> {
    pub fn or_insert(self, value: Tag) -> &'a mut Tag {
        self.or_insert_with(|| value)
    }

    pub fn or_insert_with(self, make: impl FnOnce() -> Tag) -> &'a mut Tag {
        let saved = self.writer.saved.remove(&self.key);
        self.writer
            .fields
            .entry(self.key)
            .or_insert_with(|| saved.unwrap_or_else(make))
    }
}

/// Live movement fields written by `Entity.saveWithoutId`.
pub struct Base<'a> {
    pub id: &'a str,
    pub uuid: [i32; 4],
    pub position: Vec3,
    pub motion: Vec3,
    pub rotation: [f32; 2],
    pub on_ground: bool,
    pub fall_distance: f64,
}
impl Base<'_> {
    pub fn write(&self, out: &mut EntityTagWriter, include_id: bool) {
        if include_id {
            // Entity.saveAsPassenger writes the type before saveWithoutId.
            out.insert("id".into(), string(self.id));
        } else {
            out.remove(&"id".into());
        }
        out.insert(
            "Pos".into(),
            doubles(&[self.position.x, self.position.y, self.position.z]),
        );
        out.insert(
            "Motion".into(),
            doubles(&[self.motion.x, self.motion.y, self.motion.z]),
        );
        out.insert("Rotation".into(), floats(&self.rotation));
        out.insert(
            "fall_distance".into(),
            Tag::Double(self.fall_distance.to_bits()),
        );
        out.retain_or("Fire", || Tag::Short(0));
        out.retain_or("Air", || Tag::Short(300));
        out.insert("OnGround".into(), Tag::Byte(i8::from(self.on_ground)));
        out.flag("Invulnerable", true);
        out.retain_or("PortalCooldown", || Tag::Int(0));
        out.retain("invulnerable_time");
        out.insert("UUID".into(), Tag::IntArray(self.uuid.to_vec()));
        out.retain("CustomName");
        out.flag("CustomNameVisible", false);
        out.flag("Silent", false);
        out.flag("NoGravity", false);
        out.flag("Glowing", false);
        out.retain("TicksFrozen");
        out.flag("HasVisualFire", false);
        if matches!(out.saved.get(&"Tags".into()), Some(Tag::List(tags)) if tags.is_empty()) {
            out.remove(&"Tags".into());
        }
        out.retain("Tags");
        out.retain("data");
    }

    fn tag(&self, raw: Option<&Tag>) -> Tag {
        let mut out = EntityTagWriter::new(raw);
        self.write(&mut out, true);
        out.finish()
    }

    /// Preserve Entity data without flags owned by Mob's save writer.
    fn non_mob_tag(&self, raw: Option<&Tag>) -> Tag {
        let mut out = EntityTagWriter::new(raw);
        self.write(&mut out, true);
        out.remove(&"NoAI".into());
        out.remove(&"PersistenceRequired".into());
        out.finish()
    }
}

/// `Entity.saveWithoutId`'s common fields for entities Sunrise keeps as raw
/// compounds: each one is inserted only when the loaded compound has no value
/// for it, so explicit values survive.
pub fn fill_common_defaults(tag: &mut Tag) {
    let Tag::Compound(map) = tag else {
        return;
    };
    for (key, value) in [
        ("Motion", doubles(&[0.0, 0.0, 0.0])),
        ("OnGround", Tag::Byte(0)),
        ("fall_distance", Tag::Double(0.0f64.to_bits())),
        ("Air", Tag::Short(300)),
        ("Invulnerable", Tag::Byte(0)),
        ("PortalCooldown", Tag::Int(0)),
    ] {
        map.entry(key.into()).or_insert(value);
    }
}

/// The mobs that save `Age` and `AgeLocked`.
fn saves_age(s: crate::mobs::Species) -> bool {
    use crate::mobs::{Species, finalize::ageable, species::can_be_a_baby};
    s == Species::Tadpole || (ageable(s) && can_be_a_baby(s))
}
/// `LivingEntity` + `Mob` (+ `AgeableMob`, creeper and skeleton specifics).
/// The mobs whose class extends vanilla's `Animal` (and so save `InLove`):
/// not allays, villagers, golems, dolphins or fish.
fn is_animal(id: &str) -> bool {
    matches!(
        id.strip_prefix("minecraft:").unwrap_or(id),
        "pig"
            | "cow"
            | "mooshroom"
            | "sheep"
            | "chicken"
            | "rabbit"
            | "horse"
            | "donkey"
            | "mule"
            | "zombie_horse"
            | "skeleton_horse"
            | "llama"
            | "trader_llama"
            | "goat"
            | "panda"
            | "polar_bear"
            | "fox"
            | "wolf"
            | "cat"
            | "ocelot"
            | "parrot"
            | "bee"
            | "turtle"
            | "armadillo"
            | "frog"
            | "axolotl"
            | "hoglin"
            | "strider"
            | "camel"
            | "camel_husk"
            | "sniffer"
            | "happy_ghast"
            | "nautilus"
            | "zombie_nautilus"
    )
}
use crate::mobs::persistent_anger;
/// The single player's UUID (`offline_uuid("Player")`): the entity the
/// neutral mobs are angry at.
pub(crate) fn player_uuid() -> [i32; 4] {
    let u = sunrise_protocol::crypto::offline_uuid("Player");
    [
        (u >> 96) as u32 as i32,
        (u >> 64) as u32 as i32,
        (u >> 32) as u32 as i32,
        u as u32 as i32,
    ]
}
/// The UUID a save writes for a reference to `who`. A mob is only an id here:
/// its UUID lives in the game layer (`EntityMemory.uuids`), which is what
/// `uuid_of` answers from. When it answers `None` (the mob is no longer in the
/// level) nothing is written, rather than a made-up UUID that would name nothing
/// when the save is read back. The reference comes back with [`who_by_uuid`].
///
/// `mob_to_nbt`'s `uuid_of` is this for a field of a mob's own compound; the
/// game layer's `Game::save_who_uuid` is the same rule for a field written
/// outside one (a bullet's `Target`), answered from `EntityMemory::uuid_of`.
pub fn who_uuid(who: Who, uuid_of: &mut impl FnMut(u32) -> Option<[i32; 4]>) -> Option<[i32; 4]> {
    match who {
        Who::Player => Some(player_uuid()),
        Who::Mob(id) => uuid_of(id),
    }
}
/// Replace every reference to the player `old` in a saved entity with `new`. Older
/// Sunrise worlds named their single player by the offline UUID (`Owner` on tamed
/// animals and horses, a projectile's `Owner` and an item's `Thrower`, a fox's
/// `Trusted`, `angry_at`, a leash's `UUID`); when such a world opens under the
/// account (`lan::playerdata::migrate_owner`), a chunk's entities are rewritten as
/// they load, and their next save keeps the account's UUID. A reference is the
/// player's four ints, wherever the entity keeps it; an entity's own `UUID` never
/// names the player.
pub fn migrate_player_uuid(tag: &mut Tag, old: [i32; 4], new: [i32; 4]) {
    if old == new {
        return;
    }
    match tag {
        Tag::IntArray(uuid) if uuid.as_slice() == old.as_slice() => uuid.copy_from_slice(&new),
        Tag::List(list) => {
            for tag in list {
                migrate_player_uuid(tag, old, new);
            }
        }
        Tag::Compound(map) => {
            map.for_each_mut(|_, tag| {
                migrate_player_uuid(tag, old, new);
            });
        }
        _ => {}
    }
}
/// `AngerManagement.SUSPECT_ANGER_PAIR`: one suspect's UUID and how angry the
/// warden is at it.
fn suspect_anger(uuid: [i32; 4], anger: i32) -> Tag {
    Tag::compound([
        ("uuid", Tag::IntArray(uuid.to_vec())),
        ("anger", Tag::Int(anger)),
    ])
}
/// `AngerManagement.codec`: `SUSPECT_ANGER_PAIR.listOf()` under `suspects`, in
/// the order `createUuidAngerPairs` builds it - the suspects that are in the
/// level, by the UUID each entity has, then the ones that are not, which are
/// already UUIDs. Nothing else is stored: the highest anger, the conversion
/// delay and the `NeutralMob` anger times are runtime only. An anger with no
/// suspect at all writes no `suspects` key, `lenientOptionalFieldOf`'s default.
fn warden_anger_to_nbt(
    anger: &crate::mobs::warden::AngerManagement,
    uuid_of: &mut impl FnMut(u32) -> Option<[i32; 4]>,
) -> Tag {
    let loaded = anger.suspects().iter().filter_map(|who| {
        // A player suspect by the UUID its anger recorded (#486).
        let uuid = match who {
            Who::Player => anger.player_uuid()?,
            Who::Mob(_) => who_uuid(*who, uuid_of)?,
        };
        Some(suspect_anger(uuid, anger.anger_of(*who)))
    });
    let away = anger
        .unloaded()
        .map(|(uuid, anger)| suspect_anger(uuid, anger));
    let pairs: Vec<Tag> = loaded.chain(away).collect();
    if pairs.is_empty() {
        Tag::compound(Vec::<(&str, Tag)>::new())
    } else {
        Tag::compound([("suspects", Tag::List(pairs))])
    }
}
/// [`warden_anger_to_nbt`] read back: every pair goes into the UUID map, where
/// the anger ticks resolve the ones whose entity is in the level again. A pair
/// with no four-int UUID or a negative anger drops the whole list, as
/// `lenientOptionalFieldOf` does with a value that does not decode.
fn warden_anger_from_nbt(tag: &Tag) -> Vec<([i32; 4], i32)> {
    let Some(Tag::List(pairs)) = tag.get("suspects") else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let (Some(uuid), Some(anger)) =
            (uuid_from_tag(pair.get("uuid")), number(pair.get("anger")))
        else {
            return Vec::new();
        };
        let Ok(anger) = i32::try_from(anger) else {
            return Vec::new();
        };
        if anger < 0 {
            return Vec::new();
        }
        out.push((uuid, anger));
    }
    out
}
/// `ResolvableProfile.CODEC`: the name, id and properties of a profile, then the
/// skin's texture ids and model (`PlayerSkin.Patch`: "texture", "cape", "elytra", "model").
fn profile_nbt(p: &sunrise_protocol::entity_data::Profile) -> Tag {
    let mut entries: Vec<(&'static str, Tag)> = Vec::new();
    if let Some(name) = &p.name {
        entries.push(("name", string(name)));
    }
    if let Some(id) = p.uuid {
        entries.push((
            "id",
            Tag::IntArray(vec![
                (id >> 96) as i32,
                (id >> 64) as i32,
                (id >> 32) as i32,
                id as i32,
            ]),
        ));
    }
    if !p.properties.is_empty() {
        let properties = p
            .properties
            .iter()
            .map(|property| {
                let mut entries: Vec<(&'static str, Tag)> = vec![
                    ("name", string(&property.name)),
                    ("value", string(&property.value)),
                ];
                if let Some(signature) = &property.signature {
                    entries.push(("signature", string(signature)));
                }
                Tag::compound(entries)
            })
            .collect();
        entries.push(("properties", Tag::List(properties)));
    }
    for (key, texture) in [
        ("texture", &p.body),
        ("cape", &p.cape),
        ("elytra", &p.elytra),
    ] {
        if let Some(texture) = texture {
            entries.push((key, string(texture)));
        }
    }
    if let Some(slim) = p.slim {
        entries.push(("model", string(if slim { "slim" } else { "wide" })));
    }
    Tag::compound(entries)
}
/// `ResolvableProfile.CODEC` read back: the compound `profile_nbt` writes, or a
/// bare player name (`Codec.withAlternative(FULL_CODEC, PLAYER_NAME, ...)`).
fn profile_from_nbt(tag: &Tag) -> Option<sunrise_protocol::entity_data::Profile> {
    if let Tag::String(name) = tag {
        return Some(sunrise_protocol::entity_data::Profile {
            name: Some(name.to_string_lossy()),
            uuid: None,
            properties: Vec::new(),
            body: None,
            cape: None,
            elytra: None,
            slim: None,
        });
    }
    let property = |name: String, value: &Tag, signature: Option<String>| match value {
        Tag::String(v) => Some(sunrise_core::profile::AccountProperty {
            name,
            value: v.to_string_lossy(),
            signature,
        }),
        _ => None,
    };
    let properties: Vec<sunrise_core::profile::AccountProperty> = match tag.get("properties") {
        // `ExtraCodecs.PROPERTY_MAP`: a list of properties ...
        Some(Tag::List(list)) => list
            .iter()
            .filter_map(|p| {
                let name = match p.get("name")? {
                    Tag::String(s) => s.to_string_lossy(),
                    _ => return None,
                };
                let signature = match p.get("signature") {
                    Some(Tag::String(s)) => Some(s.to_string_lossy()),
                    _ => None,
                };
                property(name, p.get("value")?, signature)
            })
            .collect(),
        // ... or a map from a name to its values.
        Some(Tag::Compound(map)) => map
            .iter()
            .flat_map(|(name, values)| match values {
                Tag::List(values) => values
                    .iter()
                    .filter_map(|v| property(name.to_string_lossy(), v, None))
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect(),
        _ => Vec::new(),
    };
    let text = |key: &str| match tag.get(key) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    };
    Some(sunrise_protocol::entity_data::Profile {
        name: text("name"),
        uuid: uuid_from_tag(tag.get("id")).map(|a| {
            a.iter()
                .fold(0u128, |u, &x| (u << 32) | u128::from(x as u32))
        }),
        properties,
        body: text("texture"),
        cape: text("cape"),
        elytra: text("elytra"),
        slim: text("model").map(|model| model == "slim"),
    })
}
/// `ConversionTracker.addAdditionalSaveData`: vanilla's conversion time, -1
/// while the mob is not converting. `counter2` is that time plus one while it
/// converts and 0 when it does not (see `mobs/undead.rs`).
fn conversion_time(m: &Mob) -> i32 {
    if m.extra.counter2 > 0 {
        m.extra.counter2 - 1
    } else {
        -1
    }
}
/// `BlockState.CODEC`: the block's name when the state is that block's
/// default, `{id, properties}` for any other state.
pub fn state_to_nbt(registry: &BlockRegistry, state: BlockStateId) -> Tag {
    match registry.state(state) {
        Some(s) if registry.default_state(&s.name) == Some(state) => string(&s.name),
        Some(s) => s.to_nbt(),
        None => string("minecraft:air"),
    }
}
/// `BlockState.CODEC` read back: a block name or `{id, properties}` (or the
/// legacy `{Name, Properties}`). A name the registry knows only as a block
/// reads as its default state, and one it does not know at all as `None`.
pub fn state_from_nbt(registry: &BlockRegistry, tag: &Tag) -> Option<BlockStateId> {
    let state = match tag.get("Name") {
        Some(Tag::String(name)) => BlockState {
            name: name.to_string_lossy(),
            properties: match tag.get("Properties") {
                Some(Tag::Compound(p)) => p
                    .iter()
                    .filter_map(|(k, v)| match v {
                        Tag::String(v) => Some((k.to_string_lossy(), v.to_string_lossy())),
                        _ => None,
                    })
                    .collect(),
                _ => Default::default(),
            },
        },
        _ => BlockState::from_nbt(tag).ok()?,
    };
    registry
        .id(&state)
        .or_else(|| registry.default_state(&state.name))
}
/// `BlockState.isAir`: air, cave air and void air.
fn is_air(registry: &BlockRegistry, state: BlockStateId) -> bool {
    registry.state(state).is_some_and(|s| {
        matches!(
            s.name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        )
    })
}
/// The mob's entity compound. `registry` is what writes a field holding a block
/// state (an enderman's `carriedBlockState`): without one, a caller that has no
/// resources (unit tests, structure templates) keeps whatever the loaded
/// compound had for it.
///
/// `uuid_of` is how a field that names another entity by UUID (a mob's
/// `angry_at`) is written: it answers the UUID of a mob by its id, from the game
/// layer's map, and `None` for one the level no longer has. A caller with no
/// such map passes `|_| None` and writes no reference.
pub fn mob_to_nbt(
    m: &Mob,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    registry: Option<&BlockRegistry>,
    uuid_of: impl FnMut(u32) -> Option<[i32; 4]>,
    items: &crate::items::Items,
) -> Tag {
    let mut retained = match raw {
        Some(tag @ Tag::Compound(_)) => tag.clone(),
        _ => Tag::Compound(Compound::new()),
    };
    ai_pause::write_ai_paused(m.ai.ai_paused, &mut retained);
    let mut out = EntityTagWriter::from_saved(retained);
    write_mob(
        m,
        uuid,
        &mut out,
        registry,
        uuid_of,
        items,
        MobSave::default(),
    );
    out.finish()
}

/// Runtime state shared by chunk saving and inspecting a live mob.
pub struct MobSave {
    pub include_id: bool,
    pub game_time: i64,
}

impl Default for MobSave {
    fn default() -> Self {
        Self {
            include_id: true,
            game_time: 0,
        }
    }
}

/// The world supplies its live leash and hurt-by projection before writing.
pub fn write_mob(
    m: &Mob,
    uuid: [i32; 4],
    map: &mut EntityTagWriter,
    registry: Option<&BlockRegistry>,
    mut uuid_of: impl FnMut(u32) -> Option<[i32; 4]>,
    items: &crate::items::Items,
    mode: MobSave,
) {
    let MobSave {
        include_id,
        game_time,
    } = mode;
    use crate::mobs::Species as S;
    let base = Base {
        id: &m.kind.id,
        uuid,
        position: m.position,
        motion: m.velocity,
        rotation: [m.yaw, m.head_pitch],
        on_ground: m.ground.on_ground,
        fall_distance: m.fall_distance,
    };
    // `Entity.saveWithoutId` consumes these live values in its own order.
    map.saved.insert(
        "Fire".into(),
        Tag::Short(m.fire_ticks.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16),
    );
    map.saved.insert("Air".into(), Tag::Short(m.air as i16));
    map.saved.insert(
        "Invulnerable".into(),
        Tag::Byte(i8::from(m.invulnerable_flag)),
    );
    for (key, on) in [
        ("CustomNameVisible", m.name_visible),
        ("Silent", m.silent),
        ("NoGravity", m.no_gravity),
        ("Glowing", m.glowing),
    ] {
        if on {
            map.saved.insert(key.into(), Tag::Byte(1));
        } else {
            map.saved.remove(&key.into());
        }
    }
    if m.custom_name_changed(map.saved.get(&"CustomName".into()))
        && let Some(name) = m.custom_name_tag()
    {
        map.saved.insert("CustomName".into(), name);
    }
    for (key, time) in [
        ("invulnerable_time", m.invulnerable_time),
        ("TicksFrozen", m.ticks_frozen),
    ] {
        if time > 0 {
            map.saved.insert(key.into(), Tag::Int(time));
        } else {
            map.saved.remove(&key.into());
        }
    }
    base.write(map, include_id);

    // LivingEntity.addAdditionalSaveData.
    mob_death::write(m, map);
    map.insert(
        "AbsorptionAmount".into(),
        Tag::Float(m.absorption.to_bits()),
    );
    map.insert(
        "current_impulse_context_reset_grace_time".into(),
        Tag::Int(m.impulse_grace_time),
    );
    map.retain("current_explosion_impact_pos");
    if matches!(
        m.kind.class_id(),
        "minecraft:horse"
            | "minecraft:donkey"
            | "minecraft:mule"
            | "minecraft:skeleton_horse"
            | "minecraft:zombie_horse"
            | "minecraft:llama"
            | "minecraft:trader_llama"
    ) {
        // AttributeMap.pack writes the instances that exist: the ones the
        // entity was loaded with. When the tag had no attributes list,
        // instances are not synthesized (SummonCommand without finalizeSpawn
        // saves attributes: []).
        if let Some(Tag::List(list)) = map.get(&"attributes".into()) {
            let modelled = [
                ("minecraft:max_health", f64::from(m.attrs.max_health)),
                ("minecraft:movement_speed", m.attrs.speed),
                ("minecraft:jump_strength", m.extra.vector[0]),
            ];
            let attributes = list
                .iter()
                .map(|a| {
                    let id = text(a.get("id"));
                    let value = modelled.iter().find(|(m, _)| Some(*m) == id.as_deref());
                    match (a, value) {
                        (Tag::Compound(c), Some((_, v))) => {
                            let mut c = c.clone();
                            c.insert("base".into(), Tag::Double(v.to_bits()));
                            Tag::Compound(c)
                        }
                        _ => a.clone(),
                    }
                })
                .collect();
            map.insert("attributes".into(), Tag::List(attributes));
        }
    }
    if m.extra.attributes.is_some() {
        put_attributes(map, m);
    }
    put_sulfur_attributes(map, m);
    // The instances their own construction and load make (finalize::made_at_load),
    // which AttributeMap.pack saves although the loaded tag had none.
    use crate::mobs::finalize::Attr;
    let made = crate::mobs::finalize::made_at_load(m);
    // AttributeMap.pack writes the list even when no instance exists.
    map.entry("attributes".into())
        .or_insert_with(|| Tag::List(Vec::new()));
    for attr in made {
        let base = crate::mobs::finalize::made_base(m, attr);
        let mut list = match map.get(&"attributes".into()) {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        let attr_id = format!("minecraft:{}", attr.name());
        if let Some(existing) = list
            .iter_mut()
            .find(|t| text(t.get("id")).as_deref() == Some(attr_id.as_str()))
        {
            // The model owns a wolf's health base.
            if (m.species(), attr) == (S::Wolf, Attr::MaxHealth)
                && let Tag::Compound(c) = existing
            {
                c.insert("base".into(), Tag::Double(base.to_bits()));
            }
        } else {
            list.push(Tag::compound([
                ("id", string(&attr_id)),
                ("base", Tag::Double(base.to_bits())),
            ]));
        }
        map.insert("attributes".into(), Tag::List(list));
    }
    put_live_attributes(&mut map.fields, &m.extra.live);
    // LivingEntity.addAdditionalSaveData: `active_effects` only when there are
    // any (MobEffectInstance.CODEC), including their modeled weaker instances.
    if m.extra.effects.is_empty() {
        map.remove(&sunrise_core::nbt::JavaString::from("active_effects"));
    } else {
        let effects = m.extra.effects.iter().map(|effect| {
            let show_icon = effect.show_icon;
            let (effect, duration, amplifier) = (effect.id, effect.duration, effect.amplifier);
            let shown = !m.extra.hidden_effects.contains(&effect);
            let ambient = m.extra.ambient_effects.contains(&effect);
            let mut e = vec![("id", string(crate::brewing::namespaced(effect)))];
            // The codec leaves out an amplifier of 0 (its default).
            if amplifier != 0 {
                e.push(("amplifier", Tag::Byte(amplifier.clamp(0, 255) as u8 as i8)));
            }
            e.push(("duration", Tag::Int(duration)));
            if ambient {
                e.push(("ambient", Tag::Byte(1)));
            }
            if !shown {
                e.push(("show_particles", Tag::Byte(0)));
            }
            e.push(("show_icon", Tag::Byte(i8::from(show_icon))));
            if let Some(hidden) = m.extra.effect_chains.iter().find(|e| e.id == effect) {
                e.push(("hidden_effect", hidden_effect_nbt(hidden)));
            }
            Tag::compound(e)
        });
        map.insert("active_effects".into(), Tag::List(effects.collect()));
    }
    map.insert(
        "FallFlying".into(),
        Tag::Byte(i8::from(m.extra.fall_flying)),
    );
    if m.species() == S::Villager {
        if let Some(pos) = crate::mobs::villager::sleeping_pos(m) {
            map.insert("sleeping_pos".into(), Tag::IntArray(pos.to_vec()));
        } else {
            map.remove(&"sleeping_pos".into());
        }
    } else {
        map.retain("sleeping_pos");
    }
    map.retain_or("Brain", || {
        Tag::compound([("memories", Tag::Compound(Compound::new()))])
    });
    map.retain("last_hurt_by_player");
    map.retain("last_hurt_by_player_memory_time");
    map.retain("last_hurt_by_mob");
    map.retain("ticks_since_last_hurt_by_mob");
    // LivingEntity equipment: what the mob holds and wears now.
    {
        let mut equipment = m.extra.equipment.clone();
        let head = sunrise_items::equipment::HEAD;
        // Held and worn items are saved with their enchantments and trim.
        for slot in sunrise_items::equipment::MAINHAND..=head {
            equipment[slot] = crate::mobs::finalize::item_stack(m, slot);
        }
        // The captain's ominous banner keeps its patterns.
        if equipment[head].as_ref().is_some_and(|banner| {
            &*banner.item == crate::mobs::raider::BANNER
                && banner.extra.is_none()
                && crate::mobs::raider::is_raider(m.species())
        }) {
            equipment[head] = Some(crate::mobs::raider::ominous_banner());
        }
        if let Some(equipment) = equipment.write(items) {
            map.insert("equipment".into(), equipment);
        } else {
            map.remove(&"equipment".into());
        }
    }
    map.retain("locator_bar_icon");
    if m.species() != S::Mannequin {
        // Mob.addAdditionalSaveData.
        map.insert(
            "CanPickUpLoot".into(),
            Tag::Byte(i8::from(m.can_pick_up_loot())),
        );
        map.insert(
            "PersistenceRequired".into(),
            Tag::Byte(i8::from(m.persistent)),
        );
        // Mob.addAdditionalSaveData: the chances that are not the default.
        if let Some(chances) = &m.extra.drop_chances {
            let set: Vec<(&str, Tag)> = crate::mobs::drops::SLOTS
                .iter()
                .zip(chances.iter())
                .filter(|(_, c)| **c != crate::mobs::drops::DEFAULT_DROP_CHANCE)
                .map(|(slot, c)| (*slot, Tag::Float(c.to_bits())))
                .collect();
            if set.is_empty() {
                map.remove(&"drop_chances".into());
            } else {
                map.insert("drop_chances".into(), Tag::compound(set));
            }
        }
        map.retain("leash");
        // Mob.addAdditionalSaveData: hasHome means radius != -1, including -2.
        // Species writers below retain their own home_pos override.
        if let Some((pos, radius)) = m.ai.home.filter(|(_, radius)| *radius != -1) {
            map.insert("home_radius".into(), Tag::Int(radius));
            map.insert("home_pos".into(), Tag::IntArray(pos.to_vec()));
        } else {
            map.remove(&"home_radius".into());
            map.remove(&"home_pos".into());
        }
        map.insert(
            "LeftHanded".into(),
            Tag::Byte(i8::from(m.extra.left_handed)),
        );
        // Mob.addAdditionalSaveData: the death loot table, and its seed when set.
        if let Some(table) = &m.extra.death_loot_table {
            map.insert(("DeathLootTable").into(), string(table));
        }
        if m.extra.death_table_seed != 0 {
            map.insert(
                ("DeathLootTableSeed").into(),
                Tag::Long(m.extra.death_table_seed),
            );
        }
        if m.no_ai {
            map.insert("NoAI".into(), Tag::Byte(1));
        } else {
            map.remove(&"NoAI".into());
        }
    } else {
        // Mannequin is an Avatar, not a Mob; discard loaded Mob-only data.
        for key in MOB_ONLY_KEYS {
            map.remove(&key.into());
        }
    }
    // Keys of older formats that structure templates still carry.
    for key in [
        "HandItems",
        "ArmorItems",
        "HandDropChances",
        "ArmorDropChances",
        "Dimension",
        "HurtByTimestamp",
        "Leashed",
    ] {
        map.remove(&sunrise_core::nbt::JavaString::from(key));
    }
    // `Mannequin.addAdditionalSaveData`: the profile whose skin it wears, whether it
    // is `immovable`, and its description (the default is not written; none is `hide_description`).
    if m.species() == crate::mobs::Species::Mannequin {
        // The empty static profile (`DEFAULT_PROFILE`) when it has none.
        let profile = m
            .profile
            .as_ref()
            .map_or_else(|| Tag::Compound(Compound::new()), profile_nbt);
        map.insert(("profile").into(), profile);
        // `LAYERS_CODEC`: the hidden layers, in `PlayerModelPart` order.
        let hidden = MODEL_PARTS
            .iter()
            .enumerate()
            .filter(|(bit, _)| m.extra.hidden_model_parts & (1 << bit) != 0)
            .map(|(_, part)| string(part))
            .collect();
        map.insert(("hidden_layers").into(), Tag::List(hidden));
        let hand = if m.extra.main_hand_left {
            "left"
        } else {
            "right"
        };
        map.insert(("main_hand").into(), string(hand));
        let pose = match m.extra.avatar_pose {
            crate::pose::Pose::Crouching => "crouching",
            crate::pose::Pose::Swimming => "swimming",
            crate::pose::Pose::FallFlying => "fall_flying",
            crate::pose::Pose::Sleeping => "sleeping",
            _ => "standing",
        };
        map.insert(("pose").into(), string(pose));
        map.insert(("immovable").into(), Tag::Byte(i8::from(m.extra.immovable)));
        match &m.extra.description {
            crate::mobs::MannequinDescription::Default => {}
            crate::mobs::MannequinDescription::Hidden => {
                map.insert(("hide_description").into(), Tag::Byte(1));
            }
            crate::mobs::MannequinDescription::Custom(component) => {
                map.insert(("description").into(), component.clone());
            }
        }
    }
    // AbstractFish.addAdditionalSaveData always writes the flag.
    // Tadpole.fromBucket is always true, and its setter ignores loaded input.
    let from_bucket = match m.species() {
        S::Tadpole => Some(true),
        S::Cod | S::Salmon | S::Pufferfish | S::TropicalFish => Some(m.extra.from_bucket),
        _ => None,
    };
    if let Some(value) = from_bucket {
        map.insert(("FromBucket").into(), Tag::Byte(i8::from(value)));
    }
    // What the mob's class saves: a content mod's living type saves as its
    // template does (the `id` written above stays its own).
    let id = m.kind.class_id();
    // AgeableMob.addAdditionalSaveData, only if canBeABaby (not a frog or a
    // parrot); a tadpole saves its own age.
    let species = m.species();
    if saves_age(species) {
        map.insert("Age".into(), Tag::Int(m.extra.age));
        if species != S::Tadpole {
            map.insert("ForcedAge".into(), Tag::Int(m.extra.forced_age));
        }
        map.insert("AgeLocked".into(), Tag::Byte(i8::from(m.extra.age_locked)));
    } else if crate::mobs::finalize::ageable(species) {
        // AgeableMob.addAdditionalSaveData writes nothing when canBeABaby is
        // false — the frog, the parrot, the zombie nautilus, the camel husk,
        // the slime and the magma cube: drop the ignored values a raw input
        // compound carried rather than writing them back out.
        for key in ["Age", "ForcedAge", "AgeLocked"] {
            map.remove(&key.into());
        }
    }
    // Animal.addAdditionalSaveData: the love ticks left, whatever the save had.
    if is_animal(id) {
        map.insert("InLove".into(), Tag::Int(m.extra.in_love));
        // Animal.addAdditionalSaveData: the player who fed it.
        if let Some(cause) = m.extra.love_cause {
            map.insert("LoveCause".into(), Tag::IntArray(cause.to_vec()));
        } else {
            map.remove(&"LoveCause".into());
        }
    }
    // Zombie.addAdditionalSaveData precedes every Zombie subclass.
    if crate::mobs::doors::is_zombie(species) {
        map.insert("IsBaby".into(), Tag::Byte(i8::from(m.is_baby())));
        map.insert(
            "CanBreakDoors".into(),
            Tag::Byte(i8::from(m.extra.can_break_doors)),
        );
        // ConversionTracker writes the conversion time before the affliction time.
        if matches!(species, S::Zombie | S::Husk) {
            map.insert("DrownedConversionTime".into(), Tag::Int(conversion_time(m)));
        } else {
            map.entry("DrownedConversionTime".into())
                .or_insert(Tag::Int(-1));
        }
        if !m.fluids.eye_in_water {
            map.insert("InWaterTime".into(), Tag::Int(-1));
        } else if matches!(species, S::Zombie | S::Husk) {
            map.insert("InWaterTime".into(), Tag::Int(m.extra.counter));
        } else {
            // Not counting for this kind: the drowning tracker writes its time as loaded, 0 by default.
            map.retain_or("InWaterTime", || Tag::Int(0));
        }
    }
    // PatrollingMonster and Raider write before their subclasses.
    crate::mobs::raider::write_nbt(m, map);
    if matches!(species, S::Villager | S::WanderingTrader) {
        // AbstractVillager writes offers before its inventory.
        put_offers(map, m, items);
    }
    // InventoryCarrier.writeInventoryToTag: the occupied stacks, even when empty.
    if matches!(id, "minecraft:pillager" | "minecraft:wandering_trader") {
        map.insert(
            "Inventory".into(),
            Tag::List(
                m.extra
                    .inventory
                    .iter()
                    .map(|stack| crate::save::item_to_nbt(stack, items))
                    .collect(),
            ),
        );
    }
    // TamableAnimal precedes Cat, Wolf and Parrot.
    if matches!(species, S::Cat | S::Wolf | S::Parrot) {
        if let Some(owner) = m.extra.owner {
            map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
        } else {
            map.remove(&"Owner".into());
        }
        map.insert("Sitting".into(), Tag::Byte(i8::from(m.extra.sitting)));
    }
    if species == S::Wolf {
        map.insert("CollarColor".into(), Tag::Byte(m.extra.color as i8));
    }
    if species == S::Chicken {
        crate::mobs::riders::write_nbt(m, map);
        map.insert("EggLayTime".into(), Tag::Int(m.extra.egg_time));
    }
    // `VariantUtils.writeVariant`: only the species that store the name. A
    // mooshroom saves `Type` (below), a fox `Type`, a horse and a llama a
    // packed `Variant` int, so those report the component but save no name.
    // ZombieNautilus writes its variant after the TamableAnimal fields below.
    if species != S::ZombieNautilus
        && crate::mobs::finalize::saves_variant_name(species)
        && let Some((_, variant)) =
            crate::mobs::finalize::species_variant_component(species, m.extra.variant)
    {
        map.insert("variant".into(), string(&variant));
    }
    if species == S::Wolf {
        write_persistent_anger(m, map, &mut uuid_of);
    }
    let sounds = crate::mobs::finalize::sound_variants(species);
    if !sounds.is_empty() {
        let sound = m.extra.sound_variant.and_then(|i| sounds.get(i as usize));
        let sound = sound.copied().unwrap_or("classic");
        map.insert(
            "sound_variant".into(),
            string(&format!("minecraft:{sound}")),
        );
    }
    match id {
        // Fox.addAdditionalSaveData.
        "minecraft:fox" => {
            // The trusted UUIDs (`Fox.TRUSTED_LIST_CODEC`: a list of int arrays).
            map.insert(
                "Trusted".into(),
                Tag::List(
                    crate::mobs::fox::trusted_entities(m)
                        .map(|uuid| Tag::IntArray(uuid.to_vec()))
                        .collect(),
                ),
            );
            map.insert(
                "Sleeping".into(),
                Tag::Byte(i8::from(crate::mobs::fox::flag(
                    m,
                    crate::mobs::fox::SLEEPING,
                ))),
            );
            let kind = if m.extra.variant == 1 { "snow" } else { "red" };
            map.insert("Type".into(), string(kind));
            for (key, bit) in [
                ("Sitting", crate::mobs::fox::SITTING),
                ("Crouching", crate::mobs::fox::CROUCHING),
            ] {
                map.insert(
                    key.into(),
                    Tag::Byte(i8::from(crate::mobs::fox::flag(m, bit))),
                );
            }
        }
        // MushroomCow.addAdditionalSaveData: the Variant codec's name and the
        // stew effects, written only while it holds any (storeNullable).
        "minecraft:mooshroom" => {
            let kind = if m.extra.variant == 1 { "brown" } else { "red" };
            map.insert("Type".into(), string(kind));
            match &m.extra.stew_effects {
                Some(effects) => {
                    let list = effects.iter().map(|(id, duration)| {
                        let mut entry = vec![("id", string(id))];
                        // SuspiciousStewEffects.Entry leaves out the default.
                        if *duration != 160 {
                            entry.push(("duration", Tag::Int(*duration)));
                        }
                        Tag::compound(entry)
                    });
                    map.insert("stew_effects".into(), Tag::List(list.collect()));
                }
                None => {
                    map.remove(&sunrise_core::nbt::JavaString::from("stew_effects"));
                }
            }
        }
        // Rabbit.addAdditionalSaveData (the legacy id; 99 is the killer bunny).
        "minecraft:rabbit" => {
            map.insert("RabbitType".into(), Tag::Int(i32::from(m.extra.variant)));
            map.insert(
                "MoreCarrotTicks".into(),
                Tag::Int(m.extra.more_carrot_ticks),
            );
        }
        "minecraft:goat" => crate::mobs::goat::write_nbt(m, map),
        "minecraft:creeper" => {
            map.insert("powered".into(), Tag::Byte(i8::from(m.extra.charged)));
            map.insert("Fuse".into(), Tag::Short(m.max_swell as i16));
            map.insert(
                "ExplosionRadius".into(),
                Tag::Byte(m.extra.explosion_radius),
            );
            map.insert("ignited".into(), Tag::Byte(i8::from(m.extra.flag)));
        }
        // `Ocelot.addAdditionalSaveData`.
        "minecraft:ocelot" => {
            map.insert("Trusting".into(), Tag::Byte(i8::from(m.extra.trusting)));
        }
        "minecraft:cat" => {
            map.insert("CollarColor".into(), Tag::Byte(m.extra.color as i8));
        }
        "minecraft:panda" => {
            use crate::mobs::panda::GENES;
            for (key, gene) in [
                ("MainGene", m.extra.variant),
                ("HiddenGene", m.extra.variant2),
            ] {
                map.insert(
                    key.into(),
                    string(GENES[usize::from(gene).min(GENES.len() - 1)]),
                );
            }
        }
        "minecraft:piglin" | "minecraft:piglin_brute" | "minecraft:hoglin" => {
            map.insert(
                "IsImmuneToZombification".into(),
                Tag::Byte(i8::from(m.extra.flag2)),
            );
            map.insert("TimeInOverworld".into(), Tag::Int(m.extra.counter2));
            if id == "minecraft:hoglin" {
                map.insert(
                    "CannotBeHunted".into(),
                    Tag::Byte(m.extra.cannot_be_hunted as i8),
                );
            }
            if species == S::PiglinBrute {
                map.remove(&"CannotHunt".into());
            } else if species == S::Piglin {
                map.insert("IsBaby".into(), Tag::Byte(i8::from(m.is_baby())));
                map.insert(
                    "CannotHunt".into(),
                    Tag::Byte(m.extra.piglin.cannot_hunt as i8),
                );
                crate::mobs::piglin::write_guarded_anger(m, map);
                // InventoryCarrier.writeInventoryToTag.
                map.insert(
                    "Inventory".into(),
                    Tag::List(
                        m.extra
                            .inventory
                            .iter()
                            .map(|stack| crate::save::item_to_nbt(stack, items))
                            .collect(),
                    ),
                );
            }
        }
        "minecraft:horse"
        | "minecraft:donkey"
        | "minecraft:mule"
        | "minecraft:skeleton_horse"
        | "minecraft:zombie_horse"
        | "minecraft:llama"
        | "minecraft:trader_llama"
        | "minecraft:camel"
        | "minecraft:camel_husk" => {
            // AbstractHorse records these before any subclass fields.
            let camel = matches!(species, S::Camel | S::CamelHusk);
            if camel {
                map.insert("EatingHaystack".into(), Tag::Byte(i8::from(m.extra.flag)));
            } else {
                map.entry("EatingHaystack".into()).or_insert(Tag::Byte(0));
            }
            map.insert("Bred".into(), Tag::Byte(i8::from(m.extra.bred)));
            map.insert("Temper".into(), Tag::Int(m.extra.equine_temper));
            map.insert("Tame".into(), Tag::Byte(i8::from(m.extra.tame)));
            if !camel && let Some(owner) = m.extra.owner {
                map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            } else {
                map.retain("Owner");
            }
            if camel {
                // LastPoseTick is negative while sitting.
                map.insert(
                    "LastPoseTick".into(),
                    Tag::Long(crate::mobs::camel::synced_pose_tick(m, game_time)),
                );
            }
            if id == "minecraft:horse" {
                let loaded = map.get(&"Variant".into()).and_then(int_value);
                let look = (m.extra.variant, m.extra.variant2);
                let v = crate::mobs::equine::saved_variant(loaded, look.0, look.1);
                map.insert("Variant".into(), Tag::Int(v));
            }
            let llama = crate::mobs::llama::is_llama(m.species());
            // Only llamas, donkeys and mules carry chests.
            if llama || matches!(id, "minecraft:donkey" | "minecraft:mule") {
                map.insert("ChestedHorse".into(), Tag::Byte(i8::from(m.extra.chested)));
            }
            if !camel && m.extra.chested && !m.extra.items.is_empty() {
                map.insert(
                    "Items".into(),
                    crate::save::encode_items(&m.extra.items, items),
                );
            } else if !camel {
                map.remove(&"Items".into());
            }
            // Llama and TraderLlama follow the AbstractChestedHorse fields.
            if llama {
                crate::mobs::llama::to_nbt(m, map);
            } else if species == S::SkeletonHorse {
                crate::mobs::riders::write_nbt(m, map);
            }
        }
        "minecraft:bat" => {
            // Bat.addAdditionalSaveData: the whole DATA_ID_FLAGS byte.
            let flags = m.extra.bat_flags | u8::from(m.extra.flag);
            map.insert("BatFlags".into(), Tag::Byte(flags as i8));
        }
        "minecraft:turtle" => {
            if let Some(home) = m.extra.home {
                map.insert("home_pos".into(), Tag::IntArray(home.to_vec()));
            }
            map.insert("has_egg".into(), Tag::Byte(i8::from(m.extra.flag)));
        }
        "minecraft:creaking" => {
            // home_pos: the heart it is bound to.
            if let Some(home) = m.extra.home {
                map.insert("home_pos".into(), Tag::IntArray(home.to_vec()));
            } else {
                map.remove(&"home_pos".into());
            }
        }
        "minecraft:zombie_villager" => {
            map.insert("VillagerData".into(), villager_data(m));
            crate::mobs::zombie_villager::write_nbt(m, map, items);
        }
        "minecraft:villager" => {
            crate::mobs::villager::write_nbt(m, map, items);
            put_memories(map, m);
        }
        // Dolphin.addAdditionalSaveData: the fish it was fed, and how wet it is.
        "minecraft:dolphin" => {
            map.insert("GotFish".into(), Tag::Byte(i8::from(m.extra.flag)));
            map.insert("Moistness".into(), Tag::Int(m.extra.counter));
        }
        "minecraft:wandering_trader" => {
            // WanderingTrader.addAdditionalSaveData: the visit's countdown and
            // the place it is walking to, a BlockPos codec (an int array).
            map.insert("DespawnDelay".into(), Tag::Int(m.extra.ints[5]));
            if let Some(p) = m.extra.point {
                // The trader keeps its target as a position; the tag is a
                // BlockPos, so only the block it is in is saved.
                map.insert(
                    "wander_target".into(),
                    Tag::IntArray(vec![
                        p[0].floor() as i32,
                        p[1].floor() as i32,
                        p[2].floor() as i32,
                    ]),
                );
            }
        }
        "minecraft:axolotl" => {
            write_axolotl_memories(m, map);
            map.insert(
                "Variant".into(),
                Tag::Int(i32::from(m.extra.variant.min(4))),
            );
            map.insert(
                "FromBucket".into(),
                Tag::Byte(i8::from(m.extra.from_bucket)),
            );
        }
        "minecraft:parrot" => {
            map.insert(
                "Variant".into(),
                Tag::Int(i32::from(m.extra.variant.min(4))),
            );
        }
        "minecraft:salmon" => {
            let size = SALMON_SIZES[usize::from(m.extra.variant).min(2)];
            map.insert("type".into(), string(size));
        }
        // Pufferfish.addAdditionalSaveData: the puff state (0 small, 1 mid,
        // 2 full).
        "minecraft:pufferfish" => {
            map.insert("PuffState".into(), Tag::Int(m.extra.size));
        }
        // TropicalFish.addAdditionalSaveData: the packed variant.
        "minecraft:tropical_fish" => {
            let pattern = m.extra.variant.min(11);
            let packed = i32::from(pattern >= 6)
                | i32::from(pattern % 6) << 8
                | i32::from(m.extra.variant2 & 15) << 16
                | i32::from(m.extra.variant3 & 15) << 24;
            map.insert("Variant".into(), Tag::Int(packed));
        }
        "minecraft:allay" => {
            // InventoryCarrier: the fetched stack; the liked player memory.
            let inv = crate::mobs::allay::inventory(m)
                .map(|stack| item_to_nbt(stack, items))
                .into_iter()
                .collect();
            map.insert("Inventory".into(), Tag::List(inv));
            if let Some(st) = &m.extra.allay {
                map.insert("listener".into(), st.vibration.to_nbt());
            }
            map.insert(
                "DuplicationCooldown".into(),
                Tag::Long(i64::from(m.extra.egg_time)),
            );
            let mut memories: Vec<(&str, Tag)> = Vec::new();
            if m.extra.tame {
                // `MemorySlot.visit`: the time to live only when it can expire.
                let mut liked = vec![(
                    "value",
                    Tag::IntArray(m.extra.owner.unwrap_or([0, 0, 0, 1]).to_vec()),
                )];
                if let Some(ttl) = m.extra.allay.as_ref().and_then(|st| st.liked_player_ttl) {
                    liked.push(("ttl", Tag::Long(ttl)));
                }
                memories.push(("minecraft:liked_player", Tag::compound(liked)));
            }
            // GlobalPos.CODEC, and the cooldown while it runs.
            if let Some(liked) = m.extra.allay.as_ref().and_then(|st| st.noteblock) {
                memories.push((
                    "minecraft:liked_noteblock",
                    Tag::compound([(
                        "value",
                        Tag::compound([
                            ("dimension", string(liked.dimension)),
                            ("pos", Tag::IntArray(liked.pos.to_vec())),
                        ]),
                    )]),
                ));
            }
            if m.extra.ints[2] > 0 {
                memories.push((
                    "minecraft:liked_noteblock_cooldown_ticks",
                    Tag::compound([("value", Tag::Int(m.extra.ints[2]))]),
                ));
            }
            // ITEM_PICKUP_COOLDOWN_TICKS (`Codec.INT`), which a throw sets to 60,
            // while the memory is there (its value 0 included).
            if let Some(ticks) = crate::mobs::allay::pickup_cooldown(m) {
                memories.push((
                    "minecraft:item_pickup_cooldown_ticks",
                    Tag::compound([("value", Tag::Int(ticks))]),
                ));
            }
            // Always written, so a forgotten note block does not come back
            // from the compound the allay was loaded with.
            map.insert(
                "Brain".into(),
                Tag::compound([("memories", Tag::compound(memories))]),
            );
        }
        "minecraft:nautilus" | "minecraft:zombie_nautilus" => {
            crate::mobs::nautilus::to_nbt(m, map);
        }
        "minecraft:sulfur_cube" => crate::mobs::sulfur_cube::to_nbt(m, map),
        "minecraft:bee" => crate::mobs::bee::to_nbt(m, map, &mut uuid_of),
        "minecraft:sniffer" => crate::mobs::sniffer::to_nbt(m, map),
        "minecraft:wither" => {
            map.insert("Invul".into(), Tag::Int(m.extra.counter));
        }
        // IronGolem.addAdditionalSaveData.
        "minecraft:iron_golem" => {
            map.insert(
                "PlayerCreated".into(),
                Tag::Byte(i8::from(m.extra.player_created)),
            );
            write_persistent_anger(m, map, &mut uuid_of);
        }
        "minecraft:polar_bear" | "minecraft:zombified_piglin" => {
            write_persistent_anger(m, map, &mut uuid_of);
        }
        // SnowGolem.addAdditionalSaveData: whether it still wears its pumpkin.
        "minecraft:snow_golem" => {
            map.insert("Pumpkin".into(), Tag::Byte(i8::from(!m.extra.sheared)));
        }
        // `Warden.addAdditionalSaveData`: the anger - one `{uuid, anger}` pair
        // per suspect - and the vibration listener, and nothing else. A warden
        // is no `NeutralMob`, so there is no `angry_at` or `anger_end_time`: the
        // entity a suspect is is named by its UUID, not by a live id.
        "minecraft:warden" => {
            if let Some(st) = &m.extra.warden {
                map.insert("anger".into(), warden_anger_to_nbt(&st.anger, &mut uuid_of));
                map.insert("listener".into(), st.vibration.to_nbt());
            }
            write_warden_owned_memories(m, map);
        }
        // Armadillo.addAdditionalSaveData: the time left to the next scute, and
        // the state it is in, which is always written (`idle` when it is not
        // rolled up).
        "minecraft:armadillo" => {
            map.insert(
                "state".into(),
                string(crate::mobs::armadillo::state_name(
                    crate::mobs::armadillo::state(m),
                )),
            );
            map.insert("scute_time".into(), Tag::Int(m.extra.egg_time));
        }
        "minecraft:ravager" => {
            map.insert("AttackTick".into(), Tag::Int(m.extra.counter));
            map.insert("StunTick".into(), Tag::Int(m.extra.counter2));
            map.insert("RoarTick".into(), Tag::Int(m.extra.anim_ticks));
        }
        "minecraft:vex" => crate::mobs::vex::write_nbt(m, map, &mut uuid_of),
        "minecraft:endermite" => {
            map.insert("Lifetime".into(), Tag::Int(m.extra.counter));
        }
        // Enderman.addAdditionalSaveData: the block it carries, saved only
        // while it carries one.
        "minecraft:enderman" => {
            match (m.extra.carried, registry) {
                (Some(state), Some(registry)) => {
                    map.insert("carriedBlockState".into(), state_to_nbt(registry, state));
                }
                // Nothing carries a block state out without the registry, and a
                // caller without one keeps what the loaded compound had, so the
                // key is only dropped when a registry could have written it.
                (_, None) => map.retain("carriedBlockState"),
                (None, Some(_)) => {
                    map.remove(&"carriedBlockState".into());
                }
            }
            write_persistent_anger(m, map, &mut uuid_of);
        }
        "minecraft:shulker" => {
            map.insert("AttachFace".into(), Tag::Byte(m.extra.variant3 as i8));
            map.insert("Peek".into(), Tag::Byte(m.extra.counter as i8));
            map.insert("Color".into(), Tag::Byte(m.extra.color as i8));
        }
        // HappyGhast.addAdditionalSaveData: the still timeout (`serverStillTimeout`,
        // kept in `counter`), which vanilla writes even at 0.
        "minecraft:happy_ghast" => {
            map.insert("still_timeout".into(), Tag::Int(m.extra.counter));
        }
        // GlowSquid.addAdditionalSaveData.
        "minecraft:glow_squid" => {
            map.insert(
                "DarkTicksRemaining".into(),
                Tag::Int(crate::mobs::aquatic::dark_ticks(m)),
            );
        }
        // Phantom.addAdditionalSaveData: the size, and the circling anchor.
        // `storeNullable` writes nothing for a phantom that has none, so its
        // save carries no `anchor_pos` at all.
        "minecraft:phantom" => {
            match m.extra.home {
                Some(anchor) => {
                    map.insert("anchor_pos".into(), Tag::IntArray(anchor.to_vec()));
                }
                None => {
                    map.remove(&"anchor_pos".into());
                }
            }
            map.insert("size".into(), Tag::Int(m.extra.size));
        }
        "minecraft:slime" | "minecraft:magma_cube" => {
            map.insert("Size".into(), Tag::Int(m.extra.size.max(1) - 1));
            map.insert("wasOnGround".into(), Tag::Byte(i8::from(m.extra.flag2)));
        }
        "minecraft:skeleton" => {
            map.insert("StrayConversionTime".into(), Tag::Int(conversion_time(m)));
            let time = if m.in_powder_snow {
                m.extra.counter
            } else {
                -1
            };
            map.insert("FreezingTime".into(), Tag::Int(time));
        }
        "minecraft:zoglin" => {
            map.insert("IsBaby".into(), Tag::Byte(i8::from(m.is_baby())));
        }
        // Sheep.addAdditionalSaveData: its wool as it is now.
        "minecraft:sheep" => {
            map.insert("Sheared".into(), Tag::Byte(i8::from(m.extra.sheared)));
            map.insert("Color".into(), Tag::Byte((m.extra.color & 15) as i8));
        }
        // CopperGolem.addAdditionalSaveData.
        "minecraft:copper_golem" => {
            map.insert(
                "next_weather_age".into(),
                Tag::Long(m.extra.vector[0] as i64),
            );
            map.insert(
                "weather_state".into(),
                string(crate::mobs::copper_golem::WEATHER[m.extra.variant.min(3) as usize]),
            );
        }
        "minecraft:ghast" => crate::mobs::nether::to_nbt(m, map),
        _ => {}
    }
    if m.species() == S::Frog {
        crate::mobs::brain::write_frog(m, map);
    }
    // Bogged.addAdditionalSaveData: whether its mushrooms were sheared.
    if m.species() == S::Bogged {
        map.insert("sheared".into(), Tag::Byte(i8::from(m.extra.sheared)));
    }
    // SpellcasterIllager.addAdditionalSaveData: save the live casting timer,
    // including zero and negative values, rather than stale loaded NBT.
    if matches!(m.species(), S::Evoker | S::Illusioner) {
        map.insert("SpellTicks".into(), Tag::Int(m.extra.counter));
    }
    // Vindicator.addAdditionalSaveData: `Johnny` only while the flag is set
    // (`if (this.isJohnny) output.putBoolean("Johnny", true)`).
    if m.species() == S::Vindicator {
        if m.extra.johnny {
            map.insert("Johnny".into(), Tag::Byte(1));
        } else {
            map.remove(&"Johnny".into());
        }
    }
    if m.death_canceled {
        map.insert("sunrise:death_canceled".into(), Tag::Byte(1));
    }
}
/// `NeutralMob.addPersistentAngerSaveData` at the class's own save slot.
fn write_persistent_anger(
    m: &Mob,
    map: &mut EntityTagWriter,
    uuid_of: &mut impl FnMut(u32) -> Option<[i32; 4]>,
) {
    map.insert(
        "anger_end_time".into(), // Keep the absolute value even after the time has passed.
        Tag::Long(m.extra.anger_end),
    );
    // `angry_at` only while there is an anger target: the UUID of the one
    // the level has (`uuid_of`). A mob target read from the save that is
    // not in the world (`Who::Mob(0)`) keeps the loaded UUID.
    match m.extra.angry_at {
        Some(crate::mobs::Who::Mob(0)) => map.retain("angry_at"),
        target => match target.and_then(|who| who_uuid(who, uuid_of)) {
            Some(uuid) => {
                map.insert("angry_at".into(), Tag::IntArray(uuid.to_vec()));
            }
            None => {
                map.remove(&"angry_at".into());
            }
        },
    }
}
const MODEL_PARTS: [&str; 7] = [
    "cape",
    "jacket",
    "left_sleeve",
    "right_sleeve",
    "left_pants_leg",
    "right_pants_leg",
    "hat",
];
const MOB_ONLY_KEYS: [&str; 10] = [
    "NoAI",
    "PersistenceRequired",
    "CanPickUpLoot",
    "LeftHanded",
    "drop_chances",
    "leash",
    "home_radius",
    "home_pos",
    "DeathLootTable",
    "DeathLootTableSeed",
];
/// The live map's instances and the loaded attributes nothing else models,
/// over the saved `attributes` list (`AttributeMap.pack`).
pub fn put_live_attributes(map: &mut Compound, live: &crate::living_attributes::LivingAttributes) {
    let mut list = match map.get(&"attributes".into()) {
        Some(Tag::List(l)) => l.clone(),
        _ => Vec::new(),
    };
    let before = list.clone();
    live.save_into(&mut list);
    if list != before {
        map.insert("attributes".into(), Tag::List(list));
    }
}
/// `AttributeInstance.Packed` of the attributes with a base or permanent
/// modifier finalizeSpawn set (or a command or a load did), with their ids.
pub fn attribute_entries(m: &Mob) -> Vec<(String, Tag)> {
    use crate::mobs::finalize::{self, Attr, Op};
    let Some(a) = m.extra.attributes.as_deref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for attr in Attr::ALL {
        let modifiers: Vec<Tag> = a
            .modifiers
            .iter()
            .filter(|x| x.attr == attr)
            .map(|x| {
                let op = match x.op {
                    Op::AddValue => "add_value",
                    Op::AddMultipliedBase => "add_multiplied_base",
                    Op::AddMultipliedTotal => "add_multiplied_total",
                };
                Tag::compound([
                    ("id", string(&x.id)),
                    ("amount", Tag::Double(x.amount.to_bits())),
                    ("operation", string(op)),
                ])
            })
            .collect();
        if modifiers.is_empty() && !a.bases.iter().any(|(b, _)| *b == attr) {
            continue;
        }
        let id = format!("minecraft:{}", attr.name());
        let mut entry = vec![
            ("id", string(&id)),
            ("base", Tag::Double(finalize::base_value(m, attr).to_bits())),
        ];
        if !modifiers.is_empty() {
            entry.push(("modifiers", Tag::List(modifiers)));
        }
        out.push((id, Tag::compound(entry)));
    }
    out
}
/// `AttributeMap.pack` for the attributes with a base or permanent modifier
/// finalizeSpawn set, over the saved list.
fn put_attributes(map: &mut EntityTagWriter, m: &Mob) {
    let mut list = match map.get(&"attributes".into()) {
        Some(Tag::List(l)) => l.clone(),
        _ => Vec::new(),
    };
    if let Some(attributes) = m.extra.attributes.as_deref() {
        for attr in crate::mobs::finalize::Attr::ALL {
            if !attributes
                .modifiers
                .iter()
                .any(|modifier| modifier.attr == attr)
            {
                saved_attributes::forget_stale_modifiers(&mut list, attr);
            }
        }
    }
    for (id, entry) in attribute_entries(m) {
        match list
            .iter_mut()
            .find(|t| text(t.get("id")).as_deref() == Some(id.as_str()))
        {
            Some(t) => *t = entry,
            None => list.push(entry),
        }
    }
    map.insert("attributes".into(), Tag::List(list));
}
/// `AttributeMap.pack` of a sulfur cube's body archetype attributes: the
/// base and permanent modifiers of each instance its own map holds (the
/// archetype's transient modifiers are not saved).
fn put_sulfur_attributes(map: &mut EntityTagWriter, m: &Mob) {
    use sunrise_world::attributes::Operation;
    if m.species() != crate::mobs::Species::SulfurCube {
        return;
    }
    let Some(attributes) = m.extra.armor_attributes.as_deref() else {
        return;
    };
    let mut list = match map.get(&"attributes".into()) {
        Some(Tag::List(l)) => l.clone(),
        _ => Vec::new(),
    };
    let mut changed = false;
    for id in crate::mobs::sulfur_cube::ARCHETYPE_ATTRIBUTES {
        let Some(instance) = attributes
            .catalog()
            .id(id)
            .and_then(|k| attributes.instance(k))
        else {
            continue;
        };
        let packed = instance.pack();
        let mut entry = vec![
            ("id", string(id)),
            ("base", Tag::Double(packed.base.to_bits())),
        ];
        if !packed.modifiers.is_empty() {
            let modifiers = packed
                .modifiers
                .iter()
                .map(|x| {
                    let op = match x.operation() {
                        Operation::AddValue => "add_value",
                        Operation::AddMultipliedBase => "add_multiplied_base",
                        Operation::AddMultipliedTotal => "add_multiplied_total",
                    };
                    Tag::compound([
                        ("id", string(x.id())),
                        ("amount", Tag::Double(x.amount().to_bits())),
                        ("operation", string(op)),
                    ])
                })
                .collect();
            entry.push(("modifiers", Tag::List(modifiers)));
        }
        let entry = Tag::compound(entry);
        match list
            .iter_mut()
            .find(|t| text(t.get("id")).as_deref() == Some(id))
        {
            Some(t) => *t = entry,
            None => list.push(entry),
        }
        changed = true;
    }
    if changed {
        map.insert("attributes".into(), Tag::List(list));
    }
}
/// `Brain.Packed`: the memories finalizeSpawn set, over the saved ones.
/// `CustomName`: a text component saved as a string or a `{text: ..}` compound.
pub(crate) fn custom_name(tag: &Tag) -> Option<String> {
    match tag.get("CustomName")? {
        Tag::String(s) => Some(s.to_string_lossy()),
        t @ Tag::Compound(_) => match t.get("text") {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        },
        _ => None,
    }
}
pub use sunrise_client_core::component::name_text;
/// `Mob.readAdditionalSaveData`: negative radii retain the live homePosition.
/// Used after the command adapter restores runtime AI, as well as fresh load.
pub fn read_mob_home(m: &mut Mob, tag: &Tag) {
    let radius = match tag.get("home_radius") {
        Some(Tag::Float(v)) => Some(f32::from_bits(*v).floor() as i32),
        Some(Tag::Double(v)) => Some(f64::from_bits(*v).floor() as i32),
        other => number(other).map(|v| v as i32),
    }
    .unwrap_or(-1);
    let pos = if radius >= 0 {
        home_block_pos(tag.get("home_pos")).unwrap_or([0; 3])
    } else {
        m.ai.home_position()
    };
    m.ai.restore_home(pos, radius);
}
/// `BlockPos.CODEC` through NbtOps.getIntStream: numeric collection entries
/// use Java Number's int conversion (floating entries truncate, unlike getIntOr above).
fn home_block_pos(tag: Option<&Tag>) -> Option<[i32; 3]> {
    match tag? {
        Tag::IntArray(v) => v.as_slice().try_into().ok(),
        Tag::ByteArray(v) if v.len() == 3 => Some([
            i32::from(v[0] as i8),
            i32::from(v[1] as i8),
            i32::from(v[2] as i8),
        ]),
        Tag::LongArray(v) if v.len() == 3 => Some([v[0] as i32, v[1] as i32, v[2] as i32]),
        Tag::List(v) if v.len() == 3 => {
            let int = |t: &Tag| match t {
                Tag::Float(v) => Some(f32::from_bits(*v) as i32),
                Tag::Double(v) => Some(f64::from_bits(*v) as i32),
                other => number(Some(other)).map(|v| v as i32),
            };
            Some([int(&v[0])?, int(&v[1])?, int(&v[2])?])
        }
        _ => None,
    }
}
mod mob_load;
pub use mob_load::{
    mob_from_nbt, mob_from_nbt_deferred, mob_from_nbt_remote, mob_from_nbt_with_random,
};

/// `LivingEntity.readAdditionalSaveData`'s `setHealth(getFloatOr("Health", getMaxHealth()))`:
/// `Mth.clamp` to `[0, getMaxHealth()]`, the max health the loaded attributes and their
/// modifiers give (a NaN stays, as `Math.min` keeps it).
pub(crate) fn loaded_health(m: &Mob, health: Option<f64>) -> f32 {
    let max = m.attrs.max_health;
    let health = health.map_or(max, |h| h as f32);
    if health < 0.0 {
        0.0
    } else if health.is_nan() {
        health
    } else {
        health.min(max)
    }
}

fn mob_from_nbt_at(
    tag: &Tag,
    kind: Arc<MobType>,
    id: u32,
    registry: Option<&BlockRegistry>,
    random: sunrise_core::random::LegacyRandom,
    position: Vec3,
    authoritative: bool,
) -> Mob {
    let [yaw, pitch] = rotation(tag);
    let mut m = if authoritative {
        Mob::with_random(id, kind, position, yaw, random)
    } else {
        Mob::remote_with_random(id, kind, position, yaw, random)
    };
    m.velocity = motion(tag);
    m.head_pitch = pitch;
    // LivingEntity.readAdditionalSaveData: `setSharedFlag(7, getBooleanOr(FallFlying, false))`.
    m.extra.fall_flying = flag(tag, "FallFlying");
    read_mob_home(&mut m, tag);
    m.ground.on_ground = flag(tag, "OnGround");
    m.fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
    // AbstractCubeMob.readAdditionalSaveData sizes before the living load, so saved bases win.
    if matches!(
        m.species(),
        crate::mobs::Species::Slime | crate::mobs::Species::MagmaCube
    ) {
        crate::mobs::cube::set_size(
            &mut m,
            (number(tag.get("Size")).unwrap_or(0) as i32).wrapping_add(1),
        );
    }
    // LivingEntity.readAdditionalSaveData: the saved attribute bases, then
    // the health, the max health when there is none.
    if let Some(Tag::List(attrs)) = tag.get("attributes") {
        for a in attrs {
            let (Some(id), Some(v)) = (text(a.get("id")), float(a.get("base"))) else {
                continue;
            };
            let v = m.kind.attribute_bounds.loaded_base(&id, v);
            // A mount's rolled health and speed stay attribute bases, which the server syncs.
            let equine = crate::mobs::equine::is_equine(m.species());
            match id.as_str() {
                "minecraft:max_health" if equine => crate::mobs::finalize::set_base(
                    &mut m,
                    crate::mobs::finalize::Attr::MaxHealth,
                    v,
                ),
                "minecraft:movement_speed" if equine => crate::mobs::finalize::set_base(
                    &mut m,
                    crate::mobs::finalize::Attr::MovementSpeed,
                    v,
                ),
                // A saved base unlike the current one (type default or cube size) becomes the base.
                "minecraft:max_health"
                    if crate::mobs::finalize::base_value(
                        &m,
                        crate::mobs::finalize::Attr::MaxHealth,
                    ) != v =>
                {
                    crate::mobs::finalize::set_base(
                        &mut m,
                        crate::mobs::finalize::Attr::MaxHealth,
                        v,
                    )
                }
                "minecraft:max_health" => m.attrs.max_health = v as f32,
                "minecraft:movement_speed"
                    if crate::mobs::finalize::base_value(
                        &m,
                        crate::mobs::finalize::Attr::MovementSpeed,
                    ) != v =>
                {
                    crate::mobs::finalize::set_base(
                        &mut m,
                        crate::mobs::finalize::Attr::MovementSpeed,
                        v,
                    )
                }
                "minecraft:movement_speed" => m.attrs.speed = v,
                "minecraft:attack_damage"
                    if crate::mobs::finalize::base_value(
                        &m,
                        crate::mobs::finalize::Attr::AttackDamage,
                    ) != v =>
                {
                    crate::mobs::finalize::set_base(
                        &mut m,
                        crate::mobs::finalize::Attr::AttackDamage,
                        v,
                    )
                }
                "minecraft:attack_damage" => m.attrs.attack_damage = v as f32,
                "minecraft:attack_knockback" => m.attrs.attack_knockback = v,
                "minecraft:flying_speed"
                    if crate::mobs::finalize::base_value(
                        &m,
                        crate::mobs::finalize::Attr::FlyingSpeed,
                    ) != v =>
                {
                    crate::mobs::finalize::set_base(
                        &mut m,
                        crate::mobs::finalize::Attr::FlyingSpeed,
                        v,
                    )
                }
                // The attributes an effect's modifier changes (#1988).
                "minecraft:safe_fall_distance"
                | "minecraft:max_absorption"
                | "minecraft:waypoint_transmit_range" => {
                    use crate::mobs::finalize;
                    if let Some(a) = finalize::command_attr(id.trim_start_matches("minecraft:"))
                        && finalize::default_value(&m, a) != Some(v)
                    {
                        finalize::set_base(&mut m, a, v);
                    }
                }
                _ => {}
            }
        }
    }
    // `AttributeMap.apply`: the saved permanent modifiers of the attributes the
    // game models (the follow range bonus finalizeSpawn made among them), and
    // the other saved attributes for the live map.
    if let Some(Tag::List(_)) = tag.get("attributes") {
        use crate::mobs::finalize::Attr;
        saved_attributes::read_attribute_modifiers(&mut m, tag);
        // An attribute the type lacks is skipped (`AttributeMap.apply`).
        let kind = m.kind.clone();
        m.extra.live.load(tag.get("attributes"), |id| {
            kind.attributes.contains_key(id)
                && !Attr::ALL.iter().any(|a| *a != Attr::Armor && a.id() == id)
        });
    }
    m.health = loaded_health(&m, float(tag.get("Health")));
    // LivingEntity.readAdditionalSaveData: `internalSetAbsorptionAmount`
    // stores the value as read; only `setAbsorptionAmount` clamps.
    m.absorption = float(tag.get("AbsorptionAmount")).map_or(0.0, |h| h as f32);
    m.hurt_time = i32::from(number(tag.get("HurtTime")).unwrap_or(0) as i16);
    // What it holds (equipment.mainhand / offhand), when saved.
    if let Some(eq) = tag.get("equipment") {
        // The hands, armour, body and saddle: the one `equipment` codec.
        m.extra.equipment = sunrise_items::equipment::Equipment::read(eq);
        // Their enchantments and trims.
        // (`gear` slot order: hands, then head to feet.)
        use sunrise_items::equipment::{CHEST, FEET, HEAD, LEGS, MAINHAND, OFFHAND};
        for (index, slot) in [MAINHAND, OFFHAND, HEAD, CHEST, LEGS, FEET]
            .into_iter()
            .enumerate()
        {
            if let Some(stack) = m.extra.slot_stack(slot) {
                let levels = stack.enchantments().to_vec();
                let trim = stack.data().and_then(|d| d.trim.clone());
                crate::mobs::gear::set(&mut m, index, levels);
                crate::mobs::gear::set_trim(&mut m, index, trim);
            }
        }
    } else {
        // Registration hands out default weapons (finalizeSpawn does in
        // vanilla); a loaded mob holds only what its file lists.
        m.extra.set_main_hand(None);
    }
    mob_death::read(&mut m, tag);
    m.ai.ai_paused = ai_pause::read(tag);
    m.impulse_grace_time =
        number(tag.get("current_impulse_context_reset_grace_time")).unwrap_or(0) as i32;
    // Mob.readAdditionalSaveData overrides what the constructor set (copper
    // golems and elder guardians start persistent).
    m.persistent = flag(tag, "PersistenceRequired");
    m.extra.death_loot_table = text(tag.get("DeathLootTable"))
        .and_then(|t| sunrise_client_core::identifier::normalize_id(&t));
    m.extra.death_table_seed = number(tag.get("DeathLootTableSeed")).unwrap_or(0);
    refs::read_last_hurt_by(&mut m, tag);
    // `NeutralMob.readPersistentAngerSaveData`: the anger end time and the
    // target come back with the mob.
    if persistent_anger(m.species()) {
        m.extra.anger_end = number(tag.get("anger_end_time")).unwrap_or(-1);
        // Anger at an entity other than the player is anger at a mob that may
        // not be here (`EntityReference`): it still is not `isAngryAtAllPlayers`.
        m.extra.angry_at = match uuid_from_tag(tag.get("angry_at")) {
            Some(uuid) if uuid == player_uuid() => Some(crate::mobs::Who::Player),
            Some(_) => Some(crate::mobs::Who::Mob(0)),
            None => None,
        };
    }
    // `Mannequin.readAdditionalSaveData`: the profile comes back with the skin, then
    // validated pose, `immovable`, `hide_description` and `description` (`updateDescription`).
    if m.species() == crate::mobs::Species::Mannequin {
        m.profile = tag.get("profile").and_then(profile_from_nbt);
        // Mannequin.POSE_CODEC accepts only these Avatar poses; an invalid
        // or absent value defaults to standing before the live dimensions read it.
        m.extra.avatar_pose = match text(tag.get("pose")).as_deref() {
            Some("crouching") => crate::pose::Pose::Crouching,
            Some("swimming") => crate::pose::Pose::Swimming,
            Some("fall_flying") => crate::pose::Pose::FallFlying,
            Some("sleeping") => crate::pose::Pose::Sleeping,
            _ => crate::pose::Pose::Standing,
        };
        // `HumanoidArm.CODEC`, right when absent or not an arm.
        m.extra.main_hand_left = text(tag.get("main_hand")).as_deref() == Some("left");
        // `LAYERS_CODEC`: every listed part hidden; an entry that is not a
        // part fails the whole list, which reads as `ALL_LAYERS` (none hidden).
        m.extra.hidden_model_parts = match tag.get("hidden_layers") {
            Some(Tag::List(parts)) => parts
                .iter()
                .try_fold(0u8, |mask, part| {
                    let name = text(Some(part))?;
                    let bit = MODEL_PARTS.iter().position(|p| *p == name)?;
                    Some(mask | 1 << bit)
                })
                .unwrap_or(0),
            _ => 0,
        };
        m.extra.immovable = flag(tag, "immovable");
        m.extra.description = if flag(tag, "hide_description") {
            crate::mobs::MannequinDescription::Hidden
        } else {
            crate::mobs::MannequinDescription::from_component(Some(
                &tag.get("description")
                    .cloned()
                    .or_else(|| crate::mobs::MannequinDescription::Default.component())
                    .unwrap_or(Tag::End),
            ))
        };
    }
    // AbstractPiglin.readAdditionalSaveData: getBooleanOr("CanPickUpLoot", true).
    let piglin = matches!(
        m.species(),
        crate::mobs::Species::Piglin | crate::mobs::Species::PiglinBrute
    );
    m.extra.pick_up_loot = tag
        .get("CanPickUpLoot")
        .and_then(int_value)
        .map_or(piglin, |value| value as i8 != 0);
    m.custom_name = custom_name(tag);
    m.custom_name_text = tag.get("CustomName").map(name_text);
    m.extra.left_handed = flag(tag, "LeftHanded");
    if let Some(Tag::Compound(c)) = tag.get("drop_chances") {
        let mut chances = [crate::mobs::drops::DEFAULT_DROP_CHANCE; 8];
        for (chance, slot) in chances.iter_mut().zip(crate::mobs::drops::SLOTS) {
            if let Some(Tag::Float(v)) = c.get(&slot.into()) {
                *chance = f32::from_bits(*v);
            }
        }
        m.extra.drop_chances = Some(Box::new(chances));
    }
    if saves_age(m.species()) {
        m.extra.age = tag.get("Age").and_then(int_value).unwrap_or(0);
        m.extra.forced_age = tag.get("ForcedAge").and_then(int_value).unwrap_or(0);
        m.extra.age_locked = flag(tag, "AgeLocked");
    }
    if is_animal(&m.kind.id) {
        m.extra.in_love = tag.get("InLove").and_then(int_value).unwrap_or(0);
        // Animal.readAdditionalSaveData: the player who fed it into love.
        m.extra.love_cause = crate::save::uuid_from_tag(tag.get("LoveCause"));
    }
    m.name_visible = flag(tag, "CustomNameVisible");
    m.no_ai = flag(tag, "NoAI");
    m.no_gravity = flag(tag, "NoGravity");
    m.glowing = flag(tag, "Glowing");
    m.ticks_frozen = number(tag.get("TicksFrozen")).unwrap_or(0) as i32;
    // LivingEntity.readAdditionalSaveData: the saved `active_effects`, the last
    // entry of an effect winning.
    if let Some(Tag::List(effects)) = tag.get("active_effects") {
        for e in effects {
            let Some(def) = text(e.get("id")).and_then(|id| crate::brewing::effect(&id)) else {
                continue;
            };
            let effect = crate::brewing::namespaced(def.id);
            let duration = number(e.get("duration")).unwrap_or(0) as i32;
            // `MobEffectInstance.Details`: Codec.BYTE uses boxed Number.byteValue,
            // then UNSIGNED_BYTE interprets the resulting byte as 0..255.
            let amplifier = e
                .get("amplifier")
                .and_then(sunrise_world::datafix::dynamic::as_number)
                .map_or(0, |n| i32::from(n.byte_value() as u8));
            // Details.CODEC: `show_particles` defaults to true, `show_icon` to `show_particles`.
            let particles = number(e.get("show_particles")) != Some(0);
            let mut stored = crate::mobs::StoredEffect::new(effect, duration, amplifier);
            stored.show_icon = number(e.get("show_icon")).map_or(particles, |v| v != 0);
            m.load_effect(stored);
            m.extra.effect_chains.retain(|e| e.id != effect);
            if let Some(hidden) = e.get("hidden_effect") {
                m.extra
                    .effect_chains
                    .push(hidden_effect_from_nbt(effect, hidden));
            }
            m.extra.hidden_effects.retain(|h| *h != effect);
            if !particles {
                m.extra.hidden_effects.push(effect);
            }
            // Details.CODEC: `ambient` defaults to false.
            m.extra.ambient_effects.retain(|h| *h != effect);
            if number(e.get("ambient")).is_some_and(|a| a != 0) {
                m.extra.ambient_effects.push(effect);
            }
        }
        // A loaded effect adds no modifier (`load_effect`): a loaded Health
        // Boost's maximum is the saved `attributes`' one, and the health
        // follows it as `setHealth` clamps.
        if m.extra
            .effects
            .iter()
            .any(|e| e.id == "minecraft:health_boost")
        {
            crate::mobs::finalize::health_boost_changed(&mut m);
        }
    }
    m.silent = flag(tag, "Silent");
    m.invulnerable_flag = flag(tag, "Invulnerable");
    // Entity.load: `getIntOr("invulnerable_time", 0)` — the entity-level
    // window (#916); the hurt cooldown never loads.
    m.invulnerable_time = match tag.get("invulnerable_time") {
        Some(Tag::Int(ticks)) => *ticks,
        _ => 0,
    };
    if m.species() == crate::mobs::Species::Bat {
        // Bat.readAdditionalSaveData: the raw BatFlags byte; bit 0 is resting.
        let flags = number(tag.get("BatFlags")).unwrap_or(0) as u8;
        m.extra.flag = flags & 1 != 0;
        m.extra.bat_flags = flags & !1;
    }
    if m.species() == crate::mobs::Species::Creeper {
        m.extra.charged = flag(tag, "powered");
        // Creeper.readAdditionalSaveData: `ignited` lights the fuse.
        m.extra.flag = flag(tag, "ignited");
        m.max_swell = number(tag.get("Fuse")).unwrap_or(30) as i16 as i32;
        // `getByteOr("ExplosionRadius", 3)`: any numeric tag, as a byte.
        m.extra.explosion_radius = number(tag.get("ExplosionRadius")).unwrap_or(3) as i8;
    }
    if m.species() == crate::mobs::Species::Ghast {
        crate::mobs::nether::from_nbt(&mut m, tag);
    }
    // InventoryCarrier.readInventoryFromTag, using the existing occupied-stack model.
    let carrier_slots = match m.species() {
        crate::mobs::Species::Pillager => 5,
        crate::mobs::Species::WanderingTrader => 8,
        _ => 0,
    };
    if carrier_slots != 0
        && let Some(Tag::List(items)) = tag.get("Inventory")
    {
        m.extra.inventory = items
            .iter()
            .filter_map(crate::save::item_from_nbt)
            .take(carrier_slots)
            .collect();
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Piglin
            | crate::mobs::Species::PiglinBrute
            | crate::mobs::Species::Hoglin
    ) {
        m.extra.flag2 = flag(tag, "IsImmuneToZombification");
        m.extra.counter2 = number(tag.get("TimeInOverworld")).unwrap_or(0) as i32;
        if m.species() == crate::mobs::Species::Piglin {
            crate::mobs::piglin::read_guarded_anger(&mut m, tag);
        }
        if m.species() == crate::mobs::Species::Hoglin {
            m.extra.cannot_be_hunted = flag(tag, "CannotBeHunted");
        }
        if m.species() == crate::mobs::Species::Piglin
            && let Some(Tag::List(items)) = tag.get("Inventory")
        {
            // InventoryCarrier.readInventoryFromTag -> SimpleContainer
            // .fromItemList (eight slots).
            m.extra.inventory = crate::mobs::villager::load_container(&m, items, 8);
        }
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Pig | crate::mobs::Species::Strider
    ) {
        // The old `Saddle` flag, when `equipment` held none.
        if flag(tag, "Saddle") && m.saddle_item().is_none() {
            m.set_saddle(Some(crate::items::ItemStack::new("minecraft:saddle", 1)));
        }
    }
    if crate::mobs::equine::is_equine(m.species()) {
        m.extra.tame = flag(tag, "Tame");
        // AbstractHorse.readAdditionalSaveData: the saved Int as is, camels included.
        m.extra.equine_temper = tag.get("Temper").and_then(int_value).unwrap_or(0);
        m.extra.bred = flag(tag, "Bred");
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        let v = number(tag.get("Variant")).unwrap_or(0) as i32;
        (m.extra.variant, m.extra.variant2) = crate::mobs::equine::horse_look(v);
        if crate::mobs::llama::is_llama(m.species()) {
            crate::mobs::llama::from_nbt(&mut m, tag);
        }
        m.extra.chested = flag(tag, "ChestedHorse");
        if m.extra.chested {
            m.extra.items = crate::save::decode_items(tag.get("Items"), 15);
        }
        if let Some(Tag::List(attrs)) = tag.get("attributes") {
            for a in attrs {
                let (Some(id), Some(v)) = (text(a.get("id")), float(a.get("base"))) else {
                    continue;
                };
                if id == "minecraft:jump_strength" {
                    m.extra.vector[0] = v;
                }
            }
        }
        if m.extra.vector[0] <= 0.0 {
            m.extra.vector[0] = if crate::mobs::equine::is_chested(m.species()) {
                0.5
            } else {
                0.7
            };
        }
    }
    if crate::mobs::finalize::saves_variant_name(m.species())
        && let Some(v) = text(tag.get("variant"))
    {
        m.extra.variant = crate::mobs::finalize::variant_index(m.species(), &v);
    }
    if m.species() == crate::mobs::Species::Chicken
        && let Some(t) = number(tag.get("EggLayTime"))
    {
        m.extra.egg_time = t as i32;
    }
    if let Some(v) = text(tag.get("sound_variant")) {
        let v = v.trim_start_matches("minecraft:");
        let sounds = crate::mobs::finalize::sound_variants(m.species());
        m.extra.sound_variant = sounds.iter().position(|n| *n == v).map(|i| i as u8);
    }
    match m.species() {
        crate::mobs::Species::Fox => {
            m.extra.variant = u8::from(text(tag.get("Type")).as_deref() == Some("snow"));
            // Fox.readAdditionalSaveData: clearTrusted, then addTrustedEntity for
            // each of "Trusted".
            crate::mobs::fox::clear_trusted(&mut m);
            if let Some(Tag::List(trusted)) = tag.get("Trusted") {
                for uuid in trusted
                    .iter()
                    .filter_map(|t| crate::save::uuid_from_tag(Some(t)))
                {
                    crate::mobs::fox::add_trusted_entity(&mut m, uuid);
                }
            }
            // Fox.readAdditionalSaveData: the sleeping, sitting and crouching flags.
            for (key, bit) in [
                ("Sleeping", crate::mobs::fox::SLEEPING),
                ("Sitting", crate::mobs::fox::SITTING),
                ("Crouching", crate::mobs::fox::CROUCHING),
            ] {
                if flag(tag, key) {
                    m.extra.ints[0] |= bit;
                }
            }
        }
        // MushroomCow.readAdditionalSaveData: a missing or unknown name is
        // Variant.DEFAULT, red; stew_effects decodes like the component's list.
        crate::mobs::Species::Mooshroom => {
            m.extra.variant = u8::from(text(tag.get("Type")).as_deref() == Some("brown"));
            if let Some(Tag::List(list)) = tag.get("stew_effects") {
                m.extra.stew_effects = list
                    .iter()
                    .map(|entry| {
                        Some((
                            Arc::from(text(entry.get("id"))?.as_str()),
                            number(entry.get("duration")).unwrap_or(160) as i32,
                        ))
                    })
                    .collect();
            }
        }
        crate::mobs::Species::Rabbit => {
            // Rabbit.Variant.byId: an unknown id is brown.
            m.extra.variant = match number(tag.get("RabbitType")).unwrap_or(0) {
                v @ (0..=5 | 99) => v as u8,
                _ => 0,
            };
        }
        crate::mobs::Species::Goat => crate::mobs::goat::read_nbt(&mut m, tag),
        _ => {}
    }
    // `Rabbit.readAdditionalSaveData`.
    if m.species() == crate::mobs::Species::Rabbit {
        m.extra.more_carrot_ticks = number(tag.get("MoreCarrotTicks")).unwrap_or(0) as i32;
    }
    // `Ocelot.readAdditionalSaveData`.
    if m.species() == crate::mobs::Species::Ocelot {
        m.extra.trusting = flag(tag, "Trusting");
    }
    if m.species() == crate::mobs::Species::Cat {
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        m.extra.tame = m.extra.owner.is_some();
        crate::mobs::tame::restore_sitting(&mut m, flag(tag, "Sitting"));
        let legacy = |c: i64| u8::try_from(c as i8).ok().filter(|c| *c < 16).unwrap_or(0);
        m.extra.color = number(tag.get("CollarColor")).map_or(crate::mobs::tame::RED, legacy);
    }
    if m.species() == crate::mobs::Species::Panda {
        let gene = |key| text(tag.get(key)).map_or(0, |n| crate::mobs::panda::gene_from_name(&n));
        m.extra.variant = gene("MainGene");
        m.extra.variant2 = gene("HiddenGene");
    }
    if m.species() == crate::mobs::Species::Parrot {
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        m.extra.tame = m.extra.owner.is_some();
        crate::mobs::tame::restore_sitting(&mut m, flag(tag, "Sitting"));
    }
    if m.species() == crate::mobs::Species::Wolf {
        let health = m.health;
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        let tame = m.extra.owner.is_some();
        crate::mobs::tame::set_tame(&mut m, tame);
        m.health = health;
        crate::mobs::tame::restore_sitting(&mut m, flag(tag, "Sitting"));
        let legacy = |c: i64| u8::try_from(c as i8).ok().filter(|c| *c < 16).unwrap_or(0);
        m.extra.color = number(tag.get("CollarColor")).map_or(crate::mobs::tame::RED, legacy);
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Villager | crate::mobs::Species::WanderingTrader
    ) {
        read_villager(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::ZombieVillager {
        read_villager(&mut m, tag);
        let xp = number(tag.get("Xp")).unwrap_or(0) as i32;
        if xp != 0 {
            m.extra.trade.get_or_insert_with(Default::default).xp = xp;
        }
        crate::mobs::zombie_villager::read_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Dolphin {
        // Dolphin.readAdditionalSaveData: a save that names neither field is an
        // unfed dolphin at full moistness.
        m.extra.flag = flag(tag, "GotFish");
        m.extra.counter = number(tag.get("Moistness")).unwrap_or(2400) as i32;
    }
    if m.species() == crate::mobs::Species::Axolotl {
        restore_axolotl_memories(&mut m, tag);
    }
    read_variant(&mut m, tag);
    m.fire_ticks = i32::from(number(tag.get("Fire")).unwrap_or(0) as i16);
    // Entity.load: `input.getIntOr("Air", getMaxAirSupply())`, 4800 for a dolphin.
    m.air = number(tag.get("Air")).map_or(m.max_air_supply(), |air| air as i32);
    if m.species() == crate::mobs::Species::Allay {
        if let Some(Tag::List(inv)) = tag.get("Inventory") {
            for stack in inv.iter().filter_map(item_from_nbt) {
                let max = m.kind.inventory_stack_limit(&stack);
                if let Some(st) = m.extra.allay.as_mut() {
                    st.add(stack, max);
                }
            }
        }
        m.extra.egg_time = number(tag.get("DuplicationCooldown")).unwrap_or(0) as i32;
        if let (Some(st), Some(listener)) = (m.extra.allay.as_mut(), tag.get("listener")) {
            st.vibration = crate::sculk::Data::from_nbt(listener);
        }
        let liked = tag
            .get("Brain")
            .and_then(|b| b.get("memories"))
            .and_then(|b| b.get("minecraft:liked_player"));
        m.extra.tame = liked.is_some();
        m.extra.owner = liked
            .and_then(|l| l.get("value"))
            .and_then(|v| crate::save::uuid_from_tag(Some(v)));
        // `Brain.readAdditionalSaveData`: a memory saved with a time to live
        // (`Codec.LONG`, read from any integer tag). `Long.MAX_VALUE` is
        // `MemorySlot.NEVER_EXPIRE`: that memory never expires.
        if let Some(st) = m.extra.allay.as_mut() {
            st.liked_player_ttl = liked
                .and_then(|l| match l.get("ttl")? {
                    Tag::Byte(v) => Some(i64::from(*v)),
                    Tag::Short(v) => Some(i64::from(*v)),
                    Tag::Int(v) => Some(i64::from(*v)),
                    Tag::Long(v) => Some(*v),
                    _ => None,
                })
                .filter(|&ttl| ttl != i64::MAX);
        }
        let memory = |key: &str| {
            tag.get("Brain")
                .and_then(|b| b.get("memories"))
                .and_then(|b| b.get(key))
                .and_then(|l| l.get("value"))
        };
        m.extra.ints[2] =
            number(memory("minecraft:liked_noteblock_cooldown_ticks")).unwrap_or(0) as i32;
        let pickup = number(memory("minecraft:item_pickup_cooldown_ticks")).map(|t| t as i32);
        crate::mobs::allay::set_pickup_cooldown(&mut m, pickup);
        if let Some(st) = m.extra.allay.as_mut()
            && let Some(noteblock) = memory("minecraft:liked_noteblock")
            && let Some(Tag::IntArray(p)) = noteblock.get("pos")
            && p.len() == 3
        {
            st.noteblock = Some(crate::mobs::allay::GlobalPos {
                dimension: crate::mobs::allay::dimension_id(
                    &text(noteblock.get("dimension")).unwrap_or_default(),
                ),
                pos: [p[0], p[1], p[2]],
            });
        }
    }
    if m.species() == crate::mobs::Species::Frog
        && tag
            .get("Brain")
            .and_then(|b| b.get("memories"))
            .and_then(|b| b.get("minecraft:is_pregnant"))
            .is_some()
    {
        crate::mobs::frog::set_pregnant(&mut m, true);
    }
    if crate::mobs::camel::is_camel(m.species()) {
        // AbstractHorse.readAdditionalSaveData: the eating state (Temper is
        // read with the other equines' above).
        m.extra.flag = flag(tag, "EatingHaystack");
        // The tick is a game time: `camel::finish_load` puts it on the mob's
        // clock once the world's time is known.
        let tick = match tag.get("LastPoseTick") {
            Some(Tag::Long(v)) => *v,
            _ => 0,
        };
        crate::mobs::camel::stand_up_instantly(&mut m);
        m.extra.pose_change_tick = tick;
        m.extra.camel_sitting = tick < 0;
        // Camels are always tame (`Camel.register` in Sunrise).
        m.extra.tame = true;
    }
    if crate::mobs::nautilus::is_nautilus(m.species()) {
        crate::mobs::nautilus::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::SulfurCube {
        crate::mobs::sulfur_cube::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Bee {
        crate::mobs::bee::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Creaking
        && let Some(Tag::IntArray(home)) = tag.get("home_pos")
        && home.len() == 3
    {
        crate::mobs::creaking::set_transient(&mut m, [home[0], home[1], home[2]]);
    }
    if m.species() == crate::mobs::Species::Turtle {
        // Turtle.readAdditionalSaveData: `home_pos` (else where it stands) and `has_egg`.
        m.extra.flag = flag(tag, "has_egg");
        m.extra.home = Some(match tag.get("home_pos") {
            Some(Tag::IntArray(home)) if home.len() == 3 => [home[0], home[1], home[2]],
            _ => [
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ],
        });
    }
    if m.species() == crate::mobs::Species::Wither {
        m.extra.counter = number(tag.get("Invul")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Sniffer {
        crate::mobs::sniffer::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::CopperGolem {
        // CopperGolem.readAdditionalSaveData: -1 (unset) when missing.
        m.extra.vector[0] = match tag.get("next_weather_age") {
            Some(Tag::Long(v)) => *v as f64,
            _ => -1.0,
        };
        let weather = match tag.get("weather_state") {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        m.extra.variant = crate::mobs::copper_golem::WEATHER
            .iter()
            .position(|w| *w == weather)
            .unwrap_or(0) as u8;
    }
    m.extra.leash_pending = crate::mobs::leash::pending_from_nbt(tag.get("leash"));
    if m.species() == crate::mobs::Species::Warden {
        // The brain starts afresh: a minute before it digs back down.
        crate::mobs::warden::finalize(&mut m, false, &mut Vec::new());
        if let Some(st) = m.extra.warden.as_mut() {
            if let Some(listener) = tag.get("listener") {
                st.vibration = crate::sculk::Data::from_nbt(listener);
            }
            // `Warden.readAdditionalSaveData`: the anger comes back as the list
            // of UUIDs it was saved as. Every suspect starts in the UUID map and
            // the anger ticks move the loaded ones into the live one, since a
            // suspect may be loaded long after the warden.
            if let Some(anger) = tag.get("anger") {
                st.anger =
                    crate::mobs::warden::AngerManagement::from_saved(warden_anger_from_nbt(anger));
            }
        }
        restore_warden_owned_memories(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Armadillo {
        let state = text(tag.get("state"));
        let scute_time = number(tag.get("scute_time")).map(|t| t as i32);
        crate::mobs::armadillo::load(
            &mut m,
            tag.get("Brain").is_some(),
            state.as_deref(),
            scute_time,
        );
    }
    if m.species() == crate::mobs::Species::Ravager {
        m.extra.counter = number(tag.get("AttackTick")).unwrap_or(0) as i32;
        m.extra.counter2 = number(tag.get("StunTick")).unwrap_or(0) as i32;
        m.extra.anim_ticks = number(tag.get("RoarTick")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Endermite {
        m.extra.counter = number(tag.get("Lifetime")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Vex {
        crate::mobs::vex::read_nbt(&mut m, tag);
    }
    // Bogged.readAdditionalSaveData: unsheared when absent.
    if m.species() == crate::mobs::Species::Bogged {
        m.extra.sheared = flag(tag, "sheared");
    }
    // Sheep.readAdditionalSaveData: white and unsheared when absent.
    if m.species() == crate::mobs::Species::Sheep {
        m.extra.color = crate::mobs::farm::saved_color(tag.get("Color"));
        m.extra.sheared = flag(tag, "Sheared");
    }
    // GlowSquid.readAdditionalSaveData: not dimmed when absent.
    if m.species() == crate::mobs::Species::GlowSquid {
        crate::mobs::aquatic::set_dark_ticks(
            &mut m,
            number(tag.get("DarkTicksRemaining")).unwrap_or(0) as i32,
        );
    }
    // IronGolem.readAdditionalSaveData: not player created when absent.
    if m.species() == crate::mobs::Species::IronGolem {
        m.extra.player_created = flag(tag, "PlayerCreated");
    }
    // SnowGolem.readAdditionalSaveData: getBooleanOr("Pumpkin", true).
    if m.species() == crate::mobs::Species::SnowGolem {
        m.extra.sheared = !tag
            .get("Pumpkin")
            .and_then(int_value)
            .is_none_or(|value| value as i8 != 0);
    }
    // Pufferfish.readAdditionalSaveData: `getIntOr("PuffState", 0)`, at most 2.
    if m.species() == crate::mobs::Species::Pufferfish {
        m.extra.size = number(tag.get("PuffState")).unwrap_or(0).min(2) as i32;
    }
    // Enderman.readAdditionalSaveData: the block it carried; an air state
    // reads as no block at all.
    if m.species() == crate::mobs::Species::Enderman
        && let Some(registry) = registry
    {
        m.extra.carried = tag
            .get("carriedBlockState")
            .and_then(|t| state_from_nbt(registry, t))
            .filter(|&state| !is_air(registry, state));
    }
    // HappyGhast.readAdditionalSaveData: the still timeout, 0 when absent (a fresh
    // ghast, or one saved by a version that did not write it).
    if m.species() == crate::mobs::Species::HappyGhast {
        m.extra.counter = number(tag.get("still_timeout")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Shulker {
        m.extra.variant3 = (number(tag.get("AttachFace")).unwrap_or(0) as i8 % 6).unsigned_abs();
        m.extra.counter = number(tag.get("Peek")).map_or(0, |v| v as i8 as i32);
        m.extra.color = number(tag.get("Color")).map_or(16, |v| v as i8 as u8);
        m.anim.v[0] = m.extra.counter as f32 * 0.01;
        m.anim.vo[0] = m.anim.v[0];
        // Shulker.setPos: square on its block.
        m.position = sunrise_core::geometry::Vec3::new(
            m.position.x.floor() + 0.5,
            (m.position.y + 0.5).floor(),
            m.position.z.floor() + 0.5,
        );
        m.previous = m.position;
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Slime | crate::mobs::Species::MagmaCube
    ) {
        m.extra.flag2 = flag(tag, "wasOnGround");
    }
    if m.species() == crate::mobs::Species::Phantom {
        // Phantom.readAdditionalSaveData: setPhantomSize(size), and the circling
        // anchor, absent or unreadable when the save has no anchor at all
        // (`input.read("anchor_pos", BlockPos.CODEC).orElse(null)`).
        crate::mobs::phantom::set_size(&mut m, number(tag.get("size")).unwrap_or(0) as i32);
        m.extra.home = home_block_pos(tag.get("anchor_pos"));
    }
    // Zombie, Piglin and Zoglin.readAdditionalSaveData: IsBaby (the baby
    // zombies of trial spawners).
    if !crate::mobs::finalize::ageable(m.species()) {
        crate::mobs::finalize::set_baby(&mut m, flag(tag, "IsBaby"));
    }
    // Zombie.readAdditionalSaveData: setCanBreakDoors.
    if crate::mobs::doors::is_zombie(m.species()) {
        crate::mobs::doors::set_can_break_doors(&mut m, flag(tag, "CanBreakDoors"));
    }
    // Zombie and Skeleton.readAdditionalSaveData: `ConversionTracker` reads its
    // affliction time (0 when absent) and starts a conversion from a saved time
    // other than -1, -1 stopping one (`setConverting(false)`).
    let tracker = match m.species() {
        crate::mobs::Species::Zombie | crate::mobs::Species::Husk => {
            Some(("InWaterTime", "DrownedConversionTime"))
        }
        crate::mobs::Species::Skeleton => Some(("FreezingTime", "StrayConversionTime")),
        _ => None,
    };
    if let Some((affliction, conversion)) = tracker {
        m.extra.counter = number(tag.get(affliction)).unwrap_or(0) as i32;
        let time = number(tag.get(conversion)).unwrap_or(-1) as i32;
        // Any other time starts a conversion (`startConversion`), a negative
        // one spent on the next tick.
        m.extra.counter2 = if time == -1 { 0 } else { time.max(0) + 1 };
    }
    // SpellcasterIllager.readAdditionalSaveData: this is the same timer used
    // by casting goals and the existing customServerAiStep countdown.
    if matches!(
        m.species(),
        crate::mobs::Species::Evoker | crate::mobs::Species::Illusioner
    ) {
        m.extra.counter = number(tag.get("SpellTicks")).unwrap_or(0) as i32;
    }
    // Vindicator.readAdditionalSaveData: `Johnny`, false when absent. A name
    // that is Johnny latches the flag again on the next AI step.
    if m.species() == crate::mobs::Species::Vindicator {
        m.extra.johnny = flag(tag, "Johnny");
    }
    crate::mobs::raider::read_nbt(&mut m, tag);
    crate::mobs::riders::read_nbt(&mut m, tag);
    // A mannequin is an `Avatar`, not a `Mob`: it reads none of
    // `Mob.readAdditionalSaveData`'s flags, after the common reads above.
    if m.species() == crate::mobs::Species::Mannequin {
        m.persistent = false;
        m.extra.pick_up_loot = false;
        m.extra.left_handed = false;
        m.no_ai = false;
    }
    // setHealth clamps a negative health to 0; a dead mob keeps its body (tickDeath).
    if m.health < 0.0 {
        m.health = 0.0;
    }
    m
}

/// `ItemEntity`: Health, Age, PickupDelay, Item, Thrower.
/// ItemEntity saved with the actual loaded item prototype, including transfers.
pub fn item_entity_to_nbt(
    e: &ItemEntity,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Tag {
    let item = sunrise_items::item_nbt::item_to_nbt(&e.stack, items);
    let base = Base {
        id: "minecraft:item",
        uuid,
        position: e.position,
        motion: e.velocity,
        rotation: e.rotation,
        on_ground: e.ground.on_ground,
        fall_distance: e.fall_distance,
    };
    let mut tag = base.non_mob_tag(raw);
    for (key, value) in [
        ("Item", item),
        ("Health", Tag::Short(e.health as i16)),
        ("Fire", Tag::Short(e.fire_ticks as i16)),
        (
            "Age",
            Tag::Short(e.age.clamp(i16::MIN as i32, i16::MAX as i32) as i16),
        ),
        (
            "PickupDelay",
            Tag::Short(
                e.pickup_delay
                    .clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16,
            ),
        ),
    ] {
        put(&mut tag, key, value);
    }
    if let Some(thrower) = e.thrower {
        put(&mut tag, "Thrower", Tag::IntArray(thrower.to_vec()));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"Thrower".into());
    }
    if let Some(target) = e.target {
        put(&mut tag, "Owner", Tag::IntArray(target.to_vec()));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"Owner".into());
    }
    tag
}
/// `PrimedTnt.readAdditionalSaveData`: `explosion_power`, 4 by default and
/// clamped to 0..128.
pub fn tnt_power(tag: &Tag) -> f32 {
    // `getFloatOr` accepts any numeric tag, not only a float.
    let power = match tag.get("explosion_power") {
        Some(Tag::Byte(v)) => f32::from(*v),
        Some(Tag::Short(v)) => f32::from(*v),
        Some(Tag::Int(v)) => *v as f32,
        Some(Tag::Long(v)) => *v as f32,
        Some(Tag::Float(bits)) => f32::from_bits(*bits),
        Some(Tag::Double(bits)) => f64::from_bits(*bits) as f32,
        _ => 4.0,
    };
    power.clamp(0.0, 128.0)
}

/// `PrimedTnt.addAdditionalSaveData`: its `fuse` and, when it is not the
/// default, its `explosion_power`, the encoded live block state, and its
/// `owner` (`EntityReference.store`). With no live owner, a loaded `owner`
/// stays as the raw compound has it.
#[allow(clippy::too_many_arguments)]
pub fn tnt_to_nbt(
    position: Vec3,
    motion: Vec3,
    on_ground: bool,
    fall_distance: f64,
    fuse: i32,
    power: f32,
    block_state: Tag,
    rotation: [f32; 2],
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    raw: Option<&Tag>,
) -> Tag {
    let mut tag = Base {
        id: "minecraft:tnt",
        uuid,
        position,
        motion,
        rotation,
        on_ground,
        fall_distance,
    }
    .tag(raw);
    put(&mut tag, "fuse", Tag::Short(fuse as i16));
    if power != 4.0 {
        put(&mut tag, "explosion_power", Tag::Float(power.to_bits()));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"explosion_power".into());
    }
    put(&mut tag, "block_state", block_state);
    if let Some(owner) = owner {
        put(&mut tag, "owner", Tag::IntArray(owner.to_vec()));
    }
    tag
}
/// `PrimedTnt.readAdditionalSaveData`: `EntityReference.read(input, "owner")`.
pub fn tnt_owner(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("owner"))
}
/// `EvokerFangs.addAdditionalSaveData`: always save the live warmup counter.
/// Known Owner UUIDs use `EntityReference.store` encoding; unresolved
/// loaded Owner data remains in the raw compound.
pub fn evoker_fangs_to_nbt(
    position: Vec3,
    yaw: f32,
    warmup: i32,
    owner: Option<[i32; 4]>,
    uuid: [i32; 4],
    raw: Option<&Tag>,
) -> Tag {
    let mut tag = Base {
        id: "minecraft:evoker_fangs",
        uuid,
        position,
        motion: raw.map(motion).unwrap_or(Vec3::ZERO),
        rotation: [yaw, raw.map(rotation).unwrap_or([0.0; 2])[1]],
        on_ground: raw.is_some_and(|t| flag(t, "OnGround")),
        fall_distance: raw.map_or(0.0, |t| ground_from_nbt(t).1),
    }
    .tag(raw);
    put(&mut tag, "Warmup", Tag::Int(warmup));
    if let Some(owner) = owner {
        put(&mut tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
    tag
}

/// `UUIDUtil.CODEC` via `NbtOps.getIntStream`: boxed Number conversions,
/// with the first-four partial value retained by `TagValueInput.read`.
fn item_uuid_from_tag(tag: Option<&Tag>) -> Option<[i32; 4]> {
    let values = sunrise_world::datafix::dynamic::as_int_stream(tag)?;
    values.get(..4)?.try_into().ok()
}

/// The `Entity` fields a saved non-mob entity carries (`Base::tag`), for a
/// caller that writes its own kind's fields around them.
pub fn entity_base_tag(
    id: &str,
    position: Vec3,
    motion: Vec3,
    rotation: [f32; 2],
    uuid: [i32; 4],
    raw: Option<&Tag>,
) -> Tag {
    Base {
        id,
        uuid,
        position,
        motion,
        rotation,
        on_ground: raw.is_some_and(|t| flag(t, "OnGround")),
        fall_distance: raw.map_or(0.0, |t| ground_from_nbt(t).1),
    }
    .tag(raw)
}

/// `ItemEntity.readAdditionalSaveData`: `getShortOr` (floored, then narrowed) for three;
/// Owner and Thrower use the numeric UUID codec.
pub fn item_entity_from_nbt(tag: &Tag, id: u32, bob_offset: f32) -> Option<ItemEntity> {
    let stack = item_from_nbt(tag.get("Item")?)?;
    let position = position(tag)?;
    let short = |key, default| match tag.get(key) {
        Some(Tag::Float(v)) => f32::from_bits(*v).floor() as i32 as i16,
        Some(Tag::Double(v)) => f64::from_bits(*v).floor() as i32 as i16,
        other => number(other).unwrap_or(default) as i16,
    };
    Some(ItemEntity {
        attachment_lifetime: Default::default(),
        rotation: rotation(tag),
        join: Default::default(),
        no_gravity: flag(tag, "NoGravity"),
        health: i32::from(short("Health", 5)),
        fire_ticks: i32::from(short("Fire", 0)),
        id,
        stack,
        thrower: item_uuid_from_tag(tag.get("Thrower")),
        target: item_uuid_from_tag(tag.get("Owner")),
        position,
        previous: position,
        velocity: motion(tag),
        ground: sunrise_world::ground_support::GroundState::from_ground_flag(flag(tag, "OnGround")),
        vertical_collision: false,
        // Entity.load: getDoubleOr, any number widened.
        fall_distance: float(tag.get("fall_distance")).unwrap_or(0.0),
        age: i32::from(short("Age", 0)),
        pickup_delay: i64::from(short("PickupDelay", 0)),
        bob_offset,
        ticks: 0,
        needs_sync: false,
        stuck: Vec3::ZERO,
        was_touching_water: false,
        // The caller gives it the own random that drew its UUID and bobOffs.
        random: sunrise_core::random::LegacyRandom::new(0),
    })
}

/// `OminousItemSpawner.addAdditionalSaveData`: its item (left out when
/// empty) and `spawn_item_after_ticks`. Its tick count is the entity's
/// `tickCount`, which isn't saved: a reloaded spawner counts from 0 again.
/// It never moves or turns: it saves the Motion, Rotation, OnGround and
/// fall_distance it was loaded with (`Entity.load`).
pub fn ominous_item_spawner_to_nbt(
    item: &crate::items::ItemStack,
    position: Vec3,
    spawn_item_after: i64,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Tag {
    let base = Base {
        id: "minecraft:ominous_item_spawner",
        uuid,
        position,
        motion: raw.map_or(Vec3::ZERO, loaded_motion),
        rotation: raw.map_or([0.0, 0.0], rotation),
        on_ground: raw.is_some_and(|t| flag(t, "OnGround")),
        fall_distance: float(raw.and_then(|t| t.get("fall_distance"))).unwrap_or(0.0),
    };
    let mut tag = base.tag(raw);
    // `ItemStack.CODEC` can't encode `ItemStack.EMPTY`: an empty item writes no key.
    if let Tag::Compound(map) = &mut tag {
        map.remove(&"item".into());
    }
    if &*item.item != "minecraft:air" {
        put(&mut tag, "item", item_to_nbt(item, items));
    }
    put(
        &mut tag,
        "spawn_item_after_ticks",
        Tag::Long(spawn_item_after),
    );
    tag
}
/// `OminousItemSpawner.readAdditionalSaveData`: the item, where it floats
/// and `spawn_item_after_ticks` (0 when absent). None without an item:
/// such a spawner would spawn nothing.
pub fn ominous_item_spawner_from_nbt(tag: &Tag) -> Option<(crate::items::ItemStack, Vec3, i64)> {
    // `readAdditionalSaveData`: `ItemStack.EMPTY` when it has none.
    let item = tag
        .get("item")
        .and_then(item_from_nbt)
        .unwrap_or_else(|| crate::items::ItemStack::new("minecraft:air", 1));
    let after = number(tag.get("spawn_item_after_ticks")).unwrap_or(0);
    Some((item, position(tag)?, after))
}

/// `ExperienceOrb`: Fire, Health, Age, Value, Count.
pub fn orb_to_nbt(o: &ExperienceOrb, uuid: [i32; 4]) -> Tag {
    let base = Base {
        id: "minecraft:experience_orb",
        uuid,
        position: o.position,
        motion: o.velocity,
        rotation: o.rotation,
        on_ground: o.ground.on_ground,
        fall_distance: o.fall_distance,
    };
    let mut tag = base.tag(None);
    for (key, value) in [
        ("Fire", Tag::Short(o.fire_ticks as i16)),
        ("Health", Tag::Short(o.health as i16)),
        ("Air", Tag::Short(o.air as i16)),
        // `putShort("Age", (short) age)`.
        ("Age", Tag::Short(o.age as i16)),
        ("Value", Tag::Short(o.value as i16)),
        ("Count", Tag::Int(o.count.min(i32::MAX as u32) as i32)),
    ] {
        put(&mut tag, key, value);
    }
    // Entity.saveWithoutId writes NoGravity only when set.
    if o.no_gravity {
        put(&mut tag, "NoGravity", Tag::Byte(1));
    }
    tag
}
/// One orb with its merged `Count`.
pub fn orbs_from_nbt(tag: &Tag) -> Vec<ExperienceOrb> {
    let Some(position) = position(tag) else {
        return Vec::new();
    };
    let count = number(tag.get("Count")).unwrap_or(1).max(1) as u32;
    let orb = ExperienceOrb {
        fire_ticks: number(tag.get("Fire")).unwrap_or(0) as i16 as i32,
        attachment_lifetime: Default::default(),
        fall_distance: float(tag.get("fall_distance")).unwrap_or(0.0),
        rotation: rotation(tag),
        join: Default::default(),
        no_gravity: flag(tag, "NoGravity"),
        id: 0,
        position,
        previous: position,
        velocity: motion(tag),
        // `getShortOr("Value", 0)`, unclamped.
        value: tag.get("Value").and_then(int_value).unwrap_or(0) as i16 as i32,
        // `getShortOr("Age", 0)`: a signed short, unclamped.
        age: tag.get("Age").and_then(int_value).unwrap_or(0) as i16 as i32,
        ground: sunrise_world::ground_support::GroundState::from_ground_flag(flag(tag, "OnGround")),
        count,
        ticks: 0,
        health: number(tag.get("Health")).unwrap_or(5) as i32,
        air: number(tag.get("Air")).map_or(300, |air| air as i32), // getIntOr("Air", max)
        // ExperienceOrb's followingPlayer is transient: never saved, so a
        // loaded orb starts without one.
        following: None,
    };
    vec![orb]
}

/// `Projectile` + `AbstractArrow.addAdditionalSaveData` of a live arrow;
/// `in_block` is its `inBlockState`, `raw` the compound it was loaded with.
pub fn arrow_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    in_block: Option<Tag>,
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Option<Tag> {
    let a = p.arrow.as_deref()?;
    let ProjectileKind::Arrow {
        base_damage,
        critical,
        ..
    } = p.kind
    else {
        return None;
    };
    let base = Base {
        // A spectral arrow is its own entity type.
        id: if a.spectral || &*a.pickup_item.item == "minecraft:spectral_arrow" {
            "minecraft:spectral_arrow"
        } else {
            "minecraft:arrow"
        },
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: p.ground.on_ground,
        fall_distance: a.fall_distance,
    };
    let mut tag = base.tag(raw);
    // `SpectralArrow.addAdditionalSaveData` always writes its integer duration.
    if base.id == "minecraft:spectral_arrow" {
        put(&mut tag, "Duration", Tag::Int(a.spectral_duration));
    }
    for (key, value) in [
        ("Fire", Tag::Short(a.fire_ticks as i16)),
        ("Air", Tag::Short(p.air_supply as i16)),
        ("life", Tag::Short(a.life as i16)),
        ("shake", Tag::Byte(a.shake as i8)),
        ("inGround", Tag::Byte(i8::from(a.in_ground))),
        ("pickup", Tag::Byte(a.pickup.byte())),
        ("damage", Tag::Double(base_damage.to_bits())),
        ("crit", Tag::Byte(i8::from(critical))),
        ("PierceLevel", Tag::Byte(a.pierce as i8)),
        ("HasBeenShot", Tag::Byte(i8::from(a.has_been_shot))),
        ("item", item_to_nbt(&a.pickup_item, items)),
    ] {
        put(&mut tag, key, value);
    }
    if let Tag::Compound(map) = &mut tag {
        let sound = a.saved_sound_event(map.get(&"SoundEvent".into()));
        map.insert("SoundEvent".into(), sound);
        // Projectile: LeftOwner only when set; Owner as the UUID.
        map.remove(&"LeftOwner".into());
        map.remove(&"inBlockState".into());
        map.remove(&"weapon".into());
        map.remove(&"can_break".into());
    }
    if let Some(weapon) = &a.fired_from_weapon {
        put(&mut tag, "weapon", item_to_nbt(weapon, items));
    }
    // `Projectile.addAdditionalSaveData`: the actual decoded predicate,
    // never a stale source compound or the pickup item's component patch.
    if let Some(predicate) = &a.can_break {
        put(&mut tag, "can_break", predicate.clone());
    }
    if p.left_owner {
        put(&mut tag, "LeftOwner", Tag::Byte(1));
    }
    if let Some(state) = in_block {
        put(&mut tag, "inBlockState", state);
    }
    // Entity.saveWithoutId writes NoGravity only when set.
    if p.no_gravity {
        put(&mut tag, "NoGravity", Tag::Byte(1));
    }
    if let Some(owner) = owner {
        put(&mut tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
    Some(tag)
}
/// A saved arrow as a live one (`readAdditionalSaveData`), with its
/// `inBlockState` compound for the caller to resolve. `effect` is the tipped
/// arrow's lasting effect from its item.
pub fn arrow_from_nbt(
    tag: &Tag,
    player: Option<[i32; 4]>,
    effect: impl FnOnce(&crate::items::ItemStack) -> Option<(&'static str, i32)>,
) -> Option<(Projectile, Option<Tag>)> {
    let position = position(tag)?;
    let owner = uuid_from_tag(tag.get("Owner"));
    let from_player = player.is_some() && owner == player;
    // `getDefaultPickupItem`: a `SpectralArrow` is picked up as its own item.
    let spectral = kind(tag).as_deref() == Some("minecraft:spectral_arrow");
    let default_item = if spectral {
        "minecraft:spectral_arrow"
    } else {
        "minecraft:arrow"
    };
    let item = tag
        .get("item")
        .and_then(item_from_nbt)
        .unwrap_or_else(|| crate::items::ItemStack::new(default_item, 1));
    let kind = ProjectileKind::Arrow {
        base_damage: float(tag.get("damage")).unwrap_or(2.0),
        critical: flag(tag, "crit"),
        effect: effect(&item),
    };
    let mut p = Projectile::new(kind, position, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = rotation(tag);
    p.left_owner = flag(tag, "LeftOwner");
    p.owner_uuid = owner;
    p.no_gravity = flag(tag, "NoGravity");
    p.ground.on_ground = flag(tag, "OnGround");
    projectile_fields::load_arrow_entity_fields(&mut p, tag);
    let a = p.arrow.as_mut()?;
    a.life = number(tag.get("life")).unwrap_or(0) as i16 as i32;
    a.shake = (number(tag.get("shake")).unwrap_or(0) as i8 as u8).into();
    a.in_ground = flag(tag, "inGround");
    a.pickup = Pickup::from_byte(number(tag.get("pickup")).unwrap_or(0));
    a.pierce = number(tag.get("PierceLevel")).unwrap_or(0) as u8;
    a.has_been_shot = flag(tag, "HasBeenShot");
    // `Projectile.readAdditionalSaveData`: one block predicate or a nonempty
    // list. Unsupported conjuncts remain saved, but cannot grant permission.
    a.can_break = tag
        .get("can_break")
        .filter(|predicate| match predicate {
            Tag::Compound(_) => true,
            Tag::List(list) => {
                !list.is_empty() && list.iter().all(|p| matches!(p, Tag::Compound(_)))
            }
            _ => false,
        })
        .cloned();
    a.spectral = spectral;
    // `SpectralArrow.readAdditionalSaveData`: absent Duration defaults to 200.
    if spectral {
        a.spectral_duration = number(tag.get("Duration")).unwrap_or(200) as i32;
    }
    a.pickup_item = item;
    a.load_weapon_and_sound(tag, item_from_nbt);
    p.stuck = a.in_ground;
    Some((p, tag.get("inBlockState").cloned()))
}

/// `LargeFireball`, `SmallFireball` and `WitherSkull`: `AbstractHurtingProjectile`'s
/// `acceleration_power`, `Projectile`'s owner and flags, `ExplosionPower`,
/// and a skull's `dangerous`.
pub fn fireball_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Option<Tag> {
    let power = match p.kind {
        ProjectileKind::LargeFireball { power } => Some(power),
        ProjectileKind::SmallFireball
        | ProjectileKind::DragonFireball
        | ProjectileKind::WindCharge
        | ProjectileKind::BreezeWindCharge
        | ProjectileKind::WitherSkull { .. } => None,
        _ => return None,
    };
    let base = Base {
        id: p.kind.id(),
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: p.ground.on_ground,
        fall_distance: p.fall_distance,
    };
    let mut tag = base.tag(raw);
    put(
        &mut tag,
        "acceleration_power",
        Tag::Double(p.acceleration.to_bits()),
    );
    put_projectile(&mut tag, p, owner);
    if projectile_fields::keeps_entity_fields(p) {
        put_projectile_entity_fields(&mut tag, p);
    }
    if let Some(power) = power {
        put(&mut tag, "ExplosionPower", Tag::Byte(power as i8));
    }
    // WitherSkull.addAdditionalSaveData: `dangerous`, false for a black skull.
    if let ProjectileKind::WitherSkull { dangerous } = p.kind {
        put(&mut tag, "dangerous", Tag::Byte(i8::from(dangerous)));
    }
    // Fireball.addAdditionalSaveData: the item `getItem` returns, a fire
    // charge unless `setItem` was given another (kept as a single one).
    if let Some(item) = &p.item {
        put(&mut tag, "Item", item_to_nbt(item, items));
    }
    Some(tag)
}

/// `Projectile.addAdditionalSaveData`: Owner, LeftOwner only when set, and
/// the explicit HasBeenShot flag.
fn put_projectile(tag: &mut Tag, p: &Projectile, owner: Option<[i32; 4]>) {
    if let Tag::Compound(map) = tag {
        map.remove(&"LeftOwner".into());
    }
    if p.left_owner {
        put(tag, "LeftOwner", Tag::Byte(1));
    }
    // Entity.saveWithoutId writes NoGravity only when set.
    if p.no_gravity {
        put(tag, "NoGravity", Tag::Byte(1));
    }
    put(tag, "HasBeenShot", Tag::Byte(i8::from(p.has_been_shot)));
    if let Some(owner) = owner {
        put(tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
}

/// The thrown item projectiles Sunrise saves: each entity type's id is also
/// the item it saves when none was loaded (`getDefaultItem`).
const THROWABLE_ITEMS: [&str; 6] = [
    "minecraft:snowball",
    "minecraft:egg",
    "minecraft:ender_pearl",
    "minecraft:splash_potion",
    "minecraft:lingering_potion",
    "minecraft:experience_bottle",
];
/// The item a throwable flies as: its stack (a potion's), the mod type's
/// item, or the item its entity type is named after.
fn throwable_item(kind: &ProjectileKind) -> Option<&str> {
    match kind {
        ProjectileKind::Thrown(t) => Some(t.item),
        ProjectileKind::Potion(stack, _) => Some(&stack.item),
        _ => THROWABLE_ITEMS.iter().find(|id| **id == kind.id()).copied(),
    }
}

/// The thrown kind an entity id names: vanilla's, or a content mod's type.
/// A potion flies as the item its entity type names, which is the item a
/// summoned one has when it names none (`getDefaultItem`).
pub fn throwable_kind(id: &str) -> Option<ProjectileKind> {
    if !THROWABLE_ITEMS.contains(&id) {
        return crate::projectiles::thrown_type(id).map(ProjectileKind::Thrown);
    }
    match id {
        "minecraft:snowball" => Some(ProjectileKind::Snowball),
        "minecraft:egg" => Some(ProjectileKind::Egg),
        "minecraft:ender_pearl" => Some(ProjectileKind::EnderPearl),
        // The entity type is the id; the `Item` read later may be another potion.
        "minecraft:experience_bottle" => Some(ProjectileKind::ExperienceBottle),
        _ => Some(ProjectileKind::Potion(
            Box::new(crate::items::ItemStack::new(id, 1)),
            id == "minecraft:lingering_potion",
        )),
    }
}

/// Whether its chunk saves it: the kinds `arrow_to_nbt`, `fireball_to_nbt`
/// and [`throwable_to_nbt`] write.
pub fn saved_with_chunk(p: &Projectile) -> bool {
    // No wildcard: a kind added to `ProjectileKind` must say here whether its
    // chunk saves it (`Entity.shouldBeSaved` is true for every projectile in
    // vanilla), and a `true` needs a writer in `entity_chunks`.
    match &p.kind {
        ProjectileKind::LargeFireball { .. }
        | ProjectileKind::SmallFireball
        | ProjectileKind::DragonFireball
        | ProjectileKind::WindCharge
        | ProjectileKind::BreezeWindCharge
        | ProjectileKind::WitherSkull { .. }
        | ProjectileKind::Snowball
        | ProjectileKind::Egg
        | ProjectileKind::EnderPearl
        | ProjectileKind::Potion(..)
        | ProjectileKind::ExperienceBottle
        | ProjectileKind::LlamaSpit
        | ProjectileKind::ShulkerBullet { .. }
        | ProjectileKind::Thrown(_) => true,
        ProjectileKind::Arrow { .. } => p.arrow.is_some(),
        // Not written yet: tridents save from their own list. Wither skulls and
        // bullets are written now.
        ProjectileKind::Trident => false,
    }
}

/// `ThrowableItemProjectile.addAdditionalSaveData`: `Projectile`'s keys and
/// the `Item` it flies as.
pub fn throwable_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Option<Tag> {
    let item = throwable_item(&p.kind)?;
    let base = Base {
        id: p.kind.id(),
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: p.ground.on_ground,
        fall_distance: p.fall_distance,
    };
    let mut tag = base.tag(raw);
    put_projectile(&mut tag, p, owner);
    put_projectile_entity_fields(&mut tag, p);
    // Entity.saveWithoutId writes NoGravity only when set.
    if p.no_gravity {
        put(&mut tag, "NoGravity", Tag::Byte(1));
    }
    // the loaded item stays (blue egg, potion effects) at `setItem`'s count 1.
    if let Tag::Compound(m) = &mut tag {
        let st = match &p.kind {
            ProjectileKind::Potion(s, _) => (**s).clone(),
            _ => crate::items::ItemStack::new(item, 1),
        };
        if let Tag::Compound(i) = m
            .entry("Item".into())
            .or_insert_with(|| item_to_nbt(&st, items))
        {
            i.insert("count".into(), Tag::Int(1));
        }
    }
    Some(tag)
}

/// `ThrowableItemProjectile.readAdditionalSaveData` for the kinds
/// [`throwable_to_nbt`] writes; the owner is the player when it is theirs.
pub fn throwable_from_nbt(tag: &Tag, player: Option<[i32; 4]>) -> Option<Projectile> {
    let mut kind = throwable_kind(&kind(tag)?)?;
    if let ProjectileKind::Potion(stack, _) = &mut kind
        && let Some(item) = tag.get("Item").and_then(item_from_nbt)
    {
        **stack = item;
        stack.count = 1;
    }
    let owner = owner_uuid(tag);
    let from_player = player.is_some() && owner == player;
    let mut p = Projectile::new(kind, position(tag)?, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = rotation(tag);
    p.left_owner = flag(tag, "LeftOwner");
    p.has_been_shot = flag(tag, "HasBeenShot");
    p.owner_uuid = owner;
    p.no_gravity = flag(tag, "NoGravity");
    load_projectile_entity_fields(&mut p, tag);
    p.ground.on_ground = flag(tag, "OnGround");
    Some(p)
}

/// `LlamaSpit` and `ShulkerBullet` as `/summon` or a chunk loads them, with
/// `ShulkerBullet.readAdditionalSaveData`'s `Dir`, `Steps` and per-axis target
/// deltas: `Projectile`'s `Owner`, `LeftOwner`, `NoGravity` and the motion.
/// `target` is the entity the caller resolved from `Target`; a bullet loaded
/// without one falls (`ShulkerBullet.tick`).
pub fn spit_or_bullet_from_nbt(
    tag: &Tag,
    player: Option<[i32; 4]>,
    target: Option<Who>,
) -> Option<Projectile> {
    let at = position(tag)?;
    let kind = match kind(tag)?.as_str() {
        "minecraft:llama_spit" => ProjectileKind::LlamaSpit,
        "minecraft:shulker_bullet" => ProjectileKind::ShulkerBullet {
            target,
            // `Dir`: `Direction.LEGACY_ID_CODEC`, the step the bullet is on,
            // absent while it has not chosen one. A byte read through
            // `Direction.from3DDataValue`, which wraps it: `abs(d % 6)`.
            dir: number(tag.get("Dir")).map(|d| ((d as i8) % 6).unsigned_abs()),
            // `getIntOr("Steps", 0)`: any int, a negative one included.
            steps: number(tag.get("Steps")).unwrap_or(0) as i32,
            delta: [
                float(tag.get("TXD")).unwrap_or(0.0),
                float(tag.get("TYD")).unwrap_or(0.0),
                float(tag.get("TZD")).unwrap_or(0.0),
            ],
            // `Entity.random` is not saved: a loaded bullet gets a stream of
            // its own from where it is, as `Projectile::new` does for an
            // arrow.
            seed: (at.x.to_bits() ^ at.z.to_bits().rotate_left(17)) | 1,
        },
        _ => return None,
    };
    let owner = owner_uuid(tag);
    let from_player = player.is_some() && owner == player;
    let mut p = Projectile::new(kind, at, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = rotation(tag);
    p.owner_uuid = owner;
    projectile_fields::load_projectile_flags_and_entity_fields(&mut p, tag);
    p.ground.on_ground = flag(tag, "OnGround");
    Some(p)
}

/// The save of [`spit_or_bullet_from_nbt`]'s kinds: `Projectile`'s keys, and
/// for a bullet the `Target`, `Dir`, `Steps` and target deltas vanilla writes.
pub fn spit_or_bullet_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    target: Option<[i32; 4]>,
    raw: Option<&Tag>,
) -> Option<Tag> {
    if !matches!(
        p.kind,
        ProjectileKind::LlamaSpit | ProjectileKind::ShulkerBullet { .. }
    ) {
        return None;
    }
    let base = Base {
        id: p.kind.id(),
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: p.ground.on_ground,
        fall_distance: p.fall_distance,
    };
    let mut tag = base.tag(raw);
    put_projectile(&mut tag, p, owner);
    put_projectile_entity_fields(&mut tag, p);
    // `ShulkerBullet.addAdditionalSaveData`: the target it homes on (written
    // only while it has one), the step it is on (omitted while it has not
    // chosen one), the steps left and the per-axis target deltas.
    if let ProjectileKind::ShulkerBullet {
        dir, steps, delta, ..
    } = &p.kind
    {
        if let Tag::Compound(map) = &mut tag {
            map.remove(&"Target".into());
            map.remove(&"Dir".into());
        }
        if let Some(target) = target {
            put(&mut tag, "Target", uuid_tag(target));
        }
        if let Some(dir) = dir {
            put(&mut tag, "Dir", Tag::Byte(*dir as i8));
        }
        put(&mut tag, "Steps", Tag::Int(*steps));
        for (key, v) in [("TXD", delta[0]), ("TYD", delta[1]), ("TZD", delta[2])] {
            put(&mut tag, key, Tag::Double(v.to_bits()));
        }
    }
    Some(tag)
}

/// `ShulkerBullet.readAdditionalSaveData`'s `Target`: the UUID of the entity
/// the bullet homes on, which vanilla keeps even while that entity is away.
pub fn bullet_target(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("Target"))
}

/// `Projectile`'s `Owner`.
pub fn owner_uuid(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("Owner"))
}

/// `Fireball.readAdditionalSaveData` (and, around it, `Projectile`'s Owner and
/// LeftOwner and `AbstractHurtingProjectile`'s acceleration power): a fireball
/// also reads the item it is drawn as through `setItem`, whose default is the
/// fire charge; a wind charge is no `Fireball` and has none.
pub fn fireball_from_nbt(tag: &Tag, player: Option<[i32; 4]>) -> Option<Projectile> {
    let kind = match kind(tag)?.as_str() {
        "minecraft:fireball" => ProjectileKind::LargeFireball {
            power: i32::from(number(tag.get("ExplosionPower")).unwrap_or(1) as i8),
        },
        "minecraft:small_fireball" => ProjectileKind::SmallFireball,
        "minecraft:dragon_fireball" => ProjectileKind::DragonFireball,
        "minecraft:wind_charge" => ProjectileKind::WindCharge,
        "minecraft:breeze_wind_charge" => ProjectileKind::BreezeWindCharge,
        // WitherSkull.readAdditionalSaveData: an absent `dangerous` is a black skull.
        "minecraft:wither_skull" => ProjectileKind::WitherSkull {
            dangerous: flag(tag, "dangerous"),
        },
        _ => return None,
    };
    let owner = uuid_from_tag(tag.get("Owner"));
    let from_player = player.is_some() && owner == player;
    let mut p = Projectile::new(kind, position(tag)?, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = rotation(tag);
    p.left_owner = flag(tag, "LeftOwner");
    p.has_been_shot = flag(tag, "HasBeenShot");
    p.owner_uuid = owner;
    p.no_gravity = flag(tag, "NoGravity");
    if projectile_fields::keeps_entity_fields(&p) {
        load_projectile_entity_fields(&mut p, tag);
    }
    p.ground.on_ground = flag(tag, "OnGround");
    p.fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
    p.acceleration = float(tag.get("acceleration_power")).unwrap_or(0.1);
    if let Some(item) = tag.get("Item").and_then(item_from_nbt) {
        p.set_item(item);
    }
    Some(p)
}

/// `FireworkRocketEntity.readAdditionalSaveData`: Life, LifeTime (0 when
/// absent: a bare `/summon` explodes at once), FireworksItem and
/// ShotAtAngle, with `Projectile`'s Owner and LeftOwner.
pub struct RocketNbt {
    pub position: Vec3,
    pub motion: Vec3,
    pub life: i32,
    pub lifetime: i32,
    pub item: Option<crate::items::ItemStack>,
    pub angled: bool,
    pub owner: Option<Who>,
    /// The saved `Owner`, whoever it names.
    pub owner_uuid: Option<[i32; 4]>,
    pub left_owner: bool,
    pub on_ground: bool,
    /// `Entity.load`'s Rotation.
    pub rotation: [f32; 2],
    /// `Projectile.hasBeenShot`: false until its first tick posts `projectile_shoot`.
    pub has_been_shot: bool,
    /// `Entity.isSilent`: no launch sound.
    pub silent: bool,
    /// `Entity.load`'s fall_distance.
    pub fall_distance: f64,
}
pub fn firework_rocket_from_nbt(tag: &Tag, player: Option<[i32; 4]>) -> Option<RocketNbt> {
    let owner = uuid_from_tag(tag.get("Owner"));
    Some(RocketNbt {
        position: position(tag)?,
        motion: motion(tag),
        life: number(tag.get("Life")).unwrap_or(0) as i32,
        lifetime: number(tag.get("LifeTime")).unwrap_or(0) as i32,
        item: tag.get("FireworksItem").and_then(item_from_nbt),
        angled: flag(tag, "ShotAtAngle"),
        owner: (player.is_some() && owner == player).then_some(Who::Player),
        owner_uuid: owner,
        left_owner: flag(tag, "LeftOwner"),
        on_ground: flag(tag, "OnGround"),
        rotation: rotation(tag),
        has_been_shot: flag(tag, "HasBeenShot"),
        silent: flag(tag, "Silent"),
        fall_distance: float(tag.get("fall_distance")).unwrap_or(0.0),
    })
}

/// What `FireworkRocketEntity.addAdditionalSaveData` writes, over `Projectile`'s
/// Owner and LeftOwner.
pub struct RocketSave<'a> {
    pub rotation: [f32; 2],
    pub position: Vec3,
    pub motion: Vec3,
    pub life: i32,
    pub lifetime: i32,
    pub item: &'a crate::items::ItemStack,
    pub angled: bool,
    pub owner: Option<[i32; 4]>,
    pub left_owner: bool,
    /// `Entity.onGround`, from the rocket's last `Entity.move`.
    pub on_ground: bool,
    pub has_been_shot: bool,
    /// `Entity.fallDistance`.
    pub fall_distance: f64,
}
pub fn firework_rocket_to_nbt(
    r: &RocketSave,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Tag {
    let mut tag = Base {
        id: "minecraft:firework_rocket",
        uuid,
        position: r.position,
        motion: r.motion,
        rotation: r.rotation,
        on_ground: r.on_ground,
        fall_distance: r.fall_distance,
    }
    .tag(raw);
    for (key, value) in [
        ("Life", Tag::Int(r.life)),
        ("LifeTime", Tag::Int(r.lifetime)),
        ("FireworksItem", item_to_nbt(r.item, items)),
        ("ShotAtAngle", Tag::Byte(i8::from(r.angled))),
        ("HasBeenShot", Tag::Byte(i8::from(r.has_been_shot))),
    ] {
        put(&mut tag, key, value);
    }
    if r.left_owner {
        put(&mut tag, "LeftOwner", Tag::Byte(1));
    }
    if let Some(owner) = r.owner {
        put(&mut tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
    tag
}

mod trident;
pub use trident::{TRIDENT_HIT_GROUND, TridentNbt, TridentSave, trident_from_nbt, trident_to_nbt};
mod hanging;
pub use hanging::*;

/// `FallingBlockEntity`: BlockState, Time, DropItem, HurtEntities.
#[allow(clippy::too_many_arguments)]
pub fn falling_block_to_nbt(
    state: Tag,
    position: Vec3,
    motion: Vec3,
    time: i32,
    drop_item: bool,
    damage: crate::falling_block::Damage,
    fall_distance: f64,
    on_ground: bool,
    rotation: [f32; 2],
    uuid: [i32; 4],
) -> Tag {
    let base = Base {
        id: "minecraft:falling_block",
        uuid,
        position,
        motion,
        rotation,
        on_ground,
        fall_distance,
    };
    let mut tag = base.tag(None);
    for (key, value) in [
        ("BlockState", state),
        ("Time", Tag::Int(time)),
        ("DropItem", Tag::Byte(i8::from(drop_item))),
    ] {
        put(&mut tag, key, value);
    }
    damage.save(&mut tag);
    tag
}
/// (block state tag, position, motion, time, fall distance)
pub fn falling_block_from_nbt(tag: &Tag) -> Option<(Tag, Vec3, Vec3, i32, f64)> {
    Some((
        tag.get("BlockState")
            .cloned()
            .unwrap_or_else(|| string("minecraft:sand")),
        position(tag)?,
        motion(tag),
        number(tag.get("Time")).unwrap_or(0) as i32,
        float(tag.get("fall_distance")).unwrap_or(0.0),
    ))
}

/// FallingBlockEntity.readAdditionalSaveData uses the modern state codec.
pub fn falling_state_from_nbt(registry: &BlockRegistry, tag: &Tag) -> Option<BlockStateId> {
    BlockState::from_nbt(tag)
        .ok()
        .and_then(|mut state| {
            state.name = sunrise_core::identifier::Identifier::parse(&state.name)
                .ok()?
                .to_string();
            registry
                .id(&state)
                .or_else(|| registry.default_state(&state.name))
        })
        .or_else(|| registry.default_state("minecraft:sand"))
}

/// `NoGravity` and `DropItem` (true when absent) of a saved falling block.
pub fn falling_block_flags(tag: &Tag) -> (bool, bool) {
    (flag(tag, "NoGravity"), flag_or(tag, "DropItem", true))
}

/// `Cushion.addAdditionalSaveData`: the block it sits on and its colour.
pub fn cushion_to_nbt(c: &Cushion, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    // A block-attached cushion never moves or turns: Motion and pitch stay as loaded.
    let (on_ground, fall_distance) = raw.map_or((false, 0.0), ground_from_nbt);
    let base = Base {
        id: "minecraft:cushion",
        uuid,
        position: c.position,
        motion: raw.map_or(Vec3::ZERO, loaded_motion),
        rotation: [c.yaw, raw.map_or(0.0, |raw| rotation(raw)[1])],
        on_ground,
        fall_distance,
    };
    let mut tag = base.tag(raw);
    put(&mut tag, "block_pos", Tag::IntArray(c.pos.to_vec()));
    put(&mut tag, "color", string(c.color_name()));
    if let Some(name) = &c.custom_name {
        put(&mut tag, "CustomName", name.clone());
    }
    tag
}
/// The position, block position, yaw and colour of a saved cushion.
pub fn cushion_from_nbt(tag: &Tag, id: u32) -> Option<Cushion> {
    let position = position(tag)?;
    let pos = match tag.get("block_pos") {
        Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
        _ => [
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        ],
    };
    let yaw = match tag.get("Rotation") {
        Some(Tag::List(l)) => l
            .first()
            .and_then(|v| crate::save::float(Some(v)))
            .unwrap_or(0.0) as f32,
        _ => 0.0,
    };
    let color =
        crate::entities::dye_index(&text(tag.get("color")).unwrap_or_else(|| "white".into()));
    Some(Cushion {
        attachment_lifetime: Default::default(),
        join: Default::default(),
        id,
        position,
        pos,
        yaw,
        color,
        check_interval: 0,
        custom_name: tag.get("CustomName").cloned(),
    })
}

#[cfg(test)]
mod cushion_tests;
/// `ItemFrame.setDirection`: a frame on a wall turns to face away from it
/// (`yRot` = the 2D data value times 90, `xRot` 0); one on a floor or a
/// ceiling looks straight down or up (`xRot` = -90 times the axis step).
/// `facing` is the 3D data value (down, up, north, south, west, east).
fn frame_rotation(facing: u8) -> [f32; 2] {
    match facing {
        0 => [0.0, 90.0],
        1 => [0.0, -90.0],
        2 => [180.0, 0.0],
        3 => [0.0, 0.0],
        4 => [90.0, 0.0],
        _ => [270.0, 0.0],
    }
}

/// ItemFrame.addAdditionalSaveData.
pub fn item_frame_to_nbt(
    f: &ItemFrame,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Tag {
    let (lo, hi) = f.bounds();
    let base = Base {
        id: f.kind(),
        uuid,
        position: Vec3::new(
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ),
        motion: raw.map_or(Vec3::ZERO, loaded_motion),
        rotation: frame_rotation(f.facing),
        on_ground: raw.is_some_and(|raw| flag(raw, "OnGround")),
        fall_distance: raw.map_or(0.0, |raw| ground_from_nbt(raw).1),
    };
    let mut tag = base.tag(raw);
    // `Entity.saveWithoutId`: `CustomName` before the frame's own fields.
    if let Some(name) = &f.custom_name {
        put(&mut tag, "CustomName", name.clone());
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"CustomName".into());
    }
    put(&mut tag, "Facing", Tag::Byte(f.facing as i8));
    put(&mut tag, "block_pos", Tag::IntArray(f.pos.to_vec()));
    put(&mut tag, "ItemRotation", Tag::Byte(f.rotation));
    put(
        &mut tag,
        "ItemDropChance",
        Tag::Float(f.drop_chance.to_bits()),
    );
    put(&mut tag, "Invisible", Tag::Byte(i8::from(f.invisible)));
    put(&mut tag, "Fixed", Tag::Byte(i8::from(f.fixed)));
    if let Tag::Compound(map) = &mut tag {
        map.remove(&sunrise_core::nbt::JavaString::from("Item"));
    }
    if let Some(item) = &f.item {
        put(&mut tag, "Item", item_to_nbt(item, items));
    }
    tag
}
/// `ItemFrame.readAdditionalSaveData`: an absent Facing defaults to Direction.DOWN.
pub fn item_frame_from_nbt(tag: &Tag, id: u32) -> Option<ItemFrame> {
    let pos = hanging::attached_block_pos(tag)?;
    Some(ItemFrame {
        attachment_lifetime: Default::default(),
        join: Default::default(),
        id,
        pos,
        facing: hanging::wrap_data_value(number(tag.get("Facing")).unwrap_or(0), 6),
        glow: kind(tag).as_deref() == Some("minecraft:glow_item_frame"),
        item: tag.get("Item").and_then(item_from_nbt),
        // `TagValueInput.getByteOr` narrows before `ItemFrame.setRotation`'s remainder.
        rotation: (tag.get("ItemRotation").and_then(int_value).unwrap_or(0) as i8) % 8,
        drop_chance: float(tag.get("ItemDropChance")).map_or(1.0, |v| v as f32),
        fixed: flag(tag, "Fixed"),
        invisible: flag(tag, "Invisible"),
        check_interval: 0,
        ticks: 0,
        custom_name: tag.get("CustomName").cloned(),
        random: crate::random_source::entity(),
    })
}
/// ArmorStand.addAdditionalSaveData with LivingEntity's `equipment`.
pub fn armor_stand_to_nbt(
    a: &ArmorStand,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Tag {
    armor_stand_tag(a, uuid, raw, items, true)
}

pub fn armor_stand_tag(
    a: &ArmorStand,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    items: &crate::items::Items,
    include_id: bool,
) -> Tag {
    let base = Base {
        id: "minecraft:armor_stand",
        uuid,
        position: a.position,
        motion: a.velocity,
        rotation: [a.yaw, a.pitch],
        on_ground: a.ground.on_ground,
        fall_distance: a.enchantment_fall_distance,
    };
    let mut out = EntityTagWriter::new(raw);
    out.saved
        .insert("Fire".into(), Tag::Short(a.fire_ticks as i16));
    for (key, value) in [
        ("NoGravity", a.no_gravity),
        ("CustomNameVisible", a.name_visible),
    ] {
        if value {
            out.saved.insert(key.into(), Tag::Byte(1));
        } else {
            out.saved.remove(&key.into());
        }
    }
    // A name from the save keeps its own shape unless it changed.
    if let Some(name) = &a.custom_name
        && !out
            .saved
            .get(&"CustomName".into())
            .is_some_and(|t| name_text(t) == *name)
    {
        out.saved.insert(
            "CustomName".into(),
            crate::scoreboard::json_to_tag(&name.component()),
        );
    }
    out.remove(&"NoAI".into());
    out.remove(&"PersistenceRequired".into());
    base.write(&mut out, include_id);

    // LivingEntity.addAdditionalSaveData.
    out.insert("Health".into(), Tag::Float(a.health.to_bits()));
    out.retain_or("HurtTime", || Tag::Short(0));
    out.retain_or("DeathTime", || Tag::Short(0));
    out.retain_or("AbsorptionAmount", || Tag::Float(0));
    out.retain_or("current_impulse_context_reset_grace_time", || Tag::Int(0));
    out.retain("current_explosion_impact_pos");
    out.retain_or("attributes", || Tag::List(Vec::new()));
    // `removeFrost`'s first-tick movement_speed, over a saved list without it.
    if a.speed_instance
        && let Some(Tag::List(list)) = out.fields.get_mut(&"attributes".into())
        && !list
            .iter()
            .any(|t| text(t.get("id")).as_deref() == Some(SPEED_ATTRIBUTE))
    {
        list.push(Tag::compound([
            ("id", string(SPEED_ATTRIBUTE)),
            ("base", Tag::Double(0.7f64.to_bits())),
        ]));
    }
    put_live_attributes(&mut out.fields, &a.live);
    out.retain("active_effects");
    out.retain_or("FallFlying", || Tag::Byte(0));
    out.retain("sleeping_pos");
    out.retain_or("Brain", || {
        Tag::compound([("memories", Tag::Compound(Compound::new()))])
    });
    out.retain("last_hurt_by_player");
    out.retain("last_hurt_by_player_memory_time");
    out.retain("last_hurt_by_mob");
    out.retain("ticks_since_last_hurt_by_mob");
    if let Some(equipment) = a.equipment.write(items) {
        out.insert("equipment".into(), equipment);
    } else {
        out.remove(&"equipment".into());
    }
    out.retain("locator_bar_icon");

    // ArmorStand.addAdditionalSaveData.
    out.insert("Invisible".into(), Tag::Byte(i8::from(a.invisible)));
    out.insert("Small".into(), Tag::Byte(i8::from(a.small)));
    out.insert("ShowArms".into(), Tag::Byte(i8::from(a.show_arms)));
    out.insert("DisabledSlots".into(), Tag::Int(a.disabled_slots));
    out.insert("NoBasePlate".into(), Tag::Byte(i8::from(a.no_base_plate)));
    if a.marker {
        out.insert("Marker".into(), Tag::Byte(1));
    } else {
        out.remove(&"Marker".into());
    }
    // ArmorStand.addAdditionalSaveData: the `Pose` parts that are not default.
    let pose: Vec<(&str, Tag)> = ArmorStand::POSE_KEYS
        .into_iter()
        .zip(a.pose)
        .zip(ArmorStand::DEFAULT_POSE)
        .filter(|((_, rotation), default)| rotation != default)
        .map(|((key, rotation), _)| (key, floats(&rotation)))
        .collect();
    out.insert("Pose".into(), Tag::compound(pose));
    out.finish()
}
/// The attribute every living entity's first tick makes (`removeFrost`).
const SPEED_ATTRIBUTE: &str = "minecraft:movement_speed";
/// Vanilla 26.3 `Entity.setXRot`: finite pitch is clamped after modulo 360,
/// non-finite pitch resets to 0.0.
fn normalize_pitch(pitch: f32) -> f32 {
    if pitch.is_finite() {
        (pitch % 360.0).clamp(-90.0, 90.0)
    } else {
        0.0
    }
}

pub fn armor_stand_from_nbt(tag: &Tag, id: u32) -> Option<ArmorStand> {
    let rot = rotation(tag);
    let mut a = ArmorStand::new(id, position(tag)?, rot[0]);
    a.pitch = normalize_pitch(rot[1]);
    a.enchantment_fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
    // AttributeMap.load makes an instance for each saved attribute.
    a.speed_instance = matches!(tag.get("attributes"), Some(Tag::List(list))
        if list.iter().any(|attr| text(attr.get("id")).as_deref() == Some(SPEED_ATTRIBUTE)));
    a.velocity = loaded_motion(tag);
    a.health = float(tag.get("Health")).unwrap_or(20.0) as f32;
    a.fire_ticks = i32::from(number(tag.get("Fire")).unwrap_or(0) as i16);
    a.custom_name = tag.get("CustomName").map(name_text);
    a.name_visible = flag(tag, "CustomNameVisible");
    if let Some(eq) = tag.get("equipment") {
        a.equipment = sunrise_items::equipment::Equipment::read(eq);
    }
    a.live.load(tag.get("attributes"), |_| true);
    a.small = flag(tag, "Small");
    a.show_arms = flag(tag, "ShowArms");
    a.disabled_slots = number(tag.get("DisabledSlots")).unwrap_or(0) as i32;
    a.no_base_plate = flag(tag, "NoBasePlate");
    a.invisible = flag(tag, "Invisible");
    a.marker = flag(tag, "Marker");
    a.no_gravity = flag(tag, "NoGravity");
    a.ground.on_ground = flag(tag, "OnGround");
    // ArmorStand.readAdditionalSaveData: each part of `Pose` that is a list of
    // three numbers, the others staying at their defaults.
    if let Some(pose) = tag.get("Pose") {
        for (rotation, key) in a.pose.iter_mut().zip(ArmorStand::POSE_KEYS) {
            if let Some(Tag::List(values)) = pose.get(key)
                && values.len() == 3
            {
                for (r, v) in rotation.iter_mut().zip(values) {
                    *r = float(Some(v)).unwrap_or(0.0) as f32;
                }
            }
        }
    }
    Some(a)
}
/// `AbstractBoat.addAdditionalSaveData` / `ContainerEntity.addChestVehicleSaveData`:
/// only chest boats own container fields; a pending table replaces `Items`.
/// Ordinary slots use `ContainerHelper.saveAllItems` through the existing item encoder.
/// `ground` is the live `OnGround` and `fall_distance` (`Entity.saveWithoutId`).
#[allow(clippy::too_many_arguments)]
pub fn boat_to_nbt(
    id: &str,
    position: Vec3,
    motion: Vec3,
    rotation: [f32; 2],
    ground: (bool, f64),
    uuid: [i32; 4],
    raw: Option<&Tag>,
    items: Option<&[Option<crate::items::ItemStack>]>,
    catalog: &crate::items::Items,
) -> Tag {
    let mut tag = Base {
        id,
        uuid,
        position,
        motion,
        rotation,
        on_ground: ground.0,
        fall_distance: ground.1,
    }
    .non_mob_tag(raw);
    if let Tag::Compound(map) = &mut tag {
        map.remove(&"Items".into());
        if let Some(items) = items {
            let table = crate::save::text(map.get(&"LootTable".into()))
                .and_then(|id| sunrise_client_core::identifier::normalize_id(&id));
            if let Some(table) = table {
                map.insert("LootTable".into(), Tag::String(table.as_str().into()));
            } else {
                map.remove(&"LootTable".into());
                map.remove(&"LootTableSeed".into());
                map.insert("Items".into(), crate::save::encode_items(items, catalog));
            }
        } else {
            map.remove(&"LootTable".into());
            map.remove(&"LootTableSeed".into());
        }
    }
    tag
}
/// A vehicle's base entity tag (minecarts, the dragon and end crystals), with
/// the live `OnGround` and `fall_distance` in `ground`.
pub fn vehicle_to_nbt(
    id: &str,
    position: Vec3,
    motion: Vec3,
    rotation: [f32; 2],
    ground: (bool, f64),
    uuid: [i32; 4],
    raw: Option<&Tag>,
) -> Tag {
    Base {
        id,
        uuid,
        position,
        motion,
        rotation,
        on_ground: ground.0,
        fall_distance: ground.1,
    }
    .tag(raw)
}
/// `Entity.load`'s `OnGround` (`getBooleanOr`: `byteValue() != 0`) and
/// `fall_distance` (`getDoubleOr`: any number's `doubleValue`), defaulting to
/// false and 0.
pub fn ground_from_nbt(tag: &Tag) -> (bool, f64) {
    (
        flag(tag, "OnGround"),
        float(tag.get("fall_distance")).unwrap_or(0.0),
    )
}

use crate::mobs::villager::professions::save::villager_data;
/// `AbstractVillager` "Offers" (`MerchantOffers.CODEC`).
pub(crate) fn put_offers(map: &mut EntityTagWriter, m: &Mob, items: &crate::items::Items) {
    let Some(t) = m.extra.trade.as_ref().filter(|t| !t.unset) else {
        return;
    };
    map.insert("Offers".into(), offers_to_nbt(&t.offers, items));
}
/// `ItemCost.CODEC`: id, count (always written) and the exact components.
fn item_cost_to_nbt(stack: &crate::items::ItemStack) -> Tag {
    // ItemCost contains an exact component predicate, not a saved stack patch.
    let mut tag = Tag::from(stack);
    if let Tag::Compound(c) = &mut tag
        && !c.contains_key(&"count".into())
    {
        c.insert("count".into(), Tag::Int(stack.count as i32));
    }
    tag
}
/// `MerchantOffers.CODEC`: `{Recipes: [...]}`, each offer without the
/// fields at their defaults.
pub fn offers_to_nbt(
    offers: &[crate::merchant::MerchantOffer],
    items: &crate::items::Items,
) -> Tag {
    let recipes = offers
        .iter()
        .map(|o| {
            let mut entries = vec![
                ("buy", item_cost_to_nbt(&o.base_cost_a.stack)),
                ("sell", item_to_nbt(&o.result, items)),
            ];
            // `MerchantOffer.CODEC` omits fields at their defaults.
            if o.uses != 0 {
                entries.push(("uses", Tag::Int(o.uses)));
            }
            if o.max_uses != 4 {
                entries.push(("maxUses", Tag::Int(o.max_uses)));
            }
            if o.price_multiplier != 0.0 {
                entries.push(("priceMultiplier", Tag::Float(o.price_multiplier.to_bits())));
            }
            if o.xp != 1 {
                entries.push(("xp", Tag::Int(o.xp)));
            }
            if !o.reward_exp {
                entries.push(("rewardExp", Tag::Byte(0)));
            }
            if o.special_price_diff != 0 {
                entries.push(("specialPrice", Tag::Int(o.special_price_diff)));
            }
            if o.demand != 0 {
                entries.push(("demand", Tag::Int(o.demand)));
            }
            if let Some(b) = &o.cost_b {
                entries.push(("buyB", item_cost_to_nbt(&b.stack)));
            }
            Tag::compound(entries)
        })
        .collect::<Vec<_>>();
    // An empty list is the field's default: left out.
    if recipes.is_empty() {
        return Tag::Compound(Compound::new());
    }
    Tag::compound([("Recipes", Tag::List(recipes))])
}
/// The offers of a `MerchantOffers.CODEC` tag.
pub fn offers_from_nbt(tag: &Tag) -> Vec<crate::merchant::MerchantOffer> {
    use crate::merchant::{ItemCost, MerchantOffer};
    let Some(Tag::List(recipes)) = tag.get("Recipes") else {
        return Vec::new();
    };
    recipes
        .iter()
        .filter_map(|r| {
            let cost = |key| {
                r.get(key)
                    .and_then(item_from_nbt)
                    .map(|stack| ItemCost { stack })
            };
            Some(MerchantOffer {
                base_cost_a: cost("buy")?,
                cost_b: cost("buyB"),
                result: r.get("sell").and_then(item_from_nbt)?,
                uses: number(r.get("uses")).unwrap_or(0) as i32,
                max_uses: number(r.get("maxUses")).unwrap_or(4) as i32,
                reward_exp: number(r.get("rewardExp")).unwrap_or(1) != 0,
                special_price_diff: number(r.get("specialPrice")).unwrap_or(0) as i32,
                demand: number(r.get("demand")).unwrap_or(0) as i32,
                price_multiplier: float(r.get("priceMultiplier")).unwrap_or(0.0) as f32,
                xp: number(r.get("xp")).unwrap_or(1) as i32,
            })
        })
        .collect()
}
/// The villager brain's `job_site`, `home` and `meeting_point` memories.
fn put_memories(map: &mut EntityTagWriter, m: &Mob) {
    let mut memories: Vec<(&str, Tag)> = [
        ("minecraft:job_site", m.extra.home),
        ("minecraft:home", m.extra.bed),
        ("minecraft:meeting_point", m.extra.meeting),
    ]
    .into_iter()
    .filter_map(|(key, p)| {
        let pos = Tag::compound([
            ("dimension", string("minecraft:overworld")),
            ("pos", Tag::IntArray(p?.to_vec())),
        ]);
        Some((key, Tag::compound([("value", pos)])))
    })
    .collect();
    crate::mobs::villager::write_memories(m, &mut memories);
    map.insert(
        "Brain".into(),
        Tag::compound([("memories", Tag::compound(memories))]),
    );
}
fn read_villager(m: &mut Mob, tag: &Tag) {
    use crate::{merchant::Trader, mobs::render::VILLAGER_TYPES};
    if let Some(data) = tag.get("VillagerData") {
        let name = |key| text(data.get(key)).map(|s| s.trim_start_matches("minecraft:").to_owned());
        if let Some(t) = name("type").and_then(|t| VILLAGER_TYPES.iter().position(|v| *v == t)) {
            m.extra.variant = t as u8;
        }
        crate::mobs::villager::professions::save::read_identity(m, data);
        m.extra.villager_level = number(data.get("level"))
            .unwrap_or(1)
            .clamp(1, i64::from(i32::MAX)) as i32;
    }
    let memory = |key: &str| {
        tag.get("Brain")
            .and_then(|b| b.get("memories"))
            .and_then(|b| b.get(key))
            .and_then(|j| j.get("value"))
            .and_then(|v| match v.get("pos") {
                Some(Tag::IntArray(a)) if a.len() == 3 => Some([a[0], a[1], a[2]]),
                _ => None,
            })
    };
    m.extra.home = memory("minecraft:job_site");
    m.extra.bed = memory("minecraft:home");
    m.extra.meeting = memory("minecraft:meeting_point");
    if m.species() == crate::mobs::Species::Villager {
        crate::mobs::villager::read_nbt(m, tag);
    }
    if m.species() == crate::mobs::Species::WanderingTrader {
        // WanderingTrader.readAdditionalSaveData: both default to idle.
        m.extra.ints[5] = number(tag.get("DespawnDelay")).unwrap_or(0) as i32;
        m.extra.point = match tag.get("wander_target") {
            Some(Tag::IntArray(a)) if a.len() == 3 => {
                Some([f64::from(a[0]), f64::from(a[1]), f64::from(a[2])])
            }
            _ => None,
        };
    }
    let offers_tag = tag.get("Offers");
    let xp = number(tag.get("Xp")).unwrap_or(0) as i32;
    if offers_tag.is_none() && xp == 0 && tag.get("LastRestock").is_none() {
        return;
    }
    let offers = offers_tag.map(offers_from_nbt).unwrap_or_default();
    m.extra.trade = Some(Box::new(Trader {
        unset: offers_tag.is_none(),
        xp: number(tag.get("Xp")).unwrap_or(0) as i32,
        offers,
        last_restock: number(tag.get("LastRestock")).unwrap_or(0),
        restocks_today: number(tag.get("RestocksToday")).unwrap_or(0) as i32,
        check_day: 0,
    }));
}

#[cfg(test)]
mod tnt_power_tests;

#[cfg(test)]
mod spell_ticks_nbt_tests;

#[cfg(test)]
mod tests {
    /// `ItemEntity.readAdditionalSaveData` reads `Age` and `PickupDelay` with
    /// `getShortOr`: an int tag is narrowed (`IntTag.shortValue`,
    /// `(short)(value & 65535)`), so 40000 is -25536 and 65536 is 0.
    #[test]
    fn an_item_narrows_its_age_and_pickup_delay_to_a_short() {
        let load = |fields: &str| {
            let snbt =
                format!("{{Pos:[0d,0d,0d],Item:{{id:\"minecraft:stone\",count:1}},{fields}}}");
            let tag = sunrise_core::nbt::parse_snbt(&snbt).unwrap().0;
            let item = super::item_entity_from_nbt(&tag, 1, 0.0).unwrap();
            (item.age, item.pickup_delay)
        };
        assert_eq!(load("Age:40000,PickupDelay:65546"), (-25536, 10));
        assert_eq!(load("Age:65536,PickupDelay:40000"), (0, -25536));
        // What vanilla writes itself reads back as it was.
        assert_eq!(load("Age:-32768s,PickupDelay:32767s"), (-32768, 32767));
        assert_eq!(load("Age:5999s,PickupDelay:10s"), (5999, 10));
        // A float or double floors first (`Mth.floor`), as `Health` does.
        assert_eq!(load("Age:-0.5f,PickupDelay:9.9d"), (-1, 9));
        // The sentinels `ItemEntity.tick` tests for (`age != -32768`: never
        // ages; `pickupDelay != 32767`: never counts down), given as ints,
        // and the ints that narrow onto them.
        assert_eq!(load("Age:-32768,PickupDelay:32767"), (-32768, 32767));
        assert_eq!(load("Age:32768,PickupDelay:98303"), (-32768, 32767));
        // `Entity.load` reads the item's `Fire` with `getShortOr` as well.
        let fire = |fields: &str| {
            let snbt =
                format!("{{Pos:[0d,0d,0d],Item:{{id:\"minecraft:stone\",count:1}},{fields}}}");
            let tag = sunrise_core::nbt::parse_snbt(&snbt).unwrap().0;
            super::item_entity_from_nbt(&tag, 1, 0.0)
                .unwrap()
                .fire_ticks
        };
        assert_eq!(
            [fire("Fire:40000"), fire("Fire:60s"), fire("Age:1s")],
            [-25536, 60, 0]
        );
        // They save as the shorts vanilla writes.
        let tag = sunrise_core::nbt::parse_snbt(
            "{Pos:[0d,0d,0d],Item:{id:\"minecraft:stone\",count:1},Age:32768,PickupDelay:98303}",
        )
        .unwrap()
        .0;
        let item = super::item_entity_from_nbt(&tag, 1, 0.0).unwrap();
        let saved = super::item_entity_to_nbt(&item, [1; 4], None, &Default::default());
        assert_eq!(saved.get("Age"), Some(&super::Tag::Short(i16::MIN)));
        assert_eq!(saved.get("PickupDelay"), Some(&super::Tag::Short(i16::MAX)));
    }

    // Added after the actual old Game proof: live mutation and a fresh absent
    // load cannot inherit a predicate from a contradictory saved compound.
    #[test]
    fn arrow_can_break_mutation_clone_and_absent_load_save_actual_state() {
        let parse = |text: &str| sunrise_core::nbt::parse_snbt(text).unwrap().0;
        let raw = parse(
            "{id:\"minecraft:arrow\",Pos:[1d,200d,1d],NoGravity:1b,marker:37,can_break:{blocks:\"minecraft:chorus_flower\"}}",
        );
        let (mut arrow, _) = super::arrow_from_nbt(&raw, None, |_| None).unwrap();
        let original = raw.get("can_break").unwrap().clone();
        assert_eq!(
            arrow.arrow.as_ref().unwrap().can_break.as_ref(),
            Some(&original)
        );
        let replacement = parse("{blocks:\"minecraft:stone\"}");
        arrow.arrow.as_mut().unwrap().can_break = Some(replacement.clone());
        let cloned = arrow.clone();
        for source in [None, Some(&raw)] {
            let saved = super::arrow_to_nbt(
                &cloned,
                [197, 0, 0, 1],
                None,
                None,
                source,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(saved.get("can_break"), Some(&replacement));
            assert_eq!(saved.get("NoGravity"), Some(&super::Tag::Byte(1)));
        }
        arrow.arrow.as_mut().unwrap().can_break = None;
        let saved = super::arrow_to_nbt(
            &arrow,
            [197, 0, 0, 1],
            None,
            None,
            Some(&raw),
            &Default::default(),
        )
        .unwrap();
        assert!(saved.get("can_break").is_none());
        assert_eq!(saved.get("marker"), Some(&super::Tag::Int(37)));
        let absent = parse("{id:\"minecraft:arrow\",Pos:[1d,200d,1d]}");
        let (fresh, _) = super::arrow_from_nbt(&absent, None, |_| None).unwrap();
        assert!(fresh.arrow.as_ref().unwrap().can_break.is_none());
        assert!(
            super::arrow_to_nbt(
                &fresh,
                [197, 0, 0, 2],
                None,
                None,
                Some(&raw),
                &Default::default()
            )
            .unwrap()
            .get("can_break")
            .is_none()
        );
    }

    /// `EvokerFangs` extends `Entity` and never moves: `Entity.saveWithoutId`
    /// writes the `fall_distance` it loaded, and 0 for fangs a spell made.
    #[test]
    fn evoker_fangs_keep_their_loaded_fall_distance() {
        let raw = sunrise_core::nbt::parse_snbt(
            "{id:\"minecraft:evoker_fangs\",Pos:[1d,200d,1d],fall_distance:5.0d,Warmup:100}",
        )
        .unwrap()
        .0;
        let at = super::Vec3::new(1.0, 200.0, 1.0);
        let fall = |raw: Option<&super::Tag>| match super::evoker_fangs_to_nbt(
            at, 0.0, 99, None, [1; 4], raw,
        )
        .get("fall_distance")
        {
            Some(super::Tag::Double(bits)) => f64::from_bits(*bits),
            other => panic!("no fall_distance: {other:?}"),
        };
        assert_eq!(fall(Some(&raw)), 5.0);
        assert_eq!(fall(None), 0.0);
    }

    /// `Shulker` reads `AttachFace` as a byte before
    /// `Direction.from3DDataValue` (`abs(data % 6)`): an int is narrowed
    /// first, so 258 is byte 2, not 258 % 6 = 0, and 250 is byte -6, face 0.
    #[test]
    fn a_shulkers_attach_face_is_narrowed_to_a_byte_first() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:shulker".into();
        ty.species = crate::mobs::Species::Shulker;
        let types = [("minecraft:shulker".into(), kind)].into();
        let face = |saved: Tag| {
            let raw = Tag::compound([
                ("id", string("minecraft:shulker")),
                ("Pos", doubles(&[4.5, 220.0, 4.5])),
                ("AttachFace", saved),
            ]);
            mob_from_nbt(&raw, &types, 1, None).unwrap().extra.variant3
        };
        assert_eq!(face(Tag::Byte(3)), 3);
        assert_eq!(face(Tag::Byte(-2)), 2);
        assert_eq!(face(Tag::Int(258)), 2);
        assert_eq!(face(Tag::Int(250)), 0);
    }
    use super::*;
    use crate::items::ItemStack;

    /// Older worlds name their single player by the offline UUID; the migration rewrites
    /// every reference to it (owner, thrower, trusted list, anger, leash) and nothing else.
    #[test]
    fn a_players_offline_uuid_becomes_the_accounts_everywhere_in_an_entity() {
        let old = [1, 2, 3, 4];
        let new = [5, 6, 7, 8];
        let mut tag = Tag::compound([
            ("id", string("minecraft:wolf")),
            ("UUID", Tag::IntArray(vec![9, 9, 9, 9])),
            ("Owner", Tag::IntArray(old.to_vec())),
            ("Thrower", Tag::IntArray(old.to_vec())),
            ("angry_at", Tag::IntArray(old.to_vec())),
            (
                "Trusted",
                Tag::List(vec![
                    Tag::IntArray(old.to_vec()),
                    Tag::IntArray(vec![3, 3, 3, 3]),
                ]),
            ),
            (
                "leash",
                Tag::compound([("UUID", Tag::IntArray(old.to_vec()))]),
            ),
            ("Health", Tag::Float(20.0f32.to_bits())),
        ]);
        migrate_player_uuid(&mut tag, old, new);
        assert_eq!(tag.get("Owner"), Some(&Tag::IntArray(new.to_vec())));
        assert_eq!(tag.get("Thrower"), Some(&Tag::IntArray(new.to_vec())));
        assert_eq!(tag.get("angry_at"), Some(&Tag::IntArray(new.to_vec())));
        assert_eq!(
            tag.get("Trusted"),
            Some(&Tag::List(vec![
                Tag::IntArray(new.to_vec()),
                Tag::IntArray(vec![3, 3, 3, 3]),
            ]))
        );
        assert_eq!(
            tag.get("leash").and_then(|leash| leash.get("UUID")),
            Some(&Tag::IntArray(new.to_vec()))
        );
        // The entity's own UUID is not the player's.
        assert_eq!(tag.get("UUID"), Some(&Tag::IntArray(vec![9, 9, 9, 9])));
    }

    /// An entity that never moves (an `AreaEffectCloud`) saves the
    /// `fall_distance` `Entity.load` read, and 0 when it was made in play.
    #[test]
    fn a_base_tag_keeps_the_loaded_fall_distance() {
        let raw = sunrise_core::nbt::parse_snbt(
            "{id:\"minecraft:area_effect_cloud\",Pos:[1d,200d,1d],fall_distance:5.0d}",
        )
        .unwrap()
        .0;
        let fall = |raw: Option<&super::Tag>| {
            let tag = super::entity_base_tag(
                "minecraft:area_effect_cloud",
                super::Vec3::new(1.0, 200.0, 1.0),
                super::Vec3::ZERO,
                [0.0, 0.0],
                [1; 4],
                raw,
            );
            match tag.get("fall_distance") {
                Some(super::Tag::Double(bits)) => f64::from_bits(*bits),
                other => panic!("no fall_distance: {other:?}"),
            }
        };
        assert_eq!(fall(Some(&raw)), 5.0);
        assert_eq!(fall(None), 0.0);
    }

    #[test]
    fn empty_saved_tags_are_omitted_without_changing_raw_or_nonempty_tags() {
        let base = Base {
            id: "minecraft:pig",
            uuid: [328, 0, 0, 1],
            position: Vec3::new(4.5, 200.0, 4.5),
            motion: Vec3::ZERO,
            rotation: [0.0; 2],
            on_ground: false,
            fall_distance: 0.0,
        };
        let raw = |tags| {
            Tag::compound([
                ("Tags", Tag::List(tags)),
                ("PortalCooldown", Tag::Int(17)),
                ("Invulnerable", Tag::Byte(1)),
            ])
        };
        let fresh = base.tag(None);
        assert!(fresh.get("Tags").is_none());
        assert_eq!(fresh.get("PortalCooldown"), Some(&Tag::Int(0)));
        for tags in [vec![string("keep")], vec![]] {
            let raw = raw(tags.clone());
            let saved = base.tag(Some(&raw));
            assert_eq!(saved.get("PortalCooldown"), Some(&Tag::Int(17)));
            assert_eq!(saved.get("Invulnerable"), Some(&Tag::Byte(1)));
            assert_eq!(raw.get("Tags"), Some(&Tag::List(tags.clone())));
            assert_eq!(
                saved.get("Tags"),
                if tags.is_empty() {
                    None
                } else {
                    Some(Tag::List(tags))
                }
                .as_ref()
            );
        }
    }

    #[test]
    fn conversion_keeps_only_the_old_mobs_command_tags() {
        let tags = Tag::List(vec![
            Tag::String("subject".into()),
            Tag::String("quest".into()),
        ]);
        let raw = Tag::compound([
            ("id", Tag::String("minecraft:husk".into())),
            ("UUID", Tag::IntArray(vec![1, 2, 3, 4])),
            ("HuskConversionTime", Tag::Int(1)),
            ("Tags", tags.clone()),
        ]);
        let original = raw.clone();
        assert_eq!(
            conversion_tags(Some(&raw)),
            Some(Tag::compound([("Tags", tags)]))
        );
        assert_eq!(raw, original);
        assert_eq!(conversion_tags(None), None);
        assert_eq!(
            conversion_tags(Some(&Tag::compound([("Tags", Tag::List(Vec::new()))]))),
            None
        );
        assert_eq!(
            conversion_tags(Some(&Tag::Compound(Default::default()))),
            None
        );
    }

    #[test]
    fn an_ageable_mob_that_cannot_be_a_baby_saves_no_age() {
        use crate::mobs::Species;
        // AgeableMob.addAdditionalSaveData writes Age only when canBeABaby is
        // true, so every ageable class that overrides it to false writes none
        // of it: a frog and a camel husk keep no raw `Age` either.
        for (species, id) in [
            (Species::Slime, "minecraft:slime"),
            (Species::MagmaCube, "minecraft:magma_cube"),
            (Species::Frog, "minecraft:frog"),
            (Species::CamelHusk, "minecraft:camel_husk"),
            (Species::Pig, "minecraft:pig"),
        ] {
            let mut kind = crate::mobs::sensor_scan_tests::kind();
            let ty = Arc::make_mut(&mut kind);
            ty.species = species;
            ty.id = id.into();
            let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
            mob.extra.age = -37;
            let expected = (species == Species::Pig).then_some(Tag::Int(-37));
            for age in [None, Some(0), Some(-24000), Some(37)] {
                let mut raw = Tag::compound([("h228_control", Tag::Int(42))]);
                if let Some(age) = age
                    && let Tag::Compound(map) = &mut raw
                {
                    map.insert("Age".into(), Tag::Int(age));
                }
                let saved = mob_to_nbt(
                    &mob,
                    [0; 4],
                    Some(&raw),
                    None,
                    |_| None,
                    &Default::default(),
                );
                assert_eq!(saved.get("Age"), expected.as_ref(), "{species:?}, {age:?}");
                assert_eq!(saved.get("h228_control"), Some(&Tag::Int(42)));
            }
            let saved = mob_to_nbt(&mob, [0; 4], None, None, |_| None, &Default::default());
            assert_eq!(
                saved.get("Age"),
                expected.as_ref(),
                "{species:?}, no raw tag"
            );
        }
    }

    #[test]
    fn zombie_nautilus_save_omits_ignored_age_fields() {
        use crate::mobs::Species;
        for (id, species) in [
            ("minecraft:zombie_nautilus", Species::ZombieNautilus),
            ("minecraft:nautilus", Species::Nautilus),
        ] {
            let mut kind = crate::mobs::sensor_scan_tests::kind();
            let def = Arc::make_mut(&mut kind);
            def.id = id.into();
            def.species = species;
            let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
            mob.extra.age = -123;
            mob.extra.forced_age = 19;
            mob.extra.age_locked = true;
            let raw = Tag::compound([
                ("Age", Tag::Int(37)),
                ("ForcedAge", Tag::Int(99)),
                ("AgeLocked", Tag::Byte(0)),
                ("audit:unrelated", Tag::Int(7)),
            ]);
            for base in [None, Some(&raw)] {
                let saved = mob_to_nbt(&mob, [0; 4], base, None, |_| None, &Default::default());
                for (key, expected) in [
                    ("Age", Tag::Int(-123)),
                    ("ForcedAge", Tag::Int(19)),
                    ("AgeLocked", Tag::Byte(1)),
                ] {
                    if species == Species::ZombieNautilus {
                        assert!(saved.get(key).is_none(), "{id}: {key}");
                    } else {
                        assert_eq!(saved.get(key), Some(&expected), "{id}: {key}");
                    }
                }
                if base.is_some() {
                    assert_eq!(saved.get("audit:unrelated"), Some(&Tag::Int(7)));
                }
            }
        }
    }

    /// `Horse.getVariant` is `Variant.byId(typeVariant & 255)` and
    /// `getMarkings` `Markings.byId((typeVariant & 0xFF00) >> 8)`; both maps
    /// are `OutOfBoundsStrategy.WRAP` (7 coats, 5 markings), so 9 is coat 2
    /// and markings 6 is 1.
    #[test]
    fn a_horses_unknown_coat_and_markings_wrap() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:horse".into();
        ty.species = crate::mobs::Species::Horse;
        let types = [("minecraft:horse".into(), kind)].into();
        let look = |saved: i32| {
            let raw = Tag::compound([
                ("id", string("minecraft:horse")),
                ("Pos", doubles(&[4.5, 220.0, 4.5])),
                ("Variant", Tag::Int(saved)),
            ]);
            let m = mob_from_nbt(&raw, &types, 1, None).unwrap();
            (m.extra.variant, m.extra.variant2)
        };
        assert_eq!(look(6 | 4 << 8), (6, 4));
        assert_eq!(look(9), (2, 0));
        assert_eq!(look(6 << 8), (0, 1));
        assert_eq!(look(255 | 255 << 8), (3, 0));
        // `Horse.addAdditionalSaveData` writes the loaded int back as it was;
        // a horse whose look changed since saves the two ids.
        let saved = |loaded: i32, change: Option<u8>| {
            let raw = Tag::compound([
                ("id", string("minecraft:horse")),
                ("Pos", doubles(&[4.5, 220.0, 4.5])),
                ("Variant", Tag::Int(loaded)),
            ]);
            let mut m = mob_from_nbt(&raw, &types, 1, None).unwrap();
            if let Some(coat) = change {
                m.extra.variant = coat;
            }
            number(
                mob_to_nbt(&m, [0; 4], Some(&raw), None, |_| None, &Default::default())
                    .get("Variant"),
            )
        };
        assert_eq!(saved(9, None), Some(9));
        assert_eq!(saved(6 << 8 | 0x7_0000, None), Some(6 << 8 | 0x7_0000));
        assert_eq!(saved(9, Some(5)), Some(5));
    }

    /// `LivingEntity.readAdditionalSaveData` reads `HurtTime` and `DeathTime`
    /// with `getShortOr`: an int tag is narrowed (`(short)(value & 65535)`),
    /// so 40000 is -25536 and never counts down.
    #[test]
    fn a_mob_narrows_its_hurt_and_death_time_to_a_short() {
        let kind = crate::mobs::sensor_scan_tests::kind();
        let types = [(kind.id.to_string(), kind.clone())].into();
        let load = |hurt: Tag, death: Tag| {
            let raw = Tag::compound([
                ("id", string(&kind.id)),
                ("Pos", doubles(&[4.5, 220.0, 4.5])),
                ("HurtTime", hurt),
                ("DeathTime", death),
            ]);
            let m = mob_from_nbt(&raw, &types, 1, None).unwrap();
            (m.hurt_time, m.death_time)
        };
        assert_eq!(load(Tag::Int(40000), Tag::Int(65546)), (-25536, 10));
        assert_eq!(load(Tag::Short(7), Tag::Short(19)), (7, 19));
    }

    /// `Axolotl.Variant.BY_ID` is `OutOfBoundsStrategy.ZERO`: an id outside
    /// 0..=4 is lucy (0). `Parrot.Variant.BY_ID` is `CLAMP`: it is the nearest
    /// end.
    #[test]
    fn an_unknown_axolotl_variant_is_lucy_and_a_parrots_is_clamped() {
        let variant = |species, id: &str, saved: i32| {
            let mut kind = crate::mobs::sensor_scan_tests::kind();
            let ty = Arc::make_mut(&mut kind);
            ty.id = id.into();
            ty.species = species;
            let types = [(id.into(), kind)].into();
            let raw = Tag::compound([
                ("id", string(id)),
                ("Pos", doubles(&[4.5, 220.0, 4.5])),
                ("Variant", Tag::Int(saved)),
            ]);
            mob_from_nbt(&raw, &types, 1, None).unwrap().extra.variant
        };
        let axolotl = |saved| variant(crate::mobs::Species::Axolotl, "minecraft:axolotl", saved);
        let parrot = |saved| variant(crate::mobs::Species::Parrot, "minecraft:parrot", saved);
        assert_eq!([0, 3, 4, 5, 7, -1].map(axolotl), [0, 3, 4, 0, 0, 0]);
        assert_eq!([0, 3, 4, 5, 7, -1].map(parrot), [0, 3, 4, 4, 4, 0]);
    }

    /// A mount keeps its body armour and a helmet (say
    /// `/item replace entity <horse> armor.head with minecraft:iron_helmet`)
    /// in their own slots through a save: vanilla holds both (`equipment.body`
    /// and `equipment.head`), one in each.
    #[test]
    fn a_horses_save_keeps_a_helmet_and_its_body_armour_apart() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:horse".into();
        ty.species = crate::mobs::Species::Horse;
        let types = [("minecraft:horse".into(), kind)].into();
        let raw = Tag::compound([
            ("id", string("minecraft:horse")),
            ("Pos", doubles(&[4.5, 220.0, 4.5])),
            ("Tame", Tag::Byte(1)),
            (
                "equipment",
                Tag::compound([
                    (
                        "body",
                        Tag::compound([
                            ("id", string("minecraft:diamond_horse_armor")),
                            ("count", Tag::Int(1)),
                        ]),
                    ),
                    (
                        "head",
                        Tag::compound([
                            ("id", string("minecraft:iron_helmet")),
                            ("count", Tag::Int(1)),
                        ]),
                    ),
                ]),
            ),
        ]);
        let m = mob_from_nbt(&raw, &types, 1, None).unwrap();
        assert_eq!(m.extra.body_id(), Some("minecraft:diamond_horse_armor"));
        assert_eq!(m.extra.head_id(), Some("minecraft:iron_helmet"));
        let saved = mob_to_nbt(&m, [0; 4], Some(&raw), None, |_| None, &Default::default());
        let eq = saved.get("equipment").unwrap();
        let id = |key: &str| {
            eq.get(key)
                .and_then(|i| text(i.get("id")))
                .unwrap_or_default()
        };
        assert_eq!(id("body"), "minecraft:diamond_horse_armor");
        assert_eq!(id("head"), "minecraft:iron_helmet");
    }

    /// `Bat.readAdditionalSaveData` sets `DATA_ID_FLAGS` to the raw `BatFlags`
    /// byte, `Bat.setResting` changes bit 0 only and `addAdditionalSaveData`
    /// writes the byte back: `6b` saves `6b`, and `-1b` saves `-2b` once the
    /// bat wakes. Saved without the loaded compound, so the bat itself keeps it.
    #[test]
    fn a_bat_saves_its_whole_flags_byte() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:bat".into();
        ty.species = crate::mobs::Species::Bat;
        let types = [("minecraft:bat".into(), kind)].into();
        let load = |flags: i8| {
            let raw = Tag::compound([
                ("id", string("minecraft:bat")),
                ("Pos", doubles(&[4.5, 220.0, 4.5])),
                ("BatFlags", Tag::Byte(flags)),
            ]);
            mob_from_nbt(&raw, &types, 1, None).unwrap()
        };
        let saved = |m: &Mob| {
            mob_to_nbt(m, [0; 4], None, None, |_| None, &Default::default())
                .get("BatFlags")
                .cloned()
        };
        assert_eq!(saved(&load(6)), Some(Tag::Byte(6)));
        let mut m = load(-1);
        assert!(crate::mobs::bat::resting(&m));
        crate::mobs::bat::set_resting(&mut m, false);
        assert_eq!(saved(&m), Some(Tag::Byte(-2)));
    }

    /// A body item taken off (`/item replace entity <cube> armor.body with minecraft:air`)
    /// is not saved again from the entity's earlier save: vanilla writes `equipment.body` only
    /// while the slot holds something (#12038).
    #[test]
    fn an_emptied_body_slot_is_not_saved_from_the_earlier_nbt() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:sulfur_cube".into();
        ty.species = crate::mobs::Species::SulfurCube;
        let types = [("minecraft:sulfur_cube".into(), kind)].into();
        let raw = Tag::compound([
            ("id", string("minecraft:sulfur_cube")),
            ("Pos", doubles(&[4.5, 220.0, 4.5])),
            (
                "equipment",
                Tag::compound([(
                    "body",
                    Tag::compound([
                        ("id", string("minecraft:oak_planks")),
                        ("count", Tag::Int(1)),
                    ]),
                )]),
            ),
        ]);
        let mut m = mob_from_nbt(&raw, &types, 1, None).unwrap();
        assert_eq!(m.extra.body_id(), Some("minecraft:oak_planks"));
        m.extra.set_body_stack(None);
        let saved = mob_to_nbt(&m, [0; 4], Some(&raw), None, |_| None, &Default::default());
        assert!(saved.get("equipment").is_none(), "{saved:?}");
    }

    /// A wolf loads its armor (`equipment.body`) into the body slot with the
    /// durability it lost, and a helmet next to it into the head slot.
    #[test]
    fn a_wolfs_save_keeps_its_armour_and_a_helmet_apart() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:wolf".into();
        ty.species = crate::mobs::Species::Wolf;
        let types = [("minecraft:wolf".into(), kind)].into();
        let armor = Tag::compound([
            ("id", string("minecraft:wolf_armor")),
            ("count", Tag::Int(1)),
            (
                "components",
                Tag::compound([("minecraft:damage", Tag::Int(9))]),
            ),
        ]);
        let raw = Tag::compound([
            ("id", string("minecraft:wolf")),
            ("Pos", doubles(&[4.5, 220.0, 4.5])),
            ("Owner", Tag::IntArray([0; 4].to_vec())),
            (
                "equipment",
                Tag::compound([
                    ("body", armor),
                    (
                        "head",
                        Tag::compound([
                            ("id", string("minecraft:iron_helmet")),
                            ("count", Tag::Int(1)),
                        ]),
                    ),
                ]),
            ),
        ]);
        let m = mob_from_nbt(&raw, &types, 1, None).unwrap();
        assert_eq!(m.extra.body_id(), Some("minecraft:wolf_armor"));
        assert_eq!(m.extra.body_damage(), 9);
        assert_eq!(m.extra.head_id(), Some("minecraft:iron_helmet"));
    }

    /// `Wolf` and `Cat` read `CollarColor` with `DyeColor.LEGACY_ID_CODEC`: a
    /// byte through `DyeColor.byId`, whose out-of-range answer is the first
    /// colour, white (`OutOfBoundsStrategy.ZERO`). Nothing wraps.
    #[test]
    fn a_collar_colour_outside_the_sixteen_reads_white() {
        for (species, id) in [
            (crate::mobs::Species::Wolf, "minecraft:wolf"),
            (crate::mobs::Species::Cat, "minecraft:cat"),
        ] {
            let mut kind = crate::mobs::sensor_scan_tests::kind();
            let ty = Arc::make_mut(&mut kind);
            ty.id = id.into();
            ty.species = species;
            let types = [(id.into(), kind)].into();
            let colour = |saved: Option<Tag>| {
                let mut fields = vec![("id", string(id)), ("Pos", doubles(&[4.5, 220.0, 4.5]))];
                fields.extend(saved.map(|c| ("CollarColor", c)));
                let m = mob_from_nbt(&Tag::compound(fields), &types, 1, None).unwrap();
                m.extra.color
            };
            assert_eq!(colour(Some(Tag::Byte(11))), 11, "{id}");
            assert_eq!(colour(Some(Tag::Byte(17))), 0, "{id}: not 17 & 15");
            assert_eq!(colour(Some(Tag::Byte(-1))), 0, "{id}: not 15");
            // Codec.BYTE narrows an int first: 259 is byte 3.
            assert_eq!(colour(Some(Tag::Int(259))), 3, "{id}");
            assert_eq!(colour(None), crate::mobs::tame::RED, "{id}");
        }
    }

    fn fused_creeper(fuse: Option<i16>) -> (Mob, Tag) {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:creeper".into();
        ty.species = crate::mobs::Species::Creeper;
        let types = [("minecraft:creeper".into(), kind)].into();
        let mut tag = Tag::compound([
            ("id", string("minecraft:creeper")),
            ("Pos", doubles(&[4.5, 220.0, 4.5])),
            ("ignited", Tag::Byte(1)),
            ("NoAI", Tag::Byte(1)),
            ("NoGravity", Tag::Byte(1)),
        ]);
        if let (Some(fuse), Tag::Compound(c)) = (fuse, &mut tag) {
            c.insert("Fuse".into(), Tag::Short(fuse));
        }
        (mob_from_nbt(&tag, &types, 1, None).unwrap(), tag)
    }

    #[test]
    fn creeper_fuse_nbt_controls_explosion_tick_and_saves_runtime_value() {
        for (fuse, boundary) in [
            (Some(1), 1),
            (None, 30),
            (Some(40), 40),
            (Some(0), 1),
            (Some(-1), 1),
        ] {
            let (mut mob, raw) = fused_creeper(fuse);
            let mut events = Vec::new();
            for tick in 1..=boundary {
                crate::mobs::hostile::creeper_tick(&mut mob, &mut events);
                assert_eq!(mob.removed, tick == boundary, "Fuse={fuse:?}, tick={tick}");
            }
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, crate::mobs::MobEvent::Explode(..)))
                    .count(),
                1
            );
            mob.max_swell = 7;
            let saved = mob_to_nbt(
                &mob,
                [0; 4],
                Some(&raw),
                None,
                |_| None,
                &Default::default(),
            );
            assert_eq!(saved.get("Fuse"), Some(&Tag::Short(7)));
        }
    }

    /// #11332: `Creeper.readAdditionalSaveData` reads `ExplosionRadius` as a
    /// byte (default 3), `explodeCreeper` blows with it, twice it when powered,
    /// and `addAdditionalSaveData` writes the value back as a byte.
    #[test]
    fn creeper_explosion_radius_nbt_sizes_the_blast_and_saves() {
        let blast = |mob: &mut Mob| -> f32 {
            let mut events = Vec::new();
            mob.max_swell = 1;
            crate::mobs::hostile::creeper_tick(mob, &mut events);
            events
                .iter()
                .find_map(|e| match e {
                    crate::mobs::MobEvent::Explode(_, radius, ..) => Some(*radius),
                    _ => None,
                })
                .expect("the creeper exploded")
        };
        for (radius, powered, expected) in [
            (None, false, 3.0),
            (None, true, 6.0),
            (Some(Tag::Byte(12)), false, 12.0),
            (Some(Tag::Byte(12)), true, 24.0),
            (Some(Tag::Byte(0)), false, 0.0),
            // getByteOr takes any numeric tag, narrowed to a byte.
            (Some(Tag::Long(300)), false, 44.0),
        ] {
            let (mut mob, raw) = fused_creeper(None);
            let mut tag = raw.clone();
            if let Tag::Compound(c) = &mut tag {
                if let Some(radius) = radius.clone() {
                    c.insert("ExplosionRadius".into(), radius);
                }
                c.insert("powered".into(), Tag::Byte(i8::from(powered)));
            }
            let types = [("minecraft:creeper".into(), mob.kind.clone())].into();
            mob = mob_from_nbt(&tag, &types, 1, None).unwrap();
            let saved = mob_to_nbt(
                &mob,
                [0; 4],
                Some(&raw),
                None,
                |_| None,
                &Default::default(),
            );
            assert_eq!(
                saved.get("ExplosionRadius"),
                Some(&Tag::Byte(expected as i8 / if powered { 2 } else { 1 })),
                "{radius:?} powered={powered}"
            );
            assert_eq!(blast(&mut mob), expected, "{radius:?} powered={powered}");
        }
    }

    #[test]
    fn creeper_fall_swell_uses_custom_fuse_without_a_zero_floor() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        for (fuse, expected) in [(Some(1), -4), (None, 25), (Some(40), 35)] {
            let (mut mob, _) = fused_creeper(fuse);
            mob.cause_fall_damage_from(
                crate::test_resources::server_loop_context(),
                level,
                100.0,
                0.0,
                &mut Vec::new(),
            );
            assert_eq!(mob.swell, expected, "Fuse={fuse:?}");
        }
    }

    #[test]
    fn villager_level_round_trips_above_byte_range() {
        use crate::mobs::Species;
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = "minecraft:villager".into();
        def.species = Species::Villager;
        let types = [("minecraft:villager".into(), kind)].into();
        let data = Tag::compound([
            ("profession", string("minecraft:farmer")),
            ("type", string("minecraft:plains")),
            ("level", Tag::Int(123456)),
        ]);
        let tag = Tag::compound([
            ("id", string("minecraft:villager")),
            ("Pos", doubles(&[0.0, 100.0, 0.0])),
            ("VillagerData", data.clone()),
        ]);
        let mob = mob_from_nbt(&tag, &types, 7, None).unwrap();
        assert_eq!(villager_data(&mob), data);
    }

    #[test]
    fn unresolved_villager_profession_round_trips_through_entity_nbt() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = "minecraft:villager".into();
        def.species = crate::mobs::Species::Villager;
        let types = [("minecraft:villager".into(), kind)].into();
        let data = Tag::compound([
            ("profession", string("example:nested/engineer")),
            ("type", string("minecraft:plains")),
            ("level", Tag::Int(1)),
        ]);
        let tag = Tag::compound([
            ("id", string("minecraft:villager")),
            ("Pos", doubles(&[0.0, 100.0, 0.0])),
            ("VillagerData", data.clone()),
        ]);
        let mob = mob_from_nbt(&tag, &types, 7, None).unwrap();
        assert_eq!(villager_data(&mob), data);
        assert_eq!(mob.extra.profession.index(), None);
        let saved = mob_to_nbt(&mob, [0; 4], None, None, |_| None, &Default::default());
        let reloaded = mob_from_nbt(&saved, &types, 8, None).unwrap();
        assert_eq!(villager_data(&reloaded), data);
    }

    #[test]
    fn loaded_villager_command_names_follow_profession_and_custom_name() {
        use crate::mobs::Species;
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = "minecraft:villager".into();
        def.species = Species::Villager;
        let types = [("minecraft:villager".into(), kind)].into();
        for (profession, key) in [
            ("none", "entity.minecraft.villager.none"),
            ("nitwit", "entity.minecraft.villager.nitwit"),
            ("cleric", "entity.minecraft.villager.cleric"),
            ("farmer", "entity.minecraft.villager.farmer"),
            ("librarian", "entity.minecraft.villager.librarian"),
        ] {
            let data = Tag::compound([
                ("profession", string(&format!("minecraft:{profession}"))),
                ("type", string("minecraft:plains")),
                ("level", Tag::Int(1)),
            ]);
            let mut tag = Tag::compound([
                ("id", string("minecraft:villager")),
                ("Pos", doubles(&[4.5, 220.0, 4.5])),
                ("NoAI", Tag::Byte(1)),
                ("VillagerData", data.clone()),
            ]);
            let mob = mob_from_nbt(&tag, &types, 7, None).unwrap();
            assert_eq!(villager_data(&mob), data);
            assert_eq!(mob.command_name().translation_key(), Some(key));
            put(&mut tag, "CustomName", string("Ada"));
            let named = mob_from_nbt(&tag, &types, 8, None).unwrap();
            assert_eq!(
                named.command_name().component(),
                crate::text::Text::literal("Ada").component()
            );
        }
    }

    #[test]
    fn other_mob_command_names_keep_their_type_and_custom_name() {
        use crate::mobs::Species;
        for (id, species, key) in [
            (
                "minecraft:zombie_villager",
                Species::ZombieVillager,
                "entity.minecraft.zombie_villager",
            ),
            (
                "minecraft:wandering_trader",
                Species::WanderingTrader,
                "entity.minecraft.wandering_trader",
            ),
            ("audit:helper", Species::Pig, "entity.audit.helper"),
        ] {
            let mut kind = crate::mobs::sensor_scan_tests::kind();
            let def = Arc::make_mut(&mut kind);
            def.id = id.into();
            def.species = species;
            let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
            mob.extra.variant2 = 4;
            assert_eq!(mob.command_name().translation_key(), Some(key));
            mob.custom_name = Some("Ada".into());
            assert_eq!(mob.command_name().plain(), "Ada");
        }
    }

    #[test]
    fn mob_loading_accepts_default_namespace_and_preserves_custom_namespaces() {
        let cow = crate::mobs::sensor_scan_tests::kind();
        let types = [
            ("minecraft:cow".into(), cow.clone()),
            ("test:cow".into(), cow),
        ]
        .into();
        let pos = || {
            Tag::List(
                [4.5_f64, 220.0, 6.5]
                    .map(|v| Tag::Double(v.to_bits()))
                    .to_vec(),
            )
        };
        for id in ["cow", ":cow", "minecraft:cow", "test:cow"] {
            let tag = Tag::compound([("id", string(id)), ("Pos", pos()), ("NoAI", Tag::Byte(1))]);
            let mob = mob_from_nbt(&tag, &types, 7, None).expect(id);
            assert_eq!(mob.position, Vec3::new(4.5, 220.0, 6.5));
            assert!(mob.no_ai);
        }
        let unknown = Tag::compound([("id", string("other:cow")), ("Pos", pos())]);
        assert!(mob_from_nbt(&unknown, &types, 8, None).is_none());
    }

    /// A saved rocket and trident come back with their own Life, LifeTime,
    /// flags and position (the keys vanilla's addAdditionalSaveData writes).
    #[test]
    fn rocket_and_trident_round_trip() {
        let item = ItemStack::new("minecraft:firework_rocket", 1);
        let tag = firework_rocket_to_nbt(
            &RocketSave {
                rotation: [90.0, 10.0],
                position: Vec3::new(1.5, 70.0, -3.25),
                motion: Vec3::new(0.0, 0.5, 0.0),
                life: 7,
                lifetime: 31,
                item: &item,
                angled: true,
                owner: None,
                left_owner: false,
                on_ground: true,
                has_been_shot: false,
                fall_distance: 2.5,
            },
            [1, 2, 3, 4],
            None,
            &Default::default(),
        );
        let r = firework_rocket_from_nbt(&tag, None).unwrap();
        assert_eq!(
            (r.life, r.lifetime, r.angled, r.on_ground, r.has_been_shot),
            (7, 31, true, true, false)
        );
        assert_eq!(r.position, Vec3::new(1.5, 70.0, -3.25));
        assert_eq!(r.rotation, [90.0, 10.0]);
        // Entity.saveWithoutId/load keep the fall.
        assert_eq!(r.fall_distance, 2.5);
        let trident = ItemStack::new("minecraft:trident", 1);
        let tag = trident_to_nbt(
            &TridentSave {
                position: Vec3::new(2.0, 64.0, 2.0),
                motion: Vec3::new(0.0, -0.25, 0.0),
                yaw: 10.0,
                pitch: 20.0,
                in_ground: true,
                life: 9,
                shake: 3,
                dealt_damage: true,
                pickup: Pickup::CreativeOnly,
                left_owner: true,
                has_been_shot: true,
                owner: Some([5, 6, 7, 8]),
                no_gravity: true,
                on_ground: true,
                fire_ticks: 0,
                fall_distance: 0.0,
                sound_event: TRIDENT_HIT_GROUND,
                item: &trident,
            },
            [1, 2, 3, 4],
            None,
            &Default::default(),
        );
        let t = trident_from_nbt(&tag).unwrap();
        // `Entity.saveWithoutId` writes NoGravity only when set.
        assert!(t.no_gravity);
        assert!(matches!(tag.get("NoGravity"), Some(Tag::Byte(1))));
        // Entity.load/saveWithoutId: OnGround as it was.
        assert!(t.on_ground);
        assert_eq!(tag.get("OnGround"), Some(&Tag::Byte(1)));
        assert_eq!((t.in_ground, t.life, t.shake), (true, 9, 3));
        assert_eq!((t.dealt_damage, t.pickup), (true, Pickup::CreativeOnly));
        assert!(t.left_owner && t.has_been_shot);
        assert_eq!(t.owner, Some([5, 6, 7, 8]));
        assert_eq!((t.yaw, t.pitch), (10.0, 20.0));
    }

    /// The item, the spawn delay and the fields Sunrise doesn't model come
    /// back; a compound without an item is no spawner.
    #[test]
    fn ominous_item_spawner_round_trip() {
        let item = ItemStack {
            item: Arc::from("minecraft:ominous_trial_key"),
            count: 1,
            damage: 0,
            extra: None,
        };
        let raw = Tag::compound([("CustomName", string("kept"))]);
        let at = Vec3::new(10.5, 64.25, -3.5);
        let tag = ominous_item_spawner_to_nbt(
            &item,
            at,
            97,
            [1, 2, 3, 4],
            Some(&raw),
            &Default::default(),
        );
        assert_eq!(
            kind(&tag).as_deref(),
            Some("minecraft:ominous_item_spawner")
        );
        assert_eq!(uuid(&tag), Some([1, 2, 3, 4]));
        assert!(matches!(
            tag.get("spawn_item_after_ticks"),
            Some(Tag::Long(97))
        ));
        assert!(tag.get("CustomName").is_some());
        let (back, pos, after) = ominous_item_spawner_from_nbt(&tag).unwrap();
        assert_eq!(
            (back.item.as_ref(), back.count),
            ("minecraft:ominous_trial_key", 1)
        );
        assert_eq!((pos, after), (at, 97));
        assert!(ominous_item_spawner_from_nbt(&raw).is_none());
        // No item is `ItemStack.EMPTY`, which writes no key back.
        let empty = ominous_item_spawner_to_nbt(
            &crate::items::ItemStack::new("minecraft:air", 1),
            at,
            0,
            [1, 2, 3, 4],
            Some(&tag),
            &Default::default(),
        );
        assert!(empty.get("item").is_none());
        let (back, pos, after) = ominous_item_spawner_from_nbt(&empty).unwrap();
        assert_eq!((back.item.as_ref(), pos, after), ("minecraft:air", at, 0));
    }

    #[test]
    fn armor_stand_names_keep_their_formatting_across_a_save() {
        let tag = Tag::compound([
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(0.0f64.to_bits()),
                    Tag::Double(64.0f64.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                ]),
            ),
            ("CustomName", string("§6Hello")),
            ("CustomNameVisible", Tag::Byte(1)),
        ]);
        let a = armor_stand_from_nbt(&tag, 7).unwrap();
        let name = a.custom_name.as_ref().unwrap();
        assert_eq!(name.plain(), "Hello");
        assert!(name.spans[0].style.color.is_some());
        assert!(a.name_visible);
        let saved = armor_stand_to_nbt(&a, [0; 4], Some(&tag), &Default::default());
        let restored = armor_stand_from_nbt(&saved, 7).unwrap();
        assert_eq!(restored.custom_name, a.custom_name);
        assert!(restored.name_visible);
    }

    #[test]
    fn armor_stand_keeps_no_gravity_through_a_save() {
        let tag = Tag::compound([
            ("id", string("minecraft:armor_stand")),
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(0.0f64.to_bits()),
                    Tag::Double(64.0f64.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                ]),
            ),
            ("NoGravity", Tag::Byte(1)),
        ]);
        let mut a = armor_stand_from_nbt(&tag, 7).unwrap();
        assert!(a.no_gravity, "summon NBT sets the flag");
        let saved = armor_stand_to_nbt(&a, [0; 4], Some(&tag), &Default::default());
        assert_eq!(saved.get("NoGravity"), Some(&Tag::Byte(1)));
        assert!(armor_stand_from_nbt(&saved, 7).unwrap().no_gravity);
        // `data merge entity @s {NoGravity:0b}` clears it again.
        a.no_gravity = false;
        let cleared = armor_stand_to_nbt(&a, [0; 4], Some(&saved), &Default::default());
        assert!(cleared.get("NoGravity").is_none());
        assert!(!armor_stand_from_nbt(&cleared, 7).unwrap().no_gravity);
    }

    /// `Entity.load`: a Motion component past 10 blocks a tick loads as 0,
    /// the others as saved.
    #[test]
    fn armor_stand_drops_a_motion_component_past_ten() {
        let doubles = |v: [f64; 3]| Tag::List(v.map(|d| Tag::Double(d.to_bits())).to_vec());
        let tag = Tag::compound([
            ("id", string("minecraft:armor_stand")),
            ("Pos", doubles([0.0, 64.0, 0.0])),
            ("Motion", doubles([1.0e10, -0.5, -10.5])),
        ]);
        let a = armor_stand_from_nbt(&tag, 7).unwrap();
        assert_eq!(a.velocity, Vec3::new(0.0, -0.5, 0.0));
        let tag = Tag::compound([
            ("id", string("minecraft:armor_stand")),
            ("Pos", doubles([0.0, 64.0, 0.0])),
            ("Motion", doubles([10.0, 0.0, -10.0])),
        ]);
        let a = armor_stand_from_nbt(&tag, 7).unwrap();
        assert_eq!(a.velocity, Vec3::new(10.0, 0.0, -10.0), "10 itself stays");
    }

    fn falling_decode_registry() -> BlockRegistry {
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
            (
                "states",
                Tag::List(
                    ["minecraft:air", "minecraft:sand", "minecraft:gravel"]
                        .into_iter()
                        .map(string)
                        .collect(),
                ),
            ),
            (
                "defaults",
                Tag::compound([
                    ("minecraft:air", Tag::Int(0)),
                    ("minecraft:sand", Tag::Int(1)),
                    ("minecraft:gravel", Tag::Int(2)),
                ]),
            ),
        ]))
        .unwrap()
    }

    fn falling_decode_input() -> Tag {
        sunrise_core::nbt::parse_snbt(
            "{Pos:[3.5d,200d,4.5d],Motion:[0d,0d,0d],Time:10,DropItem:0b,NoGravity:1b}",
        )
        .unwrap()
        .0
    }

    fn falling_decode_save(registry: &BlockRegistry, input: &Tag) -> Tag {
        let (state, pos, motion, time, distance) =
            falling_block_from_nbt(input).expect("absent BlockState is valid input");
        let actual = falling_state_from_nbt(registry, &state).unwrap();
        assert_eq!(falling_block_flags(input), (true, false));
        // Save only the decoded concrete model; no original source compound.
        let saved = falling_block_to_nbt(
            state_to_nbt(registry, actual),
            pos,
            motion,
            time,
            false,
            crate::falling_block::Damage::default(),
            distance,
            false,
            [0.0, 0.0],
            [96, 0, 0, 1],
        );
        assert_eq!(saved.get("Pos"), input.get("Pos"));
        assert_eq!(saved.get("Motion"), input.get("Motion"));
        assert_eq!(saved.get("Time"), Some(&Tag::Int(10)));
        assert_eq!(saved.get("DropItem"), Some(&Tag::Byte(0)));
        saved
    }

    #[test]
    fn falling_decode_absent_state_saves_actual_sand_without_raw() {
        let registry = falling_decode_registry();
        let saved = falling_decode_save(&registry, &falling_decode_input());
        assert_eq!(saved.get("BlockState"), Some(&string("minecraft:sand")));
    }

    #[test]
    fn falling_decode_modern_boundary_keeps_shared_legacy_reader() {
        let registry = falling_decode_registry();
        let legacy = Tag::compound([("Name", string("minecraft:gravel"))]);
        assert_eq!(
            state_from_nbt(&registry, &legacy),
            registry.default_state("minecraft:gravel")
        );
        for (state, expected) in [
            (string("minecraft:sand"), "minecraft:sand"),
            (
                Tag::compound([("id", string("minecraft:gravel"))]),
                "minecraft:gravel",
            ),
            (string("minecraft:air"), "minecraft:air"),
            (legacy, "minecraft:sand"),
            (Tag::Compound(Default::default()), "minecraft:sand"),
        ] {
            let mut input = falling_decode_input();
            put(&mut input, "BlockState", state);
            let saved = falling_decode_save(&registry, &input);
            assert_eq!(saved.get("BlockState"), Some(&string(expected)));
        }
    }

    #[test]
    fn falling_block_round_trip_keeps_impact_distance_and_settings() {
        let state = Tag::compound([("Name", string("minecraft:damaged_anvil"))]);
        let position = Vec3::new(3.5, 100.0, -7.5);
        let velocity = Vec3::new(0.0, -1.25, 0.0);
        let damage = crate::falling_block::Damage {
            cancel_drop: true,
            ..crate::falling_block::Damage::anvil()
        };
        let tag = falling_block_to_nbt(
            state.clone(),
            position,
            velocity,
            30,
            false,
            damage,
            19.75,
            true,
            [0.0, 0.0],
            [1, 2, 3, 4],
        );
        let (loaded_state, pos, motion, age, distance) = falling_block_from_nbt(&tag).unwrap();
        assert_eq!(loaded_state, state);
        assert_eq!(
            (pos, motion, age, distance),
            (position, velocity, 30, 19.75)
        );
        assert_eq!(falling_block_flags(&tag), (false, false));
        assert_eq!(crate::falling_block::Damage::from_nbt(&tag, true), damage);
        assert_eq!(uuid(&tag), Some([1, 2, 3, 4]));
    }

    #[test]
    fn loaded_item_lifetime_counters_tick_with_signed_sentinels() {
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let registry = assets.registry;
        let facts = crate::blocks::BlockFacts::load(&root, registry).unwrap();
        let collision = crate::collisions::load(&root, registry).unwrap();
        let fluids = crate::motion::FluidResources::new(&assets, &facts).unwrap();
        let mut world =
            crate::world::World::new(registry.default_state("minecraft:air").unwrap(), 0);
        world
            .chunks
            .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
        let tick = |item: &mut ItemEntity| {
            item.tick(
                &world,
                registry,
                &facts,
                &collision,
                false,
                &fluids,
                &[],
                None,
                None,
            )
        };
        let fresh = Tag::compound([
            ("id", string("minecraft:item")),
            ("Pos", doubles(&[4.5, 210.0, 4.5])),
            ("NoGravity", Tag::Byte(1)),
            (
                "Item",
                Tag::compound([("id", string("minecraft:stone")), ("count", Tag::Int(1))]),
            ),
        ]);
        for delay in [32767_i16, -1, 0, 1, 2] {
            for age in [-32768_i16, -1, 0, 5998, 5999, 6000, 32767] {
                let mut item = item_entity_from_nbt(&fresh, 1, 0.0).unwrap();
                for _ in 0..3 {
                    assert!(tick(&mut item));
                }
                assert_eq!(item.age, 3);
                let mut data = item_entity_to_nbt(&item, [0, 0, 0, 1], None, &Default::default());
                put(&mut data, "Age", Tag::Short(age));
                put(&mut data, "PickupDelay", Tag::Short(delay));
                let mut item = item_entity_from_nbt(&data, 1, 0.0).unwrap();
                let mut alive = true;
                for step in 0..=3 {
                    if step > 0 && alive {
                        alive = tick(&mut item);
                    }
                    let expected_age = if age == i16::MIN {
                        i32::from(age)
                    } else {
                        i32::from(age) + step
                    };
                    let expected_alive = step == 0 || expected_age < 6000;
                    assert_eq!(
                        alive, expected_alive,
                        "age={age}, delay={delay}, tick={step}"
                    );
                    if !alive {
                        continue;
                    }
                    let data = item_entity_to_nbt(&item, [0, 0, 0, 1], None, &Default::default());
                    let expected_delay = if delay > 0 && delay != i16::MAX {
                        (i32::from(delay) - step).max(0)
                    } else {
                        i32::from(delay)
                    };
                    assert_eq!(
                        number(data.get("Age")),
                        Some(i64::from(expected_age)),
                        "age={age}, tick={step}"
                    );
                    assert_eq!(
                        number(data.get("PickupDelay")),
                        Some(i64::from(expected_delay)),
                        "age={age}, delay={delay}, tick={step}"
                    );
                }
            }
        }
        let mut legacy = item_entity_from_nbt(&fresh, 1, 0.0).unwrap();
        legacy.pickup_delay = i64::from(u32::MAX);
        assert!(tick(&mut legacy));
        assert_eq!(legacy.pickup_delay, i64::from(u32::MAX));
    }

    #[test]
    fn item_health_uses_floor_then_short_narrowing() {
        let cases = [
            (None, 5),
            (Some(Tag::Short(5)), 5),
            (Some(Tag::Int(5)), 5),
            (Some(Tag::Long(5)), 5),
            (Some(Tag::Byte(5)), 5),
            (Some(Tag::Float((-1.5_f32).to_bits())), -2),
            (Some(Tag::Double((-1.5_f64).to_bits())), -2),
            (Some(Tag::Float(1.5_f32.to_bits())), 1),
            (Some(Tag::Double(1.5_f64.to_bits())), 1),
            (Some(Tag::Double((-40000.5_f64).to_bits())), 25535),
            (Some(Tag::Int(32767)), 32767),
            (Some(Tag::Int(32768)), -32768),
            (Some(Tag::Int(-32768)), -32768),
            (Some(Tag::Int(-32769)), 32767),
            (Some(Tag::Long(65535)), -1),
            (Some(Tag::Long(65536)), 0),
            (Some(Tag::Long(-65537)), -1),
            (Some(Tag::Long(i64::MAX)), -1),
            (Some(string("not_numeric")), 5),
            (Some(Tag::Compound(Compound::new())), 5),
        ];
        for (input, expected) in cases {
            let mut tag = Tag::compound([
                ("id", string("minecraft:item")),
                ("Pos", doubles(&[4.5, 200.0, 4.5])),
                ("NoGravity", Tag::Byte(1)),
                (
                    "Item",
                    Tag::compound([("id", string("minecraft:diamond")), ("count", Tag::Int(1))]),
                ),
            ]);
            if let Some(input) = input {
                put(&mut tag, "Health", input);
            }
            let item = item_entity_from_nbt(&tag, 9, 0.0).unwrap();
            assert_eq!(item.health, expected, "input {tag:?}");
            let saved = item_entity_to_nbt(&item, [1, 2, 3, 4], None, &Default::default());
            assert_eq!(saved.get("Health"), Some(&Tag::Short(expected as i16)));
        }
    }

    #[test]
    fn item_numeric_uuid_codec_adopts_numeric_streams_before_save() {
        let cases = [
            (Tag::IntArray(vec![54, 1, -2, 3]), [54, 1, -2, 3]),
            (
                Tag::LongArray(vec![
                    54,
                    4_294_967_297,
                    -4_294_967_298,
                    9_007_199_254_740_993,
                ]),
                [54, 1, -2, 1],
            ),
            (
                Tag::List(vec![
                    Tag::Float(54.9_f32.to_bits()),
                    Tag::Float(1.75_f32.to_bits()),
                    Tag::Float((-2.75_f32).to_bits()),
                    Tag::Float((-0.5_f32).to_bits()),
                ]),
                [54, 1, -2, 0],
            ),
            (
                Tag::List(vec![
                    Tag::Double(54.9_f64.to_bits()),
                    Tag::Double(1.75_f64.to_bits()),
                    Tag::Double((-2.75_f64).to_bits()),
                    Tag::Double((-0.5_f64).to_bits()),
                ]),
                [54, 1, -2, 0],
            ),
            (
                Tag::List(vec![
                    Tag::Long(54),
                    Tag::Long(4_294_967_297),
                    Tag::Long(-4_294_967_298),
                    Tag::Long(9_007_199_254_740_993),
                ]),
                [54, 1, -2, 1],
            ),
            (
                Tag::ByteArray(vec![54, 1, (-2_i8) as u8, 3]),
                [54, 1, -2, 3],
            ),
            // Util.fixedSize returns a partial first-four result for overlong streams.
            (Tag::IntArray(vec![54, 1, -2, 3, 4]), [54, 1, -2, 3]),
        ];
        for field in ["Owner", "Thrower"] {
            for (value, expected) in &cases {
                let raw = Tag::compound([
                    ("Pos", doubles(&[2.5, 200.0, 2.5])),
                    (
                        "Item",
                        Tag::compound([("id", string("minecraft:stone")), ("count", Tag::Int(1))]),
                    ),
                    ("CustomName", string("kept")),
                    (field, value.clone()),
                ]);
                let mut item = item_entity_from_nbt(&raw, 7, 0.0).unwrap();
                assert_eq!(
                    if field == "Owner" {
                        item.target
                    } else {
                        item.thrower
                    },
                    Some(*expected),
                    "{field}: {value:?}"
                );
                assert_eq!(
                    if field == "Owner" {
                        item.thrower
                    } else {
                        item.target
                    },
                    None
                );
                for source in [None, Some(&raw)] {
                    let saved =
                        item_entity_to_nbt(&item, [54, 0, 0, 1], source, &Default::default());
                    assert_eq!(saved.get(field), Some(&Tag::IntArray(expected.to_vec())));
                    assert_eq!(saved.get("Item").unwrap().get("count"), Some(&Tag::Int(1)));
                }
                let saved =
                    item_entity_to_nbt(&item, [54, 0, 0, 1], Some(&raw), &Default::default());
                assert_eq!(saved.get("CustomName"), raw.get("CustomName"));
                item.target = None;
                item.thrower = None;
                let cleared =
                    item_entity_to_nbt(&item, [54, 0, 0, 1], Some(&saved), &Default::default());
                assert!(cleared.get("Owner").is_none());
                assert!(cleared.get("Thrower").is_none());
            }
        }
    }

    #[test]
    fn item_numeric_uuid_codec_rejects_nonstreams_and_short_inputs() {
        let cases = [
            None,
            Some(string("00000036-0000-0001-ffff-fffe00000003")),
            Some(Tag::IntArray(vec![])),
            Some(Tag::IntArray(vec![54, 1, -2])),
            Some(Tag::Int(7)),
            Some(Tag::compound([("value", Tag::Int(7))])),
            Some(Tag::List(vec![
                Tag::Int(54),
                Tag::Int(1),
                Tag::Int(-2),
                string("3"),
            ])),
        ];
        for field in ["Owner", "Thrower"] {
            for value in &cases {
                let mut raw = Tag::compound([
                    ("Pos", doubles(&[2.5, 200.0, 2.5])),
                    (
                        "Item",
                        Tag::compound([("id", string("minecraft:stone")), ("count", Tag::Int(1))]),
                    ),
                ]);
                if let Some(value) = value {
                    put(&mut raw, field, value.clone());
                }
                let mut item = item_entity_from_nbt(&raw, 7, 0.0).unwrap();
                assert_eq!(
                    (item.target, item.thrower),
                    (None, None),
                    "{field}: {value:?}"
                );
                for source in [None, Some(&raw)] {
                    let saved =
                        item_entity_to_nbt(&item, [54, 0, 0, 1], source, &Default::default());
                    assert!(saved.get("Owner").is_none());
                    assert!(saved.get("Thrower").is_none());
                }
                item.target = Some([54, 1, -2, 3]);
                item.thrower = Some([54, 4, 5, 6]);
                let saved =
                    item_entity_to_nbt(&item, [54, 0, 0, 1], Some(&raw), &Default::default());
                assert_eq!(saved.get("Owner"), Some(&Tag::IntArray(vec![54, 1, -2, 3])));
                assert_eq!(
                    saved.get("Thrower"),
                    Some(&Tag::IntArray(vec![54, 4, 5, 6]))
                );
            }
        }
    }

    #[test]
    fn item_entities_round_trip() {
        let e = ItemEntity {
            attachment_lifetime: Default::default(),
            fall_distance: 2.5,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            health: 2,
            fire_ticks: 73,
            thrower: Some([5, 6, 7, 8]),
            previous: Vec3::ZERO,
            ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
            age: 120,
            pickup_delay: 10,
            ..ItemEntity::new(
                7,
                ItemStack {
                    item: Arc::from("minecraft:diamond_pickaxe"),
                    count: 1,
                    damage: 5,
                    extra: None,
                },
                Vec3::new(100.5, 64.0, -31.5),
                Vec3::new(0.0, -0.04, 0.0),
                0.0,
            )
        };
        let tag = item_entity_to_nbt(&e, [1, 2, 3, 4], None, &Default::default());
        assert_eq!(kind(&tag).as_deref(), Some("minecraft:item"));
        assert_eq!(uuid(&tag), Some([1, 2, 3, 4]));
        let back = item_entity_from_nbt(&tag, 9, 0.0).unwrap();
        assert_eq!(back.stack.damage, 5);
        assert_eq!((back.health, back.fire_ticks), (2, 73));
        assert_eq!(
            (back.age, back.pickup_delay, back.ground.on_ground),
            (120, 10, true)
        );
        assert_eq!(back.position.x, 100.5);
        assert_eq!(back.fall_distance, 2.5);
        assert_eq!(back.thrower, e.thrower);
        let mut cleared = back;
        cleared.thrower = None;
        let cleared = item_entity_to_nbt(&cleared, [1, 2, 3, 4], Some(&tag), &Default::default());
        assert!(cleared.get("Thrower").is_none());
    }
    #[test]
    fn nbt_numeric_flags_decode_and_save_modeled_values_without_raw() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = "minecraft:pig".into();
        def.species = crate::mobs::Species::Pig;
        let types = [("minecraft:pig".into(), kind)].into();
        for (input, expected) in [
            (None, false),
            (Some(Tag::Byte(0)), false),
            (Some(Tag::Byte(1)), true),
            (Some(Tag::Byte(-1)), true),
            (Some(Tag::Short(256)), false),
            (Some(Tag::Int(256)), false),
            (Some(Tag::Int(257)), true),
            (Some(Tag::Int(-256)), false),
            (Some(Tag::Long(256)), false),
            (Some(Tag::Long(257)), true),
            (Some(Tag::Long(4294967296)), false),
            (Some(Tag::Long(9007199254740993)), true),
            (Some(Tag::Float(0.5f32.to_bits())), false),
            (Some(Tag::Float((-0.5f32).to_bits())), true),
            (Some(Tag::Float(256.5f32.to_bits())), false),
            (Some(Tag::Float((-255.5f32).to_bits())), false),
            (Some(Tag::Double(0.5f64.to_bits())), false),
            (Some(Tag::Double((-0.5f64).to_bits())), true),
            (Some(Tag::Double(256.5f64.to_bits())), false),
            (Some(Tag::Double((-255.5f64).to_bits())), false),
            (Some(Tag::Float((-1e20f32).to_bits())), false),
            (Some(Tag::Double((-1e20f64).to_bits())), false),
            (Some(Tag::Double(1e20f64.to_bits())), true),
            (Some(string("1")), false),
            (Some(string("true")), false),
            (Some(Tag::compound([("a", Tag::Int(1))])), false),
        ] {
            let mut raw = Tag::compound([
                ("id", string("minecraft:pig")),
                ("Pos", doubles(&[4.5, 200.0, 4.5])),
                ("Health", Tag::Float(10f32.to_bits())),
                ("NoGravity", Tag::Byte(1)),
                ("Unrelated", string("keep")),
            ]);
            if let Some(value) = input.as_ref() {
                for key in ["NoAI", "CanPickUpLoot"] {
                    put(&mut raw, key, value.clone());
                }
            }
            let loaded = mob_from_nbt(&raw, &types, 7, None).unwrap();
            assert_eq!(loaded.no_ai, expected, "NoAI {input:?}");
            assert_eq!(loaded.can_pick_up_loot(), expected, "pickup {input:?}");
            assert!(loaded.no_gravity);
            for source in [None, Some(&raw)] {
                let saved = mob_to_nbt(
                    &loaded,
                    [45, 0, 0, 1],
                    source,
                    None,
                    |_| None,
                    &Default::default(),
                );
                assert_eq!(
                    saved.get("NoAI"),
                    expected.then_some(&Tag::Byte(1)),
                    "NoAI {input:?}"
                );
                assert_eq!(
                    saved.get("CanPickUpLoot"),
                    Some(&Tag::Byte(i8::from(expected))),
                    "pickup {input:?}"
                );
                if source.is_some() {
                    assert_eq!(saved.get("Unrelated"), Some(&string("keep")));
                }
            }
        }
    }

    /// `Goat` reads `HasLeftHorn` and `HasRightHorn` with `getBooleanOr(key,
    /// true)`: a number is true when its byte value is not 0, so an int 256
    /// (byte 0) is a lost horn, as `0b` is.
    #[test]
    fn a_goats_horn_flags_are_read_as_bytes() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:goat".into();
        ty.species = crate::mobs::Species::Goat;
        let types = [("minecraft:goat".into(), kind)].into();
        let lost = |left: Option<Tag>| {
            let mut fields = vec![
                ("id", string("minecraft:goat")),
                ("Pos", doubles(&[4.5, 220.0, 4.5])),
            ];
            fields.extend(left.map(|v| ("HasLeftHorn", v)));
            mob_from_nbt(&Tag::compound(fields), &types, 1, None)
                .unwrap()
                .extra
                .lost_horns
        };
        assert_eq!(lost(None), [false, false]);
        assert_eq!(lost(Some(Tag::Byte(1))), [false, false]);
        assert_eq!(lost(Some(Tag::Byte(0))), [true, false]);
        assert_eq!(lost(Some(Tag::Int(256))), [true, false]);
        assert_eq!(lost(Some(Tag::Int(257))), [false, false]);
    }

    #[test]
    fn non_mob_saves_drop_mob_flags_without_losing_entity_fields() {
        let uuid = [104, 0, 0, 1];
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = "minecraft:pig".into();
        def.species = crate::mobs::Species::Pig;
        let types = [("minecraft:pig".into(), kind)].into();
        for input in [None, Some(0), Some(1)] {
            for id in [
                "minecraft:oak_boat",
                "minecraft:oak_chest_boat",
                "minecraft:armor_stand",
                "minecraft:item",
                "minecraft:pig",
            ] {
                let mut raw = Tag::compound([
                    ("id", string(id)),
                    ("Pos", doubles(&[4.5, 200.0, 4.5])),
                    ("Motion", doubles(&[0.0, 0.0, 0.0])),
                    ("Silent", Tag::Byte(1)),
                    ("NoGravity", Tag::Byte(1)),
                    ("Invulnerable", Tag::Byte(1)),
                    ("PortalCooldown", Tag::Int(17)),
                    ("Tags", Tag::List(vec![string("audit")])),
                    ("Unrelated", string("keep")),
                    (
                        "Item",
                        item_to_nbt(
                            &crate::items::ItemStack::new("minecraft:stone", 1),
                            &Default::default(),
                        ),
                    ),
                ]);
                if let Some(value) = input {
                    put(&mut raw, "NoAI", Tag::Byte(value));
                    put(&mut raw, "PersistenceRequired", Tag::Byte(value));
                }
                let stand = armor_stand_from_nbt(&raw, 1).unwrap();
                let item = item_entity_from_nbt(&raw, 1, 0.0).unwrap();
                let boat = boat_from_nbt(&raw).unwrap();
                for source in [None, Some(&raw)] {
                    let saved = match id {
                        "minecraft:armor_stand" => {
                            armor_stand_to_nbt(&stand, uuid, source, &Default::default())
                        }
                        "minecraft:item" => {
                            item_entity_to_nbt(&item, uuid, source, &Default::default())
                        }
                        "minecraft:pig" => {
                            let mut mob = mob_from_nbt(&raw, &types, 1, None).unwrap();
                            // The writer must use live state, even opposite stale raw values.
                            mob.no_ai = input != Some(1);
                            mob.persistent = input != Some(1);
                            mob_to_nbt(&mob, uuid, source, None, |_| None, &Default::default())
                        }
                        _ => boat_to_nbt(
                            id,
                            boat.0,
                            boat.1,
                            [boat.2, 0.0],
                            (false, 0.0),
                            uuid,
                            source,
                            (id == "minecraft:oak_chest_boat").then_some(&[]),
                            &Default::default(),
                        ),
                    };
                    if id == "minecraft:pig" {
                        let live = input != Some(1);
                        assert_eq!(saved.get("NoAI"), live.then_some(&Tag::Byte(1)));
                        assert_eq!(
                            saved.get("PersistenceRequired"),
                            Some(&Tag::Byte(i8::from(live)))
                        );
                    } else {
                        assert!(saved.get("NoAI").is_none(), "{id} {input:?}");
                        assert!(saved.get("PersistenceRequired").is_none(), "{id} {input:?}");
                    }
                    assert_eq!(saved.get("UUID"), Some(&Tag::IntArray(uuid.to_vec())));
                    assert_eq!(saved.get("Pos"), raw.get("Pos"));
                    if source.is_some() {
                        for key in [
                            "Silent",
                            "NoGravity",
                            "Invulnerable",
                            "PortalCooldown",
                            "Tags",
                            "Unrelated",
                        ] {
                            assert_eq!(saved.get(key), raw.get(key), "{id} {key}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn animal_integer_loads_save_modeled_values_without_raw_source() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = "minecraft:pig".into();
        def.species = crate::mobs::Species::Pig;
        let types = [("minecraft:pig".into(), kind)].into();
        for (input, expected) in [
            (None, 0),
            (Some(Tag::Int(0)), 0),
            (Some(Tag::Byte(-7)), -7),
            (Some(Tag::Short(13)), 13),
            (Some(Tag::Int(i32::MIN)), i32::MIN),
            (Some(Tag::Int(i32::MAX)), i32::MAX),
            (Some(Tag::Long(2147483648)), i32::MIN),
            (Some(Tag::Long(-2147483649)), i32::MAX),
            (Some(Tag::Long(4294967296)), 0),
            (Some(Tag::Long(i64::MAX)), -1),
            (Some(Tag::Long(i64::MIN)), 0),
            (Some(Tag::Float(2.5f32.to_bits())), 2),
            (Some(Tag::Float((-2.5f32).to_bits())), -3),
            (Some(Tag::Double(2.5f64.to_bits())), 2),
            (Some(Tag::Double((-2.5f64).to_bits())), -3),
            (Some(Tag::Float(2147483520f32.to_bits())), 2147483520),
            (Some(Tag::Float(2147483648f32.to_bits())), i32::MAX),
            (Some(Tag::Float((-2147483648f32).to_bits())), i32::MIN),
            (Some(Tag::Float((-2147483904f32).to_bits())), i32::MIN),
            (Some(Tag::Double(2147483647.75f64.to_bits())), i32::MAX),
            (Some(Tag::Double(2147483648f64.to_bits())), i32::MAX),
            (Some(Tag::Double((-2147483647.5f64).to_bits())), i32::MIN),
            (Some(Tag::Double((-2147483648.5f64).to_bits())), i32::MIN),
            (Some(Tag::Double(1e20f64.to_bits())), i32::MAX),
            (Some(Tag::Double((-1e20f64).to_bits())), i32::MIN),
            (Some(Tag::Float(1e20f32.to_bits())), i32::MAX),
            (Some(Tag::Float((-1e20f32).to_bits())), i32::MIN),
        ] {
            let mut raw = Tag::compound([
                ("id", string("minecraft:pig")),
                ("Pos", doubles(&[4.5, 200.0, 4.5])),
                ("NoAI", Tag::Byte(1)),
                ("NoGravity", Tag::Byte(1)),
                ("AgeLocked", Tag::Byte(1)),
                ("Unrelated", string("keep")),
            ]);
            if let Some(value) = input.as_ref() {
                for key in ["Age", "ForcedAge", "InLove"] {
                    put(&mut raw, key, value.clone());
                }
            }
            let mob = mob_from_nbt(&raw, &types, 1, None).unwrap();
            assert_eq!(
                (mob.extra.age, mob.extra.forced_age, mob.extra.in_love),
                (expected, expected, expected),
                "{input:?}"
            );
            assert!(mob.no_ai && mob.no_gravity && mob.extra.age_locked);
            for source in [None, Some(&raw)] {
                let saved = mob_to_nbt(
                    &mob,
                    [0, 0, 0, 55],
                    source,
                    None,
                    |_| None,
                    &Default::default(),
                );
                for key in ["Age", "ForcedAge", "InLove"] {
                    assert_eq!(
                        saved.get(key),
                        Some(&Tag::Int(expected)),
                        "{key}: {input:?}"
                    );
                }
                assert_eq!(saved.get("NoAI"), Some(&Tag::Byte(1)));
                assert_eq!(saved.get("AgeLocked"), Some(&Tag::Byte(1)));
                assert_eq!(
                    saved.get("Unrelated"),
                    source.and_then(|tag| tag.get("Unrelated"))
                );
            }
        }
    }

    #[test]
    fn animal_nbt_preserves_signed_integer_in_love_and_defaults_after_removal() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = "minecraft:pig".into();
        def.species = crate::mobs::Species::Pig;
        let types = [("minecraft:pig".into(), kind)].into();
        for (input, expected) in [
            (Tag::Int(-32769), -32769),
            (Tag::Int(-32768), -32768),
            (Tag::Int(-129), -129),
            (Tag::Int(-128), -128),
            (Tag::Int(-1), -1),
            (Tag::Byte(-1), -1),
            (Tag::Byte(-128), -128),
            (Tag::Int(0), 0),
            (Tag::Int(1), 1),
            (Tag::Int(37), 37),
        ] {
            let raw = Tag::compound([
                ("id", string("minecraft:pig")),
                ("Pos", doubles(&[4.0, 200.0, 4.0])),
                ("NoAI", Tag::Byte(1)),
                ("NoGravity", Tag::Byte(1)),
                ("InLove", input),
            ]);
            let mob = mob_from_nbt(&raw, &types, 1, None).unwrap();
            assert_eq!(mob.extra.in_love, expected);
            // Query snapshots and saves both write the live field as an int,
            // regardless of whether there is an old compound to preserve.
            for old in [None, Some(&raw)] {
                let mut saved =
                    mob_to_nbt(&mob, [0, 0, 0, 1], old, None, |_| None, &Default::default());
                assert_eq!(saved.get("InLove"), Some(&Tag::Int(expected)));
                if let Tag::Compound(map) = &mut saved {
                    map.remove(&"InLove".into());
                }
                let cleared = mob_from_nbt(&saved, &types, 2, None).unwrap();
                assert_eq!(cleared.extra.in_love, 0);
                let saved = mob_to_nbt(
                    &cleared,
                    [0, 0, 0, 1],
                    Some(&saved),
                    None,
                    |_| None,
                    &Default::default(),
                );
                assert_eq!(saved.get("InLove"), Some(&Tag::Int(0)));
            }
        }
    }

    #[test]
    fn unknown_fields_survive() {
        let raw = Tag::compound([
            ("id", string("minecraft:item")),
            ("CustomName", string("x")),
            ("Pos", doubles(&[0.5, 64.0, 0.5])),
            (
                "Item",
                Tag::compound([("id", string("minecraft:stone")), ("count", Tag::Int(3))]),
            ),
        ]);
        let e = item_entity_from_nbt(&raw, 1, 0.0).unwrap();
        let tag = item_entity_to_nbt(&e, [0; 4], Some(&raw), &Default::default());
        assert_eq!(text(tag.get("CustomName")).as_deref(), Some("x"));
        assert_eq!(number(tag.get("Health")), Some(5));
    }
    #[test]
    fn armor_stand_loads_and_saves_pitch() {
        let tag = Tag::compound([
            ("id", string("minecraft:armor_stand")),
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(4103.5f64.to_bits()),
                    Tag::Double(300.0f64.to_bits()),
                    Tag::Double(4103.5f64.to_bits()),
                ]),
            ),
            (
                "Rotation",
                Tag::List(vec![
                    Tag::Float((-170.5f32).to_bits()),
                    Tag::Float((-90.0f32).to_bits()),
                ]),
            ),
        ]);
        let stand = armor_stand_from_nbt(&tag, 1).unwrap();
        assert_eq!(stand.yaw, -170.5);
        assert_eq!(stand.pitch, -90.0);

        let saved = armor_stand_to_nbt(&stand, [1, 2, 3, 4], None, &Default::default());
        let rot = rotation(&saved);
        assert_eq!(rot[0], -170.5);
        assert_eq!(rot[1], -90.0);

        // Out-of-range pitch (e.g. 450.0f) normalizes to 90.0f matching Entity.setXRot.
        let out_of_range = Tag::compound([
            ("id", string("minecraft:armor_stand")),
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(0.0f64.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                ]),
            ),
            (
                "Rotation",
                Tag::List(vec![
                    Tag::Float(0.0f32.to_bits()),
                    Tag::Float(450.0f32.to_bits()),
                ]),
            ),
        ]);
        let normalized = armor_stand_from_nbt(&out_of_range, 2).unwrap();
        assert_eq!(normalized.pitch, 90.0);

        // Nonfinite pitch resets to 0.0f.
        let nonfinite = Tag::compound([
            ("id", string("minecraft:armor_stand")),
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(0.0f64.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                ]),
            ),
            (
                "Rotation",
                Tag::List(vec![
                    Tag::Float(0.0f32.to_bits()),
                    Tag::Float(f32::NAN.to_bits()),
                ]),
            ),
        ]);
        let nan_stand = armor_stand_from_nbt(&nonfinite, 3).unwrap();
        assert_eq!(nan_stand.pitch, 0.0);
    }

    #[test]
    fn negative_facing_wraps_like_vanilla_direction_codecs() {
        // from3DDataValue: abs(-1 % 6) = 1 (UP); 7 -> 1; -6 -> 0.
        for (raw, want) in [(-1i8, 1u8), (7, 1), (-6, 0), (5, 5), (0, 0)] {
            let tag = Tag::compound([
                ("Facing", Tag::Byte(raw)),
                ("block_pos", Tag::IntArray(vec![0, 0, 0])),
            ]);
            assert_eq!(item_frame_from_nbt(&tag, 1).unwrap().facing, want);
        }
        // from2DDataValue: abs(-1 % 4) = 1.
        let tag = Tag::compound([
            ("facing", Tag::Byte(-1)),
            ("block_pos", Tag::IntArray(vec![0, 0, 0])),
        ]);
        assert_eq!(painting_from_nbt(&tag).unwrap().1, 1);
    }

    #[test]
    fn saved_rotation_is_the_entitys_own() {
        // `/tp <entity> x y z 90 10` sets the entity's yaw and pitch; the save
        // (`/data get ... Rotation`) writes them, not [0, 0].
        let tnt = tnt_to_nbt(
            Vec3::ZERO,
            Vec3::ZERO,
            false,
            0.0,
            20,
            4.0,
            string("minecraft:tnt"),
            [90.0, 10.0],
            [1; 4],
            None,
            None,
        );
        assert_eq!(rotation(&tnt), [90.0, 10.0]);
        let mut stand = ArmorStand::new(1, Vec3::ZERO, 0.0);
        stand.yaw = 90.0;
        stand.pitch = 10.0;
        assert_eq!(
            rotation(&armor_stand_to_nbt(
                &stand,
                [1; 4],
                None,
                &Default::default()
            )),
            [90.0, 10.0]
        );
        let cart = vehicle_to_nbt(
            "minecraft:minecart",
            Vec3::ZERO,
            Vec3::ZERO,
            [90.0, 10.0],
            (false, 0.0),
            [1; 4],
            None,
        );
        assert_eq!(rotation(&cart), [90.0, 10.0]);
    }

    #[test]
    fn entities_without_fire_save_zero_and_a_mob_saves_its_speed_instance() {
        // Entity.remainingFireTicks starts at 0; LivingEntity.removeFrost makes
        // the movement_speed instance AttributeMap.pack then saves.
        let orb = orbs_from_nbt(&Tag::compound([("Pos", doubles(&[0.0; 3]))])).remove(0);
        let tag = orb_to_nbt(&orb, [0; 4]);
        assert_eq!(tag.get("Fire"), Some(&Tag::Short(0)));
        // A stand summoned this tick has no instance yet; its first tick makes
        // it, and a save that lists it (`/data merge`, a reload) keeps it.
        let attributes =
            |stand: &ArmorStand| match armor_stand_to_nbt(stand, [0; 4], None, &Default::default())
                .get("attributes")
            {
                Some(Tag::List(attrs)) => attrs
                    .iter()
                    .map(|a| (text(a.get("id")).unwrap(), a.get("base").cloned()))
                    .collect::<Vec<_>>(),
                other => panic!("attributes: {other:?}"),
            };
        let mut stand = ArmorStand::new(1, Vec3::ZERO, 0.0);
        assert_eq!(attributes(&stand), []);
        stand.speed_instance = true;
        let speed = vec![(
            "minecraft:movement_speed".to_owned(),
            Some(Tag::Double(0.7f64.to_bits())),
        )];
        assert_eq!(attributes(&stand), speed);
        let saved = armor_stand_to_nbt(&stand, [0; 4], None, &Default::default());
        assert_eq!(attributes(&armor_stand_from_nbt(&saved, 1).unwrap()), speed);
    }
    #[test]
    fn a_loaded_stand_gains_the_speed_instance_its_first_tick_makes() {
        // Vanilla 26.3: `/summon armor_stand ~ ~ ~ {attributes:[]}` saves `[]` until the
        // first tick, whose removeFrost makes movement_speed (base 0.7d); a saved
        // instance of another attribute stays in front of it.
        let ids = |stand: &ArmorStand, raw: &Tag| match armor_stand_to_nbt(
            stand,
            [0; 4],
            Some(raw),
            &Default::default(),
        )
        .get("attributes")
        {
            Some(Tag::List(attrs)) => attrs
                .iter()
                .map(|a| (text(a.get("id")).unwrap(), a.get("base").cloned()))
                .collect::<Vec<_>>(),
            other => panic!("attributes: {other:?}"),
        };
        let health = (
            "minecraft:max_health".to_owned(),
            Some(Tag::Double(20f64.to_bits())),
        );
        let speed = (
            "minecraft:movement_speed".to_owned(),
            Some(Tag::Double(0.7f64.to_bits())),
        );
        let empty = Tag::compound([("attributes", Tag::List(Vec::new()))]);
        let saved = Tag::compound([(
            "attributes",
            Tag::List(vec![Tag::compound([
                ("id", string("minecraft:max_health")),
                ("base", Tag::Double(20f64.to_bits())),
            ])]),
        )]);
        let mut stand = ArmorStand::new(1, Vec3::ZERO, 0.0);
        assert_eq!(ids(&stand, &empty), []);
        assert_eq!(ids(&stand, &saved), std::slice::from_ref(&health));
        stand.speed_instance = true;
        assert_eq!(ids(&stand, &empty), std::slice::from_ref(&speed));
        assert_eq!(ids(&stand, &saved), [health, speed]);
    }
    #[test]
    fn summoned_trident_defaults_are_not_pickable_or_already_shot() {
        let raw = Tag::compound([
            ("Pos", doubles(&[0.0, 0.0, 0.0])),
            ("LeftOwner", Tag::Byte(0)),
        ]);
        let loaded = trident_from_nbt(&raw).unwrap();
        assert_eq!(loaded.pickup, Pickup::Disallowed);
        assert!(!loaded.left_owner && !loaded.has_been_shot);
        let trident = ItemStack::new("minecraft:trident", 1);
        let tag = trident_to_nbt(
            &TridentSave {
                position: Vec3::ZERO,
                motion: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
                in_ground: false,
                life: 0,
                shake: 0,
                dealt_damage: false,
                pickup: loaded.pickup,
                left_owner: loaded.left_owner,
                has_been_shot: loaded.has_been_shot,
                owner: loaded.owner,
                no_gravity: false,
                on_ground: false,
                fire_ticks: 0,
                fall_distance: 0.0,
                sound_event: TRIDENT_HIT_GROUND,
                item: &trident,
            },
            [0; 4],
            Some(&raw),
            &Default::default(),
        );
        assert!(tag.get("LeftOwner").is_none());
        assert_eq!(tag.get("HasBeenShot"), Some(&Tag::Byte(0)));
        assert_eq!(tag.get("pickup"), Some(&Tag::Byte(0)));
    }
    #[test]
    fn orbs_keep_their_count() {
        let at = ("Pos", doubles(&[1.0, 2.0, 3.0]));
        let loaded = [at, ("Value", Tag::Short(7)), ("Age", Tag::Short(3))];
        let o = orbs_from_nbt(&Tag::compound(
            loaded
                .into_iter()
                .chain([("Count", Tag::Int(3)), ("Air", Tag::Short(32767))]),
        ))
        .remove(0);
        let tag = orb_to_nbt(&o, [0; 4]);
        let back = orbs_from_nbt(&tag);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].value, 7);
        assert_eq!(back[0].count, 3);
        // Entity.load keeps the saved Air; the orb writes it back unchanged.
        assert_eq!(back[0].air, 32767);
        assert_eq!(tag.get("Air"), Some(&Tag::Short(32767)));
        assert_eq!(
            orb_to_nbt(&back[0], [0; 4]).get("Air"),
            Some(&Tag::Short(32767))
        );
    }
    /// `ExperienceOrb.readAdditionalSaveData` reads `Age` with `getShortOr`,
    /// unclamped, and `addAdditionalSaveData` writes `(short) age`: `Age:-100s`
    /// loads as -100, saves `-99s` after one `++age`, and an age past a short
    /// wraps.
    #[test]
    fn an_orb_keeps_a_negative_age() {
        let at = ("Pos", doubles(&[1.0, 2.0, 3.0]));
        let mut o = orbs_from_nbt(&Tag::compound([at, ("Age", Tag::Short(-100))])).remove(0);
        assert_eq!(o.age, -100);
        o.age += 1;
        assert_eq!(orb_to_nbt(&o, [0; 4]).get("Age"), Some(&Tag::Short(-99)));
        o.age = 40_000;
        assert_eq!(
            orb_to_nbt(&o, [0; 4]).get("Age"),
            Some(&Tag::Short(-25_536))
        );
    }
    #[test]
    fn every_orb_set_experience_orb_accepts_reloads_unchanged() {
        let at = Tag::compound([("Pos", doubles(&[1.0, 2.0, 3.0]))]);
        let mut o = orbs_from_nbt(&at).remove(0);
        for (value, count) in [(11, 1), (32767, 2147483647), (-32768, 1)] {
            (o.value, o.count) = (value, count);
            let back = orbs_from_nbt(&orb_to_nbt(&o, [0; 4])).remove(0);
            assert_eq!((back.value, back.count), (value, count));
        }
    }
    /// `Entity.saveWithoutId` writes `Fire` for every entity and `Entity.load`
    /// reads it back with `getShortOr("Fire", 0)`: a summon's `Fire:40s` and the
    /// `-1s` an entity carries after leaving water both survive, an absent key
    /// is 0, and the short wraps like `(short) remainingFireTicks`.
    #[test]
    fn orbs_save_and_load_their_fire() {
        let mut o = ExperienceOrb {
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 0,
            position: Vec3::new(1.0, 2.0, 3.0),
            previous: Vec3::ZERO,
            velocity: Vec3::ZERO,
            value: 7,
            age: 3,
            ground: Default::default(),
            count: 1,
            ticks: 0,
            health: 5,
            air: 300,
            following: None,
            fall_distance: 0.0,
            fire_ticks: 0,
        };
        for (ticks, saved) in [(0, 0), (40, 40), (-1, -1), (300, 300), (40_000, -25_536)] {
            o.fire_ticks = ticks;
            let tag = orb_to_nbt(&o, [0; 4]);
            assert_eq!(tag.get("Fire"), Some(&Tag::Short(saved)), "saved {ticks}");
            assert_eq!(orbs_from_nbt(&tag)[0].fire_ticks, i32::from(saved));
        }
        let mut tag = orb_to_nbt(&o, [0; 4]);
        let Tag::Compound(map) = &mut tag else {
            unreachable!()
        };
        map.remove(&"Fire".into());
        assert_eq!(orbs_from_nbt(&tag)[0].fire_ticks, 0);
    }
}

/// `Brain.Packed`: serialize each owned Warden Unit memory from its live slot.
fn write_warden_owned_memories(m: &Mob, map: &mut EntityTagWriter) {
    let brain = map
        .entry("Brain".into())
        .or_insert_with(|| Tag::Compound(Compound::new()));
    if !matches!(brain, Tag::Compound(_)) {
        *brain = Tag::Compound(Compound::new());
    }
    let Tag::Compound(brain) = brain else {
        unreachable!()
    };
    let memories = brain
        .entry("memories".into())
        .or_insert_with(|| Tag::Compound(Compound::new()));
    if !matches!(memories, Tag::Compound(_)) {
        *memories = Tag::Compound(Compound::new());
    }
    let Tag::Compound(memories) = memories else {
        unreachable!()
    };
    let mem = m.extra.warden.as_ref().map(|st| &st.brain.mem);
    for (key, slot) in [
        ("minecraft:is_emerging", mem.map(|mem| &mem.is_emerging)),
        ("minecraft:dig_cooldown", mem.map(|mem| &mem.dig_cooldown)),
        (
            "minecraft:sonic_boom_cooldown",
            mem.map(|mem| &mem.sonic_boom_cooldown),
        ),
    ] {
        if let Some(slot) = slot.filter(|slot| slot.present()) {
            let mut entry = Tag::compound([("value", Tag::Compound(Compound::new()))]);
            if slot.ttl() != i64::MAX {
                put(&mut entry, "ttl", Tag::Long(slot.ttl()));
            }
            memories.insert(key.into(), entry);
        } else {
            memories.remove(&key.into());
        }
    }
}
/// `Brain` / `ExpirableValue`: Unit values with a lenient optional numeric TTL.
fn restore_warden_owned_memories(m: &mut Mob, tag: &Tag) {
    let memories = tag.get("Brain").and_then(|brain| brain.get("memories"));
    let Some(st) = m.extra.warden.as_mut() else {
        return;
    };
    for (key, slot) in [
        ("minecraft:is_emerging", &mut st.brain.mem.is_emerging),
        ("minecraft:dig_cooldown", &mut st.brain.mem.dig_cooldown),
        (
            "minecraft:sonic_boom_cooldown",
            &mut st.brain.mem.sonic_boom_cooldown,
        ),
    ] {
        let Some(entry) = memories.and_then(|memories| memories.get(key)) else {
            // Fresh Java read/load: absent entries do not retain finalize's slot.
            slot.clear();
            continue;
        };
        if !matches!(entry.get("value"), Some(Tag::Compound(value)) if value.is_empty()) {
            continue; // Unmeasured malformed shapes retain existing initialization.
        }
        match sunrise_client_core::nbt_value::number(entry.get("ttl")) {
            Some(ttl) => slot.set_ttl((), ttl),
            None => slot.set(()),
        }
    }
}

fn write_axolotl_memories(m: &Mob, map: &mut EntityTagWriter) {
    let brain = map
        .entry("Brain".into())
        .or_insert_with(|| Tag::Compound(Compound::new()));
    if !matches!(brain, Tag::Compound(_)) {
        *brain = Tag::Compound(Compound::new());
    }
    let Tag::Compound(brain) = brain else { return };
    let memories = brain
        .entry("memories".into())
        .or_insert_with(|| Tag::Compound(Compound::new()));
    if !matches!(memories, Tag::Compound(_)) {
        *memories = Tag::Compound(Compound::new());
    }
    let Tag::Compound(memories) = memories else {
        return;
    };
    let mm = m.extra.animal_brain.as_deref().map(|b| &b.mem);
    let hunting = mm.and_then(|mem| {
        let slot = &mem.hunting_cooldown;
        slot.value().map(|value| {
            let mut entry = Tag::compound([("value", Tag::Byte(i8::from(value)))]);
            if slot.ttl() != i64::MAX {
                put(&mut entry, "ttl", Tag::Long(slot.ttl()));
            }
            entry
        })
    });
    // ExpirableValue.codec: integer value and optional finite long TTL.
    let integer_memory = |slot: &crate::mobs::animal_brain::Slot<i32>| {
        slot.value().map(|v| {
            let mut entry = Tag::compound([("value", Tag::Int(v))]);
            if slot.ttl() != i64::MAX {
                put(&mut entry, "ttl", Tag::Long(slot.ttl()));
            }
            entry
        })
    };
    let play_dead = mm.and_then(|m| integer_memory(&m.play_dead_ticks));
    let temptation = mm.and_then(|m| integer_memory(&m.temptation_cooldown_ticks));
    for (key, value) in [
        ("minecraft:has_hunting_cooldown", hunting),
        ("minecraft:play_dead_ticks", play_dead),
        ("minecraft:temptation_cooldown_ticks", temptation),
    ] {
        if let Some(value) = value {
            memories.insert(key.into(), value);
        } else {
            memories.remove(&key.into());
        }
    }
}
/// Saved variants and fish/axolotl bucket flags. Missing variants use a
/// medium salmon or index 0; a missing bucket flag is false.
fn read_variant(m: &mut Mob, tag: &Tag) {
    use crate::mobs::Species as S;
    // AbstractFish/Axolotl: false when absent. Tadpole.setFromBucket ignores the input.
    if matches!(
        m.species(),
        S::Cod | S::Salmon | S::Pufferfish | S::TropicalFish | S::Axolotl
    ) {
        m.extra.from_bucket = flag(tag, "FromBucket");
    }
    let int = |key| number(tag.get(key)).unwrap_or(0) as i32;
    match m.species() {
        S::Parrot => m.extra.variant = int("Variant").clamp(0, 4) as u8, // CLAMP
        S::Axolotl => m.extra.variant = ((int("Variant") as u32).min(5) % 5) as u8, // ZERO: lucy
        // Rabbit.Variant.LEGACY_CODEC: an unknown id is brown.
        S::Rabbit => {
            m.extra.variant = match int("RabbitType") {
                v @ (0..=5 | 99) => v as u8,
                _ => 0,
            }
        }
        S::Salmon => {
            m.extra.variant = text(tag.get("type"))
                .and_then(|t| SALMON_SIZES.iter().position(|v| *v == t))
                .unwrap_or(1) as u8;
        }
        S::TropicalFish => {
            let packed = int("Variant");
            let (size, index) = (packed & 0xFF, (packed >> 8) & 0xFF);
            // TropicalFish.Pattern.byId: an unknown id is a kob.
            m.extra.variant = if size <= 1 && index < 6 {
                (size * 6 + index) as u8
            } else {
                0
            };
            // TropicalFish.getBaseColor/getPatternColor use DyeColor.byId:
            // read the full byte, then default invalid color ids to white.
            let color = |shift| match (packed >> shift) & 0xFF {
                value @ 0..=15 => value as u8,
                _ => 0,
            };
            m.extra.variant2 = color(16);
            m.extra.variant3 = color(24);
        }
        _ => {}
    }
}
fn restore_axolotl_memories(m: &mut Mob, tag: &Tag) {
    let memories = tag.get("Brain").and_then(|b| b.get("memories"));
    // Initialize/clear the slot without applying the runtime setter's positive-TTL rule to saved data.
    crate::mobs::animal_brain::set_hunting_cooldown(m, 0);
    if let Some(b) = m.extra.animal_brain.as_mut() {
        if let Some(entry) = memories.and_then(|v| v.get("minecraft:has_hunting_cooldown"))
            && let Some(value) = float(entry.get("value"))
        {
            // NbtOps boolean numbers use doubleValue()!=0; ExpirableValue's optional TTL is lenient.
            let ttl = number(entry.get("ttl")).unwrap_or(i64::MAX);
            b.mem.hunting_cooldown.set_ttl(value != 0.0, ttl);
        }
        for (key, slot) in [
            ("minecraft:play_dead_ticks", &mut b.mem.play_dead_ticks),
            (
                "minecraft:temptation_cooldown_ticks",
                &mut b.mem.temptation_cooldown_ticks,
            ),
        ] {
            if let Some(entry) = memories.and_then(|b| b.get(key))
                && let Some(value) = number(entry.get("value"))
            {
                // ExpirableValue.codec's lenient optional TTL defaults to permanent.
                slot.set_ttl(value as i32, number(entry.get("ttl")).unwrap_or(i64::MAX));
            }
        }
    }
}
/// `Axolotl.Variant` names, by id.
pub const AXOLOTL_VARIANTS: [&str; 5] = ["lucy", "wild", "gold", "cyan", "blue"];
/// `Salmon.Variant` names, by id.
const SALMON_SIZES: [&str; 3] = ["small", "medium", "large"];
/// `TropicalFish.Pattern` names in enum order: the six small patterns, then
/// the six large ones (the order of `extra.variant`).
pub const TROPICAL_PATTERNS: [&str; 12] = [
    "kob",
    "sunstreak",
    "snooper",
    "dasher",
    "brinely",
    "spotty",
    "flopper",
    "stripey",
    "glitter",
    "blockfish",
    "betty",
    "clayfish",
];
/// `Bucketable.saveDefaultDataToBucketTag`, with the age an axolotl and a
/// tadpole add (`saveToBucketTag`).
pub fn bucket_data(m: &Mob) -> Tag {
    let mut data = Tag::compound([("Health", Tag::Float(m.health.to_bits()))]);
    if matches!(
        m.species(),
        crate::mobs::Species::Axolotl | crate::mobs::Species::Tadpole
    ) {
        put(&mut data, "Age", Tag::Int(m.extra.age));
        put(
            &mut data,
            "AgeLocked",
            Tag::Byte(i8::from(m.extra.age_locked)),
        );
    }
    for (key, on) in [
        ("NoAI", m.no_ai),
        ("Silent", m.silent),
        ("NoGravity", m.no_gravity),
        ("Glowing", m.glowing),
        ("Invulnerable", m.invulnerable_flag),
        ("PersistenceRequired", m.persistent),
    ] {
        if on {
            put(&mut data, key, Tag::Byte(1));
        }
    }
    if let Some(ttl) = crate::mobs::animal_brain::hunting_cooldown(m) {
        put(&mut data, "HuntingCooldown", Tag::Long(ttl));
    }
    // SulfurCube.saveToBucketTag: after the default data, its age.
    if m.species() == crate::mobs::Species::SulfurCube {
        put(&mut data, "age", Tag::Int(m.extra.age));
        put(
            &mut data,
            "age_locked",
            Tag::Byte(i8::from(m.extra.age_locked)),
        );
    }
    data
}
pub fn bucket_components(m: &Mob) -> crate::item_data::StackData {
    let mut data = crate::item_data::StackData::default();
    data.custom_name = m
        .custom_name
        .as_ref()
        .map(|name| Tag::String(name.as_str().into()));
    data.set_other("minecraft:bucket_entity_data", Some(bucket_data(m)));
    // SulfurCube.saveToBucketTag: the body block, when it has one.
    if m.species() == crate::mobs::Species::SulfurCube
        && let Some(body) = m.extra.body_stack()
    {
        data.set_other(
            "minecraft:sulfur_cube_content",
            Some(crate::item_data::raw_template_tag(body)),
        );
    }
    if m.species() == crate::mobs::Species::Axolotl {
        let variant = AXOLOTL_VARIANTS
            .get(usize::from(m.extra.variant))
            .unwrap_or(&"lucy");
        data.set_other("minecraft:axolotl/variant", Some(string(variant)));
    }
    // Salmon.saveToBucketTag: the size.
    if m.species() == crate::mobs::Species::Salmon {
        let size = SALMON_SIZES
            .get(usize::from(m.extra.variant))
            .unwrap_or(&"medium");
        data.set_other("minecraft:salmon/size", Some(string(size)));
    }
    // TropicalFish.saveToBucketTag: the pattern and both colours.
    if m.species() == crate::mobs::Species::TropicalFish {
        let dye = |i: u8| crate::mobs::render::DYE_NAMES[usize::from(i) & 15];
        let pattern = TROPICAL_PATTERNS
            .get(usize::from(m.extra.variant))
            .unwrap_or(&"kob");
        data.set_other("minecraft:tropical_fish/pattern", Some(string(pattern)));
        data.set_other(
            "minecraft:tropical_fish/base_color",
            Some(string(dye(m.extra.variant2))),
        );
        data.set_other(
            "minecraft:tropical_fish/pattern_color",
            Some(string(dye(m.extra.variant3))),
        );
    }
    data
}
/// `Entity.applyComponentsFromItemStack` → the spawned mob's
/// `applyImplicitComponents`: a bucket's contents (`MobBucketItem.spawn`) and
/// a spawn egg (`PostSpawnProcessor.apply`) both land here. Vanilla applies
/// these after `finalizeSpawn`, so a component wins over the biome variant.
pub fn apply_item_components(m: &mut Mob, stack: &crate::items::ItemStack) {
    let Some(components) = stack.data() else {
        return;
    };
    if let Some(name) = stack.custom_name() {
        m.custom_name = Some(name);
    }
    if m.species() == crate::mobs::Species::SulfurCube
        && let Some(body) = components.template_stack("minecraft:sulfur_cube_content")
    {
        m.extra.set_body_stack(Some(body));
    }
    if let Some(data) = components.other("minecraft:bucket_entity_data") {
        if let Some(v) = number(data.get("Age")) {
            m.extra.age = v as i32;
        }
        if let Some(v) = number(data.get("AgeLocked")) {
            m.extra.age_locked = v != 0;
        }
        // Keep old Sunrise buckets readable; the modern component below takes precedence.
        if let Some(v) = number(data.get("Variant")) {
            m.extra.variant = v as u8;
        }
        if let Some(v) = float(data.get("Health")) {
            m.health = v as f32;
        }
        for (key, flag) in [
            ("NoAI", &mut m.no_ai),
            ("Silent", &mut m.silent),
            ("NoGravity", &mut m.no_gravity),
            ("Glowing", &mut m.glowing),
            ("Invulnerable", &mut m.invulnerable_flag),
        ] {
            if let Some(v) = number(data.get(key)) {
                *flag = v != 0;
            }
        }
        m.persistent |= flag(data, "PersistenceRequired");
        if m.species() == crate::mobs::Species::Axolotl {
            crate::mobs::animal_brain::set_hunting_cooldown(
                m,
                number(data.get("HuntingCooldown")).unwrap_or(0),
            );
        }
    }
    if m.species() == crate::mobs::Species::Axolotl
        && let Some(name) = text(components.other("minecraft:axolotl/variant"))
        && let Some(id) = AXOLOTL_VARIANTS
            .iter()
            .position(|&v| v == name.trim_start_matches("minecraft:"))
    {
        m.extra.variant = id as u8;
    }
    // Salmon.applyImplicitComponents: the size.
    if m.species() == crate::mobs::Species::Salmon
        && let Some(name) = text(components.other("minecraft:salmon/size"))
        && let Some(id) = SALMON_SIZES
            .iter()
            .position(|&v| v == name.trim_start_matches("minecraft:"))
    {
        m.extra.variant = id as u8;
    }
    // TropicalFish.applyImplicitComponent: the pattern and both colours.
    if m.species() == crate::mobs::Species::TropicalFish {
        let name = |key| text(components.other(key));
        let dye = |key| {
            name(key).and_then(|n| {
                crate::mobs::render::DYE_NAMES
                    .iter()
                    .position(|&v| v == n.trim_start_matches("minecraft:"))
            })
        };
        if let Some(id) = name("minecraft:tropical_fish/pattern").and_then(|n| {
            TROPICAL_PATTERNS
                .iter()
                .position(|&v| v == n.trim_start_matches("minecraft:"))
        }) {
            m.extra.variant = id as u8;
        }
        if let Some(id) = dye("minecraft:tropical_fish/base_color") {
            m.extra.variant2 = id as u8;
        }
        if let Some(id) = dye("minecraft:tropical_fish/pattern_color") {
            m.extra.variant3 = id as u8;
        }
    }
    // Chicken, Cow and Pig.applyImplicitComponents: their variant and sound
    // variant components, both registry names (`minecraft:warm`,
    // `minecraft:picky`).
    let species = m.species();
    let (variant, sound) = match species {
        crate::mobs::Species::Chicken => (
            Some("minecraft:chicken/variant"),
            Some("minecraft:chicken/sound_variant"),
        ),
        crate::mobs::Species::Cow => (
            Some("minecraft:cow/variant"),
            Some("minecraft:cow/sound_variant"),
        ),
        crate::mobs::Species::Pig => (
            Some("minecraft:pig/variant"),
            Some("minecraft:pig/sound_variant"),
        ),
        // ZombieNautilus.applyImplicitComponents: the variant only.
        crate::mobs::Species::ZombieNautilus => (Some("minecraft:zombie_nautilus/variant"), None),
        // Fox and Frog.applyImplicitComponent: `setVariant` from the component
        // alone, so a spawn egg carrying one hatches that variant.
        crate::mobs::Species::Fox => (Some("minecraft:fox/variant"), None),
        crate::mobs::Species::Frog => (Some("minecraft:frog/variant"), None),
        // Horse.applyImplicitComponent: `setVariant` sets the coat colour and
        // keeps the markings the spawn picked (`getTypeVariant() & -256`),
        // which Sunrise keeps in `extra.variant2` and leaves alone here.
        crate::mobs::Species::Horse => (Some("minecraft:horse/variant"), None),
        // Llama.applyImplicitComponent, which a trader llama inherits.
        crate::mobs::Species::Llama | crate::mobs::Species::TraderLlama => {
            (Some("minecraft:llama/variant"), None)
        }
        _ => (None, None),
    };
    if let Some(name) = variant.and_then(|id| text(components.other(id))) {
        m.extra.variant = crate::mobs::finalize::variant_index(species, &name);
    }
    if let Some(name) = sound.and_then(|id| text(components.other(id)))
        && let Some(i) = crate::mobs::finalize::sound_variants(species)
            .iter()
            .position(|v| *v == name.trim_start_matches("minecraft:"))
    {
        m.extra.sound_variant = Some(i as u8);
    }
    // MushroomCow.applyImplicitComponents: `Variant.DEFAULT` is red, as the
    // save's `Type` reads it.
    if species == crate::mobs::Species::Mooshroom
        && let Some(name) = text(components.other("minecraft:mooshroom/variant"))
    {
        m.extra.variant = u8::from(name.trim_start_matches("minecraft:") == "brown");
    }
    // Sheep.applyImplicitComponents: the wool colour, a DyeColor id.
    if species == crate::mobs::Species::Sheep
        && let Some(name) = text(components.other("minecraft:sheep/color"))
        && let Some(i) = crate::mobs::render::DYE_NAMES
            .iter()
            .position(|c| *c == name.trim_start_matches("minecraft:"))
    {
        m.extra.color = i as u8;
    }
}

#[cfg(test)]
mod axolotl_nbt_tests;

#[cfg(test)]
mod sheep_nbt_tests;

#[cfg(test)]
mod item_component_tests;

#[cfg(test)]
mod fireball_nbt_tests;

#[cfg(test)]
mod neutral_anger_tests;

#[cfg(test)]
mod uuid_bridge_tests;

#[cfg(test)]
mod warden_anger_tests;

#[cfg(test)]
mod held_item_name_tests;

#[cfg(test)]
mod shulker_bullet_nbt_tests;

#[cfg(test)]
#[path = "spectral_duration_nbt_tests.rs"]
mod spectral_duration_nbt_tests;

#[cfg(test)]
mod mod_thrown_nbt_tests;

#[cfg(test)]
mod equipment_tests;

#[cfg(test)]
mod axolotl_memory_ttl_tests;

#[cfg(test)]
mod generic_home_nbt_tests;

#[cfg(test)]
mod piglin_live_anger_memory_tests;

#[cfg(test)]
mod pickup_loot_save_tests;

#[cfg(test)]
mod stand_fall_save_tests;

#[cfg(test)]
mod animal_love_cause_tests;

#[cfg(test)]
mod carrier_inventory_tests;

/// Added controls (H192): the sulfur cube's saved archetype attributes are
/// its map's base and permanent modifiers, replacing a stale entry of the
/// same id once, never its transient modifiers.
#[cfg(test)]
mod sulfur_attribute_save_tests;

#[cfg(test)]
mod mob_hidden_speed_tests;

#[cfg(test)]
mod mob_effect_amplifier_codec;

#[cfg(test)]
mod camel_inherited_saved_tests;

#[cfg(test)]
mod equine_temper_preservation_tests;
