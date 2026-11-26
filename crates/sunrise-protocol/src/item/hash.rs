//! `HashedPatchMap.create` / `HashedStack.create`: each patched component is
//! hashed as its `Codec` form through `HashOps` (the client's
//! `decoratedHashOpsGenerator`: `component.encodeValue(hashOps).asInt()`).
//!
//! Sunrise holds a component as the wire form (registry ids, no names), so the
//! codec form is rebuilt from it with a registry lookup for the ids that the
//! codec writes as identifiers. [`raw`] decodes the stream bytes of the components
//! Sunrise keeps as bytes ([`Component::Raw`]). A component with no codec form
//! Sunrise can build (vanilla has none, an id has no name, or a codec is not
//! ported yet) refuses the stack; it must not produce an incomplete patch.
mod exact;
mod gameplay;
pub(crate) mod ops;
mod raw;
use super::consume_effects::AdmissionLookup;
use super::{
    Component, FireworkExplosion, Hashed, Stack, StackHashError, WirePatch, component as kind,
    persistent,
};
use crate::{buf::Reader, hash_ops::HashCode};
use ops::{Ops, Record};

/// Registry names for the ids a component holds.
pub trait Names {
    /// The name of entry `id` of `registry` (`minecraft:enchantment`, ...).
    fn name(&self, registry: &str, id: i32) -> Option<String>;

    /// The consume-effect type at this connection's stream ID.
    fn consume_effect_kind(&self, id: i32) -> Option<super::consume_effects::Kind> {
        super::consume_effects::builtin_name(id).map(|_| super::consume_effects::Kind::Builtin(id))
    }

    /// Effects already projected from this same stream patch, including its nested templates.
    fn consume_effect_admissions(&self) -> Option<&AdmissionLookup<'_>> {
        None
    }

    /// Synchronized registry entry `(name, data)` at `id`, if present.
    fn registry_entry(
        &self,
        registry: &str,
        id: i32,
    ) -> Option<(&str, Option<&sunrise_core::nbt::Tag>)> {
        let _ = (registry, id);
        None
    }

    /// Whether `registry` contains entry `name`.
    fn contains_entry(&self, registry: &str, name: &str) -> bool {
        let _ = (registry, name);
        false
    }
}
/// No registries: only the components that hold no registry id hash.
pub struct NoNames;
impl Names for NoNames {
    fn name(&self, _: &str, _: i32) -> Option<String> {
        None
    }
}

const RARITY: [&str; 4] = ["common", "uncommon", "rare", "epic"];
const DYE: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];
/// `FireworkExplosion.Shape` names by id.
const SHAPE: [&str; 5] = ["small_ball", "large_ball", "star", "creeper", "burst"];
/// The `Codec.BOOL` fields of the text component codec (`Style`, `NbtContents`,
/// `PlayerSprite`).
const TEXT_BOOLS: [&str; 7] = [
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
    "interpret",
    "plain",
];

