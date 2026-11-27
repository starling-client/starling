//! SynchedEntityData values (ClientboundSetEntityDataPacket): an entry index,
//! the EntityDataSerializers id, and the serializer's value, ending with 0xFF.
use crate::{
    Result,
    buf::{Codec, Ident, Opt, Pos, Reader, TrustedComponent},
    item::{Slot, Stack, Template},
    packets::login::PropertyCodec,
};
use sunrise_core::{nbt::Tag, profile::AccountProperty, vanilla_bytes::Writer};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(i8),
    Int(i32),
    Long(i64),
    Float(f32),
    String(String),
    Text(Tag),
    OptText(Option<Tag>),
    Item(Option<Stack>),
    Bool(bool),
    Floats(Vec<f32>),
    Pos([i32; 3]),
    OptPos(Option<[i32; 3]>),
    OptUuid(Option<u128>),
    /// Any VarInt-valued serializer (direction, block state, pose, variants ...).
    Id(i32),
    /// Several VarInts (VillagerData).
    Ids(Vec<i32>),
    /// A particle choice: the `minecraft:particle_type` id plus its data.
    /// Rendered nowhere (the client spawns no world particles yet); decoded
    /// so the rest of an entry list survives a particle slot.
    Particle(Particle),
    /// A particle list (PARTICLES: a VarInt count, then each particle).
    Particles(Particles),
    /// A `ResolvableProfile` (a mannequin's DATA_PROFILE).
    Profile(Profile),
    /// `OPTIONAL_GLOBAL_POS`: optional `(dimension, block_pos)`.
    OptGlobalPos(Option<(String, [i32; 3])>),
    /// `PAINTING_VARIANT` (`Holder<PaintingVariant>`): a resolved registry entry or inline variant.
    PaintingVariant(PaintingHolder),
}
/// `Holder<PaintingVariant>`: either a resolved `minecraft:painting_variant` entry or an inline value.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintingHolder {
    /// Resolved registry reference (`id` is 0-based; encoded on the wire as `id + 1`).
    Reference {
        id: i32,
        name: String,
        variant: PaintingVariant,
    },
    /// Inline `PaintingVariant.DIRECT_STREAM_CODEC` (encoded on the wire as `0`).
    Direct(PaintingVariant),
}
/// `PaintingVariant.DIRECT_STREAM_CODEC`: inline painting variant when
/// `PAINTING_VARIANT`'s holder id is 0.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintingVariant {
    pub width: i32,
    pub height: i32,
    pub asset_id: String,
    pub title: Option<Tag>,
    pub author: Option<Tag>,
}
/// `ResolvableProfile.STREAM_CODEC`: the GameProfile a mannequin or player
/// head resolves — either a full profile (id, name and properties on the
/// wire) or a partial one (a name and/or id to look up) — followed by a
/// `PlayerSkin.Patch` of texture overrides.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub name: Option<String>,
    pub uuid: Option<u128>,
    /// `PropertyMap`: its `textures` property carries the skin.
    pub properties: Vec<AccountProperty>,
    /// `PlayerSkin.Patch`: texture asset ids overriding the profile's.
    pub body: Option<String>,
    pub cape: Option<String>,
    pub elytra: Option<String>,
    /// `PlayerModelType`: true is the slim (4px arms) model.
    pub slim: Option<bool>,
}
/// `ByteBufCodecs.GAME_PROFILE_PROPERTIES`.
fn read_properties(r: &mut Reader) -> Result<Vec<AccountProperty>> {
    let n = r.count(16)?;
    (0..n).map(|_| PropertyCodec::read(r)).collect()
}
fn write_properties(p: &[AccountProperty], w: &mut Writer) {
    w.varint(p.len() as i32);
    p.iter().for_each(|p| PropertyCodec::write(p, w));
}
fn read_profile(r: &mut Reader) -> Result<Profile> {
    let mut p = Profile::default();
    // ByteBufCodecs.either: true is the full GameProfile, false a Partial.
    if r.bool()? {
        p.uuid = Some(r.uuid()?);
        p.name = Some(r.string(16)?);
        p.properties = read_properties(r)?;
    } else {
        if r.bool()? {
            p.name = Some(r.string(16)?);
        }
        if r.bool()? {
            p.uuid = Some(r.uuid()?);
        }
        p.properties = read_properties(r)?;
    }
    // PlayerSkin.Patch: three optional texture ids, then the model type.
    if r.bool()? {
        p.body = Some(r.string(32767)?);
    }
    if r.bool()? {
        p.cape = Some(r.string(32767)?);
    }
    if r.bool()? {
        p.elytra = Some(r.string(32767)?);
    }
    if r.bool()? {
        p.slim = Some(r.bool()?);
    }
    Ok(p)
}
fn write_profile(p: &Profile, w: &mut Writer) {
    let full = p.uuid.is_some() && p.name.is_some();
    w.bool(full);
    if full {
        w.uuid(p.uuid.unwrap_or(0));
        w.string(p.name.as_deref().unwrap_or(""));
    } else {
        w.bool(p.name.is_some());
        if let Some(name) = &p.name {
            w.string(name);
        }
        w.bool(p.uuid.is_some());
        if let Some(uuid) = p.uuid {
            w.uuid(uuid);
        }
    }
    write_properties(&p.properties, w);
    for texture in [&p.body, &p.cape, &p.elytra] {
        w.bool(texture.is_some());
        if let Some(texture) = texture {
            w.string(texture);
        }
    }
    w.bool(p.slim.is_some());
    if let Some(slim) = p.slim {
        w.bool(slim);
    }
}
/// One `ParticleTypes.STREAM_CODEC` value: the `minecraft:particle_type` id,
/// then the data its type carries (26.3 registry order).
#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    pub id: i32,
    pub data: Option<ParticleData>,
}
const COMPLEX_PARTICLE_TAG: u8 = 0xFF;

/// Compact `EntityDataSerializers.PARTICLES` storage: simple vanilla particles
/// (`0..128`, no payload) take one byte in `tags`; complex or mod particles
/// store `0xFF` in `tags` and their full [`Particle`] in `extras`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Particles {
    tags: Vec<u8>,
    extras: Vec<Particle>,
}
/// Iterator over [`Particles`], borrowing complex entries and synthesizing simple ones inline.
pub struct ParticlesIter<'a> {
    tags: std::slice::Iter<'a, u8>,
    extras: std::slice::Iter<'a, Particle>,
}
impl<'a> Iterator for ParticlesIter<'a> {
    type Item = std::borrow::Cow<'a, Particle>;

    fn next(&mut self) -> Option<Self::Item> {
        let &tag = self.tags.next()?;
        if tag == COMPLEX_PARTICLE_TAG {
            Some(std::borrow::Cow::Borrowed(
                self.extras.next().expect("matching complex particle"),
            ))
        } else {
            Some(std::borrow::Cow::Owned(Particle {
                id: i32::from(tag),
                data: None,
            }))
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.tags.size_hint()
    }
}
impl ExactSizeIterator for ParticlesIter<'_> {}

impl Particles {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.tags.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    pub fn push(&mut self, particle: Particle) {
        if particle.data.is_none() && (0..PARTICLE_TYPE_COUNT).contains(&particle.id) {
            self.tags.push(particle.id as u8);
        } else {
            self.tags.push(COMPLEX_PARTICLE_TAG);
            self.extras.push(particle);
        }
    }

