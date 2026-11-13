//! Starling: the `mod_api` module native mods named before Horizon's mod API rework, for the mods
//! made for it. Horizon's own API is the `mod_api` crate now (horizon-mod-vocab), with the
//! `Native` prefix dropped from its types; this module is that crate with the old names beside
//! it, and the old module's re-exports. Starling's native_mod_adapt.py points a mod's
//! `horizon_client::mod_api` and `crate::mod_api` here (a module of that name would be
//! ambiguous with the crate). The game's old hooks and helpers are in game/starling/native_compat.rs.
pub use ::mod_api::*;

pub use crate::extras::Corner;
pub use crate::extras_hud::Corners;
pub use crate::first_person::Hand;
pub use crate::hud::HudBatch;
pub use crate::items::ItemStack;
pub use crate::text::Text;
pub use horizon_core::nbt::{Compound, Tag};
pub use horizon_core::{
    geometry::Direction,
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
pub use horizon_protocol::packets::play::Clientbound as ClientboundPlayPacket;
pub use horizon_world::registry::BlockStateId;
pub use serde_json;

/// `mobs::render::render_layers` with `render_with`'s parameters, as Horizon had it: mods pick
/// one of the two as a function. The split's also hands back the posed parts; this asks for none.
pub fn render_layers_old(
    m: &crate::mobs::Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: crate::mobs::render::Atlas,
    light: u32,
    data: &crate::mobs::MobData,
    extra: &[(&crate::mobs::Part, [f32; 4], u32, u32, bool)],
    out: &mut Vec<crate::mesher::Vertex>,
) {
    crate::mobs::render::render_layers(m, alpha, camera, atlas, light, data, extra, None, out);
}

/// A stack of the panel's events as the old panel gave it: the item by its id.
pub fn native_stack_old(stack: &::mod_api::Stack) -> NativeStack {
    NativeStack {
        item: ::mod_api::__private::item_key_id(stack.item)
            .map(|id| id.to_string())
            .unwrap_or_else(|| "minecraft:air".to_owned()),
        count: stack.count,
        components: stack.components.clone(),
    }
}

/// A native mod as Options > Mods lists it (`native_mods`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeMod {
    /// Its id: its settings folder and its keys' lines.
    pub id: String,
    /// `Name 1.0`.
    pub title: String,
    pub description: String,
    /// Its keys: name, label and default key (`key.keyboard.z`). The page
    /// lists them first; `native_mod_key` reads the player's choice.
    pub keys: Vec<[String; 3]>,
}

/// An item the panel shows (`native_panel_items`): its id (`minecraft:diamond`), a count (1
/// draws none) and its components beyond the item's own, in vanilla's item syntax, or "".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeStack {
    pub item: String,
    pub count: u32,
    pub components: String,
}

/// The types that only lost their `Native` prefix.
pub type NativeCommand = ::mod_api::Command;
pub type NativeDataMapType = ::mod_api::game::DataMapType;
pub type NativeDispensed = ::mod_api::Dispensed;
pub type NativeFalling = ::mod_api::Falling;
pub type NativeHeightmap = ::mod_api::Heightmap;
pub type NativeLifecycleContext = ::mod_api::LifecycleContext;
pub type NativeMinimap = ::mod_api::Minimap;
pub type NativeMobRender<'a> = ::mod_api::game::MobRender<'a>;
pub type NativeModEvent = ::mod_api::game::ModEvent;
pub type NativePanelEvent = ::mod_api::PanelEvent;
pub type NativePlayerRender<'a> = ::mod_api::game::PlayerRender<'a>;
pub type NativeScreenView<'a> = ::mod_api::ScreenView<'a>;
pub type NativeServerPlayer = ::mod_api::ServerPlayer;
pub type NativeShake = ::mod_api::game::Shake;
pub type NativeZoom = ::mod_api::Zoom;