fn string_with<O: Ops>(s: &str) -> O::Value {
    O::create_string(s)
}
fn identifier_with<O: Ops>(s: &str) -> Option<O::Value> {
    let name = sunrise_world::component_value::source::normalized_identifier(s).ok()?;
    Some(O::create_string(&name))
}
/// `Identifier.CODEC` of registry entry `id`.
fn entry_with<O: Ops>(names: &dyn Names, registry: &str, id: i32) -> Option<O::Value> {
    if id < 0 {
        return None;
    }
    names
        .name(registry, id)
        .and_then(|n| identifier_with::<O>(&n))
        .or_else(|| {
            (registry == "minecraft:data_component_type")
                .then(|| super::persistent::component_name(id))
                .flatten()
                .map(O::create_string)
        })
}
fn pick_with<O: Ops>(table: &[&str], id: i32) -> Option<O::Value> {
    usize::try_from(id)
        .ok()
        .and_then(|i| table.get(i))
        .map(|n| string_with::<O>(n))
}
/// `ComponentSerialization.CODEC`.
fn text_with<O: Ops>(tag: &sunrise_core::nbt::Tag) -> Option<O::Value> {
    O::saved_component("minecraft:custom_name", tag)
}
/// `Filterable.codec`: the full record (`Codec.withAlternative` encodes with
/// the first codec), the filtered variant absent when there is none.
fn filterable_with<O: Ops>(raw: O::Value, filtered: Option<O::Value>) -> O::Value {
    Record::<O>::new()
        .field("raw", raw)
        .optional("filtered", filtered)
        .build()
}
/// `Codec.xxx.optionalFieldOf(name, default)`: absent when equal to the default.
fn or_default_with<O: Ops, T: PartialEq>(
    rec: Record<O>,
    name: &str,
    v: T,
    default: T,
    hash: O::Value,
) -> Record<O> {
    if v == default {
        rec
    } else {
        rec.field(name, hash)
    }
}
fn int_list_with<O: Ops>(values: &[i32]) -> O::Value {
    O::create_list(values.iter().map(|v| O::create_int(*v)))
}
fn explosion_with<O: Ops>(e: &FireworkExplosion) -> Option<O::Value> {
    let rec = Record::<O>::new().field("shape", pick_with::<O>(&SHAPE, e.shape)?);
    let rec = or_default_with::<O, _>(
        rec,
        "colors",
        e.colors.is_empty(),
        true,
        int_list_with::<O>(&e.colors),
    );
    let rec = or_default_with::<O, _>(
        rec,
        "fade_colors",
        e.fade.is_empty(),
        true,
        int_list_with::<O>(&e.fade),
    );
    let rec = or_default_with::<O, _>(rec, "has_trail", e.trail, false, O::create_boolean(e.trail));
    Some(
        or_default_with::<O, _>(
            rec,
            "has_twinkle",
            e.twinkle,
            false,
            O::create_boolean(e.twinkle),
        )
        .build(),
    )
}
/// `ItemEnchantments.CODEC`: an identifier-keyed map of levels.
fn enchantments_with<O: Ops>(list: &[(i32, i32)], names: &dyn Names) -> Option<O::Value> {
    let mut entries = Vec::with_capacity(list.len());
    for (id, level) in list {
        entries.push((
            entry_with::<O>(names, "minecraft:enchantment", *id)?,
            O::create_int(*level),
        ));
    }
    Some(O::create_map(entries))
}
impl Component {
    pub fn code(&self, id: i32, names: &dyn Names) -> Option<HashCode> {
        let name = persistent::component_name(id)?;
        if !sunrise_world::component_value::is_persistent(name) {
            return None;
        }
        let value = persistent::component_value_named(id, name, self, names).ok()?;
        ops::Values::hash_component(name, &value)
    }
    /// Decode the retained stream value, preserving native-only state.
    pub(crate) fn code_with<O: Ops>(&self, id: i32, names: &dyn Names) -> Option<O::Value> {
        if let Some(value) = O::special_component(id, self, names) {
            return value;
        }
        // Recursive stream schemas must not keep the leaf conversion's frame on the stack.
        match self {
            Self::Stacks(stacks) => {
                let list = stacks
                    .iter()
                    .map(|s| O::template(s, names))
                    .collect::<Option<Vec<_>>>()?;
                Some(O::create_list(list))
            }
            Self::Raw(bytes) => raw::code_with::<O>(id, bytes, names),
            _ => self.code_leaf::<O>(id, names),
        }
    }

