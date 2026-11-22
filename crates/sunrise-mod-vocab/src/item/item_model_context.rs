//! What item model providers (`register_item_models`, `register_range_select_item_model_property`,
//! `register_select_item_model_property`) read besides the stack: display context, living entity
//! holding the item, client level, and draw seed. Absence stays `None`: an item in a screen has
//! no owner, and one drawn outside a world has no level.
use crate::{GlobalPos, Identifier, ItemDisplayContext};

/// The living entity holding the item (`ItemOwner`), as the item model reads it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ItemModelOwner {
    /// Its position.
    pub position: [f64; 3],
    /// `getVisualRotationYInDegrees`.
    pub visual_rotation_y_degrees: f32,
    /// Its entity type (`minecraft:player`).
    pub entity_type: Identifier,
    /// Whether its main arm is the left one.
    pub main_arm_left: bool,
    /// Whether it is using this very stack (`isUsingItem() && getUseItem() == stack`).
    pub using_this_item: bool,
    /// `getUseItemRemainingTicks`.
    pub use_item_remaining_ticks: i32,
    /// `stack.getUseDuration(owner)`.
    pub use_duration: i32,
    /// `CrossbowItem.getChargeDuration(stack, owner)`.
    pub crossbow_charge_duration: i32,
    /// Whether its fishing hook was cast with this item.
    pub fishing_cast_with_this_item: bool,
    /// Whether the stack is in its selected hotbar slot.
    pub is_selected: bool,
    /// Whether the stack is carried on the cursor.
    pub is_carried: bool,
    /// Whether it is the entity the camera views from.
    pub is_view_entity: bool,
    /// Its team's colour, `0xRRGGBB`, when it has one.
    pub team_color: Option<i32>,
    /// `ItemCooldowns.getCooldownPercent(stack, 0)`.
    pub cooldown_percent: f32,
    /// Whether it is this client's player.
    pub is_local_player: bool,
    /// Where it last died, when known.
    pub last_death_location: Option<GlobalPos>,
}

/// The level an item is drawn in (`ClientLevel`), as the item model reads it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ItemModelLevel {
    /// The dimension's key (`minecraft:overworld`).
    pub dimension: Identifier,
    /// `getGameTime`.
    pub game_time: i64,
    /// `EnvironmentAttributes.SUN_ANGLE` at the owner, in degrees.
    pub sun_angle: f32,
    /// `EnvironmentAttributes.MOON_PHASE` index at the owner.
    pub moon_phase: i32,
    /// The world's respawn point, when set.
    pub respawn: Option<GlobalPos>,
    /// `tickRateManager().runsNormally()`.
    pub runs_normally: bool,
}

/// The surroundings an item model reads besides the stack (`ItemModel.update`,
/// `RangeSelectItemModelProperty.get`, `SelectItemModelProperty.get`): display context, living
/// holder, client level, and draw seed.
#[derive(Clone, Debug, PartialEq)]
// no wire: in-process draw context passed to bound item model providers.
#[non_exhaustive]
pub struct ItemModelContext {
    /// Where the item is drawn (a screen, a hand, the ground, a frame, the head).
    pub display: ItemDisplayContext,
    /// The living entity holding the item (`None` in a screen with no holder).
    pub owner: Option<ItemModelOwner>,
    /// The client level the item is drawn in (`None` outside a world).
    pub level: Option<ItemModelLevel>,
    /// The draw's random seed.
    pub seed: i32,
}

impl ItemModelContext {
    // hidden: the game supplies this read-only callback input
    #[doc(hidden)]
    pub fn new(
        display: ItemDisplayContext,
        owner: Option<ItemModelOwner>,
        level: Option<ItemModelLevel>,
        seed: i32,
    ) -> Self {
        Self {
            display,
            owner,
            level,
            seed,
        }
    }
}

/// Whose display transform a layer of a model node draws under (`ItemTransform` of
/// `ItemTransforms`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ItemTransformMode {
    /// The layer's own model's transform for the display context, as every model has.
    #[default]
    Model,
    /// None: the layer is drawn only centred (the model's origin at the item's middle), so a
    /// wrapping model whose own transform already placed the item doesn't apply one twice.
    None,
}

/// A model a bound item model provider answers (`register_item_models`): one of the node's
/// declared `models`, and whose display transform it draws under.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ItemModelLayer {
    /// The key of the model in the node's `models`; a key it doesn't have draws nothing.
    pub model: String,
    /// Whose display transform the model, and every model it draws, goes under.
    pub item_transform: ItemTransformMode,
}

impl ItemModelLayer {
    /// The node's model `model`, under its own display transform.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            item_transform: ItemTransformMode::Model,
        }
    }

    /// The same layer under `item_transform`.
    pub fn with_item_transform(mut self, item_transform: ItemTransformMode) -> Self {
        self.item_transform = item_transform;
        self
    }
}
