//! Dispenser use of buckets: `BucketPickup.pickupBlock` for an empty bucket and
//! `DispensibleContainerItem.emptyContents` for filled ones (`BucketItem`,
//! `MobBucketItem`, `SolidBucketItem`), both without a player. Each is decided first
//! ([`FluidWrite`]) and done after, at once for a bucket, or later for a mod's container, whose
//! transaction keeps the block until its commit.
use super::{
    Behaviours, LevelAccess, Sim, Ticks,
    classes::{Class, Interface},
};
use crate::{
    fluid::{FluidKind, FluidState},
    registry::BlockStateId,
};
use std::sync::Arc;
use sunrise_core::{position::BlockPos, random::RandomSource};

/// What a filled bucket empties into the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BucketContent {
    /// Water or lava (mob buckets hold water).
    Fluid(FluidKind),
    /// `SolidBucketItem`: the block placed into an empty space (powder snow).
    Powder,
    /// `MobBucketItem` holding water: its empty sound posts no `FLUID_PLACE`.
    MobWater,
    /// `MobBucketItem` without a fluid (the sulfur cube bucket): only the sound.
    Nothing,
}

impl BucketContent {
    /// `BucketItem.content == Fluids.WATER` (`MobBucketItem` holding water
    /// counts too): the only content a vanilla bucket pours inside a liquid
    /// container, and the only one `canBeReplaced(Fluid)` sees as water.
    pub fn is_water(self) -> bool {
        matches!(self, Self::Fluid(FluidKind::Water) | Self::MobWater)
    }
    pub fn fluid(self) -> Option<FluidKind> {
        match self {
            Self::Fluid(kind) => Some(kind),
            Self::MobWater => Some(FluidKind::Water),
            Self::Powder | Self::Nothing => None,
        }
    }
}

/// The liquid-container query's living user. Only buckets operated by a
/// player lend an entity; dispensers and fluid flow use the default.
#[derive(Clone, Copy, Debug, Default)]
pub struct LiquidUser {
    pub entity_id: Option<u32>,
    pub creative: bool,
}

/// What `BucketPickup.pickupBlock` does to a block, decided against the level as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drain {
    /// The block it leaves, written with `flags`.
    pub to: BlockStateId,
    pub flags: i32,
    /// A waterlogged block drained (`SimpleWaterloggedBlock`), which breaks after if it can't
    /// survive.
    pub drained: bool,
    /// That drained block can't survive as the level is: it is destroyed with its drops.
    pub breaks: bool,
    /// Powder snow, which breaks with its particles (level event 2001).
    pub burst: bool,
}

/// How `placeLiquid` puts a lit candle or campfire out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Douse {
    /// `CampfireBlock.douse`: `entity.generic.extinguish_fire`.
    Campfire,
    /// `AbstractCandleBlock.extinguish`: `block.candle.extinguish`.
    Candle,
}

/// What `LiquidBlockContainer.placeLiquid` does, decided against the level as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacedLiquid {
    /// The waterlogged state, put out where it was lit.
    pub to: BlockStateId,
    /// The fluid ticked there, `delay` ticks on.
    pub fluid: FluidKind,
    pub delay: i32,
    pub douse: Option<Douse>,
    /// A dried ghast plays its own place-in-water sound.
    pub dried_ghast: bool,
}

/// What a world pickup or placement of fluid changes ([`Behaviours::plan_fluid_pickup`],
/// [`Behaviours::plan_bucket_pour`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FluidChange {
    /// `BucketPickup.pickupBlock`.
    Drain(Drain),
    /// `BucketItem.emptyContents` into a block that holds no liquid: `replaced`, when there is
    /// one, is destroyed with its drops first; then the fluid's source is set, unless it is
    /// there already (`set` false).
    Pour {
        replaced: Option<BlockStateId>,
        source: BlockStateId,
        set: bool,
    },
    /// `emptyContents` of water into a `LiquidBlockContainer`: `placeLiquid`, or nothing but
    /// the bucket's sound where the container refuses it (None).
    Fill(Option<PlacedLiquid>),
    /// `WATER_EVAPORATES`: the water hisses away at `pitch`, and the block stays.
    Hiss { pitch: f32 },
}