    #[inline(never)]
    fn code_leaf<O: Ops>(&self, id: i32, names: &dyn Names) -> Option<O::Value> {
        Some(match self {
            Self::Int(v) => match id {
                kind::RARITY => pick_with::<O>(&RARITY, *v)?,
                kind::BASE_COLOR => pick_with::<O>(&DYE, *v)?,
                // MapPostProcessing has no persistent codec.
                kind::MAP_POST_PROCESSING => return None,
                _ => O::create_int(*v),
            },
            // `Unit.CODEC` is an empty map; creative_slot_lock has no codec.
            Self::Unit if id == kind::CREATIVE_SLOT_LOCK => return None,
            Self::Unit => O::empty_map(),
            Self::Bool(v) => O::create_boolean(*v),
            Self::Text(t) => text_with::<O>(t)?,
            Self::Nbt(t) if id == kind::CUSTOM_DATA => {
                O::saved_component("minecraft:custom_data", t)?
            }
            Self::Nbt(t) => O::from_nbt(t, &[])?,
            Self::Ident(s) => identifier_with::<O>(s)?,
            Self::Lore(lines) => O::create_list(
                lines
                    .iter()
                    .map(text_with::<O>)
                    .collect::<Option<Vec<_>>>()?,
            ),
            Self::Enchantments(list) => enchantments_with::<O>(list, names)?,
            Self::AttackRange(v) => {
                const FIELDS: [(&str, f32); 6] = [
                    ("min_reach", 0.0),
                    ("max_reach", 3.0),
                    ("min_creative_reach", 0.0),
                    ("max_creative_reach", 5.0),
                    ("hitbox_margin", 0.3),
                    ("mob_factor", 1.0),
                ];
                let mut rec = Record::<O>::new();
                for ((name, default), v) in FIELDS.iter().zip(v) {
                    rec = or_default_with::<O, _>(
                        rec,
                        name,
                        v.to_bits(),
                        default.to_bits(),
                        O::create_f32(*v),
                    );
                }
                rec.build()
            }
            Self::Potion {
                potion,
                color,
                rest,
            } => {
                let rec = Record::<O>::new()
                    .optional(
                        "potion",
                        match potion {
                            Some(p) => Some(entry_with::<O>(names, "minecraft:potion", *p)?),
                            None => None,
                        },
                    )
                    .optional("custom_color", color.map(O::create_int));
                // An empty tail is no custom effects and no name.
                if rest.is_empty() {
                    rec
                } else {
                    raw::potion_tail_with::<O>(rec, rest, names)?
                }
                .build()
            }
            Self::CustomModelData {
                floats,
                flags,
                strings,
                colors,
            } => {
                let list = |rec: Record<O>, name: &str, items: Vec<O::Value>| {
                    if items.is_empty() {
                        rec
                    } else {
                        rec.field(name, O::create_list(items))
                    }
                };
                let rec = list(
                    Record::<O>::new(),
                    "floats",
                    floats.iter().map(|f| O::create_f32(*f)).collect(),
                );
                let rec = list(
                    rec,
                    "flags",
                    flags.iter().map(|f| O::create_boolean(*f)).collect(),
                );
                let rec = list(
                    rec,
                    "strings",
                    strings.iter().map(|s| string_with::<O>(s)).collect(),
                );
                list(
                    rec,
                    "colors",
                    colors.iter().map(|c| O::create_int(*c)).collect(),
                )
                .build()
            }
            Self::TooltipDisplay { hide, hidden } => {
                let rec = or_default_with::<O, _>(
                    Record::<O>::new(),
                    "hide_tooltip",
                    *hide,
                    false,
                    O::create_boolean(*hide),
                );
                let mut list = Vec::with_capacity(hidden.len());
                let mut seen = std::collections::HashSet::new();
                for h in hidden {
                    if !seen.insert(*h) {
                        continue;
                    }
                    list.push(entry_with::<O>(names, "minecraft:data_component_type", *h)?);
                }
                if list.is_empty() {
                    rec
                } else {
                    rec.field("hidden_components", O::set(list))
                }
                .build()
            }
            Self::WritableBook { pages, filtered } => if pages.is_empty() {
                Record::<O>::new()
            } else {
                Record::<O>::new().field(
                    "pages",
                    O::create_list(pages.iter().enumerate().map(|(i, p)| {
                        filterable_with::<O>(
                            string_with::<O>(p),
                            filtered
                                .get(i)
                                .and_then(Option::as_ref)
                                .map(|f| string_with::<O>(f)),
                        )
                    })),
                )
            }
            .build(),
            Self::WrittenBook(b) => {
                let rec = Record::<O>::new()
                    .field(
                        "title",
                        filterable_with::<O>(
                            string_with::<O>(&b.title),
                            b.title_filtered.as_deref().map(string_with::<O>),
                        ),
                    )
                    .field("author", string_with::<O>(&b.author));
                let rec = or_default_with::<O, _>(
                    rec,
                    "generation",
                    b.generation,
                    0,
                    O::create_int(b.generation),
                );
                let rec = if b.pages.is_empty() {
                    rec
                } else {
                    let mut pages = Vec::with_capacity(b.pages.len());
                    for (i, page) in b.pages.iter().enumerate() {
                        let filtered = match b.pages_filtered.get(i).and_then(Option::as_ref) {
                            Some(value) => Some(text_with::<O>(value)?),
                            None => None,
                        };
                        pages.push(filterable_with::<O>(text_with::<O>(page)?, filtered));
                    }
                    rec.field("pages", O::create_list(pages))
                };
                or_default_with::<O, _>(
                    rec,
                    "resolved",
                    b.resolved,
                    false,
                    O::create_boolean(b.resolved),
                )
                .build()
            }
            Self::Fireworks { flight, explosions } => {
                // `ExtraCodecs.UNSIGNED_BYTE` is `Codec.BYTE`: a byte tag.
                let rec = or_default_with::<O, _>(
                    Record::<O>::new(),
                    "flight_duration",
                    *flight,
                    0,
                    O::create_byte(*flight as i8),
                );
                let mut list = Vec::with_capacity(explosions.len());
                for e in explosions {
                    list.push(explosion_with::<O>(e)?);
                }
                if list.is_empty() {
                    rec
                } else {
                    rec.field("explosions", O::create_list(list))
                }
                .build()
            }
            Self::FireworkExplosion(e) => explosion_with::<O>(e)?,
            Self::LodestoneTracker { target, tracked } => {
                let target = match target {
                    Some((dimension, pos)) => Some(
                        Record::<O>::new()
                            .field("dimension", identifier_with::<O>(dimension)?)
                            .field("pos", O::create_int_list(pos))
                            .build(),
                    ),
                    None => None,
                };
                or_default_with::<O, _>(
                    Record::<O>::new().optional("target", target),
                    "tracked",
                    *tracked,
                    true,
                    O::create_boolean(*tracked),
                )
                .build()
            }
            Self::Trim { material, pattern } => Record::<O>::new()
                .field(
                    "material",
                    entry_with::<O>(names, "minecraft:trim_material", *material)?,
                )
                .field(
                    "pattern",
                    entry_with::<O>(names, "minecraft:trim_pattern", *pattern)?,
                )
                .build(),
            Self::StewEffects(list) => {
                let mut out = Vec::with_capacity(list.len());
                for (effect, duration) in list {
                    let rec = Record::<O>::new().field(
                        "id",
                        entry_with::<O>(names, "minecraft:mob_effect", *effect)?,
                    );
                    out.push(
                        or_default_with::<O, _>(
                            rec,
                            "duration",
                            *duration,
                            160,
                            O::create_int(*duration),
                        )
                        .build(),
                    );
                }
                O::create_list(out)
            }
            Self::Instrument(i) => entry_with::<O>(names, "minecraft:instrument", *i)?,
            Self::BannerPatterns(layers) => {
                let mut out = Vec::with_capacity(layers.len());
                for (pattern, color) in layers {
                    out.push(
                        Record::<O>::new()
                            .field(
                                "pattern",
                                entry_with::<O>(names, "minecraft:banner_pattern", *pattern)?,
                            )
                            .field("color", pick_with::<O>(&DYE, *color)?)
                            .build(),
                    );
                }
                O::create_list(out)
            }
            Self::PaintingVariant(i) => entry_with::<O>(names, "minecraft:painting_variant", *i)?,
            Self::Profile(p) => raw::profile_with::<O>(p),
            Self::TrimDirect { .. } => {
                let mut writer = ::sunrise_core::vanilla_bytes::Writer::new();
                self.write(id, &mut writer);
                raw::code_with::<O>(id, &writer.data, names)?
            }
            Self::Raw(_) | Self::Stacks(_) => return None,
        })
    }