    pub fn iter(&self) -> ParticlesIter<'_> {
        ParticlesIter {
            tags: self.tags.iter(),
            extras: self.extras.iter(),
        }
    }

    pub fn to_vec(&self) -> Vec<Particle> {
        self.iter().map(std::borrow::Cow::into_owned).collect()
    }

    pub fn has_sunrise_only(&self) -> bool {
        self.extras.iter().any(Particle::sunrise_only)
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&Particle) -> bool) {
        let mut extras = std::mem::take(&mut self.extras).into_iter();
        let mut kept_extras = Vec::new();
        self.tags.retain(|&tag| {
            if tag == COMPLEX_PARTICLE_TAG {
                let p = extras.next().expect("matching complex particle");
                if keep(&p) {
                    kept_extras.push(p);
                    true
                } else {
                    false
                }
            } else {
                let p = Particle {
                    id: i32::from(tag),
                    data: None,
                };
                keep(&p)
            }
        });
        self.extras = kept_extras;
    }

    fn write(&self, w: &mut Writer) {
        w.varint(self.tags.len() as i32);
        let mut extras = self.extras.iter();
        for &tag in &self.tags {
            if tag == COMPLEX_PARTICLE_TAG {
                extras.next().expect("matching complex particle").write(w);
            } else {
                w.varint(i32::from(tag));
            }
        }
    }
}

