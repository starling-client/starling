//! A mob lent during finalization or the position check, or just after a natural spawn attempt:
//! the scoped read-only view ([`BorrowedUnspawnedMob`]) and mutable editor
//! ([`BorrowedUnspawnedMobMut`]), the shared read/write helpers used by pending children, and
//! the spawn group data at the API boundary (`SpawnGroup`).

use super::Game;
use super::api_values::{TryIntoApi, TryIntoEngine};
use super::living_attributes::{AttributeWrite, collect_equipment, write_unspawned_attribute};
use mod_api::{
    AgeableGroup, AttributeModifier, AttributeOperation, EntityRef, EquipmentSlot, Identifier,
    ItemStack, SpawnGroup, TropicalVariant, UnspawnedMobError, UnspawnedMobRead, UnspawnedMobWrite,
    WorldEpoch,
};
use std::sync::Arc;
use sunrise_client::items::Items;
use sunrise_client::mobs::finalize::{Ageable, Group};
use sunrise_client::mobs::{Mob, equine, llama, render};
use sunrise_client::text::Text;
use sunrise_world::attribute_map::ValueKind;
use sunrise_world::attributes::{Operation, entity::Catalog};

/// The most tags an entity holds (`Entity.addTag`).
const MAX_TAGS: usize = 1024;

/// Read-only scoped borrow of an unspawned mob (or metadata-only when `mob` is `None`).
pub(in crate::game) struct BorrowedUnspawnedMob<'a> {
    kind: &'a str,
    position: [f64; 3],
    mob: Option<&'a Mob>,
    pub(super) live_tags: Option<mod_api::EntityTags<'a>>,
    attributes: &'a Arc<Catalog>,
}

impl UnspawnedMobRead for BorrowedUnspawnedMob<'_> {
    fn kind(&self) -> Result<Identifier, UnspawnedMobError> {
        match self.mob {
            Some(mob) => read_mob_kind(mob),
            None => Identifier::parse(self.kind).map_err(|_| UnspawnedMobError::Unsupported),
        }
    }

    fn position(&self) -> Result<[f64; 3], UnspawnedMobError> {
        match self.mob {
            Some(mob) => read_mob_position(mob),
            None => Ok(self.position),
        }
    }

    fn equipment(&self, slot: EquipmentSlot) -> Result<Option<ItemStack>, UnspawnedMobError> {
        read_mob_equipment(self.mob.ok_or(UnspawnedMobError::Unsupported)?, slot)
    }

    fn attribute_base(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        read_mob_attribute_base(
            self.mob.ok_or(UnspawnedMobError::Unsupported)?,
            self.attributes,
            attribute,
        )
    }

    fn attribute_value(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        read_mob_attribute_value(
            self.mob.ok_or(UnspawnedMobError::Unsupported)?,
            self.attributes,
            attribute,
        )
    }

    fn tags(&self) -> Result<Vec<String>, UnspawnedMobError> {
        if let Some(tags) = &self.live_tags {
            return Ok(tags.iter_utf16().map(String::from_utf16_lossy).collect());
        }
        read_mob_tags(self.mob.ok_or(UnspawnedMobError::Unsupported)?)
    }

    fn custom_name(&self) -> Result<Option<mod_api::Text>, UnspawnedMobError> {
        read_mob_custom_name(self.mob.ok_or(UnspawnedMobError::Unsupported)?)
    }
}

/// Mutable scoped loan of an unspawned mob (or metadata-only when `mob` is `None`).
pub(in crate::game) struct BorrowedUnspawnedMobMut<'a> {
    kind: &'a str,
    position: [f64; 3],
    mob: Option<&'a mut Mob>,
    items: &'a Items,
    attributes: &'a Arc<Catalog>,
}

impl UnspawnedMobRead for BorrowedUnspawnedMobMut<'_> {
    fn kind(&self) -> Result<Identifier, UnspawnedMobError> {
        match self.mob.as_deref() {
            Some(mob) => read_mob_kind(mob),
            None => Identifier::parse(self.kind).map_err(|_| UnspawnedMobError::Unsupported),
        }
    }

    fn position(&self) -> Result<[f64; 3], UnspawnedMobError> {
        match self.mob.as_deref() {
            Some(mob) => read_mob_position(mob),
            None => Ok(self.position),
        }
    }

    fn equipment(&self, slot: EquipmentSlot) -> Result<Option<ItemStack>, UnspawnedMobError> {
        read_mob_equipment(
            self.mob.as_deref().ok_or(UnspawnedMobError::Unsupported)?,
            slot,
        )
    }

    fn attribute_base(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        read_mob_attribute_base(
            self.mob.as_deref().ok_or(UnspawnedMobError::Unsupported)?,
            self.attributes,
            attribute,
        )
    }

    fn attribute_value(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        read_mob_attribute_value(
            self.mob.as_deref().ok_or(UnspawnedMobError::Unsupported)?,
            self.attributes,
            attribute,
        )
    }

    fn tags(&self) -> Result<Vec<String>, UnspawnedMobError> {
        read_mob_tags(self.mob.as_deref().ok_or(UnspawnedMobError::Unsupported)?)
    }

    fn custom_name(&self) -> Result<Option<mod_api::Text>, UnspawnedMobError> {
        read_mob_custom_name(self.mob.as_deref().ok_or(UnspawnedMobError::Unsupported)?)
    }
}

