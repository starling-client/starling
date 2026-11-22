//! Immutable block metadata and registry facts (`host.block_states()`).
//! Queries consume owned API block states, never world numeric IDs. Both sides read the
//! game's published tables and borrow their property definitions.
use crate::{
    BlockKey, BlockMetadata, BlockSoundType, BlockState, BlockStateError, BlockStateRange, Host,
    LevelReadError, PushReaction, host::block_states::BlockStatesBackend,
};

// no wire: borrows a name and values from the host's state definition.
/// One property and its legal values in a state definition. Copying the view does not copy
/// the name or values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StateProperty<'a> {
    /// Property name as serialized.
    pub name: &'a str,
    /// Legal serialized values in definition order.
    pub values: &'a [String],
}

// no wire: borrows the host's resident property definitions.
/// A borrowed view of the properties of a block or world fluid, in property-name order.
// exhaustive: its field is private
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateProperties<'a>(&'a [(String, Vec<String>)]);

impl<'a> StateProperties<'a> {
    /// Lend the game's ordered property slice.
    // hidden: the engine lends its resident definitions; mods obtain the view from the metadata service.
    #[doc(hidden)]
    pub fn of(properties: &'a [(String, Vec<String>)]) -> Self {
        Self(properties)
    }

    /// Read each property in definition order without copying its name or values.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = StateProperty<'a>> + use<'a> {
        self.0
            .iter()
            .map(|(name, values)| StateProperty { name, values })
    }

    /// The number of properties.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether this definition has no properties.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A property constraint used when matching a state.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct StateConstraint {
    /// Property name.
    pub name: String,
    /// Required serialized value.
    pub value: String,
}

impl StateConstraint {
    /// Construct a property constraint.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Vanilla `WeatheringCopper.WeatherState`, from fresh copper to fully oxidized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum WeatheringAge {
    /// Fresh and unaffected by weathering.
    Unaffected,
    /// First stage after fresh copper.
    Exposed,
    /// Second stage after fresh copper.
    Weathered,
    /// Final stage.
    Oxidized,
}

/// A vanilla block class, as Java's `instanceof` asks it of a block, subclasses included. Read it
/// with [`BlockStates::block_class`]: it is the engine's own classification of the block, not a
/// tag, so a datapack that edits `minecraft:stairs` does not change it. A mod block declared on
/// a vanilla behaviour template belongs to that template's classes.
// exhaustive: the engine matches every class; adding one is a deliberate API change
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum BlockClass {
    /// `StairBlock`.
    Stair,
    /// `EntityBlock`: the blocks that own a block entity by class (a chest, a furnace).
    Entity,
    /// `IronBarsBlock`, which includes glass panes and stained glass panes.
    IronBars,
    /// `StainedGlassPaneBlock`, a subclass of `IronBarsBlock`.
    StainedGlassPane,
    /// `TrapDoorBlock`.
    TrapDoor,
}

/// An invalid constraint, identifying its index in the supplied list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum StateMatchError {
    /// The definition has no such property.
    UnknownProperty(u32),
    /// The property does not admit this value.
    InvalidValue(u32),
}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// The block-state facts service: `host.block_states()`.
// exhaustive: its field is private; only `host.block_states()` builds one
pub struct BlockStates<'a> {
    backend: &'a dyn BlockStatesBackend,
}

/// Reaches [`BlockStates`] from any host.
pub trait HostBlockStates {
    /// Facts about block states, as the game's tables hold them.
    fn block_states(&self) -> BlockStates<'_>;
}

impl<H: Host + ?Sized> HostBlockStates for H {
    fn block_states(&self) -> BlockStates<'_> {
        match self.backend() {
            Some(backend) => BlockStates::of(backend.open()),
            None => BlockStates::unavailable(),
        }
    }
}

impl<'a> BlockStates<'a> {
    /// The facts of a game-side table view, as a level view hands them out.
    // hidden: a game-side constructor, as `BackendRef::of`; mods reach the facts through a host or a level view
    #[doc(hidden)]
    pub fn of(backend: &'a dyn BlockStatesBackend) -> Self {
        Self { backend }
    }