/// A world pickup or placement of fluid at `pos`, decided against the level as it is: what
/// moves (`fluid`, a source), the block there then (`from`) and what changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FluidWrite {
    pub pos: BlockPos,
    pub from: BlockStateId,
    pub fluid: FluidKind,
    pub change: FluidChange,
}

impl FluidWrite {
    /// The source state a bucket passes to `LiquidBlockContainer.placeLiquid`.
    pub fn source_fluid_state(&self) -> FluidState {
        FluidState::new(self.fluid, 8, false)
    }
    /// The block the level holds once it is done: `from` where it doesn't change.
    pub fn to(&self, b: &Behaviours) -> BlockStateId {
        match self.change {
            FluidChange::Drain(drain) if drain.breaks => b.legacy_block(b.fluid(drain.to)),
            FluidChange::Drain(drain) => drain.to,
            FluidChange::Pour { source, .. } => source,
            FluidChange::Fill(Some(placed)) => placed.to,
            FluidChange::Fill(None) | FluidChange::Hiss { .. } => self.from,
        }
    }
    /// The block broken with its drops, as it is when it breaks: the one the fluid is poured
    /// over, or a drained one that can't survive.
    pub fn broken(&self) -> Option<BlockStateId> {
        match self.change {
            FluidChange::Drain(drain) if drain.breaks => Some(drain.to),
            FluidChange::Pour { replaced, .. } => replaced,
            _ => None,
        }
    }
    /// Whether the bucket's sound and fluid game event go out: not for the hiss, which is
    /// its own sound.
    pub fn sounds(&self) -> bool {
        !matches!(self.change, FluidChange::Hiss { .. })
    }
}

/// What `emptyContents` decides at a block.
enum Pour {
    /// The block takes no liquid this way: a bucket with a hit result tries the block beside.
    Refused,
    /// Nothing can be set there.
    Failed,
    Planned(FluidWrite),
}

/// `Fluid.getBucket`: the filled bucket that holds `fluid`, flowing or not. None for the
/// empty fluid, whose bucket is `Items.AIR`.
pub fn fluid_bucket(fluid: FluidKind) -> Option<&'static str> {
    match fluid {
        FluidKind::Water | FluidKind::FlowingWater => Some("minecraft:water_bucket"),
        FluidKind::Lava | FluidKind::FlowingLava => Some("minecraft:lava_bucket"),
        FluidKind::Mod(fluid) | FluidKind::ModFlowing(fluid) => fluid.get().bucket.as_deref(),
        FluidKind::Empty => None,
    }
}

/// `Fluid.getPickupSound`: the sound of a bucket filling with `fluid`, flowing or not. None
/// for the empty fluid, which has none.
pub fn fluid_pickup_sound(fluid: FluidKind) -> Option<&'static str> {
    match fluid {
        FluidKind::Water
        | FluidKind::FlowingWater
        | FluidKind::Mod(_)
        | FluidKind::ModFlowing(_) => Some("minecraft:item.bucket.fill"),
        FluidKind::Lava | FluidKind::FlowingLava => Some("minecraft:item.bucket.fill_lava"),
        FluidKind::Empty => None,
    }
}

