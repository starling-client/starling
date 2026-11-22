//! Registered block landing/bounce reactions and their ordered world effects.
use crate::{
    block_entity_push::{BlockPushError, BlockPushWorld, push_entities_up},
    compression::invalid,
    movement_properties::MovementProperties,
    registry::BlockStateId,
};
use std::{collections::BTreeMap, fmt, io};
use sunrise_core::position::BlockPos;
#[derive(Clone, Debug)]
pub enum FallDefinition {
    Default,
    Hay,
    Honey { volume: f32, pitch: f32 },
    Slime,
    PowderSnow,
    PointedDripstone,
    Farmland { base_block: String },
    TurtleEgg,
}
#[derive(Clone, Debug)]
pub struct LandingDefinition {
    pub fall: FallDefinition,
    pub shelf_mushroom_bounce: bool,
}
#[derive(Clone, Copy, Debug)]
enum Rule {
    Default,
    Hay,
    Honey { volume: f32, pitch: f32 },
    Slime,
    PowderSnow,
    DripstoneTip,
    Farmland { base: BlockStateId },
    TurtleEgg { remaining: Option<BlockStateId> },
}
#[derive(Clone, Copy, Debug)]
struct State {
    rule: Rule,
    reduction: f32,
    shelf_mushroom_bounce: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandingEntityKind {
    Other,
    Living,
    Player,
    Zombie,
    Turtle,
    Bat,
    Item,
    PrimedTnt,
}
impl LandingEntityKind {
    fn living(self) -> bool {
        matches!(
            self,
            Self::Living | Self::Player | Self::Zombie | Self::Turtle | Self::Bat
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandingSound {
    HoneySlide,
    HoneyFall,
    TurtleEggBreak,
    ShelfMushroomBounce,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandingDamage {
    Fall,
    Stalagmite,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageSourceOwner {
    Entity,
    Level,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandingEvent {
    BlockChange,
    BlockDestroy,
}

/// The remaining work after a registered block's landing pre-effects.
#[derive(Debug)]
pub enum BlockFall {
    Done,
    Trample(FarmlandTrample),
    Damage(FallDamageCall),
}

/// The selected `causeFallDamage` call, including HoneyBlock's sound after it returns.
#[derive(Clone, Copy, Debug)]
pub struct FallDamageCall {
    owner: DamageSourceOwner,
    kind: LandingDamage,
    distance: f64,
    multiplier: f32,
    honey_fall_sound: Option<(f32, f32)>,
}

impl FallDamageCall {
    fn new(owner: DamageSourceOwner, kind: LandingDamage, distance: f64, multiplier: f32) -> Self {
        Self {
            owner,
            kind,
            distance,
            multiplier,
            honey_fall_sound: None,
        }
    }

    /// Resolve the operands for a body that can suspend inside `causeFallDamage`.
    pub fn resolve<W: BlockLandingWorld>(
        self,
        world: &mut W,
    ) -> Result<(f64, f32, W::DamageSource), LandingError<W::Error>> {
        let source = world
            .damage_source(self.owner, self.kind)
            .map_err(LandingError::World)?;
        Ok((self.distance, self.multiplier, source))
    }

    /// Resolve the selected source and apply `causeFallDamage`, but not the block's tail.
    pub fn apply<W: BlockLandingWorld>(
        self,
        world: &mut W,
    ) -> Result<bool, LandingError<W::Error>> {
        let (distance, multiplier, source) = self.resolve(world)?;
        world
            .cause_fall_damage(distance, multiplier, source)
            .map_err(LandingError::World)
    }

    /// The block work that follows the selected hurt, without its damage operands.
    pub fn tail(self) -> FallDamageTail {
        FallDamageTail(self.honey_fall_sound)
    }

    fn finish<W: BlockLandingWorld>(self, world: &mut W) -> Result<(), LandingError<W::Error>> {
        let damaged = self.apply(world)?;
        self.tail().after_hurt(world, damaged)
    }
}

/// The registered block's reaction after the selected hurt returns.
#[derive(Clone, Copy, Debug, Default)]
pub struct FallDamageTail(Option<(f32, f32)>);

impl FallDamageTail {
    /// Finish the block's reaction once the owning hurt has returned.
    pub fn after_hurt<W: BlockLandingWorld>(
        self,
        world: &mut W,
        damaged: bool,
    ) -> Result<(), LandingError<W::Error>> {
        if damaged && let Some((volume, pitch)) = self.0 {
            let sound = world.known_sound(LandingSound::HoneyFall);
            world
                .entity_sound(sound, volume, pitch)
                .map_err(LandingError::World)?;
        }
        Ok(())
    }
}

/// A successful trample check, before either the lift or `Block.fallOn`.
#[derive(Clone, Copy, Debug)]
pub struct FarmlandTrample {
    pub position: BlockPos,
    pub state: BlockStateId,
    pub base: BlockStateId,
    pub distance: f64,
    damage: FallDamageCall,
}

impl FarmlandTrample {
    /// The superclass call selected from the original farmland, before the write.
    pub fn damage_call(self) -> FallDamageCall {
        self.damage
    }

    /// `super.fallOn` always follows the trample decision, even on refusal or cancellation.
    pub fn finish_fall<W: BlockLandingWorld>(
        self,
        world: &mut W,
    ) -> Result<(), LandingError<W::Error>> {
        self.damage.finish(world)
    }
}

pub trait BlockLandingWorld: BlockPushWorld {
    type DamageSource: Copy;
    type Sound: Copy;
    fn is_server_level(&self) -> bool;
    fn is_client_side(&mut self) -> Result<bool, Self::Error>;
    fn random_float(&mut self) -> Result<f32, Self::Error>;
    fn random_int(&mut self, bound: i32) -> Result<i32, Self::Error>;
    fn under_spawn_protection(&mut self, position: BlockPos) -> Result<bool, Self::Error>;
    fn mob_griefing(&mut self) -> Result<bool, Self::Error>;
    fn entity_dimensions(&self) -> (f32, f32);
    fn suppressing_bounce(&mut self) -> Result<bool, Self::Error>;
    fn damage_source(
        &mut self,
        owner: DamageSourceOwner,
        kind: LandingDamage,
    ) -> Result<Self::DamageSource, Self::Error>;
    fn cause_fall_damage(
        &mut self,
        distance: f64,
        multiplier: f32,
        source: Self::DamageSource,
    ) -> Result<bool, Self::Error>;
    fn known_sound(&self, sound: LandingSound) -> Self::Sound;
    fn living_fall_sound(&mut self, big: bool) -> Result<Self::Sound, Self::Error>;
    fn entity_sound(
        &mut self,
        sound: Self::Sound,
        volume: f32,
        pitch: f32,
    ) -> Result<(), Self::Error>;
    /// Sound source BLOCKS, with no excluded/source entity.
    fn block_sound(
        &mut self,
        position: BlockPos,
        sound: Self::Sound,
        volume: f32,
        pitch: f32,
    ) -> Result<(), Self::Error>;
    fn broadcast_entity_event(&mut self, event: u8) -> Result<(), Self::Error>;
    fn destroy_block(&mut self, position: BlockPos, drops: bool) -> Result<bool, Self::Error>;
    fn set_block(
        &mut self,
        position: BlockPos,
        state: BlockStateId,
        flags: u32,
    ) -> Result<bool, Self::Error>;
    fn block_event(
        &mut self,
        event: LandingEvent,
        position: BlockPos,
        state: BlockStateId,
        with_entity: bool,
    ) -> Result<(), Self::Error>;
    fn level_event(&mut self, event: i32, position: BlockPos, data: i32)
    -> Result<(), Self::Error>;
}
#[derive(Debug)]
pub enum LandingError<E> {
    Data(io::Error),
    World(E),
    Push(BlockPushError<E>),
}
impl<E: fmt::Display> fmt::Display for LandingError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(e) => e.fmt(f),
            Self::World(e) => e.fmt(f),
            Self::Push(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for LandingError<E> {}
pub struct BlockLandingRegistry {
    states: Vec<State>,
}
impl BlockLandingRegistry {
    pub fn new(
        properties: &MovementProperties<'_>,
        definitions: &BTreeMap<String, LandingDefinition>,
    ) -> io::Result<Self> {
        let registry = properties.registry();
        if definitions
            .keys()
            .any(|name| registry.default_state(name).is_none())
        {
            return Err(invalid("unknown landing definition"));
        }
        let mut states = Vec::with_capacity(registry.len());
        for index in 0..registry.len() {
            let id = BlockStateId(index as u32);
            let state = registry.state(id).unwrap();
            let definition = definitions
                .get(&state.name)
                .ok_or_else(|| invalid("missing landing definition"))?;
            let rule = match &definition.fall {
                FallDefinition::Default => Rule::Default,
                FallDefinition::Hay => Rule::Hay,
                FallDefinition::Honey { volume, pitch } => Rule::Honey {
                    volume: *volume,
                    pitch: *pitch,
                },
                FallDefinition::Slime => Rule::Slime,
                FallDefinition::PowderSnow => Rule::PowderSnow,
                FallDefinition::PointedDripstone => {
                    let direction = state
                        .properties
                        .get("vertical_direction")
                        .map(String::as_str);
                    let thickness = state.properties.get("thickness").map(String::as_str);
                    if !matches!(direction, Some("up" | "down"))
                        || !matches!(
                            thickness,
                            Some("tip" | "tip_merge" | "frustum" | "middle" | "base")
                        )
                    {
                        return Err(invalid("invalid landing dripstone state"));
                    }
                    if direction == Some("up") && thickness == Some("tip") {
                        Rule::DripstoneTip
                    } else {
                        Rule::Default
                    }
                }
                FallDefinition::Farmland { base_block } => Rule::Farmland {
                    base: registry
                        .default_state(base_block)
                        .ok_or_else(|| invalid("unknown farmland base block"))?,
                },
                FallDefinition::TurtleEgg => {
                    let eggs = state
                        .properties
                        .get("eggs")
                        .and_then(|s| s.parse::<i32>().ok())
                        .filter(|v| (1..=4).contains(v))
                        .ok_or_else(|| invalid("invalid landing egg count"))?;
                    let remaining = if eggs == 1 {
                        None
                    } else {
                        let mut next = state.clone();
                        next.properties
                            .insert("eggs".into(), (eggs - 1).to_string());
                        Some(
                            registry
                                .id(&next)
                                .ok_or_else(|| invalid("missing decremented egg state"))?,
                        )
                    };
                    Rule::TurtleEgg { remaining }
                }
            };
            states.push(State {
                rule,
                reduction: properties.coefficients(id)?.fall_distance_reduction,
                shelf_mushroom_bounce: definition.shelf_mushroom_bounce,
            });
        }
        Ok(Self { states })
    }
    fn state<E>(&self, id: BlockStateId) -> Result<State, LandingError<E>> {
        self.states
            .get(id.0 as usize)
            .copied()
            .ok_or_else(|| LandingError::Data(invalid("landing state outside registry")))
    }
    /// Whether the state plays the stock shelf mushroom bounce sound.
    pub fn has_shelf_bounce(&self, state: BlockStateId) -> bool {
        self.states
            .get(state.0 as usize)
            .is_some_and(|s| s.shelf_mushroom_bounce)
    }

    pub fn bounce_on<W: BlockLandingWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: BlockPos,
        entity: LandingEntityKind,
    ) -> Result<(), LandingError<W::Error>> {
        if self.state(state)?.shelf_mushroom_bounce
            && !matches!(
                entity,
                LandingEntityKind::Item | LandingEntityKind::PrimedTnt
            )
        {
            let sound = world.known_sound(LandingSound::ShelfMushroomBounce);
            world
                .block_sound(position, sound, 1.0, 1.0)
                .map_err(LandingError::World)?;
        }
        Ok(())
    }
    /// `TurtleEggBlock.stepOn`: a step that is not careful breaks an egg one
    /// time in 100.
    pub fn step_on<W: BlockLandingWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: BlockPos,
        entity: LandingEntityKind,
    ) -> Result<(), LandingError<W::Error>> {
        if let Rule::TurtleEgg { remaining } = self.state(state)?.rule {
            self.destroy_egg(world, remaining, state, position, entity, 100)?;
        }
        Ok(())
    }
    /// `TurtleEggBlock.destroyEgg`: one egg breaks with chance 1 in `randomness`.
    fn destroy_egg<W: BlockLandingWorld>(
        &self,
        world: &mut W,
        remaining: Option<BlockStateId>,
        state: BlockStateId,
        position: BlockPos,
        entity: LandingEntityKind,
        randomness: i32,
    ) -> Result<(), LandingError<W::Error>> {
        if !world.is_server_level()
            || matches!(entity, LandingEntityKind::Turtle | LandingEntityKind::Bat)
        {
            return Ok(());
        }
        let allowed = if entity == LandingEntityKind::Player {
            !world
                .under_spawn_protection(position)
                .map_err(LandingError::World)?
        } else {
            entity.living() && world.mob_griefing().map_err(LandingError::World)?
        };
        if allowed && world.random_int(randomness).map_err(LandingError::World)? == 0 {
            let sound = world.known_sound(LandingSound::TurtleEggBreak);
            let pitch = 0.9f32 + world.random_float().map_err(LandingError::World)? * 0.2f32;
            world
                .block_sound(position, sound, 0.7, pitch)
                .map_err(LandingError::World)?;
            if let Some(next) = remaining {
                world
                    .set_block(position, next, 2)
                    .map_err(LandingError::World)?;
                world
                    .block_event(LandingEvent::BlockDestroy, position, state, false)
                    .map_err(LandingError::World)?;
                world
                    .level_event(2001, position, state.0 as i32)
                    .map_err(LandingError::World)?;
            } else {
                world
                    .destroy_block(position, false)
                    .map_err(LandingError::World)?;
            }
        }
        Ok(())
    }
    pub fn fall_on<W: BlockLandingWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: BlockPos,
        entity: LandingEntityKind,
        distance: f64,
    ) -> Result<(), LandingError<W::Error>> {
        self.fall_on_until_trample(world, state, position, entity, distance, false)
            .map(|_| ())
    }

    /// Run `fallOn` synchronously, unless the owner must handle a selected trample.
    pub fn fall_on_until_trample<W: BlockLandingWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: BlockPos,
        entity: LandingEntityKind,
        distance: f64,
        pause_trample: bool,
    ) -> Result<Option<FarmlandTrample>, LandingError<W::Error>> {
        match self.begin_fall_on(world, state, position, entity, distance, pause_trample)? {
            BlockFall::Done => {}
            BlockFall::Trample(trample) => return Ok(Some(trample)),
            BlockFall::Damage(call) => call.finish(world)?,
        }
        Ok(None)
    }

    /// Run the selected `fallOn` rule up to its call to `causeFallDamage`, or
    /// up to the trample if the owner must handle it.
    pub fn begin_fall_on<W: BlockLandingWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: BlockPos,
        entity: LandingEntityKind,
        distance: f64,
        pause_trample: bool,
    ) -> Result<BlockFall, LandingError<W::Error>> {
        let data = self.state(state)?;
        let superclass = FallDamageCall::new(
            DamageSourceOwner::Entity,
            LandingDamage::Fall,
            distance * f64::from(1.0f32 - data.reduction),
            1.0,
        );
        let call = match data.rule {
            Rule::Default => superclass,
            Rule::Hay => {
                FallDamageCall::new(DamageSourceOwner::Level, LandingDamage::Fall, distance, 0.2)
            }
            Rule::Honey { volume, pitch } => {
                let sound = world.known_sound(LandingSound::HoneySlide);
                world
                    .entity_sound(sound, 1.0, 1.0)
                    .map_err(LandingError::World)?;
                if !world.is_client_side().map_err(LandingError::World)? {
                    world
                        .broadcast_entity_event(54)
                        .map_err(LandingError::World)?;
                }
                FallDamageCall {
                    honey_fall_sound: Some((volume * 0.5, pitch * 0.75)),
                    ..FallDamageCall::new(
                        DamageSourceOwner::Level,
                        LandingDamage::Fall,
                        distance,
                        0.2,
                    )
                }
            }
            Rule::Slime => {
                if world.suppressing_bounce().map_err(LandingError::World)? {
                    return Ok(BlockFall::Done);
                }
                FallDamageCall::new(DamageSourceOwner::Level, LandingDamage::Fall, distance, 0.0)
            }
            Rule::PowderSnow => {
                if (distance >= 4.0 || distance.is_nan()) && entity.living() {
                    let sound = world
                        .living_fall_sound(distance >= 7.0 || distance.is_nan())
                        .map_err(LandingError::World)?;
                    world
                        .entity_sound(sound, 1.0, 1.0)
                        .map_err(LandingError::World)?;
                }
                return Ok(BlockFall::Done);
            }
            Rule::DripstoneTip => FallDamageCall::new(
                DamageSourceOwner::Level,
                LandingDamage::Stalagmite,
                distance + 2.5,
                2.0,
            ),
            Rule::Farmland { base } => {
                if world.is_server_level()
                    && f64::from(world.random_float().map_err(LandingError::World)?)
                        < distance - 0.5
                    && entity.living()
                {
                    let allowed = if entity == LandingEntityKind::Player {
                        !world
                            .under_spawn_protection(position)
                            .map_err(LandingError::World)?
                    } else {
                        world.mob_griefing().map_err(LandingError::World)?
                    };
                    if allowed {
                        let (width, height) = world.entity_dimensions();
                        if width * width * height > 0.512f32 {
                            if pause_trample {
                                return Ok(BlockFall::Trample(FarmlandTrample {
                                    position,
                                    state,
                                    base,
                                    distance,
                                    damage: superclass,
                                }));
                            }
                            let next = push_entities_up(world, state, base, position)
                                .map_err(LandingError::Push)?;
                            world
                                .set_block(position, next, 3)
                                .map_err(LandingError::World)?;
                            world
                                .block_event(LandingEvent::BlockChange, position, next, true)
                                .map_err(LandingError::World)?;
                        }
                    }
                }
                superclass
            }
            Rule::TurtleEgg { remaining } => {
                if entity != LandingEntityKind::Zombie {
                    self.destroy_egg(world, remaining, state, position, entity, 3)?;
                }
                superclass
            }
        };
        Ok(BlockFall::Damage(call))
    }
}