impl UnspawnedMobWrite for BorrowedUnspawnedMobMut<'_> {
    fn set_equipment(
        &mut self,
        slot: EquipmentSlot,
        item: Option<ItemStack>,
    ) -> Result<(), UnspawnedMobError> {
        write_mob_equipment(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            self.items,
            self.attributes,
            slot,
            item,
        )
    }

    fn set_attribute_base(
        &mut self,
        attribute: &Identifier,
        base: f64,
    ) -> Result<(), UnspawnedMobError> {
        write_mob_attribute_base(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            self.items,
            self.attributes,
            attribute,
            base,
        )
    }

    fn add_attribute_modifier(
        &mut self,
        attribute: &Identifier,
        modifier: AttributeModifier,
    ) -> Result<(), UnspawnedMobError> {
        write_mob_add_attribute_modifier(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            self.items,
            self.attributes,
            attribute,
            modifier,
        )
    }

    fn remove_attribute_modifier(
        &mut self,
        attribute: &Identifier,
        id: &Identifier,
    ) -> Result<bool, UnspawnedMobError> {
        write_mob_remove_attribute_modifier(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            self.items,
            self.attributes,
            attribute,
            id,
        )
    }

    fn add_tag(&mut self, tag: &str) -> Result<bool, UnspawnedMobError> {
        write_mob_add_tag(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            tag,
        )
    }

    fn remove_tag(&mut self, tag: &str) -> Result<bool, UnspawnedMobError> {
        write_mob_remove_tag(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            tag,
        )
    }

    fn set_custom_name(&mut self, name: Option<mod_api::Text>) -> Result<(), UnspawnedMobError> {
        write_mob_set_custom_name(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            name,
        )
    }
}

pub(super) fn read_mob_kind(mob: &Mob) -> Result<Identifier, UnspawnedMobError> {
    Identifier::parse(&mob.kind.id).map_err(|_| UnspawnedMobError::Unsupported)
}

pub(super) fn read_mob_position(mob: &Mob) -> Result<[f64; 3], UnspawnedMobError> {
    Ok([mob.position.x, mob.position.y, mob.position.z])
}

pub(super) fn read_mob_equipment(
    mob: &Mob,
    slot: EquipmentSlot,
) -> Result<Option<ItemStack>, UnspawnedMobError> {
    mob.equipment_stacks()[slot as usize]
        .try_into_api()
        .map_err(|error| error.into_write_error().into())
}

pub(super) fn read_mob_attribute_base(
    mob: &Mob,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
) -> Result<f64, UnspawnedMobError> {
    read_mob_attribute(mob, attributes, attribute, ValueKind::Base)
}

pub(super) fn read_mob_attribute_value(
    mob: &Mob,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
) -> Result<f64, UnspawnedMobError> {
    read_mob_attribute(mob, attributes, attribute, ValueKind::Computed)
}

fn read_mob_attribute(
    mob: &Mob,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
    kind: ValueKind,
) -> Result<f64, UnspawnedMobError> {
    mob.extra
        .live
        .read(
            attributes,
            &mob.kind.id,
            &attribute.to_string(),
            kind,
            || {
                sunrise_client::entity_nbt::attribute_entries(mob)
                    .into_iter()
                    .map(|(_, entry)| entry)
                    .collect()
            },
        )
        .ok_or(UnspawnedMobError::Unsupported)
}

pub(super) fn read_mob_tags(mob: &Mob) -> Result<Vec<String>, UnspawnedMobError> {
    Ok(mob.extra.spawn_tags.clone())
}

pub(super) fn read_mob_custom_name(mob: &Mob) -> Result<Option<mod_api::Text>, UnspawnedMobError> {
    mob.custom_name_text.try_into_api().map_err(Into::into)
}