/// `BucketItem.playEmptySound` for a bucket of `fluid`, flowing or not: lava's own sound, and
/// the plain one for water and a mod's world fluid. None for the empty fluid, which no bucket
/// holds.
pub fn fluid_empty_sound(fluid: FluidKind) -> Option<&'static str> {
    match fluid {
        FluidKind::Water
        | FluidKind::FlowingWater
        | FluidKind::Mod(_)
        | FluidKind::ModFlowing(_) => Some("minecraft:item.bucket.empty"),
        FluidKind::Lava | FluidKind::FlowingLava => Some("minecraft:item.bucket.empty_lava"),
        FluidKind::Empty => None,
    }
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    /// `BucketPickup.pickupBlock(user, level, pos, state)`: the filled bucket
    /// that results, None when the block gives nothing. `creative` is whether
    /// `user` is a creative player (only barriers care).
    pub(crate) fn bucket_pickup(&mut self, pos: BlockPos, creative: bool) -> Option<&'static str> {
        let b = self.b;
        let state = self.state(pos);
        let item = if b.instance_of(state, Class::Liquid) {
            fluid_bucket(b.fluid(state).kind())?
        } else if b.implements(state, Interface::SimpleWaterlogged)
            && b.table.has(state, b.props.waterlogged)
        {
            "minecraft:water_bucket"
        } else if b.is_block(state, b.named.powder_snow) {
            "minecraft:powder_snow_bucket"
        } else if b.is_block(state, b.named.bubble_column) {
            "minecraft:water_bucket"
        } else {
            return None;
        };
        self.pickup_block(pos, state, creative).then_some(item)
    }
    /// `BucketItem.emptyContents(user, level, pos, hitResult)`: where the
    /// contents went, None when nothing was emptied. `beside` is
    /// `hitResult.getBlockPos().relative(hitResult.getDirection())` (None for
    /// a dispenser, which has no hit result). `sound` is the bucket's empty
    /// sound (mob buckets have their own); `sneaking` is
    /// `user.isShiftKeyDown()`, which only counts with a hit result;
    /// `creative` is whether that user is a creative player.
    pub(crate) fn bucket_empty(
        &mut self,
        pos: BlockPos,
        beside: Option<BlockPos>,
        content: BucketContent,
        sound: &str,
        sneaking: bool,
        user: LiquidUser,
    ) -> Option<BlockPos> {
        let b = self.b;
        let fluid = match content {
            BucketContent::Powder => {
                if self.is_in_valid_bounds(pos) && b.is_air(self.state(pos)) {
                    let snow = b.table.default_state(b.named.powder_snow);
                    self.set_block_and_update(pos, snow);
                    // SolidBucketItem.emptyContents: one FLUID_PLACE, then the place sound.
                    self.game_event(pos, "minecraft:fluid_place");
                    let sound: Arc<str> = sound.into();
                    self.sound(pos, &sound, 1.0, 1.0);
                    return Some(pos);
                }
                return None;
            }
            BucketContent::Nothing => {
                let sound: Arc<str> = sound.into();
                self.sound(pos, &sound, 1.0, 1.0);
                return Some(pos);
            }
            BucketContent::MobWater => FluidKind::Water,
            BucketContent::Fluid(fluid) => fluid,
        };
        let mut write = self.plan_bucket_pour(pos, beside, fluid, sneaking, user)?;
        if matches!(write.change, FluidChange::Fill(_))
            && self
                .level
                .place_liquid(
                    self.ticks,
                    write.pos,
                    write.from,
                    write.source_fluid_state(),
                )
                .is_some()
        {
            // BucketItem ignores the container's boolean; the fluid utility does not.
            write.change = FluidChange::Fill(None);
        }
        self.apply_bucket_pour(write, sound, content == BucketContent::MobWater)
    }
    fn plan_bucket_pour(
        &mut self,
        pos: BlockPos,
        beside: Option<BlockPos>,
        fluid: FluidKind,
        sneaking: bool,
        user: LiquidUser,
    ) -> Option<FluidWrite> {
        let plan = match self.plan_pour(pos, fluid, sneaking && beside.is_some(), user) {
            // A refused click tries the block beside the hit, without a hit result.
            Pour::Refused => self.plan_pour(beside?, fluid, false, user),
            plan => plan,
        };
        match plan {
            Pour::Planned(write) => Some(write),
            Pour::Refused | Pour::Failed => None,
        }
    }
    fn apply_bucket_pour(&mut self, write: FluidWrite, sound: &str, mob: bool) -> Option<BlockPos> {
        let pos = write.pos;
        match write.change {
            FluidChange::Hiss { pitch } => {
                self.hiss(pos, pitch);
                return Some(pos);
            }
            FluidChange::Fill(placed) => {
                if let Some(placed) = placed {
                    self.apply_placed_liquid(pos, placed);
                }
            }
            FluidChange::Pour {
                replaced,
                source,
                set,
            } => {
                // `BlockState.liquid()`: water, lava and the bubble column. A bubble column is
                // not a `LiquidBlock`, and breaking it first spills break particles and a
                // `block_destroy` a sculk sensor hears.
                if replaced.is_some() {
                    self.destroy_block(pos, true, super::flags::UPDATE_LIMIT);
                }
                if set
                    && !self.set_block(pos, source, super::flags::UPDATE_ALL_IMMEDIATE)
                    && !self.fluid_at(pos).is_source()
                {
                    return None;
                }
            }
            FluidChange::Drain(_) => unreachable!("a pour plans no drain"),
        }
        self.empty_sound(pos, sound, mob);
        Some(pos)
    }
    /// What `BucketItem.emptyContents` of `fluid` at `pos` does, decided against the level as
    /// it is. `hit` is a sneaking player's click with a hit result, which a block that merely
    /// holds the fluid refuses.
    fn plan_pour(&mut self, pos: BlockPos, fluid: FluidKind, hit: bool, user: LiquidUser) -> Pour {
        let b = self.b;
        let state = self.state(pos);
        // canBeReplaced(fluid): replaceable or not solid (portals refuse).
        let may_replace = !b.instance_of(state, Class::EndPortal)
            && !b.instance_of(state, Class::EndGateway)
            && (b.is_replaceable(state) || !b.is_solid(state));
        let liquid_container = self.is_liquid_container(state);
        let answer = self.level.can_place_liquid(pos, state, fluid, user);
        let container = answer.unwrap_or_else(|| {
            liquid_container && self.can_place_liquid(state, fluid, user.creative)
        });
        // `canPlaceFluidInsideBlock`: air always takes the fluid.
        if !(b.is_air(state) || (may_replace || container) && !hit) {
            return Pour::Refused;
        }
        let planned = |change| {
            Pour::Planned(FluidWrite {
                pos,
                from: state,
                fluid: super::fluids::source_of(fluid),
                change,
            })
        };
        let water = matches!(fluid, FluidKind::Water | FluidKind::FlowingWater);
        if water && self.level.dimension_type().water_evaporates {
            // WATER_EVAPORATES: the water hisses away.
            let pitch = 2.6 + (self.random().next_float() - self.random().next_float()) * 0.8;
            return planned(FluidChange::Hiss { pitch });
        }
        if answer.unwrap_or(liquid_container && water) {
            let source = FluidState::new(super::fluids::source_of(fluid), 8, false);
            let placed = self.plan_place_liquid(pos, state, source, user.creative);
            return planned(FluidChange::Fill(placed));
        }
        if !self.is_in_valid_bounds(pos) || !self.level.is_loaded(pos) {
            return Pour::Failed;
        }
        let replaced = (may_replace && !b.liquid(state) && !b.is_air(state)).then_some(state);
        // What breaking it leaves: its own fluid.
        let left = replaced.map_or(state, |state| b.legacy_block(b.fluid(state)));
        let source = FluidState::new(super::fluids::source_of(fluid), 8, false);
        let source: BlockStateId = b.legacy_block(source);
        planned(FluidChange::Pour {
            replaced,
            source,
            set: left != source,
        })
    }
    /// `WATER_EVAPORATES`' hiss.
    fn hiss(&mut self, pos: BlockPos, pitch: f32) {
        self.emit(super::LevelEvent::Sound {
            pos,
            sound: "minecraft:block.fire.extinguish".into(),
            source: None,
            volume: 0.5,
            pitch,
        });
    }
    /// What a [`FluidWrite`] done later does once its block is set, before its drops: the
    /// fluid tick `placeLiquid` schedules.
    fn fluid_write_ticks(&mut self, write: &FluidWrite) {
        if let FluidChange::Fill(Some(placed)) = write.change {
            self.schedule_fluid_tick(write.pos, placed.fluid, placed.delay);
        }
    }
    /// What a [`FluidWrite`] done later does after its drops: the broken block's particles and
    /// `block_destroy`, a doused block's sound and `block_change`, a dried ghast's sound, or
    /// the hiss.
    fn fluid_write_effects(&mut self, write: &FluidWrite) {
        let pos = write.pos;
        if let Some(broken) = write.broken() {
            self.destroy_particles(pos, broken);
            self.game_event_state(pos, "minecraft:block_destroy", broken);
        }
        match write.change {
            FluidChange::Fill(Some(placed)) => {
                self.douse(pos, placed.douse);
                self.placed_in_water(pos, placed);
            }
            FluidChange::Hiss { pitch } => self.hiss(pos, pitch),
            _ => {}
        }
    }
    /// `playEmptySound`: `MobBucketItem` overrides it without the game event.
    fn empty_sound(&mut self, pos: BlockPos, sound: &str, mob: bool) {
        let sound: Arc<str> = sound.into();
        self.sound(pos, &sound, 1.0, 1.0);
        if !mob {
            self.game_event(pos, "minecraft:fluid_place");
        }
    }
}

