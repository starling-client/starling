//! Load-time content declarations. These are drafts: the game admits a namespace atomically
//! using the same checks as content JSON files. Registry-dependent validity is not promised
//! by these records; no engine state is written by constructing or editing them.
use crate::ArmPose;
use std::collections::BTreeMap;

/// A fence gate's sound override, resolved once when content loads.
/// A missing site keeps the template wood's sound for that site.
// exhaustive: the declaration format has exactly an all-sites sound or named-site overrides.
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
pub enum GateSound {
    /// Use one sound event at every site.
    All(String),
    /// Override only the named sites.
    Sites {
        /// Player interaction, including client prediction.
        on_use: Option<String>,
        /// An explosion that triggers an unpowered gate.
        on_explosion: Option<String>,
        /// A redstone power change that changes the open state.
        on_redstone: Option<String>,
    },
}

#[cfg(test)]
mod gate_sound_tests {
    use super::GateSound;
    use crate::wire::{Reader, Wire, Writer};

    #[test]
    fn both_gate_sound_forms_cross_wire() {
        for sound in [
            GateSound::All("gate:open".into()),
            GateSound::Sites {
                on_use: Some("gate:use".into()),
                on_explosion: None,
                on_redstone: Some("gate:wire".into()),
            },
        ] {
            let mut out = Writer::default();
            sound.put(&mut out);
            let back: GateSound = Reader::decode(&out.into_bytes()).unwrap();
            assert_eq!(back, sound);
        }
    }
}

/// BlockProperties in a load-time content declaration.
#[derive(Clone, Debug, PartialEq, Default)]
// exhaustive: declaration drafts mirror the closed content schema and support record construction.
#[derive(sunrise_mod_api_macros::Wire)]
pub struct BlockProperties {
    /// Destroy time in seconds of hand mining (-1: unbreakable).
    pub hardness: Option<f32>,
    /// Explosion resistance; unset inherits the template.
    pub explosion_resistance: Option<f32>,
    /// Light emitted, 0 to 15.
    pub light: Option<u8>,
    /// `true`: the block's parts answer `light_emission`, the light a cell emits by where it is
    /// (`IBlockExtension.hasDynamicLightEmission`); `light` is a cell's without an answer.
    pub dynamic_light: Option<bool>,
    /// Whether drops require the appropriate tool.
    pub requires_tool: Option<bool>,
    /// Surface friction; unset inherits the template.
    pub friction: Option<f32>,
    /// A `MapColor` id, 0 to 61.
    pub map_color: Option<u8>,
    /// A vanilla block whose sounds this block uses.
    pub sounds: Option<String>,
    /// Which states tick randomly, whatever the template does (`isRandomlyTicking`).
    pub random_ticks: Option<RandomTicks>,
    /// How the block is drawn, whatever the template does (`BlockBehaviour.getRenderShape`).
    pub render_shape: Option<RenderShape>,
    /// Vanilla's piston push reaction, or the template's when unset.
    pub push_reaction: Option<PushReaction>,
    /// `false`: the block occludes nothing, so its neighbours' faces are drawn and light passes
    /// it (`BlockBehaviour.Properties.noOcclusion`); `true`: occludes whatever the template
    /// does, using the declared outline or the template's published occlusion shape. Unset
    /// inherits the template.
    pub occludes: Option<bool>,
    /// `false`: the breaking crack (`destroy_stage_0..9`) is not drawn on the block, as for a
    /// block broken by use; unset or `true`, it is drawn. Only the overlay: the break and the
    /// destroy-progress packets are vanilla's.
    pub destroy_overlay: Option<bool>,
    /// Per-column model offset and translation of declared outline, collision and interaction shapes.
    /// Block-entity published shapes remain as published. Unset inherits the template.
    pub offset: Option<OffsetType>,
    /// Maximum horizontal displacement, 0 to 0.25 (default 0.25 for an explicit offset mode).
    pub max_horizontal_offset: Option<f32>,
    /// Maximum downward displacement, 0 to 0.2 (default 0.2 for XYZ; XZ has no vertical offset).
    pub max_vertical_offset: Option<f32>,
    /// Vanilla's note-block instrument; unset inherits the template.
    pub instrument: Option<crate::NoteInstrument>,
    /// Lava ignition flag. The per-position `ignited_by_lava` hook still answers first.
    pub ignited_by_lava: Option<bool>,
    /// Whether placement and features may replace this block; the held-own-item rule still applies.
    pub replaceable: Option<bool>,
    /// Skip faces against any state of this same block; false disables the template's render rule.
    /// Leaf and ice graphics culling still follows the template class.
    pub skip_same_block_faces: Option<bool>,
    /// Opt into `VegetationBlock` survival and neighbour checks independently of the
    /// template. Templates that already inherit `VegetationBlock` keep their rules.
    /// Otherwise the default support is `minecraft:supports_vegetation`.
    pub vegetation: bool,
}

