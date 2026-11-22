//! A living entity drawn on a screen or the HUD ([`DrawContext::entity`](crate::DrawContext::entity)):
//! vanilla's `InventoryScreen.renderEntityInInventory`, which draws the inventory's player, a
//! horse's in its menu, and what recipe viewers and data screens show of a mob.
use crate::{Compound, EntityRef};
use std::fmt;

/// Which living entity [`DrawContext::entity`](crate::DrawContext::entity) draws.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GuiEntity {
    /// A new entity of living type `kind` (`minecraft:zombie`, `mymod:golem`), made for the
    /// screen and never added to a level, loaded from `data` as `/summon <kind> ~ ~ ~ <data>`
    /// reads it (`{IsBaby:1b}`, a variant); empty `data` is the type's defaults. The game keeps
    /// the last few it made, so drawing the same `kind` and `data` every frame makes it once.
    Kind {
        /// The entity type.
        kind: String,
        /// Its saved data; an `id` in it must name `kind`.
        data: Compound,
    },
    /// An entity of the client's level as it is this frame (what it wears, its pose), turned
    /// for the screen, as `InventoryScreen` draws the player. A client handle
    /// (`ClientHost::crosshair_target`, `local_player`).
    Existing(EntityRef),
}

impl GuiEntity {
    /// A new entity of living type `kind` with its type's defaults.
    pub fn kind(kind: impl Into<String>) -> Self {
        Self::Kind {
            kind: kind.into(),
            data: Compound::new(),
        }
    }

    /// A new entity of living type `kind` loaded from `data`.
    pub fn with_data(kind: impl Into<String>, data: Compound) -> Self {
        Self::Kind {
            kind: kind.into(),
            data,
        }
    }
}

/// How a [`GuiEntity`] is turned.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GuiEntityLook {
    /// `renderEntityInInventoryFollowsMouse`: the body and the head turned toward this point,
    /// in GUI pixels (the cursor, as the inventory's player is), by vanilla's
    /// `atan((centre - point) / 40)` angles.
    At([f32; 2]),
    /// `renderEntityInInventory` with the mod's own turn: the body `yaw` degrees from facing
    /// the viewer (a spinning mob adds to it each frame), the head pitched `pitch` degrees
    /// (down is positive), the head facing the way the body does.
    Turned {
        /// Degrees the body is turned from facing the viewer.
        yaw: f32,
        /// Degrees the head is pitched, down positive.
        pitch: f32,
    },
}

/// Where and how big a [`GuiEntity`] is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct GuiEntityView {
    /// The box (x0, y0, x1, y1, in GUI pixels) the entity stands centred in; what lies outside
    /// it is cut off.
    pub rect: [f32; 4],
    /// GUI pixels a block (vanilla's `size`: 30 for the inventory's player).
    pub size: f32,
    /// Blocks the entity is moved down from the box's centre (vanilla's `offsetY`: 0.0625 for
    /// the inventory's player).
    pub offset_y: f32,
    /// How it is turned.
    pub look: GuiEntityLook,
}

impl GuiEntityView {
    /// A view of `rect`, `size` GUI pixels a block, `offset_y` blocks down, turned as `look`.
    pub fn new(rect: [f32; 4], size: f32, offset_y: f32, look: GuiEntityLook) -> Self {
        Self {
            rect,
            size,
            offset_y,
            look,
        }
    }
}

/// Why [`DrawContext::entity`](crate::DrawContext::entity) drew nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GuiEntityError {
    /// No living entity type has that id: an unknown one, or a type that isn't living.
    UnknownType,
    /// `data` names another type in its `id`, or doesn't load as that type.
    BadData,
    /// The handle names no living entity of the client's level: stale, gone, a server
    /// handle, or an entity that isn't living.
    NoEntity,
    /// The box is empty, or `size`, `offset_y` or the look isn't finite, or `size` isn't
    /// positive.
    BadView,
    /// The context has no world to draw in.
    Unavailable,
}

impl fmt::Display for GuiEntityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnknownType => "no living entity type has that id",
            Self::BadData => "the data does not load as that entity type",
            Self::NoEntity => "the handle names no living entity of the client's level",
            Self::BadView => "the view's box, size, offset or look is not drawable",
            Self::Unavailable => "there is no world to draw the entity in",
        })
    }
}

impl std::error::Error for GuiEntityError {}