pub(super) fn write_mob_equipment(
    mob: &mut Mob,
    items: &Items,
    attributes: &Arc<Catalog>,
    slot: EquipmentSlot,
    item: Option<ItemStack>,
) -> Result<(), UnspawnedMobError> {
    let stack = item
        .try_into_engine(items)
        .map_err(|_| UnspawnedMobError::Invalid)?;
    if !mob.set_equipment_stack(slot as usize, stack) {
        return Err(UnspawnedMobError::Unsupported);
    }
    collect_equipment(attributes, items, mob);
    Ok(())
}

pub(super) fn write_mob_attribute_base(
    mob: &mut Mob,
    items: &Items,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
    base: f64,
) -> Result<(), UnspawnedMobError> {
    if !base.is_finite() {
        return Err(UnspawnedMobError::Invalid);
    }
    write_mob_attribute(
        mob,
        items,
        attributes,
        attribute,
        AttributeWrite::Base(base),
    )
    .map(|_| ())
}

pub(super) fn write_mob_add_attribute_modifier(
    mob: &mut Mob,
    items: &Items,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
    modifier: AttributeModifier,
) -> Result<(), UnspawnedMobError> {
    let operation = match modifier.operation {
        AttributeOperation::AddValue => Operation::AddValue,
        AttributeOperation::AddMultipliedBase => Operation::AddMultipliedBase,
        AttributeOperation::AddMultipliedTotal => Operation::AddMultipliedTotal,
        _ => return Err(UnspawnedMobError::Invalid),
    };
    if !modifier.amount.is_finite() {
        return Err(UnspawnedMobError::Invalid);
    }
    let write = AttributeWrite::add(
        &modifier.id.to_string(),
        modifier.amount,
        operation,
        true,
        false,
    )
    .ok_or(UnspawnedMobError::Invalid)?;
    match write_mob_attribute(mob, items, attributes, attribute, write)? {
        true => Ok(()),
        false => Err(UnspawnedMobError::Invalid),
    }
}

pub(super) fn write_mob_remove_attribute_modifier(
    mob: &mut Mob,
    items: &Items,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
    id: &Identifier,
) -> Result<bool, UnspawnedMobError> {
    write_mob_attribute(
        mob,
        items,
        attributes,
        attribute,
        AttributeWrite::Remove(id.to_string()),
    )
}

fn write_mob_attribute(
    mob: &mut Mob,
    items: &Items,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
    write: AttributeWrite,
) -> Result<bool, UnspawnedMobError> {
    let name = attribute.to_string();
    // Every write here is a mod's, so a base stays in the attribute's range.
    let write = match write {
        AttributeWrite::Base(base) => AttributeWrite::Base(
            attributes
                .definition(&name)
                .map_or(base, |d| d.sanitize(base)),
        ),
        other => other,
    };
    write_unspawned_attribute(attributes, items, mob, &name, write)
        .ok_or(UnspawnedMobError::Unsupported)
}

pub(super) fn write_mob_add_tag(mob: &mut Mob, tag: &str) -> Result<bool, UnspawnedMobError> {
    let tags = &mut mob.extra.spawn_tags;
    if tags.iter().any(|t| t == tag) {
        return Ok(false);
    }
    if tag.is_empty() || tags.len() >= MAX_TAGS {
        return Err(UnspawnedMobError::Invalid);
    }
    tags.push(tag.to_owned());
    Ok(true)
}

pub(super) fn write_mob_remove_tag(mob: &mut Mob, tag: &str) -> Result<bool, UnspawnedMobError> {
    let tags = &mut mob.extra.spawn_tags;
    let before = tags.len();
    tags.retain(|t| t != tag);
    Ok(tags.len() != before)
}

pub(super) fn write_mob_set_custom_name(
    mob: &mut Mob,
    name: Option<mod_api::Text>,
) -> Result<(), UnspawnedMobError> {
    let name = name.try_into_engine(&())?;
    mob.custom_name = name.as_ref().map(Text::plain);
    mob.custom_name_text = name;
    Ok(())
}