/// Vanilla's per-column offset mode (`BlockBehaviour.OffsetType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: these are the closed declaration modes.
#[derive(sunrise_mod_api_macros::Wire)]
pub enum OffsetType {
    /// Disable the offset, even on a template that has one.
    None,
    /// Horizontal offset only.
    Xz,
    /// Horizontal and downward vertical offset.
    Xyz,
}

/// Vanilla's piston reaction for a block state or an entity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: these variants are the closed content declaration vocabulary.
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PushReaction {
    /// Vanilla's default: pistons can push and pull.
    #[default]
    PushPull,
    /// Declaration option.
    Push,
    /// Declaration option.
    Popped,
    /// Declaration option.
    Immoveable,
    /// Do not push this entity.
    IgnoreEntity,
}

/// RenderShape in a load-time content declaration.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: these variants are the closed content declaration vocabulary.
#[derive(sunrise_mod_api_macros::Wire)]
pub enum RenderShape {
    /// Declaration option.
    Model,
    /// Declaration option.
    EntityblockAnimated,
    /// Declaration option.
    Invisible,
}

/// RandomTicks in a load-time content declaration.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: these variants are the closed content declaration vocabulary.
#[derive(sunrise_mod_api_macros::Wire)]
pub enum RandomTicks {
    /// Every state (`true`), or none (`false`, for a template that ticks).
    All(bool),
    /// The states whose own state property `property` is one of `values`.
    States {
        /// State property name.
        property: String,
        /// Accepted state values.
        values: Vec<String>,
    },
}

/// BlockShapes in a load-time content declaration.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: declaration drafts mirror the closed content schema and support record construction.
#[derive(sunrise_mod_api_macros::Wire)]
pub struct BlockShapes {
    /// State property names selecting a shape, in tuple order.
    pub properties: Vec<String>,
    /// Comma-separated property-value tuples mapped to boxes in block units (one block is 1).
    pub values: BTreeMap<String, Vec<[f64; 6]>>,
    /// Boxes used when no tuple matches.
    pub default: Option<Vec<[f64; 6]>>,
}

/// Per-state light emission, overriding `BlockProperties::light`.
#[derive(Clone, Debug, PartialEq, Default)]
// exhaustive: declaration drafts mirror the closed content schema.
#[derive(sunrise_mod_api_macros::Wire)]
pub struct StateLight {
    /// State property names selecting emission, in tuple order.
    pub properties: Vec<String>,
    /// Comma-separated property-value tuples mapped to emission from 0 to 15.
    pub values: BTreeMap<String, u8>,
    /// Emission for unmatched states; without it every state must match a tuple.
    pub default: Option<u8>,
}

/// Entity category selecting a load-time shape override.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
// exhaustive: entity categories are the closed content schema vocabulary.
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ShapeEntityKind {
    /// Dropped items.
    Item,
    /// Players.
    Player,
    /// Other living entities.
    Living,
    /// Non-living entities.
    Other,
}

/// A vanilla block's context-dependent collision a declared block takes ([`Block::context_collision`]),
/// whatever its template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: these variants are the closed content declaration vocabulary.
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ContextCollision {
    /// Scaffolding's (`ScaffoldingBlock.getCollisionShape`): solid on top unless the entity is
    /// descending, open below, and the bottom row's floor. The block's states need scaffolding's
    /// `distance` (0 to 7) and `bottom` properties, from the template or declared; loading
    /// refuses the block, by name, otherwise.
    Scaffolding,
}

/// BlockStateProperty in a load-time content declaration.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: these variants are the closed content declaration vocabulary.
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BlockStateProperty {
    /// `false` and `true`, in vanilla's spelling.
    Boolean,
    /// A list of values, in the order the state space is built in.
    Enum {
        /// Allowed values in state order.
        values: Vec<String>,
    },
}