impl From<Vec<Particle>> for Particles {
    fn from(particles: Vec<Particle>) -> Self {
        let mut out = Self {
            tags: Vec::with_capacity(particles.len()),
            extras: Vec::new(),
        };
        for p in particles {
            out.push(p);
        }
        out
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum ParticleData {
    /// Block state id (block, block_marker, falling_dust, dust_pillar,
    /// block_crumble).
    State(i32),
    /// ARGB color (entity_effect, tinted_leaves, flash).
    Color(i32),
    /// Dust: color then scale.
    ColorScale(i32, f32),
    /// Dust color transition: from color, to color, scale.
    ColorScale2(i32, i32, f32),
    /// Spell (effect, instant_effect): color then power.
    ColorPower(i32, f32),
    /// A bare float (dragon_breath power, sculk_charge roll).
    Scalar(f32),
    /// Delay in ticks (shriek).
    Delay(i32),
    /// Trail: target (3 doubles), ARGB color, duration.
    Trail([f64; 3], i32, i32),
    /// An item (ItemStackTemplate: item, count, patch).
    Item(Stack),
    /// Geyser and geyser_plume: water blocks.
    Geyser(i32),
    /// Geyser_base and geyser_poof: water blocks, then the burst impulse.
    GeyserBase(i32, f32),
    /// Vibration: where it travels to, then its arrival time in ticks.
    Vibration { source: PositionSource, ticks: i32 },
    /// Sunrise extension: a custom particle's namespaced ID and length-delimited
    /// stream options. Asset-only and unit types have an empty body.
    Mod { name: String, options: Vec<u8> },
}
/// Sunrise extension for a native mod's particle type. Vanilla's
/// `minecraft:particle_type` ids are registry ids and never negative, so this
/// one cannot collide with a vanilla type.
pub const MOD_PARTICLE_ID: i32 = -1;
/// Number of built-in `minecraft:particle_type` entries in 26.3 (`0..=127`).
pub const PARTICLE_TYPE_COUNT: i32 = 128;
/// `PositionSource.STREAM_CODEC`: a `minecraft:position_source_type` id
/// (block 0, entity 1), then that type's data.
#[derive(Clone, Debug, PartialEq)]
pub enum PositionSource {
    Block([i32; 3]),
    Entity { id: i32, y_offset: f32 },
}
impl Particle {
    /// A mod's particle travels as `MOD_PARTICLE_ID` (-1), its name and its options.
    /// Vanilla reads the id with `IdMap.byIdOrThrow`, which throws on -1 and
    /// drops the connection, so such a particle may only go to Sunrise clients.
    pub fn sunrise_only(&self) -> bool {
        self.id == MOD_PARTICLE_ID
    }
    pub fn read(r: &mut Reader) -> Result<Particle> {
        let id = r.varint()?;
        if id == MOD_PARTICLE_ID {
            let name = r.string(32767)?;
            if !name.contains(':') {
                return Err(crate::Error::new(format!("No value with id {id}")));
            }
            return Ok(Particle {
                id,
                data: Some(ParticleData::Mod {
                    // Identifier.STREAM_CODEC, followed by options bounded by the play frame.
                    name,
                    options: r.byte_array(crate::frame::MAX_UNCOMPRESSED)?,
                }),
            });
        }
        // `minecraft:particle_type` registry order as of 26.3 (`IdMap.byIdOrThrow`).
        let data = match id {
            1 | 2 | 36 | 121 | 125 => Some(ParticleData::State(r.varint()?)),
            28 | 46 | 52 => Some(ParticleData::Color(r.i32()?)),
            21 => {
                let color = r.i32()?;
                Some(ParticleData::ColorScale(color, r.f32()?))
            }
            22 => {
                let from = r.i32()?;
                let to = r.i32()?;
                Some(ParticleData::ColorScale2(from, to, r.f32()?))
            }
            23 | 56 => {
                let color = r.i32()?;
                Some(ParticleData::ColorPower(color, r.f32()?))
            }
            15 | 48 => Some(ParticleData::Scalar(r.f32()?)),
            115 => Some(ParticleData::Delay(r.varint()?)),
            59 => {
                let at = [r.f64()?, r.f64()?, r.f64()?];
                let color = r.i32()?;
                Some(ParticleData::Trail(at, color, r.varint()?))
            }
            57 => Some(ParticleData::Item(Template::read(r)?)),
            58 => {
                let source = match r.varint()? {
                    0 => PositionSource::Block(crate::buf::Pos::read(r)?),
                    1 => {
                        let id = r.varint()?;
                        PositionSource::Entity {
                            id,
                            y_offset: r.f32()?,
                        }
                    }
                    kind => return Err(crate::Error::new(format!("position source type {kind}"))),
                };
                Some(ParticleData::Vibration {
                    source,
                    ticks: r.varint()?,
                })
            }
            7 | 10 => Some(ParticleData::Geyser(r.i32()?)),
            8 | 9 => {
                let water = r.i32()?;
                Some(ParticleData::GeyserBase(water, r.f32()?))
            }
            0..PARTICLE_TYPE_COUNT => None,
            other => return Err(crate::Error::new(format!("No value with id {other}"))),
        };
        Ok(Particle { id, data })
    }
    pub fn write(&self, w: &mut Writer) {
        w.varint(self.id);
        let Some(data) = &self.data else { return };
        match data {
            ParticleData::State(s) | ParticleData::Delay(s) => w.varint(*s),
            ParticleData::Color(c) | ParticleData::Geyser(c) => w.i32(*c),
            ParticleData::ColorScale(c, f)
            | ParticleData::ColorPower(c, f)
            | ParticleData::GeyserBase(c, f) => {
                w.i32(*c);
                w.f32(*f);
            }
            ParticleData::ColorScale2(a, b, f) => {
                w.i32(*a);
                w.i32(*b);
                w.f32(*f);
            }
            ParticleData::Scalar(f) => w.f32(*f),
            ParticleData::Trail(at, c, d) => {
                at.iter().for_each(|v| w.f64(*v));
                w.i32(*c);
                w.varint(*d);
            }
            ParticleData::Item(s) => Template::write(s, w),
            ParticleData::Mod { name, options } => {
                w.string(name);
                w.byte_array(options);
            }
            ParticleData::Vibration { source, ticks } => {
                match source {
                    PositionSource::Block(pos) => {
                        w.varint(0);
                        crate::buf::Pos::write(pos, w);
                    }
                    PositionSource::Entity { id, y_offset } => {
                        w.varint(1);
                        w.varint(*id);
                        w.f32(*y_offset);
                    }
                }
                w.varint(*ticks);
            }
        }
    }
}
/// Serializer ids in EntityDataSerializers registration order.
pub mod serializer {
    pub const BYTE: i32 = 0;
    pub const INT: i32 = 1;
    pub const LONG: i32 = 2;
    pub const FLOAT: i32 = 3;
    pub const STRING: i32 = 4;
    pub const COMPONENT: i32 = 5;
    pub const OPTIONAL_COMPONENT: i32 = 6;
    pub const ITEM_STACK: i32 = 7;
    pub const BOOLEAN: i32 = 8;
    pub const ROTATIONS: i32 = 9;
    pub const BLOCK_POS: i32 = 10;
    pub const OPTIONAL_BLOCK_POS: i32 = 11;
    pub const DIRECTION: i32 = 12;
    pub const OPTIONAL_LIVING_ENTITY_REFERENCE: i32 = 13;
    pub const BLOCK_STATE: i32 = 14;
    pub const OPTIONAL_BLOCK_STATE: i32 = 15;
    pub const PARTICLE: i32 = 16;
    pub const PARTICLES: i32 = 17;
    pub const VILLAGER_DATA: i32 = 18;
    pub const OPTIONAL_UNSIGNED_INT: i32 = 19;
    pub const POSE: i32 = 20;
    pub const CAT_VARIANT: i32 = 21;
    pub const CAT_SOUND_VARIANT: i32 = 22;
    pub const COW_VARIANT: i32 = 23;
    pub const COW_SOUND_VARIANT: i32 = 24;
    pub const WOLF_VARIANT: i32 = 25;
    pub const WOLF_SOUND_VARIANT: i32 = 26;
    pub const FROG_VARIANT: i32 = 27;
    pub const PIG_VARIANT: i32 = 28;
    pub const PIG_SOUND_VARIANT: i32 = 29;
    pub const CHICKEN_VARIANT: i32 = 30;
    pub const CHICKEN_SOUND_VARIANT: i32 = 31;
    pub const ZOMBIE_NAUTILUS_VARIANT: i32 = 32;
    pub const OPTIONAL_GLOBAL_POS: i32 = 33;
    pub const PAINTING_VARIANT: i32 = 34;
    pub const SNIFFER_STATE: i32 = 35;
    pub const ARMADILLO_STATE: i32 = 36;
    pub const COPPER_GOLEM_STATE: i32 = 37;
    pub const WEATHERING_COPPER_STATE: i32 = 38;
    pub const VECTOR3: i32 = 39;
    pub const QUATERNION: i32 = 40;
    pub const RESOLVABLE_PROFILE: i32 = 41;
    pub const HUMANOID_ARM: i32 = 42;
    pub const DYE_COLOR: i32 = 43;
}

/// `HumanoidArm.BY_ID`: 1 is right; 0 and any out-of-range id select left.
pub(crate) fn humanoid_arm_id(id: i32) -> i32 {
    i32::from(id == 1)
}

/// `PaintingVariant.DIRECT_CODEC`: resolve a synchronized `minecraft:painting_variant` entry tag.
fn decode_painting_variant_nbt(tag: &Tag, r: &Reader) -> Result<PaintingVariant> {
    let Tag::Compound(_) = tag else {
        return Err(crate::Error::new("PaintingVariant must be a compound tag"));
    };
    let axis = |field: &str| -> Result<i32> {
        let value = match tag.get(field) {
            Some(Tag::Byte(v)) => i64::from(*v),
            Some(Tag::Short(v)) => i64::from(*v),
            Some(Tag::Int(v)) => i64::from(*v),
            Some(Tag::Long(v)) => *v,
            _ => {
                return Err(crate::Error::new(format!(
                    "PaintingVariant {field} must be an integer in 1..=16"
                )));
            }
        };
        if !(1..=16).contains(&value) {
            return Err(crate::Error::new(format!(
                "PaintingVariant {field} {value} outside 1..=16"
            )));
        }
        Ok(value as i32)
    };
    let width = axis("width")?;
    let height = axis("height")?;
    let asset_raw = match tag.get("asset_id") {
        Some(Tag::String(s)) => s
            .to_string()
            .map_err(|_| crate::Error::new("PaintingVariant asset_id: invalid UTF-16"))?,
        _ => return Err(crate::Error::new("PaintingVariant asset_id is required")),
    };
    let asset_id = sunrise_core::identifier::Identifier::parse(&asset_raw)
        .map_err(|e| crate::Error::new(format!("PaintingVariant asset_id: {e}")))?
        .to_string();
    let title = match tag.get("title") {
        Some(t) => {
            TrustedComponent::validate_tag(t, r)?;
            Some(t.clone())
        }
        None => None,
    };
    let author = match tag.get("author") {
        Some(a) => {
            TrustedComponent::validate_tag(a, r)?;
            Some(a.clone())
        }
        None => None,
    };
    Ok(PaintingVariant {
        width,
        height,
        asset_id,
        title,
        author,
    })
}

fn read_value(kind: i32, r: &mut Reader) -> Result<Value> {
    use serializer::*;
    Ok(match kind {
        BYTE => Value::Byte(r.i8()?),
        INT => Value::Int(r.varint()?),
        LONG => Value::Long(r.varlong()?),
        FLOAT => Value::Float(r.f32()?),
        STRING => Value::String(r.string(32767)?),
        COMPONENT => Value::Text(TrustedComponent::read(r)?),
        OPTIONAL_COMPONENT => Value::OptText(Opt::<TrustedComponent>::read(r)?),
        ITEM_STACK => Value::Item(Slot::read(r)?),
        BOOLEAN => Value::Bool(r.bool()?),
        ROTATIONS | VECTOR3 => Value::Floats(vec![r.f32()?, r.f32()?, r.f32()?]),
        QUATERNION => Value::Floats(vec![r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
        BLOCK_POS => Value::Pos(Pos::read(r)?),
        OPTIONAL_BLOCK_POS => Value::OptPos(if r.bool()? { Some(Pos::read(r)?) } else { None }),
        OPTIONAL_LIVING_ENTITY_REFERENCE => {
            Value::OptUuid(if r.bool()? { Some(r.uuid()?) } else { None })
        }
        // VillagerData: type, profession and level.
        VILLAGER_DATA => Value::Ids(vec![r.varint()?, r.varint()?, r.varint()?]),
        OPTIONAL_GLOBAL_POS => Value::OptGlobalPos(if r.bool()? {
            Some((Ident::read(r)?, Pos::read(r)?))
        } else {
            None
        }),
        PAINTING_VARIANT => match r.varint()? {
            0 => Value::PaintingVariant(PaintingHolder::Direct(PaintingVariant {
                width: r.varint()?,
                height: r.varint()?,
                asset_id: Ident::read(r)?,
                title: Opt::<TrustedComponent>::read(r)?,
                author: Opt::<TrustedComponent>::read(r)?,
            })),
            holder => {
                let Some(id) = holder.checked_sub(1).filter(|&id| id >= 0) else {
                    return Err(crate::Error::new(format!(
                        "No value with id {}",
                        holder.wrapping_sub(1)
                    )));
                };
                let Some((name, tag)) = r.registry_entry("minecraft:painting_variant", id) else {
                    return Err(crate::Error::new(format!("No value with id {id}")));
                };
                let tag = tag.ok_or_else(|| {
                    crate::Error::new(format!("Missing synchronized painting variant {name}"))
                })?;
                let variant = decode_painting_variant_nbt(tag, r)?;
                Value::PaintingVariant(PaintingHolder::Reference {
                    id,
                    name: name.to_owned(),
                    variant,
                })
            }
        },
        // VarInt-encoded: direction, block states, optional ints, pose, the
        // holder-registry variants (cat to zombie nautilus), the
        // enum states (sniffer to weathering copper), and dye.
        BLOCK_STATE
        | OPTIONAL_BLOCK_STATE
        | DIRECTION
        | OPTIONAL_UNSIGNED_INT
        | POSE
        | CAT_VARIANT..=ZOMBIE_NAUTILUS_VARIANT
        | SNIFFER_STATE..=WEATHERING_COPPER_STATE
        | DYE_COLOR => Value::Id(r.varint()?),
        HUMANOID_ARM => Value::Id(humanoid_arm_id(r.varint()?)),
        PARTICLE => Value::Particle(Particle::read(r)?),
        RESOLVABLE_PROFILE => Value::Profile(read_profile(r)?),
        PARTICLES => {
            // ByteBufCodecs.list() has no 65536 wire cap; store simple particles in 1 byte.
            let n = r.count(r.remaining())?;
            let mut tags = Vec::with_capacity(n.min(65536));
            let mut extras = Vec::new();
            for _ in 0..n {
                let particle = Particle::read(r)?;
                if particle.data.is_none() && (0..PARTICLE_TYPE_COUNT).contains(&particle.id) {
                    tags.push(particle.id as u8);
                } else {
                    tags.push(COMPLEX_PARTICLE_TAG);
                    extras.push(particle);
                }
            }
            tags.shrink_to_fit();
            extras.shrink_to_fit();
            Value::Particles(Particles { tags, extras })
        }
        other => {
            return Err(crate::Error::new(format!(
                "Unknown serializer type {other}"
            )));
        }
    })
}
fn write_value(kind: i32, v: &Value, w: &mut Writer) {
    match v {
        Value::Byte(b) => w.i8(*b),
        Value::Int(i) | Value::Id(i) => w.varint(*i),
        Value::Ids(ids) => ids.iter().for_each(|i| w.varint(*i)),
        Value::Long(l) => w.varlong(*l),
        Value::Float(f) => w.f32(*f),
        Value::String(s) => w.string(s),
        Value::Text(t) => w.nbt(t),
        Value::OptText(t) => {
            w.bool(t.is_some());
            if let Some(t) = t {
                w.nbt(t);
            }
        }
        Value::Item(s) => Slot::write(s, w),
        Value::Bool(b) => w.bool(*b),
        Value::Floats(f) => f.iter().for_each(|f| w.f32(*f)),
        Value::Pos(p) => crate::buf::Pos::write(p, w),
        Value::OptPos(p) => {
            w.bool(p.is_some());
            if let Some(p) = p {
                crate::buf::Pos::write(p, w);
            }
        }
        Value::OptUuid(u) => {
            w.bool(u.is_some());
            if let Some(u) = u {
                w.uuid(*u);
            }
        }
        Value::Particle(p) => p.write(w),
        Value::Particles(list) => list.write(w),
        Value::Profile(p) => write_profile(p, w),
        Value::OptGlobalPos(pos) => {
            w.bool(pos.is_some());
            if let Some((dim, block)) = pos {
                Ident::write(dim, w);
                Pos::write(block, w);
            }
        }
        Value::PaintingVariant(PaintingHolder::Reference { id, .. }) => {
            w.varint(id + 1);
        }
        Value::PaintingVariant(PaintingHolder::Direct(v)) => {
            w.varint(0);
            w.varint(v.width);
            w.varint(v.height);
            Ident::write(&v.asset_id, w);
            w.bool(v.title.is_some());
            if let Some(t) = &v.title {
                w.nbt(t);
            }
            w.bool(v.author.is_some());
            if let Some(a) = &v.author {
                w.nbt(a);
            }
        }
    }
    let _ = kind;
}

/// (index, serializer id, value) entries.
pub struct EntityData;
impl Codec<Vec<(u8, i32, Value)>> for EntityData {
    fn read(r: &mut Reader) -> Result<Vec<(u8, i32, Value)>> {
        let mut out = Vec::new();
        loop {
            let index = r.u8()?;
            if index == 0xFF {
                return Ok(out);
            }
            let kind = r.varint()?;
            out.push((index, kind, read_value(kind, r)?));
        }
    }
    fn write(v: &Vec<(u8, i32, Value)>, w: &mut Writer) {
        for (index, kind, value) in v {
            w.u8(*index);
            w.varint(*kind);
            write_value(*kind, value, w);
        }
        w.u8(0xFF);
    }
}

/// ItemEntity.DATA_ITEM (after the eight Entity data entries).
pub const ITEM_ENTITY_ITEM: u8 = 8;

// Generated retained-allocation traversals; regenerate with protocol-retained-gen.
impl crate::retained::OwnedHeap for Value {
    fn owned_heap_bytes(&self) -> usize {
        match self {
            Self::Byte(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Int(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Long(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Float(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::String(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Text(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::OptText(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Item(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Bool(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Floats(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Pos(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::OptPos(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::OptUuid(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Id(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Ids(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Particle(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Particles(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Profile(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::OptGlobalPos(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::PaintingVariant(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
        }
    }
}
impl crate::retained::OwnedHeap for PaintingHolder {
    fn owned_heap_bytes(&self) -> usize {
        match self {
            Self::Reference { id, name, variant } => 0usize
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(id))
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(name))
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(variant)),
            Self::Direct(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
        }
    }
}
impl crate::retained::OwnedHeap for PaintingVariant {
    fn owned_heap_bytes(&self) -> usize {
        let Self {
            width,
            height,
            asset_id,
            title,
            author,
        } = self;
        0usize
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(width))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(height))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(asset_id))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(title))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(author))
    }
}
impl crate::retained::OwnedHeap for Profile {
    fn owned_heap_bytes(&self) -> usize {
        let Self {
            name,
            uuid,
            properties,
            body,
            cape,
            elytra,
            slim,
        } = self;
        0usize
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(name))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(uuid))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(properties))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(body))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(cape))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(elytra))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(slim))
    }
}
impl crate::retained::OwnedHeap for Particle {
    fn owned_heap_bytes(&self) -> usize {
        let Self { id, data } = self;
        0usize
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(id))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(data))
    }
}
impl crate::retained::OwnedHeap for Particles {
    fn owned_heap_bytes(&self) -> usize {
        let Self { tags, extras } = self;
        0usize
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(tags))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(extras))
    }
}
impl crate::retained::OwnedHeap for ParticleData {
    fn owned_heap_bytes(&self) -> usize {
        match self {
            Self::State(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Color(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::ColorScale(field_0, field_1) => 0usize
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_1)),
            Self::ColorScale2(field_0, field_1, field_2) => 0usize
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_1))
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_2)),
            Self::ColorPower(field_0, field_1) => 0usize
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_1)),
            Self::Scalar(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Delay(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Trail(field_0, field_1, field_2) => 0usize
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_1))
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_2)),
            Self::Item(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Geyser(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::GeyserBase(field_0, field_1) => 0usize
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_1)),
            Self::Vibration { source, ticks } => 0usize
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(source))
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(ticks)),
            Self::Mod { name, options } => 0usize
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(name))
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(options)),
        }
    }
}
impl crate::retained::OwnedHeap for PositionSource {
    fn owned_heap_bytes(&self) -> usize {
        match self {
            Self::Block(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Entity { id, y_offset } => 0usize
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(id))
                .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(y_offset)),
        }
    }
}
impl crate::retained::OwnedHeap for EntityData {
    fn owned_heap_bytes(&self) -> usize {
        let Self = self;
        0usize
    }
}
// End generated retained traversals.

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt;

    #[test]
    fn arm_ids_decode_the_same_in_metadata_and_client_information() {
        use crate::packets::configuration::ClientInformation;

        for (wire_id, expected) in [(0, 0), (1, 1), (-1, 0), (2, 0)] {
            let mut metadata = Writer::new();
            EntityData::write(
                &vec![(15, serializer::HUMANOID_ARM, Value::Id(wire_id))],
                &mut metadata,
            );
            assert_eq!(
                EntityData::read(&mut Reader::new(&metadata.data)).unwrap(),
                [(15, serializer::HUMANOID_ARM, Value::Id(expected))]
            );

            let mut info = Writer::new();
            ClientInformation::write(
                &ClientInformation {
                    main_hand: wire_id,
                    ..ClientInformation::new(8)
                },
                &mut info,
            );
            assert_eq!(
                ClientInformation::read(&mut Reader::new(&info.data))
                    .unwrap()
                    .main_hand,
                expected
            );
        }
    }

    #[test]
    fn round_trips_and_stops_at_unknown_serializers() {
        let entries = vec![
            (0u8, serializer::BYTE, Value::Byte(0x02)),
            (1, serializer::INT, Value::Int(300)),
            (2, serializer::OPTIONAL_COMPONENT, Value::OptText(None)),
            (6, serializer::POSE, Value::Id(5)),
            (
                8,
                serializer::ITEM_STACK,
                Value::Item(Some(Stack::new(3, 12))),
            ),
        ];
        let mut w = Writer::new();
        EntityData::write(&entries, &mut w);
        assert_eq!(*w.data.last().unwrap(), 0xFF);
        assert_eq!(
            EntityData::read(&mut Reader::new(&w.data)).unwrap(),
            entries
        );
        // Serializer 200 is unknown: decode fails instead of silently truncating.
        let mut bytes = vec![0, 0, 7, 9, 200, 1, 2, 3];
        bytes.push(0xFF);
        assert!(EntityData::read(&mut Reader::new(&bytes)).is_err());
        // A block particle decodes fully and round-trips.
        let entries = vec![(
            10u8,
            serializer::PARTICLE,
            Value::Particle(Particle {
                id: 1,
                data: Some(ParticleData::State(2)),
            }),
        )];
        let mut w = Writer::new();
        EntityData::write(&entries, &mut w);
        assert_eq!(
            EntityData::read(&mut Reader::new(&w.data)).unwrap(),
            entries
        );
    }

    fn component_entry(kind: i32, component: &[u8]) -> Writer {
        let mut w = Writer::new();
        w.u8(2);
        w.varint(kind);
        if kind == serializer::OPTIONAL_COMPONENT {
            w.bool(true);
        }
        w.bytes(component);
        w
    }

    #[test]
    fn entity_components_exceed_the_generic_nbt_budget() {
        // A component can be a list of literal strings. This list exceeds
        // both the ordinary network NBT byte budget and its node budget.
        const TEXT: &str = "literal";
        const SIBLINGS: i32 = 262_144;
        let text = Tag::String(TEXT.into());
        let mut component = Writer::new();
        component.bytes(&[9, 8]); // TAG_List of TAG_String
        component.i32(SIBLINGS);
        for _ in 0..SIBLINGS {
            component.u16(TEXT.len() as u16);
            component.bytes(TEXT.as_bytes());
        }
        assert!(Reader::new(&component.data).nbt().is_err());

        for kind in [serializer::COMPONENT, serializer::OPTIONAL_COMPONENT] {
            let mut w = component_entry(kind, &component.data);
            w.bytes(&[3, 0, 7, 0xff]); // A byte entry after the component, then the terminator.

            let entries = EntityData::read(&mut Reader::new(&w.data)).unwrap();
            let strings = match &entries[0] {
                (2, serializer::COMPONENT, Value::Text(Tag::List(strings)))
                | (2, serializer::OPTIONAL_COMPONENT, Value::OptText(Some(Tag::List(strings)))) => {
                    strings
                }
                _ => panic!("expected a component list"),
            };
            assert_eq!(entries[0].1, kind);
            assert_eq!(strings.len(), SIBLINGS as usize);
            assert!(strings.iter().all(|s| s == &text));
            assert_eq!(entries[1..], [(3, serializer::BYTE, Value::Byte(7))]);
        }
    }

    #[test]
    fn entity_components_reject_nbt_that_is_not_a_component() {
        let mut tag = Writer::new();
        tag.nbt(&Tag::Int(7));
        for kind in [serializer::COMPONENT, serializer::OPTIONAL_COMPONENT] {
            let mut w = component_entry(kind, &tag.data);
            w.u8(0xff);
            let err = EntityData::read(&mut Reader::new(&w.data)).unwrap_err();
            assert!(err.to_string().starts_with("Failed to decode:"), "{err}");
        }
    }

    #[test]
    fn entity_components_keep_vanillas_nbt_depth_limit() {
        // An unused field isolates the NBT depth limit from the component codec.
        let mut unused = Tag::Int(0);
        for _ in 0..511 {
            unused = Tag::List(vec![unused]);
        }
        for (depth, unused) in [(512, unused.clone()), (513, Tag::List(vec![unused]))] {
            let component =
                Tag::compound([("text", Tag::String("name".into())), ("unused", unused)]);
            let bytes = nbt::write_network(
                &component,
                nbt::Limits {
                    max_depth: 513,
                    ..nbt::Limits::default()
                },
            )
            .unwrap();
            for kind in [serializer::COMPONENT, serializer::OPTIONAL_COMPONENT] {
                let mut w = component_entry(kind, &bytes);
                w.u8(0xff);

                let decoded = EntityData::read(&mut Reader::new(&w.data));
                if depth == 512 {
                    let expected = if kind == serializer::COMPONENT {
                        Value::Text(component.clone())
                    } else {
                        Value::OptText(Some(component.clone()))
                    };
                    assert_eq!(decoded.unwrap(), [(2, kind, expected)]);
                } else {
                    assert!(
                        decoded
                            .unwrap_err()
                            .to_string()
                            .ends_with("depth limit exceeded")
                    );
                }
            }
        }
    }

    /// `EntityDataSerializers.VECTOR3` and `QUATERNION`
    /// (`FriendlyByteBuf.writeVector3f`, `writeQuaternion`): three or four
    /// big-endian floats.
    #[test]
    fn vectors_and_quaternions_are_big_endian_floats() {
        use std::f32::consts::FRAC_1_SQRT_2;
        let entries = vec![
            (
                11u8,
                serializer::VECTOR3,
                Value::Floats(vec![1.0, -2.0, 0.25]),
            ),
            (
                13,
                serializer::QUATERNION,
                Value::Floats(vec![0.0, 0.0, FRAC_1_SQRT_2, FRAC_1_SQRT_2]),
            ),
        ];
        let mut w = Writer::new();
        EntityData::write(&entries, &mut w);
        #[rustfmt::skip]
        let bytes = [
            11, 39, 0x3F, 0x80, 0, 0, 0xC0, 0, 0, 0, 0x3E, 0x80, 0, 0,
            13, 40, 0, 0, 0, 0, 0, 0, 0, 0, 0x3F, 0x35, 0x04, 0xF3, 0x3F, 0x35, 0x04, 0xF3,
            0xFF,
        ];
        assert_eq!(w.data, bytes);
        assert_eq!(EntityData::read(&mut Reader::new(&bytes)).unwrap(), entries);
    }

    /// Every particle data shape (26.3 codecs) round-trips, and a PARTICLES
    /// list is a VarInt count then each particle, so the entry after it
    /// still decodes.
    #[test]
    fn particles_round_trip() {
        let all = [
            (1, Some(ParticleData::State(2))),
            (7, Some(ParticleData::Geyser(5))),
            (8, Some(ParticleData::GeyserBase(3, 0.5))),
            (15, Some(ParticleData::Scalar(1.0))),
            (21, Some(ParticleData::ColorScale(0xff0000, 1.5))),
            (22, Some(ParticleData::ColorScale2(1, 2, 0.75))),
            (23, Some(ParticleData::ColorPower(-1, 2.0))),
            (28, Some(ParticleData::Color(0x7f00ff00))),
            (
                57,
                Some(ParticleData::Item(Stack {
                    item: 42,
                    count: 3,
                    added: Vec::new(),
                    removed: vec![7],
                    wire: Default::default(),
                })),
            ),
            // A native mod's type: the sentinel id, then the namespaced name.
            (
                MOD_PARTICLE_ID,
                Some(ParticleData::Mod {
                    name: "mymod:spark".into(),
                    options: vec![1, 3, 7],
                }),
            ),
            (
                58,
                Some(ParticleData::Vibration {
                    source: PositionSource::Block([-12, 64, 300]),
                    ticks: 20,
                }),
            ),
            (
                58,
                Some(ParticleData::Vibration {
                    source: PositionSource::Entity {
                        id: 99,
                        y_offset: 1.25,
                    },
                    ticks: 8,
                }),
            ),
            (59, Some(ParticleData::Trail([1.5, -2.0, 3.25], 9, 40))),
            (115, Some(ParticleData::Delay(6))),
            (0, None),
        ]
        .map(|(id, data)| Particle { id, data });
        let mut entries: Vec<_> = all
            .iter()
            .map(|p| (10u8, serializer::PARTICLE, Value::Particle(p.clone())))
            .collect();
        entries.push((
            11,
            serializer::PARTICLES,
            Value::Particles(all.to_vec().into()),
        ));
        entries.push((12, serializer::BYTE, Value::Byte(4)));
        let mut w = Writer::new();
        EntityData::write(&entries, &mut w);
        assert_eq!(
            EntityData::read(&mut Reader::new(&w.data)).unwrap(),
            entries
        );
        // Wire shape: an empty list is one 0 byte, then the next entry.
        let bytes = [11, serializer::PARTICLES as u8, 0, 12, 0, 4, 0xFF];
        assert_eq!(
            EntityData::read(&mut Reader::new(&bytes)).unwrap(),
            vec![
                (
                    11,
                    serializer::PARTICLES,
                    Value::Particles(Particles::new())
                ),
                (12, serializer::BYTE, Value::Byte(4)),
            ]
        );
        // Unregistered 26.3 particle ids fail decode before payload dispatch.
        for invalid_id in [PARTICLE_TYPE_COUNT, 999, -2] {
            let mut bad = Writer::new();
            bad.u8(10);
            bad.varint(serializer::PARTICLE);
            bad.varint(invalid_id);
            bad.u8(0xFF);
            assert!(
                EntityData::read(&mut Reader::new(&bad.data)).is_err(),
                "particle id {invalid_id} must fail"
            );
        }
        // ByteBufCodecs.list() has no 65536 wire cap, and simple particles stay 1 byte each.
        let mut many = Writer::new();
        many.u8(10);
        many.varint(serializer::PARTICLES);
        many.varint(65_537);
        for _ in 0..65_537 {
            many.varint(0);
        }
        many.u8(0xFF);
        let decoded = EntityData::read(&mut Reader::new(&many.data)).unwrap();
        assert_eq!(decoded.len(), 1);
        match &decoded[0].2 {
            Value::Particles(list) => {
                assert_eq!(list.len(), 65_537);
                assert_eq!(crate::retained::OwnedHeap::owned_heap_bytes(list), 65_537);
            }
            other => panic!("expected Particles, got {other:?}"),
        }
    }

    #[test]
    fn painting_variant_holders_resolve_and_inline_components_validate() {
        let mut registries = crate::registry::Dynamic::default();
        let kebab_tag = Tag::compound([
            ("width", Tag::Int(1)),
            ("height", Tag::Int(1)),
            ("asset_id", Tag::String("minecraft:kebab".into())),
        ]);
        registries.registries.insert(
            "minecraft:painting_variant".into(),
            vec![
                ("minecraft:kebab".into(), Some(kebab_tag)),
                ("minecraft:missing_data".into(), None),
                (
                    "minecraft:bad_width".into(),
                    Some(Tag::compound([
                        ("width", Tag::Int(0)),
                        ("height", Tag::Int(1)),
                        ("asset_id", Tag::String("minecraft:kebab".into())),
                    ])),
                ),
            ],
        );
        registries.registries.insert(
            "minecraft:item".into(),
            vec![("minecraft:diamond".into(), None)],
        );
        // Valid registry holder id 1 -> 0-based index 0 ("minecraft:kebab") with resolved value.
        let mut w = Writer::new();
        w.u8(9);
        w.varint(serializer::PAINTING_VARIANT);
        w.varint(1);
        w.u8(0xFF);
        let entries = EntityData::read(&mut Reader::new(&w.data).with_names(&registries)).unwrap();
        assert_eq!(
            entries,
            vec![(
                9,
                serializer::PAINTING_VARIANT,
                Value::PaintingVariant(PaintingHolder::Reference {
                    id: 0,
                    name: "minecraft:kebab".into(),
                    variant: PaintingVariant {
                        width: 1,
                        height: 1,
                        asset_id: "minecraft:kebab".into(),
                        title: None,
                        author: None,
                    },
                }),
            )]
        );
        // Missing data (holder 2), invalid data (holder 3), out-of-range, negative, and i32::MIN fail decode.
        for bad_holder in [2, 3, 4, -1, i32::MIN] {
            let mut bad = Writer::new();
            bad.u8(9);
            bad.varint(serializer::PAINTING_VARIANT);
            bad.varint(bad_holder);
            bad.u8(0xFF);
            assert!(
                EntityData::read(&mut Reader::new(&bad.data).with_names(&registries)).is_err(),
                "painting holder {bad_holder} must fail decode"
            );
        }
        // Inline painting variant rejects Tag::End, non-component tags, empty compounds, and unknown hover items.
        let bad_hover_item = Tag::compound([
            ("text", Tag::String("hover".into())),
            (
                "hover_event",
                Tag::compound([
                    ("action", Tag::String("show_item".into())),
                    ("id", Tag::String("minecraft:not_a_real_item".into())),
                ]),
            ),
        ]);
        for bad_title in [
            Tag::End,
            Tag::Int(7),
            Tag::Compound(Default::default()),
            bad_hover_item,
        ] {
            let mut bad = Writer::new();
            bad.u8(9);
            bad.varint(serializer::PAINTING_VARIANT);
            bad.varint(0);
            bad.varint(2);
            bad.varint(1);
            Ident::write(&"minecraft:alban".into(), &mut bad);
            bad.bool(true);
            bad.nbt(&bad_title);
            bad.bool(false);
            bad.u8(0xFF);
            assert!(
                EntityData::read(&mut Reader::new(&bad.data).with_names(&registries)).is_err(),
                "inline painting variant with malformed title {bad_title:?} must fail"
            );
        }
        // Valid inline painting variant with text components and a registered hover item round-trips.
        let inline = PaintingVariant {
            width: 2,
            height: 1,
            asset_id: "minecraft:alban".into(),
            title: Some(Tag::compound([
                ("text", Tag::String("Inline".into())),
                (
                    "hover_event",
                    Tag::compound([
                        ("action", Tag::String("show_item".into())),
                        ("id", Tag::String("minecraft:diamond".into())),
                    ]),
                ),
            ])),
            author: Some(Tag::compound([("text", Tag::String("Author".into()))])),
        };
        let expected = vec![(
            9,
            serializer::PAINTING_VARIANT,
            Value::PaintingVariant(PaintingHolder::Direct(inline)),
        )];
        let mut ok = Writer::new();
        EntityData::write(&expected, &mut ok);
        assert_eq!(
            EntityData::read(&mut Reader::new(&ok.data).with_names(&registries)).unwrap(),
            expected
        );
    }

    /// RESOLVABLE_PROFILE (a mannequin's DATA_PROFILE): both ResolvableProfile
    /// contents round-trip, and an entry after it still decodes.
    #[test]
    fn resolvable_profile_round_trips() {
        let full = Profile {
            name: Some("Gommunity".into()),
            uuid: Some(0x0123_4567_89AB_CDEF_0123_4567_89AB_CDEF),
            properties: vec![AccountProperty {
                name: "textures".into(),
                value: "e30=".into(),
                signature: Some("sig".into()),
            }],
            body: Some("minecraft:entity/player/wide/steve".into()),
            cape: None,
            elytra: None,
            slim: Some(false),
        };
        let partial = Profile {
            name: Some("Gommunity".into()),
            ..Profile::default()
        };
        let mut w = Writer::new();
        EntityData::write(
            &vec![
                (
                    17,
                    serializer::RESOLVABLE_PROFILE,
                    Value::Profile(full.clone()),
                ),
                (
                    17,
                    serializer::RESOLVABLE_PROFILE,
                    Value::Profile(partial.clone()),
                ),
                (18, serializer::BOOLEAN, Value::Bool(true)),
            ],
            &mut w,
        );
        assert_eq!(
            EntityData::read(&mut Reader::new(&w.data)).unwrap(),
            vec![
                (17, serializer::RESOLVABLE_PROFILE, Value::Profile(full)),
                (17, serializer::RESOLVABLE_PROFILE, Value::Profile(partial)),
                (18, serializer::BOOLEAN, Value::Bool(true)),
            ]
        );
    }

    fn entity_data_witness() -> serde_json::Value {
        let dir = std::env::var("SUNRISE_NETWORK_WITNESS_DIR").expect(
            "SUNRISE_NETWORK_WITNESS_DIR must be set (run via tools/test_network_parity.py)",
        );
        let path = std::path::Path::new(&dir).join("entity_data.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        serde_json::from_str(&text).unwrap()
    }

    fn entity_data_hex(hex: &str) -> Vec<u8> {
        (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn painting_registries(witness: &serde_json::Value) -> crate::registry::Dynamic {
        let mut registries = crate::registry::Dynamic::default();
        let entries = witness["painting_variant_definitions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let name = v["name"].as_str().unwrap().to_owned();
                let tag = Tag::compound([
                    ("width", Tag::Int(v["width"].as_i64().unwrap() as i32)),
                    ("height", Tag::Int(v["height"].as_i64().unwrap() as i32)),
                    (
                        "asset_id",
                        Tag::String(v["asset_id"].as_str().unwrap().into()),
                    ),
                ]);
                (name, Some(tag))
            })
            .collect();
        registries
            .registries
            .insert("minecraft:painting_variant".into(), entries);
        let registered_item = witness["hover_item_component_validation"]["registered_item_id"]
            .as_str()
            .unwrap()
            .to_owned();
        registries
            .registries
            .insert("minecraft:item".into(), vec![(registered_item, None)]);
        registries
    }

    #[test]
    #[ignore = "daily"]
    fn unknown_entity_data_serializers_fail_decode_like_26_3() {
        let witness = entity_data_witness();
        let registries = painting_registries(&witness);
        for row in witness["unknown_serializers"].as_array().unwrap() {
            let name = row["name"].as_str().unwrap();
            let serializer_id = row["serializer_id"].as_i64().unwrap() as i32;
            let data = entity_data_hex(row["entity_data_hex"].as_str().unwrap());
            let Err(err) = EntityData::read(&mut Reader::new(&data).with_names(&registries)) else {
                panic!("{name}: serializer {serializer_id} must fail decode");
            };
            assert!(
                err.to_string().contains(&serializer_id.to_string()),
                "{name}: error {err:?} should report serializer {serializer_id}"
            );
            let body = entity_data_hex(row["packet_body_hex"].as_str().unwrap());
            assert!(
                crate::packets::play::Clientbound::decode_reader(
                    crate::ids::play::clientbound::SET_ENTITY_DATA,
                    Reader::new(&body).with_names(&registries),
                )
                .is_err(),
                "{name}: SET_ENTITY_DATA packet decode must fail"
            );
        }
        for row in witness["invalid_painting_holders"].as_array().unwrap() {
            let name = row["name"].as_str().unwrap();
            let data = entity_data_hex(row["entity_data_hex"].as_str().unwrap());
            assert!(
                EntityData::read(&mut Reader::new(&data).with_names(&registries)).is_err(),
                "invalid painting holder {name} must fail decode"
            );
        }
        let bad_inline = entity_data_hex(
            witness["malformed_inline_painting_component"]["entity_data_hex"]
                .as_str()
                .unwrap(),
        );
        assert!(
            EntityData::read(&mut Reader::new(&bad_inline).with_names(&registries)).is_err(),
            "malformed inline painting component must fail decode"
        );
        let bad_hover = entity_data_hex(
            witness["hover_item_component_validation"]["unregistered_item_hex"]
                .as_str()
                .unwrap(),
        );
        assert!(
            EntityData::read(&mut Reader::new(&bad_hover).with_names(&registries)).is_err(),
            "unregistered hover_event show_item must fail decode"
        );
        let bad_particle = entity_data_hex(
            witness["particle_metadata"]["unregistered_128_hex"]
                .as_str()
                .unwrap(),
        );
        assert!(
            EntityData::read(&mut Reader::new(&bad_particle).with_names(&registries)).is_err(),
            "unregistered particle id 128 must fail decode"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn valid_26_3_optional_global_pos_and_painting_variant_decode_completely() {
        let witness = entity_data_witness();
        let registries = painting_registries(&witness);
        for row in witness["valid_serializers"].as_array().unwrap() {
            let name = row["name"].as_str().unwrap();
            let data = entity_data_hex(row["entity_data_hex"].as_str().unwrap());
            let entries = EntityData::read(&mut Reader::new(&data).with_names(&registries))
                .unwrap_or_else(|e| panic!("{name}: failed to decode valid 26.3 data: {e}"));
            let expected_count = row["entry_count"].as_u64().unwrap() as usize;
            assert_eq!(entries.len(), expected_count, "{name}: entry count");
            let trailing_index = row["trailing_index"].as_u64().unwrap() as u8;
            let trailing_byte = row["trailing_byte"].as_i64().unwrap() as i8;
            assert_eq!(
                entries.last(),
                Some(&(trailing_index, serializer::BYTE, Value::Byte(trailing_byte))),
                "{name}: trailing entry"
            );
            let mut w = Writer::new();
            EntityData::write(&entries, &mut w);
            assert_eq!(w.data, data, "{name}: exact wire round-trip");

            let body = entity_data_hex(row["packet_body_hex"].as_str().unwrap());
            let entity_id = row["entity_id"].as_i64().unwrap() as i32;
            let crate::packets::play::Clientbound::SetEntityData(pkt) =
                crate::packets::play::Clientbound::decode_reader(
                    crate::ids::play::clientbound::SET_ENTITY_DATA,
                    Reader::new(&body).with_names(&registries),
                )
                .unwrap()
            else {
                panic!("{name}: expected SetEntityData");
            };
            assert_eq!(pkt.id, entity_id, "{name}: entity_id");
            assert_eq!(pkt.values, entries, "{name}: packet entries");
        }
        // Synchronized painting_variant override resolves to the synced definition at decode time.
        let sync_override = &witness["painting_variant_sync_override"];
        let mut override_reg = crate::registry::Dynamic::default();
        override_reg.registries.insert(
            "minecraft:painting_variant".into(),
            vec![(
                sync_override["resolved_name"].as_str().unwrap().to_owned(),
                Some(Tag::compound([
                    (
                        "width",
                        Tag::Int(sync_override["resolved_width"].as_i64().unwrap() as i32),
                    ),
                    (
                        "height",
                        Tag::Int(sync_override["resolved_height"].as_i64().unwrap() as i32),
                    ),
                    (
                        "asset_id",
                        Tag::String(sync_override["resolved_asset_id"].as_str().unwrap().into()),
                    ),
                ])),
            )],
        );
        let override_bytes = entity_data_hex(sync_override["entity_data_hex"].as_str().unwrap());
        let decoded_override =
            EntityData::read(&mut Reader::new(&override_bytes).with_names(&override_reg))
                .expect("synchronized painting override must decode");
        assert_eq!(
            decoded_override,
            vec![(
                9,
                serializer::PAINTING_VARIANT,
                Value::PaintingVariant(PaintingHolder::Reference {
                    id: 0,
                    name: sync_override["resolved_name"].as_str().unwrap().to_owned(),
                    variant: PaintingVariant {
                        width: sync_override["resolved_width"].as_i64().unwrap() as i32,
                        height: sync_override["resolved_height"].as_i64().unwrap() as i32,
                        asset_id: sync_override["resolved_asset_id"]
                            .as_str()
                            .unwrap()
                            .to_owned(),
                        title: None,
                        author: None,
                    },
                }),
            )]
        );
        // Registered hover_event show_item decodes cleanly.
        let good_hover = entity_data_hex(
            witness["hover_item_component_validation"]["registered_item_hex"]
                .as_str()
                .unwrap(),
        );
        assert!(
            witness["hover_item_component_validation"]["registered_item_decoded_ok"]
                .as_bool()
                .unwrap()
        );
        assert_eq!(
            EntityData::read(&mut Reader::new(&good_hover).with_names(&registries))
                .expect("registered hover_event show_item must decode")
                .len(),
            1
        );
        assert!(
            witness["particle_metadata"]["list_65537_ok"]
                .as_bool()
                .unwrap()
        );
        let expected_particles = witness["particle_metadata"]["list_65537_count"]
            .as_u64()
            .unwrap() as usize;
        let mut many = Writer::new();
        many.u8(10);
        many.varint(serializer::PARTICLES);
        many.varint(expected_particles as i32);
        for _ in 0..expected_particles {
            many.varint(0);
        }
        many.u8(0xFF);
        let decoded = EntityData::read(&mut Reader::new(&many.data).with_names(&registries))
            .expect("65537 simple particles must decode");
        match &decoded[0].2 {
            Value::Particles(list) => {
                assert_eq!(list.len(), expected_particles);
                assert_eq!(
                    crate::retained::OwnedHeap::owned_heap_bytes(list),
                    expected_particles
                );
            }
            other => panic!("expected Particles, got {other:?}"),
        }
    }
}
