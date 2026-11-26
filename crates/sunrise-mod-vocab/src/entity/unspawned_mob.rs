//! A mob the game has made but not yet added to a level: its typed refusal, its scoped read-only
//! view ([`UnspawnedMobView`]) and mutable editor ([`UnspawnedMobEditor`]), and the spawn group
//! data (`SpawnGroupData`) the mobs of one pack share.

use crate::{AttributeModifier, EntityRef, EquipmentSlot, Identifier, ItemStack, Text};

/// Why a read or an edit of an unspawned mob was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: returned only by scoped native unspawned-mob views and editors.
pub enum UnspawnedMobError {
    /// The retained custom name exceeds the component boundary.
    Text(crate::text::TextFormError),
    /// This mob's type has no such attribute, or the game keeps no mob behind this loan (the
    /// ender dragon, which it stores apart from mobs, or a position check where the game creates
    /// the mob later from saved data or the world generator).
    Unsupported,
    /// The value is refused: an unknown attribute, a non-finite number, a modifier id the
    /// attribute already has, an empty or full tag list, invalid item data, an out-of-bounds
    /// list index, or an invalid pending-child specification.
    Invalid,
}

impl From<crate::text::TextFormError> for UnspawnedMobError {
    fn from(error: crate::text::TextFormError) -> Self {
        Self::Text(error)
    }
}

impl From<crate::StackWriteError> for UnspawnedMobError {
    fn from(_: crate::StackWriteError) -> Self {
        Self::Invalid
    }
}

/// Engine backing for read-only access to an unspawned mob.
// hidden: the engine owns unspawned-mob read access; listeners use [`UnspawnedMobView`].
#[doc(hidden)]
pub trait UnspawnedMobRead {
    fn kind(&self) -> Result<Identifier, UnspawnedMobError>;
    fn position(&self) -> Result<[f64; 3], UnspawnedMobError>;
    fn equipment(&self, slot: EquipmentSlot) -> Result<Option<ItemStack>, UnspawnedMobError>;
    fn attribute_base(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError>;
    fn attribute_value(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError>;
    fn tags(&self) -> Result<Vec<String>, UnspawnedMobError>;
    fn custom_name(&self) -> Result<Option<Text>, UnspawnedMobError>;
}

/// Engine backing for mutable access to an unspawned mob.
// hidden: the engine owns unspawned-mob mutable access; listeners use [`UnspawnedMobEditor`].
#[doc(hidden)]
pub trait UnspawnedMobWrite: UnspawnedMobRead {
    fn set_equipment(
        &mut self,
        slot: EquipmentSlot,
        item: Option<ItemStack>,
    ) -> Result<(), UnspawnedMobError>;
    fn set_attribute_base(
        &mut self,
        attribute: &Identifier,
        base: f64,
    ) -> Result<(), UnspawnedMobError>;
    fn add_attribute_modifier(
        &mut self,
        attribute: &Identifier,
        modifier: AttributeModifier,
    ) -> Result<(), UnspawnedMobError>;
    fn remove_attribute_modifier(
        &mut self,
        attribute: &Identifier,
        id: &Identifier,
    ) -> Result<bool, UnspawnedMobError>;
    fn add_tag(&mut self, tag: &str) -> Result<bool, UnspawnedMobError>;
    fn remove_tag(&mut self, tag: &str) -> Result<bool, UnspawnedMobError>;
    fn set_custom_name(&mut self, name: Option<Text>) -> Result<(), UnspawnedMobError>;
}

/// A read-only scoped borrow of a spawn candidate. `spawn_cluster_size` lends the mob after
/// the insertion attempt; other spawn hooks lend it before insertion.
///
/// The borrow lasts only for the hook or part call that receives it. Where the site does not
/// construct the mob ahead of time (chunk-generation packs, spawners with entity data, or the
/// ender dragon), [`Self::kind`] and [`Self::position`] still succeed and every other read
/// answers [`UnspawnedMobError::Unsupported`].
// exhaustive: only the engine constructs a view.
// no wire: a scoped read-only borrow, not a payload.
#[derive(Clone, Copy)]
pub struct UnspawnedMobView<'a> {
    inner: &'a dyn UnspawnedMobRead,
}

impl<'a> UnspawnedMobView<'a> {
    // hidden: the engine constructs a view around its scoped unspawned-mob borrow.
    #[doc(hidden)]
    pub fn new(inner: &'a dyn UnspawnedMobRead) -> Self {
        Self { inner }
    }

    /// The mob's entity type (`minecraft:zombie`).
    pub fn kind(&self) -> Result<Identifier, UnspawnedMobError> {
        self.inner.kind()
    }

    /// The candidate’s current position.
    pub fn position(&self) -> Result<[f64; 3], UnspawnedMobError> {
        self.inner.position()
    }