/// An unvalidated draft; `Content::declare` validates it at load.
/// Refused drafts are reported by the content admission path as
/// `native blocks and items of <namespace>: <reason>`, without admitting part of that namespace.
/// `Default` and [`Block::new`] leave out what a content file leaves out: every field is empty
/// except `item`, true, as in a file.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Block {
    /// `<mod id>:<path>`.
    pub id: String,
    /// The vanilla block it copies, `minecraft:<path>`.
    pub template: String,
    /// properties.
    pub properties: BlockProperties,
    /// Per-state emission, taking precedence over `properties.light`.
    /// Defaults to `None`, leaving uniform or template light unchanged.
    pub state_light: Option<StateLight>,
    /// shapes.
    pub shapes: Option<BlockShapes>,
    /// collision shapes.
    pub collision_shapes: Option<BlockShapes>,
    /// Outline overrides for actual entity queries; omitted kinds use the default.
    pub outline_context_shapes: BTreeMap<ShapeEntityKind, BlockShapes>,
    /// Collision overrides for actual entity queries; omitted kinds use the default.
    pub collision_context_shapes: BTreeMap<ShapeEntityKind, BlockShapes>,
    /// Ray-face override, without replacing the outline hit point or creating a hit.
    pub interaction_shapes: Option<BlockShapes>,
    /// Whether the block's block entity publishes the block's outline and collision shapes
    /// (`BlockEntityRef::set_shapes`), which then replace the declared ones until cleared.
    /// Refused at load, by name, on a block that no block entity type owns. False when unset.
    pub shapes_from_block_entity: bool,
    /// A vanilla block's context collision it takes, over its shapes for entities that have no
    /// `collision_context_shapes` of their own. `None` when unset.
    pub context_collision: Option<ContextCollision>,
    /// State properties of its own, which its template does not have.
    /// A boolean `waterlogged` adds SimpleWaterloggedBlock behaviour: buckets fill
    /// and drain it, placement copies source water at the cell, and shape updates schedule
    /// a water tick. The placement and shape Part answers override those defaults.
    /// Refused if `waterlogged` is an enum or the template already has it, is a
    /// standalone liquid such as lava, or already holds water in every state.
    /// A double plant keeps its class and companion-half behaviour.
    pub state_properties: BTreeMap<String, BlockStateProperty>,
    /// State properties overlaid on the template default.
    pub default_state: BTreeMap<String, String>,
    /// Also add a block item with the same id; true unless set false, as in a file.
    pub item: bool,
    /// A fence gate's opening sounds (`WoodType.fenceGateOpen`). `All` replaces
    /// every site; `Sites` keeps the template wood's sound for each missing site.
    /// Needs a `*_fence_gate` template.
    pub open_sound: Option<GateSound>,
    /// A fence gate's closing sounds (`WoodType.fenceGateClose`), with the same
    /// per-site fallback as `open_sound`.
    pub close_sound: Option<GateSound>,
    /// The item this block is, for pick-block and `Block.asItem`, when
    /// it is not a block item of its own: an item of this mod (a bare name is
    /// this mod's) or a vanilla one (`minecraft:<path>`), checked to exist when
    /// the content loads. Needs `item` false.
    pub block_item: Option<String>,
    /// The item pick-block takes from this block (`getCloneItemStack`), when it is not
    /// the block's item: a mature crop picks its seed. An item of this mod (a bare name is
    /// this mod's) or a vanilla one (`minecraft:<path>`), checked to exist when the content
    /// loads. Absent is the block's item.
    pub pick_item: Option<String>,
    /// Its item's component patch: `Some(tag)` sets a component, `None` removes it.
    /// Component names do not include the removal prefix `!`; repeated IDs are refused.
    pub components: Vec<(String, Option<crate::Tag>)>,
    /// Suspicious stew effects this block supplies, in order, as pairs of effect ID
    /// and duration in ticks. Its block items feed these effects to brown mooshrooms
    /// and generate stews in Creative. `None` declares no effect source.
    pub suspicious_stew_effects: Option<Vec<(String, i32)>>,
    /// A connecting block (template `minecraft:chorus_plant`, a `PipeBlock`
    /// without the chorus plant's rules): the block tag, without `#`, whose
    /// blocks it joins besides itself (a cable's machines).
    pub connects_to: Option<String>,
    /// A connecting block's width in pixels (2 to 14): its shapes, the
    /// chorus plant's (10) when unset.
    pub thickness: Option<f32>,
    /// A sign's `WoodType` (template a vanilla sign), which names its editor's
    /// background: `<namespace>:<path>` a mod's texture, a bare name vanilla's.
    /// The template's when unset.
    pub wood_type: Option<String>,
    /// The block a standing or ceiling sign's item places on a wall
    /// (`StandingAndWallBlockItem.wallBlock`): one of this mod's blocks (a bare
    /// name is this mod's), whose template is the matching wall sign and which
    /// has no item. The template's when unset.
    pub wall_block: Option<String>,
    /// A sign's new text colour (`SignBlockEntity.createDefaultSignText`): a `DyeColor` name,
    /// black when unset. Applies to a sign when it is placed, front and back.
    pub sign_text_color: Option<String>,
    /// Whether a new sign's text glows, false when unset.
    pub sign_text_glow: Option<bool>,
    /// The block farmland turns into: one of this mod's (a bare name is this mod's) or a
    /// vanilla one (`minecraft:<path>`); the template's base when unset.
    pub base_block: Option<String>,
}

