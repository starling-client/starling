use super::{HashMap, HashSet, PackOverlay, Path, Result};
use std::sync::Arc;

/// One generation of bound entity type tags. The ordered view is sent to
/// clients; the membership and type bits answer gameplay queries.
#[derive(Clone, Default)]
pub struct EntityTypeTags {
    ids: Arc<[String]>,
    ordered: HashMap<String, Vec<String>>,
    members: Arc<HashMap<String, HashSet<String>>>,
    bits: Vec<u32>,
}

/// The fixed vanilla tags gameplay asks about, each resolved to one bit per
/// entity type whenever the membership is bound or reloaded.
const FLAGS: [(&str, u32); 21] = [
    (
        "minecraft:burn_in_daylight",
        EntityTypeTags::BURN_IN_DAYLIGHT,
    ),
    (
        "minecraft:powder_snow_walkable_mobs",
        EntityTypeTags::POWDER_SNOW_WALKABLE,
    ),
    (
        "minecraft:can_breathe_under_water",
        EntityTypeTags::CAN_BREATHE_UNDER_WATER,
    ),
    (
        "minecraft:dismounts_underwater",
        EntityTypeTags::DISMOUNTS_UNDERWATER,
    ),
    (
        "minecraft:non_controlling_rider",
        EntityTypeTags::NON_CONTROLLING_RIDER,
    ),
    (
        "minecraft:cannot_be_age_locked",
        EntityTypeTags::CANNOT_BE_AGE_LOCKED,
    ),
    (
        "minecraft:beehive_inhabitors",
        EntityTypeTags::BEEHIVE_INHABITORS,
    ),
    (
        "minecraft:cannot_be_pushed_onto_boats",
        EntityTypeTags::CANNOT_BE_PUSHED_ONTO_BOATS,
    ),
    (
        "minecraft:can_turn_in_boats",
        EntityTypeTags::CAN_TURN_IN_BOATS,
    ),
    (
        "minecraft:inverted_healing_and_harm",
        EntityTypeTags::INVERTED_HEALING_AND_HARM,
    ),
    (
        "minecraft:can_float_while_ridden",
        EntityTypeTags::CAN_FLOAT_WHILE_RIDDEN,
    ),
    (
        "minecraft:axolotl_always_hostiles",
        EntityTypeTags::AXOLOTL_ALWAYS_HOSTILES,
    ),
    (
        "minecraft:axolotl_hunt_targets",
        EntityTypeTags::AXOLOTL_HUNT_TARGETS,
    ),
    (
        "minecraft:freeze_immune_entity_types",
        EntityTypeTags::FREEZE_IMMUNE,
    ),
    (
        "minecraft:fall_damage_immune",
        EntityTypeTags::FALL_DAMAGE_IMMUNE,
    ),
    (
        "minecraft:immune_to_infested",
        EntityTypeTags::IMMUNE_TO_INFESTED,
    ),
    (
        "minecraft:immune_to_oozing",
        EntityTypeTags::IMMUNE_TO_OOZING,
    ),
    (
        "minecraft:ignores_poison_and_regen",
        EntityTypeTags::IGNORES_POISON_AND_REGEN,
    ),
    (
        "minecraft:cannot_be_dismounted_by_item_usage",
        EntityTypeTags::CANNOT_BE_DISMOUNTED_BY_ITEM_USAGE,
    ),
    ("minecraft:raiders", EntityTypeTags::RAIDERS),
    (
        "minecraft:not_affected_by_geysers",
        EntityTypeTags::NOT_AFFECTED_BY_GEYSERS,
    ),
];

impl EntityTypeTags {
    pub const REGISTRY: &str = "minecraft:entity_type";
    pub const BURN_IN_DAYLIGHT: u32 = 1;
    pub const POWDER_SNOW_WALKABLE: u32 = 1 << 1;
    pub const CAN_BREATHE_UNDER_WATER: u32 = 1 << 2;
    pub const DISMOUNTS_UNDERWATER: u32 = 1 << 3;
    pub const NON_CONTROLLING_RIDER: u32 = 1 << 4;
    pub const CANNOT_BE_AGE_LOCKED: u32 = 1 << 5;
    pub const BEEHIVE_INHABITORS: u32 = 1 << 6;
    pub const CANNOT_BE_PUSHED_ONTO_BOATS: u32 = 1 << 7;
    pub const CAN_TURN_IN_BOATS: u32 = 1 << 8;
    pub const INVERTED_HEALING_AND_HARM: u32 = 1 << 9;
    pub const CAN_FLOAT_WHILE_RIDDEN: u32 = 1 << 10;
    pub const AXOLOTL_ALWAYS_HOSTILES: u32 = 1 << 11;
    pub const AXOLOTL_HUNT_TARGETS: u32 = 1 << 12;
    pub const FREEZE_IMMUNE: u32 = 1 << 13;
    pub const FALL_DAMAGE_IMMUNE: u32 = 1 << 14;
    pub const IMMUNE_TO_INFESTED: u32 = 1 << 15;
    pub const IMMUNE_TO_OOZING: u32 = 1 << 16;
    pub const IGNORES_POISON_AND_REGEN: u32 = 1 << 17;
    pub const CANNOT_BE_DISMOUNTED_BY_ITEM_USAGE: u32 = 1 << 18;
    pub const RAIDERS: u32 = 1 << 19;
    pub const NOT_AFFECTED_BY_GEYSERS: u32 = 1 << 20;