impl Behaviours {
    /// `BucketPickup.pickupBlock(user, level, pos, state)`: the filled bucket
    /// item. A dispenser has no user (`false`).
    pub fn bucket_pickup<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        creative: bool,
    ) -> Option<&'static str> {
        self.sim(level, ticks).bucket_pickup(pos, creative)
    }
    /// `DispensibleContainerItem.emptyContents(user, level, pos, hitResult)`:
    /// where the contents went. `beside` is the block beside the clicked
    /// face, tried when `pos` refuses (None without a hit result).
    pub fn bucket_empty<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        beside: Option<BlockPos>,
        content: BucketContent,
        sound: &str,
        sneaking: bool,
        creative: bool,
    ) -> Option<BlockPos> {
        self.bucket_empty_as(
            level,
            ticks,
            pos,
            beside,
            content,
            sound,
            sneaking,
            LiquidUser {
                entity_id: None,
                creative,
            },
        )
    }
    /// Empty a bucket while lending its living user to the read-only query.
    pub fn bucket_empty_as<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        beside: Option<BlockPos>,
        content: BucketContent,
        sound: &str,
        sneaking: bool,
        user: LiquidUser,
    ) -> Option<BlockPos> {
        self.sim(level, ticks)
            .bucket_empty(pos, beside, content, sound, sneaking, user)
    }
    /// `BucketItem.use` chooses the clicked container only when it can take the fluid.
    pub fn bucket_can_fill<L: LevelAccess + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
        state: BlockStateId,
        fluid: FluidKind,
        user: LiquidUser,
    ) -> bool {
        level
            .can_place_liquid(pos, state, fluid, user)
            .unwrap_or_else(|| {
                fluid == FluidKind::Water
                    && self.is_liquid_container(state)
                    && self.can_place_liquid(state, fluid, user.creative)
            })
    }
    /// Decide a pour without handing a write host to the block part yet. The
    /// caller chooses the direct or journalled host, then applies the remainder.
    pub fn plan_bucket_pour<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        beside: Option<BlockPos>,
        fluid: FluidKind,
        sneaking: bool,
        user: LiquidUser,
    ) -> Option<FluidWrite> {
        self.sim(level, ticks)
            .plan_bucket_pour(pos, beside, fluid, sneaking, user)
    }
    /// Apply a bucket plan after an optional container override. A caller that
    /// ran the override replaces the template write with `FluidChange::Fill(None)`.
    pub fn apply_bucket_pour<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        write: FluidWrite,
        sound: &str,
        mob: bool,
    ) -> Option<BlockPos> {
        self.sim(level, ticks).apply_bucket_pour(write, sound, mob)
    }
    /// `BucketPickup.pickupBlock` for a mod's fluid container (NeoForge's
    /// `FluidUtil.tryPickupFluid`), decided against the level as it is and not done: the source
    /// it takes and what it changes. None where the block gives no fluid, powder snow
    /// included. `creative` is whether the player is a creative one (only barriers care).
    pub fn plan_fluid_pickup<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        creative: bool,
    ) -> Option<FluidWrite> {
        let sim = self.sim(level, ticks);
        let from = sim.state(pos);
        let drain = sim.plan_pickup_block(pos, from, creative)?;
        if drain.burst {
            return None;
        }
        let fluid = match self.instance_of(from, Class::Liquid) {
            true => super::fluids::source_of(self.fluid(from).kind()),
            // A waterlogged block and a bubble column give water.
            false => FluidKind::Water,
        };
        Some(FluidWrite {
            pos,
            from,
            fluid,
            change: FluidChange::Drain(drain),
        })
    }
    /// What `write`, decided earlier, does once its block is set and before its drops: the fluid
    /// tick `placeLiquid` schedules.
    pub fn fluid_write_ticks<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        write: &FluidWrite,
    ) {
        self.sim(level, ticks).fluid_write_ticks(write);
    }
    /// What `write`, decided earlier, does after its drops: the broken block's particles and
    /// `block_destroy`, a doused block's sound and `block_change`, a dried ghast's sound, or
    /// the hiss. The bucket's own sound and game event are the caller's.
    pub fn fluid_write_effects<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        write: &FluidWrite,
    ) {
        self.sim(level, ticks).fluid_write_effects(write);
    }
}