/// Rarity in a load-time content declaration.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: these variants are the closed content declaration vocabulary.
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Rarity {
    /// Declaration option.
    Common,
    /// Declaration option.
    Uncommon,
    /// Declaration option.
    Rare,
    /// Declaration option.
    Epic,
}

/// An unvalidated draft; `Content::declare` validates it at load.
/// Refused drafts are reported by the content admission path as
/// `native blocks and items of <namespace>: <reason>`, without admitting part of that namespace.
#[derive(Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Item {
    /// Identifier in the declaring mod's namespace.
    pub id: String,
    /// The vanilla item it copies, `minecraft:<path>`.
    pub template: String,
    /// Maximum stack size, validated as 1 to 99 at admission.
    pub max_stack_size: Option<u8>,
    /// Rarity overriding the template's.
    pub rarity: Option<Rarity>,
    /// A component patch over the template: `Some(tag)` sets, `None` removes.
    /// Component names are namespaced identifiers, without a removal prefix `!`; repeated IDs are refused.
    pub components: Vec<(String, Option<crate::Tag>)>,
    /// What it leaves in the crafting grid (`Item.Properties.craftRemainder`):
    /// a vanilla item (`minecraft:<path>`) or one of the mod's own (a bare name is the mod's).
    pub crafting_remainder: Option<String>,
    /// The block placed by this item, if any: one of this mod's (a bare name is this mod's)
    /// or a vanilla one (`minecraft:<path>`).
    pub block: Option<String>,
    /// The arm pose a humanoid takes holding it, in place of vanilla's (`ArmPose`).
    pub arm_pose: Option<ArmPose>,
}

impl Default for Block {
    fn default() -> Self {
        Self {
            id: String::new(),
            template: String::new(),
            properties: BlockProperties::default(),
            state_light: None,
            shapes: None,
            collision_shapes: None,
            outline_context_shapes: BTreeMap::new(),
            collision_context_shapes: BTreeMap::new(),
            interaction_shapes: None,
            shapes_from_block_entity: false,
            context_collision: None,
            state_properties: BTreeMap::new(),
            default_state: BTreeMap::new(),
            item: true,
            open_sound: None,
            close_sound: None,
            block_item: None,
            pick_item: None,
            components: Vec::new(),
            suspicious_stew_effects: None,
            connects_to: None,
            thickness: None,
            wood_type: None,
            wall_block: None,
            sign_text_color: None,
            sign_text_glow: None,
            base_block: None,
        }
    }
}
impl Block {
    /// Whether to add a block item with the same identifier; a block has one unless told not to.
    pub fn with_item(mut self, item: bool) -> Self {
        self.item = item;
        self
    }
    /// Start an unvalidated declaration request; content admission checks both identifiers.
    pub fn new(id: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            template: template.into(),
            ..Self::default()
        }
    }
}
impl Item {
    /// Start an unvalidated declaration request; content admission checks both identifiers.
    pub fn new(id: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            template: template.into(),
            ..Self::default()
        }
    }
}