    /// [`Component::code`] as `HashCode.asInt`, the value the packet carries.
    pub fn hash(&self, id: i32, names: &dyn Names) -> Option<i32> {
        self.code(id, names).map(HashCode::as_int)
    }
}

impl Hashed {
    /// `HashedStack.matches`: compare the entire effective patch as a map and set,
    /// independent of wire order. A value we cannot hash cannot confirm a claim.
    pub fn matches(&self, stack: &Stack, names: &dyn Names) -> bool {
        self.matches_projected(stack, || Self::create(stack, names))
    }

    /// Use the source values admitted for native effects, without projecting those values again.
    pub fn matches_with_admitted(
        &self,
        stack: &Stack,
        names: &dyn Names,
        admitted: &AdmissionLookup<'_>,
    ) -> bool {
        self.matches_projected(stack, || Self::create_with_admitted(stack, names, admitted))
    }

    fn matches_projected(
        &self,
        stack: &Stack,
        project: impl FnOnce() -> Result<Self, StackHashError>,
    ) -> bool {
        self.item == stack.item
            && self.count == stack.count
            && project().is_ok_and(|expected| {
                self.added.len() == expected.added.len()
                    && self.removed.len() == expected.removed.len()
                    && expected
                        .added
                        .iter()
                        .all(|entry| self.added.contains(entry))
                    && expected.removed.iter().all(|id| self.removed.contains(id))
            })
    }