    /// The service of a level view with no tables: every fact answers `None` or `Unavailable`.
    pub(crate) fn unavailable() -> Self {
        Self::of(&NoBlockTables)
    }

    /// The block-state tables behind this service.
    // hidden: mods call the service, never its backend
    #[doc(hidden)]
    pub fn backend(&self) -> &'a dyn BlockStatesBackend {
        self.backend
    }

    /// Borrow the block's property names and legal values from the host's tables. An unknown
    /// block or unavailable tables return `None`; a known block without properties returns an
    /// empty view.
    pub fn definition(&self, block: crate::BlockKey) -> Option<StateProperties<'a>> {
        self.backend.block_state_definition(block)
    }
}

/// The reads of a level view with no block tables: the one place that says what such a view answers,
/// `None` for a fact and `Unavailable` for a refusal. [`BlockStates`] reads through it.
struct NoBlockTables;

impl BlockStatesBackend for NoBlockTables {
    fn block_weathering_age(&self, _: BlockState) -> Option<WeatheringAge> {
        None
    }
    fn block_next_weathering_stage(&self, _: BlockState) -> Option<BlockState> {
        None
    }
    fn block_previous_weathering_stage(&self, _: BlockState) -> Option<BlockState> {
        None
    }
    fn block_state_waxed(&self, _: BlockState) -> Option<BlockState> {
        None
    }
    fn block_state_unwaxed(&self, _: BlockState) -> Option<BlockState> {
        None
    }
    fn block_metadata_name(&self, _: BlockKey) -> Result<&str, BlockStateError> {
        Err(BlockStateError::Unavailable)
    }
    fn block_metadata_property(
        &self,
        _: BlockState,
        _: &str,
    ) -> Result<Option<&str>, BlockStateError> {
        Err(BlockStateError::Unavailable)
    }
    fn block_metadata_with(
        &self,
        _: BlockState,
        _: &str,
        _: &str,
    ) -> Result<BlockState, BlockStateError> {
        Err(BlockStateError::Unavailable)
    }
    fn block_metadata_of(
        &self,
        _: BlockKey,
        _: &[StateConstraint],
    ) -> Result<BlockState, BlockStateError> {
        Err(BlockStateError::Unavailable)
    }
    fn block_state_replaceable(&self, _: BlockState) -> Option<bool> {
        None
    }
    fn block_state_destroy_speed(&self, _: BlockState) -> Option<f32> {
        None
    }
    fn block_state_correct_tool_for_drops(
        &self,
        _: &crate::ItemStack,
        _: BlockState,
    ) -> Option<bool> {
        None
    }
    fn block_state_requires_correct_tool(&self, _: BlockState) -> Option<bool> {
        None
    }
    fn block_state_explosion_resistance(&self, _: BlockState) -> Option<f32> {
        None
    }
    fn block_state_sound(&self, _: BlockState) -> Option<BlockSoundType> {
        None
    }
    fn block_state_has_block_entity(&self, _: BlockState) -> Option<bool> {
        None
    }
    fn block_state_map_color(&self, _: BlockState) -> Option<u8> {
        None
    }
    fn block_state_map_color_rgba(&self, _: u8) -> Option<[u8; 4]> {
        None
    }
    fn block_state_light_emission(&self, _: BlockState) -> Option<u8> {
        None
    }
    fn block_state_friction(&self, _: BlockState) -> Option<f32> {
        None
    }
    fn block_state_is_solid_render(&self, _: BlockState) -> Option<bool> {
        None
    }
    fn block_state_is_redstone_conductor(&self, _: BlockState) -> Option<bool> {
        None
    }
    fn block_state_class(
        &self,
        _: BlockState,
        _: BlockClass,
    ) -> Result<Option<bool>, LevelReadError> {
        Err(LevelReadError::Unavailable)
    }
    fn block_state_piston_push_reaction(
        &self,
        _: BlockState,
    ) -> Result<Option<PushReaction>, LevelReadError> {
        Err(LevelReadError::Unavailable)
    }
    fn block_states_of(&self, _: BlockKey) -> Option<BlockStateRange> {
        None
    }
    fn block_state_definition(&self, _: BlockKey) -> Option<StateProperties<'_>> {
        None
    }
    fn block_state_matches(
        &self,
        _: BlockState,
        _: BlockKey,
        _: &[StateConstraint],
    ) -> Option<Result<bool, StateMatchError>> {
        None
    }
    fn fluid_state_definition(&self, _: crate::FluidKey) -> Option<crate::FluidDefinition<'_>> {
        None
    }
    fn fluid_type_properties(&self, _: crate::FluidKey) -> Option<crate::FluidTypeProperties> {
        None
    }
    fn fluid_legacy_block(&self, _: crate::FluidState) -> Option<BlockState> {
        None
    }
    fn fluid_state_validate(
        &self,
        _: crate::FluidKey,
        _: &[StateConstraint],
    ) -> Result<(), crate::FluidStateError> {
        Err(crate::FluidStateError::NoWorldState)
    }
}