impl Game<'_> {
    /// Builds a read-only scoped borrow of an unspawned mob (or metadata-only when `mob` is `None`).
    pub(in crate::game) fn borrowed_unspawned_mob<'a>(
        &'a self,
        kind: &'a str,
        position: [f64; 3],
        mob: Option<&'a Mob>,
    ) -> BorrowedUnspawnedMob<'a> {
        BorrowedUnspawnedMob {
            kind,
            position,
            mob,
            live_tags: None,
            attributes: &self.motion.attributes,
        }
    }

    /// Builds a mutable scoped loan of an unspawned mob (or metadata-only when `mob` is `None`).
    pub(in crate::game) fn borrowed_unspawned_mob_mut<'a>(
        &'a self,
        kind: &'a str,
        position: [f64; 3],
        mob: Option<&'a mut Mob>,
    ) -> BorrowedUnspawnedMobMut<'a> {
        BorrowedUnspawnedMobMut {
            kind,
            position,
            mob,
            items: world_items!(self),
            attributes: &self.motion.attributes,
        }
    }

    /// The world a lend's handles belong to; a game with none open lends
    /// under epoch 0, which no open world has.
    pub(in crate::game) fn lend_epoch(&self) -> WorldEpoch {
        self.native_lifecycle.epoch().unwrap_or(WorldEpoch::new(0))
    }

    /// The tags a mod gave the mobs from index `from` on before they joined
    /// move to where the level keeps a joined entity's tags.
    pub(super) fn join_spawn_tags(&mut self, from: usize) {
        for i in from..self.mobs.len() {
            if self.mobs[i].extra.spawn_tags.is_empty() {
                continue;
            }
            let given = std::mem::take(&mut self.mobs[i].extra.spawn_tags);
            let target = super::commands::Target::Mob(self.mobs[i].id);
            let mut tags = self.tags_of(target);
            for tag in given {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
            self.set_entity_tags(target, &tags);
        }
    }
}