    /// The stack in `slot` (`LivingEntity.getItemBySlot`); `None` for an empty slot.
    pub fn equipment(&self, slot: EquipmentSlot) -> Result<Option<ItemStack>, UnspawnedMobError> {
        self.inner.equipment(slot)
    }

    /// An attribute's base value, before modifiers (`AttributeMap.getBaseValue`).
    /// `Unsupported` for an attribute the mob's type does not have.
    pub fn attribute_base(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        self.inner.attribute_base(attribute)
    }

    /// An attribute's value with its modifiers, its equipment's included
    /// (`AttributeMap.getValue`). `Unsupported` for an attribute the mob's type does not have.
    pub fn attribute_value(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        self.inner.attribute_value(attribute)
    }

    /// The tags on the mob so far (`Entity.getTags`).
    pub fn tags(&self) -> Result<Vec<String>, UnspawnedMobError> {
        self.inner.tags()
    }

    /// The mob's custom name, if it has one.
    pub fn custom_name(&self) -> Result<Option<Text>, UnspawnedMobError> {
        self.inner.custom_name()
    }
}

/// A mutable scoped loan of a mob the game has made but not yet added to a level.
///
/// Edits apply at once, so reads on the editor and later listeners see them. The loan lasts only
/// for the hook call that receives it. Where the game keeps no mob behind the loan (such as the
/// ender dragon), [`Self::kind`] and [`Self::position`] still succeed and every other read or
/// edit answers [`UnspawnedMobError::Unsupported`].
// exhaustive: only the engine constructs an editor.
// no wire: an exclusive scoped loan, not a payload.
pub struct UnspawnedMobEditor<'a> {
    inner: &'a mut dyn UnspawnedMobWrite,
}

impl<'a> UnspawnedMobEditor<'a> {
    // hidden: the engine constructs an editor around its scoped unspawned-mob loan.
    #[doc(hidden)]
    pub fn new(inner: &'a mut dyn UnspawnedMobWrite) -> Self {
        Self { inner }
    }

    /// Borrow this editor as a read-only [`UnspawnedMobView`].
    pub fn as_view(&self) -> UnspawnedMobView<'_> {
        UnspawnedMobView::new(&*self.inner)
    }

    /// The mob's entity type (`minecraft:zombie`).
    pub fn kind(&self) -> Result<Identifier, UnspawnedMobError> {
        self.inner.kind()
    }

    /// Where the mob will join the level.
    pub fn position(&self) -> Result<[f64; 3], UnspawnedMobError> {
        self.inner.position()
    }

    /// The stack in `slot` (`LivingEntity.getItemBySlot`); `None` for an empty slot.
    pub fn equipment(&self, slot: EquipmentSlot) -> Result<Option<ItemStack>, UnspawnedMobError> {
        self.inner.equipment(slot)
    }

    /// An attribute's base value, before modifiers (`AttributeMap.getBaseValue`).
    /// `Unsupported` for an attribute the mob's type does not have.
    pub fn attribute_base(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        self.inner.attribute_base(attribute)
    }

    /// An attribute's value with its modifiers, its equipment's included
    /// (`AttributeMap.getValue`). `Unsupported` for an attribute the mob's type does not have.
    pub fn attribute_value(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        self.inner.attribute_value(attribute)
    }

    /// The tags on the mob so far (`Entity.getTags`).
    pub fn tags(&self) -> Result<Vec<String>, UnspawnedMobError> {
        self.inner.tags()
    }

    /// The mob's custom name, if it has one.
    pub fn custom_name(&self) -> Result<Option<Text>, UnspawnedMobError> {
        self.inner.custom_name()
    }

    /// Puts `item` in `slot` (`Mob.setItemSlot`): an API-owned stack; `None` empties the slot.
    /// Invalid item keys, counts, damage or components are refused with `Invalid`, without
    /// changing the slot. The stack's attribute modifiers apply to the mob at once.
    pub fn set_equipment(
        &mut self,
        slot: EquipmentSlot,
        item: Option<ItemStack>,
    ) -> Result<(), UnspawnedMobError> {
        self.inner.set_equipment(slot, item)
    }

    /// Sets an attribute's base value (`AttributeInstance.setBaseValue`), kept within the
    /// attribute's range.
    pub fn set_attribute_base(
        &mut self,
        attribute: &Identifier,
        base: f64,
    ) -> Result<(), UnspawnedMobError> {
        self.inner.set_attribute_base(attribute, base)
    }

    /// Adds a permanent modifier (`AttributeInstance.addPermanentModifier`), saved with the mob.
    pub fn add_attribute_modifier(
        &mut self,
        attribute: &Identifier,
        modifier: AttributeModifier,
    ) -> Result<(), UnspawnedMobError> {
        self.inner.add_attribute_modifier(attribute, modifier)
    }

    /// Removes the modifier `id` from the attribute; `false` when it had none.
    pub fn remove_attribute_modifier(
        &mut self,
        attribute: &Identifier,
        id: &Identifier,
    ) -> Result<bool, UnspawnedMobError> {
        self.inner.remove_attribute_modifier(attribute, id)
    }

    /// Adds a tag (`Entity.addTag`); `false` when the mob has it already.
    pub fn add_tag(&mut self, tag: &str) -> Result<bool, UnspawnedMobError> {
        self.inner.add_tag(tag)
    }

    /// Removes a tag (`Entity.removeTag`); `false` when the mob did not have it.
    pub fn remove_tag(&mut self, tag: &str) -> Result<bool, UnspawnedMobError> {
        self.inner.remove_tag(tag)
    }

    /// Names the mob (`Entity.setCustomName`), or takes its name away with `None`.
    /// A component deeper than 512 nodes is refused with `UnspawnedMobError::Text`
    /// before changing the mob's name.
    pub fn set_custom_name(&mut self, name: Option<Text>) -> Result<(), UnspawnedMobError> {
        self.inner.set_custom_name(name)
    }
}