impl BlockMetadata for BlockStates<'_> {
    fn name(&self, block: BlockKey) -> Result<&str, BlockStateError> {
        self.backend.block_metadata_name(block)
    }
    fn property(&self, state: BlockState, name: &str) -> Result<Option<&str>, BlockStateError> {
        self.backend.block_metadata_property(state, name)
    }
    fn with(
        &self,
        state: BlockState,
        name: &str,
        value: &str,
    ) -> Result<BlockState, BlockStateError> {
        self.backend.block_metadata_with(state, name, value)
    }
    fn of(
        &self,
        block: BlockKey,
        properties: &[StateConstraint],
    ) -> Result<BlockState, BlockStateError> {
        self.backend.block_metadata_of(block, properties)
    }
}

impl BlockStates<'_> {
    /// `WeatheringCopper.getAge`. None for an unknown state or a block that
    /// doesn't implement weathering, including waxed copper; ask `unwaxed`
    /// first to get the age of a waxed block's underlying stage.
    pub fn weathering_age(&self, state: BlockState) -> Option<WeatheringAge> {
        self.backend.block_weathering_age(state)
    }

    /// `WeatheringCopper.getNext`: the next weathering stage, retaining
    /// compatible state properties. None if no such stage or state exists.
    /// Reads the host's loaded oxidation maps before the vanilla chain.
    pub fn next_weathering_stage(&self, state: BlockState) -> Option<BlockState> {
        self.backend.block_next_weathering_stage(state)
    }

    /// `WeatheringCopper.getPrevious`: the previous weathering stage,
    /// retaining compatible state properties. None if no such stage or state
    /// exists. Reads the host's loaded oxidation maps before the vanilla chain.
    pub fn previous_weathering_stage(&self, state: BlockState) -> Option<BlockState> {
        self.backend.block_previous_weathering_stage(state)
    }

    /// `HoneycombItem.getWaxed`: the waxed counterpart, retaining compatible
    /// state properties. None if the state does not have one or is unknown.
    /// Reads the host's loaded waxables map before vanilla's.
    pub fn waxed(&self, state: BlockState) -> Option<BlockState> {
        self.backend.block_state_waxed(state)
    }

    /// `HoneycombItem.WAX_OFF_BY_BLOCK`: the unwaxed counterpart, retaining
    /// compatible state properties. None if the state does not have one or
    /// is unknown. Reads the host's loaded waxables map before vanilla's.
    pub fn unwaxed(&self, state: BlockState) -> Option<BlockState> {
        self.backend.block_state_unwaxed(state)
    }

    /// `BlockState.canBeReplaced()`: whether placing a block where `state` stands replaces it
    /// (air, grass, water, snow layers). None for an unknown state, or a host without a game.
    pub fn replaceable(&self, state: BlockState) -> Option<bool> {
        self.backend.block_state_replaceable(state)
    }
    /// How hard `state` is to break (`BlockBehaviour.Properties.destroyTime`, as
    /// `BlockState.getDestroySpeed` reads it): stone 1.5, obsidian 50, -1 for a block that
    /// can't be broken. None as for `replaceable`.
    pub fn destroy_speed(&self, state: BlockState) -> Option<f32> {
        self.backend.block_state_destroy_speed(state)
    }
    /// `ItemStack.isCorrectToolForDrops(state)`: whether `stack`'s `minecraft:tool` rules call it
    /// the correct tool for `state`'s block (the first rule that sets `correct_for_drops` and
    /// names the block decides; no component or no such rule is false). Reads only the stack: for
    /// the answer the game uses for a player, see `PlayerRef::has_correct_tool_for_drops`. None
    /// as for `replaceable`.
    pub fn is_correct_tool_for_drops(
        &self,
        stack: &crate::ItemStack,
        state: BlockState,
    ) -> Option<bool> {
        self.backend
            .block_state_correct_tool_for_drops(stack, state)
    }
    /// Whether `state` drops its loot only when mined with a correct tool
    /// (`BlockState.requiresCorrectToolForDrops()`): stone and ores yes, dirt and logs no. Which
    /// tool is correct for a state is `BlockStates::is_correct_tool_for_drops`.
    /// None as for `replaceable`.
    pub fn requires_correct_tool(&self, state: BlockState) -> Option<bool> {
        self.backend.block_state_requires_correct_tool(state)
    }
    /// The explosion resistance `state`'s block declares (`Block.getExplosionResistance`): stone
    /// 6, obsidian 1200, bedrock 3,600,000. What a blast meets at a position, parts and fluids
    /// included, is [`LevelRef::explosion_resistance`](crate::LevelRef::explosion_resistance).
    /// None as for `replaceable`.
    pub fn explosion_resistance(&self, state: BlockState) -> Option<f32> {
        self.backend.block_state_explosion_resistance(state)
    }
    /// `state`'s sounds (`BlockState.getSoundType()`). None as for `replaceable`.
    pub fn sound(&self, state: BlockState) -> Option<BlockSoundType> {
        self.backend.block_state_sound(state)
    }
    /// `BlockState.hasBlockEntity()`. None as for `replaceable`.
    pub fn has_block_entity(&self, state: BlockState) -> Option<bool> {
        self.backend.block_state_has_block_entity(state)
    }
    /// `BlockState.getMapColor`: the state's `MapColor.id`, 0 for none (`MapColor.NONE`).
    /// None as for `replaceable`.
    pub fn map_color(&self, state: BlockState) -> Option<u8> {
        self.backend.block_state_map_color(state)
    }
    /// Decode a packed map color (`MapColor.getColorFromPackedId`) to `[red, green, blue, alpha]`.
    /// Pack a palette ID (such as one from [`Self::map_color`]) as `id * 4 + shade`, where
    /// shades 0–3 scale each RGB channel by 180, 220, 255 or 135 out of 255, rounding down.
    /// ID 0 and undefined palette IDs are transparent (`[0; 4]`); other colors have alpha 255.
    /// Returns `None` only when the host has no game behind it.
    pub fn map_color_rgba(&self, packed: u8) -> Option<[u8; 4]> {
        self.backend.block_state_map_color_rgba(packed)
    }
    /// `BlockState.getLightEmission()`: the light level the state gives off, 0 to 15 (a torch 14,
    /// glowstone 15). A block that declares `dynamic_light` answers per cell through its parts,
    /// which this does not ask. None as for `replaceable`.
    pub fn light_emission(&self, state: BlockState) -> Option<u8> {
        self.backend.block_state_light_emission(state)
    }
    /// The friction of `state`'s block (`BlockBehaviour.getFriction`): 0.6 for most blocks, 0.98
    /// for ice, 0.8 for slime. None as for `replaceable`.
    pub fn friction(&self, state: BlockState) -> Option<f32> {
        self.backend.block_state_friction(state)
    }
    /// `BlockState.isSolidRender()`: whether the state is a full, occluding cube for rendering
    /// and light (stone yes; glass, leaves and slabs no). This is not redstone conduction.
    /// None as for `replaceable`.
    pub fn is_solid_render(&self, state: BlockState) -> Option<bool> {
        self.backend.block_state_is_solid_render(state)
    }
    /// Whether `state`'s block is of vanilla class `class`, subclasses included (Java's
    /// `instanceof`). It reads the engine's class of the block, never a tag, so it does not move
    /// when a pack edits a tag. `Ok(None)` for an unknown state; the refusal the read met
    /// otherwise (`Unavailable`: a host with no game behind it, or a phase that cannot read).
    pub fn block_class(
        &self,
        state: BlockState,
        class: BlockClass,
    ) -> Result<Option<bool>, LevelReadError> {
        self.backend.block_state_class(state, class)
    }
    /// `BlockState.getPistonPushReaction()`: the resolved reaction used for piston simulation
    /// and client replay. Includes the active [`crate::BlockPart::piston_push_reaction`] answer
    /// resolved at load or reload. This read does not call Parts or test movement eligibility.
    /// `Ok(None)` for an unknown block or local ordinal; `Err(LevelReadError::Unavailable)`
    /// when the backend or required block-state tables are unavailable.
    pub fn piston_push_reaction(
        &self,
        state: BlockState,
    ) -> Result<Option<PushReaction>, LevelReadError> {
        self.backend.block_state_piston_push_reaction(state)
    }
    /// `BlockState.isRedstoneConductor` when vanilla decides from `state` alone. None for an
    /// unknown state, no game backend, or a context-dependent predicate (such as shulker boxes).
    /// For a placed block see [`LevelRef::is_redstone_conductor`](crate::LevelRef::is_redstone_conductor).
    pub fn is_redstone_conductor(&self, state: BlockState) -> Option<bool> {
        self.backend.block_state_is_redstone_conductor(state)
    }
    /// Match a state against a block and partial property constraints. Unknown properties and
    /// values are errors even when the state belongs to a different block. None means the
    /// owning thread has no resolved tables or the state is unknown.
    pub fn matches(
        &self,
        state: BlockState,
        block: crate::BlockKey,
        constraints: &[StateConstraint],
    ) -> Option<Result<bool, StateMatchError>> {
        self.backend.block_state_matches(state, block, constraints)
    }

    /// The states of `block` (`minecraft:oak_stairs`), `StateDefinition.getPossibleStates`.
    /// Local ordinals retain their meaning if world numeric IDs reorder on reload.
    /// None before metadata is published, for an unknown block or for a host without a game.
    pub fn states_of(&self, block: crate::BlockKey) -> Option<BlockStateRange> {
        self.backend.block_states_of(block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_property_can_be_copied_from_a_shared_reference() {
        let table = [("powered".to_owned(), vec!["false".into(), "true".into()])];
        let property = StateProperties::of(&table).iter().next().unwrap();
        let borrowed = &property;
        let copied = *borrowed;
        assert_eq!(copied.name, "powered");
        assert_eq!(copied.values, ["false", "true"]);
    }

    /// A mod's test double: a host with no game behind it.
    struct Double;
    impl Host for Double {}
    mod_api::impl_host_areas!(Double);

    #[test]
    fn a_host_without_a_game_answers_none() {
        let facts = Double.block_states();
        let state: BlockState = crate::__private::block_state_from_ordinal(crate::BlockKey(0), 0);
        assert_eq!(facts.replaceable(state), None);
        assert_eq!(facts.destroy_speed(state), None);
        assert_eq!(
            facts.is_correct_tool_for_drops(&crate::ItemStack::of(crate::ItemKey(0)), state),
            None
        );
        assert_eq!(facts.sound(state), None);
        assert_eq!(facts.has_block_entity(state), None);
        assert_eq!(facts.map_color(state), None);
        assert_eq!(facts.map_color_rgba(6), None);
        assert_eq!(facts.states_of(crate::BlockKey(0)), None);
        assert_eq!(
            facts.piston_push_reaction(state),
            Err(LevelReadError::Unavailable)
        );
        assert_eq!(facts.definition(crate::BlockKey(0)), None);
    }
}