    /// The component registry's entity type index, including content-mod types.
    pub fn registry_ids(
        rows: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Arc<[String]>> {
        let mut ids = rows
            .keys()
            .map(|name| sunrise_core::identifier::Identifier::parse(name).map(|id| id.to_string()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.sort_unstable();
        ids.dedup();
        Ok(ids.into())
    }

    pub fn ids(&self) -> &Arc<[String]> {
        &self.ids
    }

    /// Bind pack tags to the prepared or frozen entity registry's sorted IDs.
    pub fn load(root: &Path, overlay: &PackOverlay, ids: Arc<[String]>) -> Result<Self> {
        let ordered = sunrise_world::tag_layers::load(root, "entity_type", overlay, |name| {
            ids.binary_search_by(|id| id.as_str().cmp(name)).is_ok()
        })?;
        Ok(Self::from_ordered(ordered, ids))
    }

    pub fn from_ordered(ordered: HashMap<String, Vec<String>>, ids: Arc<[String]>) -> Self {
        let members: HashMap<String, HashSet<String>> = ordered
            .iter()
            .map(|(tag, values)| (tag.clone(), values.iter().cloned().collect()))
            .collect();
        let flags = FLAGS.map(|(tag, bit)| (members.get(tag), bit));
        let bits = ids
            .iter()
            .map(|id| {
                flags.iter().fold(0, |bits, &(tag, bit)| {
                    bits | (u32::from(tag.is_some_and(|tag| tag.contains(id))) * bit)
                })
            })
            .collect();
        Self {
            ids,
            ordered,
            members: Arc::new(members),
            bits,
        }
    }

    pub fn ordered(&self) -> &HashMap<String, Vec<String>> {
        &self.ordered
    }

    pub fn members(&self) -> &Arc<HashMap<String, HashSet<String>>> {
        &self.members
    }

    pub fn contains(&self, tag: &str, id: &str) -> bool {
        self.members
            .get(tag)
            .is_some_and(|members| members.contains(id))
    }

    #[inline]
    pub fn has_bit(&self, slot: Option<usize>, flag: u32) -> bool {
        slot.and_then(|slot| self.bits.get(slot))
            .is_some_and(|bits| bits & flag != 0)
    }

    /// A type's slot for [`Self::has_bit`]: its place in the sorted registry
    /// IDs, found by a binary search. A mob's type keeps its slot from load
    /// (`MobType.flags.entity_tag_slot`); this is for a type known only by its ID.
    pub fn slot(&self, id: &str) -> Option<usize> {
        self.ids
            .binary_search_by(|known| known.as_str().cmp(id))
            .ok()
    }

    /// [`Self::has_bit`] for a type known only by its ID, through [`Self::slot`]:
    /// for per-event reads, not per-tick ones.
    pub fn has_flag(&self, id: &str, flag: u32) -> bool {
        self.has_bit(self.slot(id), flag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each fixed tag's bit follows the bound membership, and a rebind (a
    /// `/reload` or the server's tags) moves it with the membership.
    #[test]
    fn every_flag_follows_a_rebound_membership() {
        let ids: Arc<[String]> = ["test:a".to_owned(), "test:b".to_owned()].into();
        let bind = |member: &str| {
            EntityTypeTags::from_ordered(
                FLAGS
                    .iter()
                    .map(|(tag, _)| ((*tag).to_owned(), vec![member.to_owned()]))
                    .collect(),
                ids.clone(),
            )
        };
        let (first, reloaded) = (bind("test:a"), bind("test:b"));
        for (tag, bit) in FLAGS {
            assert!(first.has_flag("test:a", bit), "{tag}");
            assert!(!first.has_flag("test:b", bit), "{tag}");
            assert!(!reloaded.has_flag("test:a", bit), "{tag}");
            assert!(reloaded.has_flag("test:b", bit), "{tag}");
            assert_eq!(
                reloaded.has_bit(Some(1), bit),
                reloaded.contains(tag, "test:b")
            );
        }
        assert!(!first.has_flag("test:unknown", EntityTypeTags::RAIDERS));
        assert_eq!(
            FLAGS.iter().fold(0, |all, (_, bit)| all | bit).count_ones(),
            21
        );
    }
}