/// `AgeableMob.AgeableMobGroupData`: whether the pack's later members may be babies.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct AgeableGroup {
    /// How many mobs of the pack have been finalized with this group so far.
    pub size: i32,
    /// Whether a later member may spawn as a baby.
    pub should_spawn_baby: bool,
    /// The chance a later member is a baby.
    pub baby_chance: f32,
}

impl AgeableGroup {
    /// A group no mob has used yet.
    pub fn new(should_spawn_baby: bool, baby_chance: f32) -> Self {
        Self {
            size: 0,
            should_spawn_baby,
            baby_chance,
        }
    }
}

/// A tropical fish's look, as a school shares it (`TropicalFish.TropicalFishGroupData`), by the
/// names vanilla saves (`kob`, `orange`, `white`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TropicalVariant {
    /// The pattern (`TropicalFish.Pattern`).
    pub pattern: String,
    /// The body's dye colour.
    pub base_color: String,
    /// The pattern's dye colour.
    pub pattern_color: String,
}

impl TropicalVariant {
    /// A look of `pattern`, `base_color` and `pattern_color`.
    pub fn new(pattern: String, base_color: String, pattern_color: String) -> Self {
        Self {
            pattern,
            base_color,
            pattern_color,
        }
    }
}

/// `SpawnGroupData`: what the mobs of one pack share, handed from one member's finalization to
/// the next. Variants are named as vanilla saves them (`creamy`, `snow`, `lucy`); a name the
/// game does not know drops the group.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SpawnGroup {
    /// `AgeableMob.AgeableMobGroupData`, for the ageable mobs with no group of their own.
    Ageable(AgeableGroup),
    /// `Zombie.ZombieGroupData`, and `Husk.HuskGroupData`'s camel try.
    Zombie {
        /// Whether the zombie is a baby.
        baby: bool,
        /// Whether a baby may ride a chicken.
        jockey: bool,
        /// For a husk, whether it already tried to ride a camel husk; `None` for other zombies.
        tried_camel: Option<bool>,
    },
    /// `Spider.SpiderEffectsGroupData`: the effect a spider spawns with, if any.
    Spider {
        /// The effect (`minecraft:speed`).
        effect: Option<Identifier>,
    },
    /// `Horse.HorseGroupData`: the coat colour.
    Horse {
        /// The ageable part.
        ageable: AgeableGroup,
        /// `Horse.Variant`'s name (`creamy`).
        variant: String,
    },
    /// `Llama.LlamaGroupData`.
    Llama {
        /// The ageable part.
        ageable: AgeableGroup,
        /// `Llama.Variant`'s name (`creamy`).
        variant: String,
    },
    /// `Fox.FoxGroupData`.
    Fox {
        /// The ageable part.
        ageable: AgeableGroup,
        /// `Fox.Variant`'s name (`red`, `snow`).
        variant: String,
    },
    /// `Rabbit.RabbitGroupData`.
    Rabbit {
        /// The ageable part.
        ageable: AgeableGroup,
        /// `Rabbit.Variant`'s name (`brown`, `evil`).
        variant: String,
    },
    /// `Axolotl.AxolotlGroupData`: the two variants a member picks between.
    Axolotl {
        /// The ageable part.
        ageable: AgeableGroup,
        /// `Axolotl.Variant`'s names (`lucy`).
        variants: [String; 2],
    },
    /// `Wolf.WolfPackData`.
    Wolf {
        /// The ageable part.
        ageable: AgeableGroup,
        /// An entry of the world's `minecraft:wolf_variant` registry.
        variant: Identifier,
    },
    /// `AbstractSchoolingFish.SchoolSpawnGroupData`: the school's leader, and for tropical
    /// fish the look the school shares.
    School {
        /// The leader.
        leader: EntityRef,
        /// The tropical fish look; `None` for other fish.
        tropical: Option<TropicalVariant>,
    },
}