    /// Hash a retained stream patch using the effect values already decoded from it.
    pub fn create_with_admitted(
        stack: &Stack,
        names: &dyn Names,
        admitted: &AdmissionLookup<'_>,
    ) -> Result<Self, StackHashError> {
        struct Projection<'a> {
            names: &'a dyn Names,
            admitted: &'a AdmissionLookup<'a>,
        }
        impl Names for Projection<'_> {
            fn name(&self, registry: &str, id: i32) -> Option<String> {
                self.names.name(registry, id)
            }

            fn consume_effect_kind(&self, id: i32) -> Option<super::consume_effects::Kind> {
                self.names.consume_effect_kind(id)
            }

            fn consume_effect_admissions(&self) -> Option<&AdmissionLookup<'_>> {
                Some(self.admitted)
            }
        }
        Self::create(stack, &Projection { names, admitted })
    }

    /// Hash all effective sets on a nonempty stack, or refuse the whole patch.
    /// An absent slot is represented by the caller, separately from failure.
    pub fn create(stack: &Stack, names: &dyn Names) -> Result<Self, StackHashError> {
        // For a stack kept as the server sent it, hash the same retained patch
        // that DelimitedSlot would write. Malformed bytes are not an empty patch.
        let decoded = stack
            .wire
            .0
            .as_deref()
            .map(|patch| WirePatch::decode_reader(Reader::new(patch).with_names(names)))
            .transpose()
            .map_err(StackHashError::Wire)?;
        let (sets, removals) = decoded
            .as_ref()
            .map_or((&stack.added, &stack.removed), |(a, r)| (a, r));
        let (sets, removed) = super::effective_wire_patch(sets, removals);
        let mut added: Vec<_> = sets
            .map(|(id, component)| {
                let hash = component
                    .hash(*id, names)
                    .ok_or(StackHashError::Component(*id))?;
                Ok((*id, hash))
            })
            .collect::<Result<_, _>>()?;
        added.reverse();
        Ok(Self {
            item: stack.item,
            count: stack.count,
            added,
            removed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Component as C, WrittenBook, component as k};
    use sunrise_core::nbt::Tag;

    /// Ids for the tests, by registry (the position in the table is the id).
    struct Table;
    impl Names for Table {
        fn name(&self, registry: &str, id: i32) -> Option<String> {
            let names: &[&str] = match registry {
                "minecraft:enchantment" => &["minecraft:sharpness", "minecraft:protection"],
                "minecraft:potion" => &["minecraft:water", "minecraft:swiftness"],
                "minecraft:mob_effect" => &["minecraft:speed", "minecraft:night_vision"],
                "minecraft:trim_material" => &["minecraft:gold"],
                "minecraft:trim_pattern" => &["minecraft:sentry"],
                "minecraft:instrument" => &["minecraft:ponder_goat_horn"],
                "minecraft:banner_pattern" => &["minecraft:creeper"],
                "minecraft:painting_variant" => &["minecraft:kebab"],
                "minecraft:item" => &[
                    "minecraft:air",
                    "minecraft:firework_rocket",
                    "minecraft:arrow",
                ],
                "minecraft:data_component_type" => {
                    return match id {
                        k::DAMAGE => Some("minecraft:damage"),
                        k::ENCHANTMENTS => Some("minecraft:enchantments"),
                        k::RARITY => Some("minecraft:rarity"),
                        _ => None,
                    }
                    .map(str::to_owned);
                }
                _ => return None,
            };
            names.get(usize::try_from(id).ok()?).map(|n| n.to_string())
        }
    }

    fn text(s: &str) -> Tag {
        Tag::String(s.into())
    }
    fn explosion(shape: i32, colors: &[i32], fade: &[i32], trail: bool) -> FireworkExplosion {
        FireworkExplosion {
            shape,
            colors: colors.to_vec(),
            fade: fade.to_vec(),
            trail,
            twinkle: false,
        }
    }

    /// Hashes printed by vanilla 26.3 (`TypedDataComponent.encodeValue` through a
    /// `RegistryOps<HashCode>` over `HashOps.CRC32C_INSTANCE`, then `asInt`) for
    /// the same component values.
    #[test]
    fn component_hashes_match_vanilla() {
        let nbt = Tag::compound([
            ("n", Tag::Int(7)),
            ("flag", Tag::Byte(1)),
            ("s", text("hi")),
            ("l", Tag::List(vec![Tag::Int(1)])),
        ]);
        let styled = Tag::compound([
            ("text", text("Hi")),
            ("bold", Tag::Byte(1)),
            ("color", text("red")),
        ]);
        let extra = Tag::compound([
            ("text", text("A")),
            (
                "extra",
                Tag::List(vec![Tag::compound([
                    ("translate", text("block.minecraft.stone")),
                    ("italic", Tag::Byte(0)),
                ])]),
            ),
        ]);
        let two = Tag::compound([("text", text("two")), ("underlined", Tag::Byte(1))]);
        let fe = explosion(2, &[0xFF0000, 0x00FF00], &[1], true);
        let fireworks_template = {
            let mut s = Stack::new(1, 3);
            s.added.push((k::DAMAGE, C::Int(2)));
            s.removed.push(k::RARITY);
            s
        };
        let cases: Vec<(&str, i32, C, i32)> = vec![
            ("damage", k::DAMAGE, C::Int(5), 645064431),
            ("max_stack_size", k::MAX_STACK_SIZE, C::Int(16), 1769065625),
            ("repair_cost", k::REPAIR_COST, C::Int(3), -499649379),
            ("rarity", k::RARITY, C::Int(3), -292715907),
            ("unbreakable", k::UNBREAKABLE, C::Unit, -982207288),
            (
                "glint",
                k::ENCHANTMENT_GLINT_OVERRIDE,
                C::Bool(true),
                -1019818302,
            ),
            (
                "item_model",
                k::ITEM_MODEL,
                C::Ident("minecraft:stick".into()),
                -1153337500,
            ),
            ("custom_data", k::CUSTOM_DATA, C::Nbt(nbt), 588551639),
            (
                "enchantments",
                k::ENCHANTMENTS,
                C::Enchantments(vec![(0, 3), (1, 4)]),
                156916527,
            ),
            (
                "custom_name",
                k::CUSTOM_NAME,
                C::Text(text("Hi")),
                485089409,
            ),
            (
                "custom_name_styled",
                k::CUSTOM_NAME,
                C::Text(styled),
                -204970011,
            ),
            ("item_name_extra", k::ITEM_NAME, C::Text(extra), 868407222),
            ("lore", k::LORE, C::Lore(vec![text("one"), two]), 931033850),
            ("lore_empty", k::LORE, C::Lore(vec![]), -1978007022),
            ("dyed", k::DYED_COLOR, C::Int(0xA06540), -1304070639),
            ("base_color", k::BASE_COLOR, C::Int(14), -1939582294),
            ("map_id", k::MAP_ID, C::Int(12), 225652657),
            (
                "ominous",
                k::OMINOUS_BOTTLE_AMPLIFIER,
                C::Int(2),
                1064459813,
            ),
            (
                "cmd",
                k::CUSTOM_MODEL_DATA,
                C::CustomModelData {
                    floats: vec![1.5],
                    flags: vec![true, false],
                    strings: vec!["a".into()],
                    colors: vec![0xFF0000],
                },
                -2044548697,
            ),
            (
                "cmd_flags",
                k::CUSTOM_MODEL_DATA,
                C::CustomModelData {
                    floats: vec![],
                    flags: vec![true],
                    strings: vec![],
                    colors: vec![],
                },
                1169437062,
            ),
            (
                "tooltip",
                k::TOOLTIP_DISPLAY,
                C::TooltipDisplay {
                    hide: true,
                    hidden: vec![k::DAMAGE, k::ENCHANTMENTS],
                },
                1281445049,
            ),
            (
                "tooltip_default",
                k::TOOLTIP_DISPLAY,
                C::TooltipDisplay {
                    hide: false,
                    hidden: vec![],
                },
                -982207288,
            ),
            (
                "attack_range",
                k::ATTACK_RANGE,
                C::AttackRange([0.0, 4.0, 0.0, 6.0, 0.3, 1.0]),
                -287603881,
            ),
            (
                "attack_range_default",
                k::ATTACK_RANGE,
                C::AttackRange([0.0, 3.0, 0.0, 5.0, 0.3, 1.0]),
                -982207288,
            ),
            (
                "firework_explosion",
                k::FIREWORK_EXPLOSION,
                C::FireworkExplosion(fe.clone()),
                282963373,
            ),
            (
                "firework_explosion_plain",
                k::FIREWORK_EXPLOSION,
                C::FireworkExplosion(explosion(4, &[], &[], false)),
                -1702512602,
            ),
            (
                "fireworks",
                k::FIREWORKS,
                C::Fireworks {
                    flight: 2,
                    explosions: vec![fe],
                },
                -1263695327,
            ),
            (
                "fireworks_empty",
                k::FIREWORKS,
                C::Fireworks {
                    flight: 0,
                    explosions: vec![],
                },
                -982207288,
            ),
            (
                "potion",
                k::POTION_CONTENTS,
                C::Potion {
                    potion: Some(1),
                    color: Some(0x123456),
                    rest: vec![],
                },
                360529701,
            ),
            (
                "potion_water",
                k::POTION_CONTENTS,
                C::Potion {
                    potion: Some(0),
                    color: None,
                    rest: vec![],
                },
                1311226275,
            ),
            (
                "stew",
                k::SUSPICIOUS_STEW_EFFECTS,
                C::StewEffects(vec![(0, 160), (1, 100)]),
                -1574156649,
            ),
            (
                "lodestone",
                k::LODESTONE_TRACKER,
                C::LodestoneTracker {
                    target: Some(("minecraft:overworld".into(), [1, 64, -3])),
                    tracked: false,
                },
                -1499562110,
            ),
            (
                "lodestone_none",
                k::LODESTONE_TRACKER,
                C::LodestoneTracker {
                    target: None,
                    tracked: true,
                },
                -982207288,
            ),
            (
                "trim",
                k::TRIM,
                C::Trim {
                    material: 0,
                    pattern: 0,
                },
                -1936516638,
            ),
            ("instrument", k::INSTRUMENT, C::Instrument(0), -1123313158),
            (
                "banner",
                k::BANNER_PATTERNS,
                C::BannerPatterns(vec![(0, 5)]),
                1048823812,
            ),
            (
                "painting",
                k::PAINTING_VARIANT,
                C::PaintingVariant(0),
                -670067346,
            ),
            (
                "writable",
                k::WRITABLE_BOOK_CONTENT,
                C::WritableBook {
                    pages: vec!["page one".into(), "two".into()],
                    filtered: vec![],
                },
                1900254632,
            ),
            (
                "written",
                k::WRITTEN_BOOK_CONTENT,
                C::WrittenBook(WrittenBook {
                    title: "Title".into(),
                    author: "Author".into(),
                    generation: 1,
                    pages: vec![text("p1")],
                    resolved: true,
                    ..WrittenBook::default()
                }),
                -112185549,
            ),
            (
                "written_min",
                k::WRITTEN_BOOK_CONTENT,
                C::WrittenBook(WrittenBook {
                    title: "T".into(),
                    author: "A".into(),
                    generation: 0,
                    pages: vec![],
                    resolved: false,
                    ..WrittenBook::default()
                }),
                -809804424,
            ),
            (
                "charged",
                k::CHARGED_PROJECTILES,
                C::Stacks(vec![Stack::new(2, 1), fireworks_template.clone()]),
                -1839603258,
            ),
            (
                "bundle",
                k::BUNDLE_CONTENTS,
                C::Stacks(vec![fireworks_template]),
                785458737,
            ),
        ];
        for (name, id, component, expected) in cases {
            assert_eq!(component.hash(id, &Table), Some(expected), "{name}");
        }
    }

    #[test]
    fn components_without_a_codec_form_are_not_hashed() {
        // Unresolvable registry ids, bytes Sunrise only reads past, and the
        // components vanilla has no persistent codec for.
        assert_eq!(
            C::Enchantments(vec![(9, 1)]).hash(k::ENCHANTMENTS, &Table),
            None
        );
        // additional_trade_cost has no persistent codec either.
        assert_eq!(C::Raw(vec![1]).hash(42, &Table), None);
        assert_eq!(C::Unit.hash(k::CREATIVE_SLOT_LOCK, &Table), None);
        assert_eq!(C::Int(0).hash(k::MAP_POST_PROCESSING, &Table), None);
        // Custom effects cut short.
        let custom_effects = C::Potion {
            potion: None,
            color: None,
            rest: vec![1, 0, 0],
        };
        assert_eq!(custom_effects.hash(k::POTION_CONTENTS, &Table), None);
    }

    #[test]
    fn a_prediction_must_match_the_whole_patch() {
        let mut stack = Stack::new(1, 2);
        stack.added.push((k::DAMAGE, C::Int(5)));
        stack.removed.push(k::RARITY);
        // `TypedDataComponent.encodeValue` with vanilla 26.3's CRC32C HashOps.
        let correct = Hashed {
            item: 1,
            count: 2,
            added: vec![(k::DAMAGE, 645064431)],
            removed: vec![k::RARITY],
        };
        assert!(correct.matches(&stack, &Table));
        let wrong_hash = Hashed {
            added: vec![(k::DAMAGE, 0)],
            ..correct.clone()
        };
        assert!(!wrong_hash.matches(&stack, &Table));
        let missing_removal = Hashed {
            removed: vec![],
            ..correct.clone()
        };
        assert!(!missing_removal.matches(&stack, &Table));
        let wrong_removed_key = Hashed {
            removed: vec![k::TOOL],
            ..correct.clone()
        };
        assert!(!wrong_removed_key.matches(&stack, &Table));
        let wrong_added_key = Hashed {
            added: vec![(k::MAP_ID, 645064431)],
            ..correct.clone()
        };
        assert!(!wrong_added_key.matches(&stack, &Table));

        stack.added.push((k::MAP_POST_PROCESSING, C::Int(0)));
        assert!(!correct.matches(&stack, &Table), "missing unhashable key");
        let mut unprovable = correct;
        unprovable.added.push((k::MAP_POST_PROCESSING, 0));
        assert!(
            !unprovable.matches(&stack, &Table),
            "unavailable value hash"
        );
    }

    #[test]
    fn repeated_enchantment_levels_keep_first_key_order_and_last_values() {
        let entries = C::Enchantments(vec![(1, 0), (0, 1), (1, 2), (0, 3)]);
        let effective = C::Enchantments(vec![(1, 2), (0, 3)]);
        let value = persistent::component_value(k::ENCHANTMENTS, &entries, &Table).unwrap();
        let ids = |registry: &str, name: &str| match (registry, name) {
            ("minecraft:enchantment", "minecraft:sharpness") => Some(0),
            ("minecraft:enchantment", "minecraft:protection") => Some(1),
            _ => None,
        };
        assert_eq!(
            persistent::component_from_value(k::ENCHANTMENTS, &value, &ids).unwrap(),
            effective
        );
        assert_eq!(
            entries.hash(k::ENCHANTMENTS, &Table),
            effective.hash(k::ENCHANTMENTS, &Table)
        );
    }

    #[test]
    fn a_hashed_stack_uses_effective_sets_and_refuses_an_incomplete_patch() {
        let mut stack = Stack::new(1, 2);
        stack.added.push((k::DAMAGE, C::Int(1)));
        stack.added.push((k::DAMAGE, C::Int(5)));
        stack.added.push((k::RARITY, C::Int(1)));
        stack.removed.extend([k::RARITY, k::RARITY]);
        assert_eq!(
            Hashed::create(&stack, &Table).unwrap(),
            Hashed {
                item: 1,
                count: 2,
                added: vec![(k::DAMAGE, 645064431)],
                removed: vec![k::RARITY]
            }
        );
        // Without names the plain ones still hash.
        assert_eq!(
            Hashed::of(&stack).unwrap().added,
            vec![(k::DAMAGE, 645064431)]
        );
        // A stack kept as the server sent it hashes the patch it holds.
        use crate::buf::Codec as _;
        let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
        crate::item::DelimitedSlot::write(&Some(stack.clone()), &mut w);
        let kept = crate::item::DelimitedSlot::read(&mut crate::buf::Reader::new(&w.data))
            .unwrap()
            .unwrap();
        let mut bare = Stack::new(1, 2);
        bare.wire = kept.wire;
        assert_eq!(
            Hashed::create(&bare, &Table),
            Hashed::create(&stack, &Table)
        );
        bare.wire.0 = Some(vec![0, 0, 0].into());
        assert!(matches!(
            Hashed::create(&bare, &Table),
            Err(StackHashError::Wire(_))
        ));

        // A valid sibling must not hide an invalid set, on either input path.
        stack.added.push((k::DAMAGE, C::Int(-1)));
        assert_eq!(
            Hashed::create(&stack, &Table),
            Err(StackHashError::Component(k::DAMAGE))
        );
        let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
        crate::item::DelimitedSlot::write(&Some(stack), &mut w);
        let kept = crate::item::DelimitedSlot::read(&mut crate::buf::Reader::new(&w.data))
            .unwrap()
            .unwrap();
        assert_eq!(
            Hashed::create(&kept, &Table),
            Err(StackHashError::Component(k::DAMAGE))
        );
    }
}