/// The pack's group data as the API's, a school's leader a handle of `epoch`;
/// None where a value has no API name.
pub(in crate::game) fn native_spawn_group(
    group: &Group,
    epoch: WorldEpoch,
    variants: &sunrise_client::mobs::variants::VariantTables,
) -> Option<SpawnGroup> {
    let named = |names: &[&'static str], i: u8| names.get(usize::from(i)).map(|n| (*n).to_owned());
    Some(match group {
        Group::Ageable(a) => SpawnGroup::Ageable(native_ageable(a)),
        Group::Zombie { baby, jockey, husk } => SpawnGroup::Zombie {
            baby: *baby,
            jockey: *jockey,
            tried_camel: *husk,
        },
        Group::Spider(effect) => SpawnGroup::Spider {
            effect: match effect {
                Some(e) => Some(Identifier::parse(e).ok()?),
                None => None,
            },
        },
        Group::Horse(a, v) => SpawnGroup::Horse {
            ageable: native_ageable(a),
            variant: named(&equine::VARIANT_NAMES, *v)?,
        },
        Group::Llama(a, v) => SpawnGroup::Llama {
            ageable: native_ageable(a),
            variant: named(&llama::VARIANTS, *v)?,
        },
        Group::Fox(a, v) => SpawnGroup::Fox {
            ageable: native_ageable(a),
            variant: named(&FOX_VARIANTS, *v)?,
        },
        Group::Rabbit(a, v) => SpawnGroup::Rabbit {
            ageable: native_ageable(a),
            variant: RABBIT_VARIANTS
                .iter()
                .find(|(id, _)| id == v)
                .map(|(_, n)| (*n).to_owned())?,
        },
        Group::Axolotl(a, [first, second]) => SpawnGroup::Axolotl {
            ageable: native_ageable(a),
            variants: [
                named(&sunrise_client::entity_nbt::AXOLOTL_VARIANTS, *first)?,
                named(&sunrise_client::entity_nbt::AXOLOTL_VARIANTS, *second)?,
            ],
        },
        Group::Wolf(a, v) => SpawnGroup::Wolf {
            ageable: native_ageable(a),
            variant: Identifier::parse(variants.wolf.id(*v)?).ok()?,
        },
        Group::School(leader, look) => SpawnGroup::School {
            leader: EntityRef::new(epoch, *leader),
            tropical: match look {
                Some((pattern, base, color)) => Some(TropicalVariant::new(
                    named(&sunrise_client::entity_nbt::TROPICAL_PATTERNS, *pattern)?,
                    named(&render::DYE_NAMES, *base)?,
                    named(&render::DYE_NAMES, *color)?,
                )),
                None => None,
            },
        },
    })
}

/// The API's group data as the game's; None for a name the game does not know,
/// which drops the group.
pub(in crate::game) fn spawn_group(
    group: &SpawnGroup,
    variants: &sunrise_client::mobs::variants::VariantTables,
) -> Option<Group> {
    let index = |names: &[&str], name: &str| names.iter().position(|n| *n == name).map(|i| i as u8);
    Some(match group {
        SpawnGroup::Ageable(a) => Group::Ageable(ageable(a)),
        SpawnGroup::Zombie {
            baby,
            jockey,
            tried_camel,
        } => Group::Zombie {
            baby: *baby,
            jockey: *jockey,
            husk: *tried_camel,
        },
        SpawnGroup::Spider { effect } => Group::Spider(match effect {
            Some(id) => {
                let def = sunrise_client_core::potions::effect(&id.to_string())?;
                Some(sunrise_client_core::potions::namespaced(def.id))
            }
            None => None,
        }),
        SpawnGroup::Horse {
            ageable: a,
            variant,
        } => Group::Horse(ageable(a), index(&equine::VARIANT_NAMES, variant)?),
        SpawnGroup::Llama {
            ageable: a,
            variant,
        } => Group::Llama(ageable(a), index(&llama::VARIANTS, variant)?),
        SpawnGroup::Fox {
            ageable: a,
            variant,
        } => Group::Fox(ageable(a), index(&FOX_VARIANTS, variant)?),
        SpawnGroup::Rabbit {
            ageable: a,
            variant,
        } => Group::Rabbit(
            ageable(a),
            RABBIT_VARIANTS.iter().find(|(_, n)| n == variant)?.0,
        ),
        SpawnGroup::Axolotl {
            ageable: a,
            variants: [first, second],
        } => Group::Axolotl(
            ageable(a),
            [
                index(&sunrise_client::entity_nbt::AXOLOTL_VARIANTS, first)?,
                index(&sunrise_client::entity_nbt::AXOLOTL_VARIANTS, second)?,
            ],
        ),
        SpawnGroup::Wolf {
            ageable: a,
            variant,
        } => {
            let r = variants.wolf.lookup(variant.as_str())?;
            Group::Wolf(ageable(a), r)
        }
        SpawnGroup::School { leader, tropical } => Group::School(
            leader.id(),
            match tropical {
                Some(look) => Some((
                    index(
                        &sunrise_client::entity_nbt::TROPICAL_PATTERNS,
                        &look.pattern,
                    )?,
                    index(&render::DYE_NAMES, &look.base_color)?,
                    index(&render::DYE_NAMES, &look.pattern_color)?,
                )),
                None => None,
            },
        ),
        _ => return None,
    })
}

/// `Fox.Variant` names, by the game's index.
const FOX_VARIANTS: [&str; 2] = ["red", "snow"];
/// `Rabbit.Variant`: the game's id and its saved name.
const RABBIT_VARIANTS: [(u8, &str); 7] = [
    (0, "brown"),
    (1, "white"),
    (2, "black"),
    (3, "white_splotched"),
    (4, "gold"),
    (5, "salt"),
    (99, "evil"),
];

fn native_ageable(a: &Ageable) -> AgeableGroup {
    let mut out = AgeableGroup::new(a.should_spawn_baby, a.baby_chance);
    out.size = a.size;
    out
}

fn ageable(a: &AgeableGroup) -> Ageable {
    Ageable {
        size: a.size,
        should_spawn_baby: a.should_spawn_baby,
        baby_chance: a.baby_chance,
    }
}

#[cfg(test)]
mod tests {
    use super::super::commands::Target;
    use super::super::creatures::SpawnSite;
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use mod_api::{FinalizeSpawnEvent, Flow, Host};
    use std::sync::{Arc, Mutex};
    use sunrise_client::mobs::finalize::{self, Attr};
    use sunrise_client::mobs::spawn_rules::Reason;

    type Listen = dyn Fn(&dyn Host, &mut FinalizeSpawnEvent<'_>) -> Flow + Send + Sync;
    /// Answers `finalize_spawn` with the test's closure.
    struct Listener(Box<Listen>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listener {
        fn finalize_spawn(&self, host: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
            (self.0)(host, event)
        }
    }
    fn listening<R>(
        listen: impl Fn(&dyn Host, &mut FinalizeSpawnEvent<'_>) -> Flow + Send + Sync + 'static,
        run: impl FnOnce() -> R,
    ) -> R {
        let listener = Listener(Box::new(listen));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("unspawned", Box::new(listener))].into(),
            run,
        )
    }

    const AT: [f64; 3] = [8.5, 64.0, 8.5];
    fn id(s: &str) -> Identifier {
        Identifier::parse(s).unwrap()
    }
    fn pumpkin() -> ItemStack {
        super::super::api_values::test_snapshot(&sunrise_client::items::ItemStack::new(
            "minecraft:carved_pumpkin",
            1,
        ))
    }
    fn spawn_pig(game: &mut Game<'_>) -> Option<u32> {
        game.spawn_mob(
            "minecraft:pig",
            AT,
            false,
            SpawnSite::Finalizing(Reason::Natural),
        )
    }

    /// What a listener edits through the editor is on the mob that joins the
    /// level, and the handle reads the edits back inside the call.
    #[test]
    #[ignore = "daily"]
    fn edits_through_the_editor_are_on_the_mob_that_joins() {
        type Seen = (
            Result<Identifier, UnspawnedMobError>,
            Result<f64, UnspawnedMobError>,
            Result<Option<ItemStack>, UnspawnedMobError>,
        );
        let seen: Arc<Mutex<Option<Seen>>> = Arc::new(Mutex::new(None));
        let saw = seen.clone();
        let max_health = id("minecraft:max_health");
        listening(
            move |_, event| {
                let mob = &mut event.mob;
                mob.set_equipment(EquipmentSlot::Head, Some(pumpkin()))
                    .unwrap();
                let mut oversized = pumpkin();
                oversized.set_count(100).unwrap();
                assert_eq!(
                    mob.set_equipment(EquipmentSlot::Head, Some(oversized)),
                    Err(UnspawnedMobError::Invalid)
                );
                assert_eq!(mob.equipment(EquipmentSlot::Head), Ok(Some(pumpkin())));
                mob.set_equipment(EquipmentSlot::Head, None).unwrap();
                assert_eq!(mob.equipment(EquipmentSlot::Head), Ok(None));
                mob.set_equipment(EquipmentSlot::Head, Some(pumpkin()))
                    .unwrap();
                mob.set_attribute_base(&max_health, 20.0).unwrap();
                assert_eq!(mob.add_tag("test:tagged"), Ok(true));
                mob.set_custom_name(Some(mod_api::Text::literal("Bacon")))
                    .unwrap();
                *saw.lock().unwrap() = Some((
                    mob.kind(),
                    mob.attribute_base(&max_health),
                    mob.equipment(EquipmentSlot::Head),
                ));
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).unwrap();
                    let m = game.mobs.iter().find(|m| m.id == spawned).unwrap();
                    let head = &m.equipment_stacks()[EquipmentSlot::Head as usize];
                    assert_eq!(
                        head.as_ref().map(|s| &*s.item),
                        Some("minecraft:carved_pumpkin")
                    );
                    assert_eq!(finalize::base_value(m, Attr::MaxHealth), 20.0);
                    assert_eq!(m.custom_name.as_deref(), Some("Bacon"));
                    assert_eq!(game.tags_of(Target::Mob(spawned)), ["test:tagged"]);
                });
                // Item keys resolve with the game inside this scope, so the stack is read here.
                let (kind, base, head) = seen.lock().unwrap().take().expect("the hook ran");
                assert_eq!(kind, Ok(id("minecraft:pig")));
                assert_eq!(base, Ok(20.0));
                assert_eq!(head, Ok(Some(pumpkin())));
            },
        );
    }

    /// A mod's base written through the editor stays in the attribute's range, as documented.
    #[test]
    #[ignore = "daily"]
    fn a_base_written_through_the_editor_stays_in_range() {
        let seen: Arc<Mutex<Option<Result<f64, UnspawnedMobError>>>> = Arc::new(Mutex::new(None));
        let saw = seen.clone();
        let max_health = id("minecraft:max_health");
        listening(
            move |_, event| {
                let mob = &mut event.mob;
                mob.set_attribute_base(&max_health, 2000.0).unwrap();
                *saw.lock().unwrap() = Some(mob.attribute_base(&max_health));
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).unwrap();
                    let m = game.mobs.iter().find(|m| m.id == spawned).unwrap();
                    assert_eq!(finalize::base_value(m, Attr::MaxHealth), 1024.0);
                });
                assert_eq!(seen.lock().unwrap().take(), Some(Ok(1024.0)));
            },
        );
    }
    /// `Flow::Cancel` skips finalization and keeps the listener's edits on
    /// the mob that still joins; `spawn_cancelled` adds no mob at all.
    #[test]
    #[ignore = "daily"]
    fn both_cancels_keep_their_contract_with_edits() {
        let max_health = id("minecraft:max_health");
        listening(
            move |_, event| {
                event.mob.set_attribute_base(&max_health, 20.0).unwrap();
                Flow::Cancel
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).expect("a skipped finalization still spawns");
                    let m = game.mobs.iter().find(|m| m.id == spawned).unwrap();
                    assert_eq!(finalize::base_value(m, Attr::MaxHealth), 20.0);
                    assert!(
                        !finalize::has_modifier(
                            m,
                            Attr::FollowRange,
                            "minecraft:random_spawn_bonus"
                        ),
                        "no finalizeSpawn ran"
                    );
                });
            },
        );
        listening(
            |_, event| {
                assert_eq!(event.mob.add_tag("test:gone"), Ok(true));
                event.spawn_cancelled = true;
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let before = game.mobs.len();
                    assert_eq!(spawn_pig(game), None);
                    assert_eq!(game.mobs.len(), before, "a stopped spawn adds no mob");
                });
            },
        );
    }

    /// Every live site hands `group` over empty today; group data a listener
    /// leaves there is what finalization reads: an ageable group whose later
    /// members are all babies makes this pig one.
    #[test]
    #[ignore = "daily"]
    fn the_group_a_listener_leaves_is_what_finalization_reads() {
        with_game(|game| {
            let alone = spawn_pig(game).unwrap();
            let m = game.mobs.iter().find(|m| m.id == alone).unwrap();
            assert!(!m.is_baby(), "a pack's first member is never a baby");
        });
        let given = Arc::new(Mutex::new(Vec::new()));
        let heard = given.clone();
        listening(
            move |_, event| {
                heard.lock().unwrap().push(event.group.clone());
                let mut ageable = AgeableGroup::new(true, 1.0);
                ageable.size = 1;
                *event.group = Some(SpawnGroup::Ageable(ageable));
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).unwrap();
                    let m = game.mobs.iter().find(|m| m.id == spawned).unwrap();
                    assert!(m.is_baby());
                });
            },
        );
        assert_eq!(*given.lock().unwrap(), [None]);
    }

    /// A mod goal that only names itself: a mob carries it under this name
    /// while it has its goal, and as `ModGoal` once a copy left it out.
    struct Kept;
    impl mod_api::Goal for Kept {
        fn flags(&self) -> mod_api::GoalFlags {
            mod_api::GoalFlags::MOVE
        }
        fn name(&self) -> &'static str {
            "Kept"
        }
        fn can_use(&mut self, _: &mut mod_api::GoalContext<'_>) -> bool {
            false
        }
    }
    struct KeptGoal;
    #[mod_api::sunrise_part]
    impl mod_api::EntityPart for KeptGoal {
        fn register_goals(
            &self,
            _host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::RegisterGoalsEvent<'_, '_>,
        ) {
            event.goals.add_goal(7, Kept);
        }
    }
    struct KeptGoalMod;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for KeptGoalMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.entity_part("minecraft:zombie", KeptGoal);
        }
    }

    /// Unwinding out of an unspawned-mob loan leaves the mob intact in place
    /// (mod goal and all, never copied or replaced) and leaves no broken state
    /// for a subsequent `finalize_spawn` call.
    #[test]
    #[ignore = "daily"]
    fn an_engine_unwind_still_ends_the_unspawned_mob_lend() {
        let installed = || vec![mod_api::ModEntry::new("kept-goal", Box::new(KeptGoalMod))];
        mod_platform::testing::with_mods(installed().into(), || {
            super::super::api_values::install_parts(&installed()).unwrap();
            with_game(|game| {
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut zombie = Mob::new(70, kind, sunrise_core::geometry::Vec3::ZERO, 0.0);
                game.flush_unjoined_mob_parts(&mut zombie);
                zombie.custom_name = Some("Lent".to_owned());
                let goals = |m: &Mob| -> Vec<&'static str> {
                    m.ai.goals.entries().map(|(_, name, _)| name).collect()
                };
                assert!(
                    goals(&zombie).contains(&"Kept"),
                    "the part attached its goal"
                );
                let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut borrowed =
                        game.borrowed_unspawned_mob_mut("minecraft:zombie", AT, Some(&mut zombie));
                    let editor = mod_api::UnspawnedMobEditor::new(&mut borrowed);
                    assert_eq!(editor.kind(), Ok(id("minecraft:zombie")));
                    std::panic::resume_unwind(Box::new("unspawned mob fixture fault"));
                }));
                assert_eq!(
                    panicked.unwrap_err().downcast_ref::<&str>(),
                    Some(&"unspawned mob fixture fault")
                );
                assert_eq!(zombie.id, 70);
                assert_eq!(zombie.custom_name.as_deref(), Some("Lent"));
                let back = goals(&zombie);
                assert!(
                    back.contains(&"Kept"),
                    "the mod goal keeps its goal: {back:?}"
                );
                assert!(!back.contains(&"ModGoal"), "no copied mod goal: {back:?}");
            });
            super::super::api_values::install_parts(&[]).unwrap();
        });
        listening(
            |_, event| {
                assert_eq!(event.mob.add_tag("test:after_panic"), Ok(true));
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).unwrap();
                    assert_eq!(game.tags_of(Target::Mob(spawned)), ["test:after_panic"]);
                });
            },
        );
    }

    /// Group data crosses the boundary by the names vanilla saves its
    /// variants under, both ways, and a name the game does not know drops
    /// the group.
    #[test]
    fn group_data_crosses_by_vanilla_names() {
        let epoch = WorldEpoch::new(3);
        let ageable = Ageable::babies(true);
        let api = AgeableGroup::new(true, 0.05);
        let variants = sunrise_client::mobs::variants::VariantTables {
            wolf: sunrise_client::mobs::variants::VariantTable::from_entries(vec![
                (
                    ("minecraft".into(), "spotted".into()),
                    sunrise_client::mobs::variants::WolfVariant {
                        assets: sunrise_client::mobs::variants::WolfAssetInfo {
                            wild: "minecraft:entity/wolf/wolf_spotted".into(),
                            tame: "minecraft:entity/wolf/wolf_spotted_tame".into(),
                            angry: "minecraft:entity/wolf/wolf_spotted_angry".into(),
                        },
                        baby_assets: sunrise_client::mobs::variants::WolfAssetInfo {
                            wild: "minecraft:entity/wolf/wolf_spotted_baby".into(),
                            tame: "minecraft:entity/wolf/wolf_spotted_tame_baby".into(),
                            angry: "minecraft:entity/wolf/wolf_spotted_angry_baby".into(),
                        },
                        spawn_conditions: Vec::new(),
                    },
                ),
                (
                    ("test".into(), "mint".into()),
                    sunrise_client::mobs::variants::WolfVariant {
                        assets: sunrise_client::mobs::variants::WolfAssetInfo {
                            wild: "test:entity/wolf/mint".into(),
                            tame: "test:entity/wolf/mint_tame".into(),
                            angry: "test:entity/wolf/mint_angry".into(),
                        },
                        baby_assets: sunrise_client::mobs::variants::WolfAssetInfo {
                            wild: "test:entity/wolf/mint_baby".into(),
                            tame: "test:entity/wolf/mint_tame_baby".into(),
                            angry: "test:entity/wolf/mint_angry_baby".into(),
                        },
                        spawn_conditions: Vec::new(),
                    },
                ),
            ]),
            ..Default::default()
        };
        let spotted = variants.wolf.lookup("minecraft:spotted").unwrap();
        let mint = variants.wolf.lookup("test:mint").unwrap();
        let cases = [
            (
                Group::Horse(ageable, 1),
                SpawnGroup::Horse {
                    ageable: api,
                    variant: "creamy".into(),
                },
            ),
            (
                Group::Rabbit(ageable, 99),
                SpawnGroup::Rabbit {
                    ageable: api,
                    variant: "evil".into(),
                },
            ),
            (
                Group::Fox(ageable, 1),
                SpawnGroup::Fox {
                    ageable: api,
                    variant: "snow".into(),
                },
            ),
            (
                Group::Axolotl(ageable, [0, 4]),
                SpawnGroup::Axolotl {
                    ageable: api,
                    variants: ["lucy".into(), "blue".into()],
                },
            ),
            (
                Group::Wolf(ageable, spotted),
                SpawnGroup::Wolf {
                    ageable: api,
                    variant: id("minecraft:spotted"),
                },
            ),
            (
                Group::Wolf(ageable, mint),
                SpawnGroup::Wolf {
                    ageable: api,
                    variant: id("test:mint"),
                },
            ),
            (
                Group::Spider(Some("minecraft:speed")),
                SpawnGroup::Spider {
                    effect: Some(id("minecraft:speed")),
                },
            ),
            (
                Group::School(7, Some((0, 1, 0))),
                SpawnGroup::School {
                    leader: EntityRef::new(epoch, 7),
                    tropical: Some(TropicalVariant::new(
                        "kob".into(),
                        "orange".into(),
                        "white".into(),
                    )),
                },
            ),
            (
                Group::Zombie {
                    baby: true,
                    jockey: false,
                    husk: Some(true),
                },
                SpawnGroup::Zombie {
                    baby: true,
                    jockey: false,
                    tried_camel: Some(true),
                },
            ),
        ];
        for (game, api) in cases {
            assert_eq!(
                native_spawn_group(&game, epoch, &variants).as_ref(),
                Some(&api)
            );
            assert_eq!(spawn_group(&api, &variants), Some(game));
        }
        let unknown = SpawnGroup::Horse {
            ageable: api,
            variant: "plaid".into(),
        };
        assert_eq!(spawn_group(&unknown, &variants), None);
        let unknown_wolf = SpawnGroup::Wolf {
            ageable: api,
            variant: id("test:missing"),
        };
        assert_eq!(spawn_group(&unknown_wolf, &variants), None);
    }
}
