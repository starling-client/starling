//! Item-owned client behaviour for worn equipment, tooltip fonts and dropped stacks. Registered anew
//! at resource load.
use crate::{ClientHost, Component, Identifier, ItemKey, ItemStack, ItemStackView, ModelPart, Tag};
use std::sync::Arc;

/// The equipment asset's material list used for this piece.
// no wire: selects a material list during a local client draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the humanoid renderer has these three layer kinds
pub enum EquipmentLayerType {
    /// Adult head, chest and feet.
    Humanoid,
    /// Adult legs.
    HumanoidLeggings,
    /// Baby humanoids, including their legs; not small armour stands.
    HumanoidBaby,
}

/// A model baked from a registered [`ModelPart`] tree on this resource load.
/// Stale references are reported on each use and select the vanilla model without disabling the provider.
// no wire: indexes this client's resource-load model table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: only the engine constructs resource-load references
pub struct ArmorModelRef {
    generation: u64,
    index: u32,
}
impl ArmorModelRef {
    /// Use the original humanoid armour model for this slot.
    pub const VANILLA: Self = Self {
        generation: 0,
        index: 0,
    };
    /// Engine constructor; mods obtain references from the reload registrar.
    // hidden: engine binding only; mods use the resource-load registrar
    #[doc(hidden)]
    pub const fn __new(generation: u64, index: u32) -> Self {
        Self { generation, index }
    }
    /// Engine lookup identity.
    // hidden: engine lookup only; mods use the resource-load registrar
    #[doc(hidden)]
    pub const fn __parts(self) -> (u64, u32) {
        (self.generation, self.index)
    }
}

/// An atlas sprite bound during this resource load, or [`Self::DEFAULT`] for the original texture.
/// Stale references are reported on each use and select the default without disabling the provider.
// no wire: indexes this client's resource-load texture table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: only the engine constructs resource-load references
pub struct ArmorTextureRef {
    generation: u64,
    index: u32,
}
impl ArmorTextureRef {
    /// Keep the renderer's original texture choice, including pack/CIT selection.
    pub const DEFAULT: Self = Self {
        generation: 0,
        index: 0,
    };
    /// Engine constructor; mods obtain references from the reload registrar.
    // hidden: engine binding only; mods use the resource-load registrar
    #[doc(hidden)]
    pub const fn __new(generation: u64, index: u32) -> Self {
        Self { generation, index }
    }
    /// Engine lookup identity.
    // hidden: engine lookup only; mods use the resource-load registrar
    #[doc(hidden)]
    pub const fn __parts(self) -> (u64, u32) {
        (self.generation, self.index)
    }
}

/// One material layer borrowed from the equipment asset during a draw callback.
// no wire: borrows the texture name for the local draw call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EquipmentLayer<'a> {
    /// The asset's expanded sprite name before renderer overrides such as CIT,
    /// e.g. `mymod:entity/equipment/humanoid/chest_inner` for `mymod:chest_inner`.
    pub texture: &'a str,
    /// Whether this layer takes the stack's dye colour.
    pub dyeable: bool,
    /// Packed RGB used when no dyed colour is present. None skips an undyed dyeable layer.
    pub color_when_undyed: Option<u32>,
    /// Whether the equipment asset asks for the player's own texture.
    pub use_player_texture: bool,
}
impl<'a> EquipmentLayer<'a> {
    /// Lend a layer at the engine draw boundary.
    // hidden: engine draw only; mods receive the layer in the callback
    #[doc(hidden)]
    pub fn __new(
        texture: &'a str,
        dyeable: bool,
        color_when_undyed: Option<u32>,
        use_player_texture: bool,
    ) -> Self {
        Self {
            texture,
            dyeable,
            color_when_undyed,
            use_player_texture,
        }
    }
}

struct DyedColor;

impl Component for DyedColor {
    type Value = u32;

    fn id(&self) -> &str {
        "minecraft:dyed_color"
    }

    fn decode(&self, value: &Tag) -> Option<u32> {
        match value {
            Tag::Int(rgb) => Some(*rgb as u32 | 0xFF00_0000),
            _ => None,
        }
    }

    fn encode(&self, value: u32) -> Tag {
        Tag::Int((value & 0x00FF_FFFF) as i32)
    }
}

/// The single provider attached to one item for worn equipment rendering.
/// Only visible equipped pieces which pass the equippable slot/asset gate are queried.
/// The live [`ItemStackView`] and asset [`EquipmentLayer`] are lent for the call. Register trees
/// and textures through the reload registrar and use the references it returns.
/// A refused stack is reported each time and draws the piece as vanilla for that call. Stale
/// references are reported each time and select the default for that layer. A panic logs and shows an
/// error toast naming the mod, and disables its callback for this item for the rest of the session,
/// including across resource reloads.
pub trait ClientItemExtensions: Send + Sync + 'static {
    /// Choose the tree for one material layer, after its nonzero tint and texture resolve.
    /// Humanoid pose and slot visibility are copied onto the registered tree.
    /// This per-layer selection deliberately differs from NeoForge's one model per piece:
    /// it allows an inner and an outer chest shape to use distinct textures without painting
    /// both textures onto both shapes. There are no additional per-piece model callbacks.
    /// The trim draws once on the last material layer actually drawn's tree, or on vanilla's
    /// model if no material drew. The trim does not call this method a second time.
    fn armor_layer_model(
        &self,
        host: &dyn ClientHost,
        stack: &ItemStackView<'_>,
        layer_type: EquipmentLayerType,
        layer: &EquipmentLayer<'_>,
        index: u32,
    ) -> ArmorModelRef {
        let _ = (host, stack, layer_type, layer, index);
        ArmorModelRef::VANILLA
    }
    /// The live stack's dyed colour as packed ARGB, or zero when undyed.
    /// The default reads just the dyed colour override, not the full stack.
    fn default_dye_color(&self, host: &dyn ClientHost, stack: &ItemStackView<'_>) -> u32 {
        let _ = host;
        stack.get(&DyedColor).unwrap_or(0)
    }
    /// Packed ARGB for this material; exactly zero skips texture and model callbacks.
    /// Asked before either callback. Nondyeable materials are white by default.
    fn armor_layer_tint_color(
        &self,
        host: &dyn ClientHost,
        stack: &ItemStackView<'_>,
        layer: &EquipmentLayer<'_>,
        index: u32,
        dye: u32,
    ) -> u32 {
        let _ = (host, stack, index);
        if !layer.dyeable {
            0xFFFF_FFFF
        } else if dye != 0 {
            dye
        } else {
            layer.color_when_undyed.map_or(0, |rgb| rgb | 0xFF00_0000)
        }
    }
    /// Choose this layer's texture. `default_texture` names the renderer's texture after pack/CIT
    /// selection, or the asset's texture if the renderer could not bind it. Return
    /// [`ArmorTextureRef::DEFAULT`] to keep that selection, or a reload-bound sprite to override it.
    /// An override uses the sprite's UVs bound during registration, with no later texture override.
    /// A stale reference is logged and keeps the default selection.
    fn armor_texture(
        &self,
        host: &dyn ClientHost,
        stack: &ItemStackView<'_>,
        layer_type: EquipmentLayerType,
        layer: &EquipmentLayer<'_>,
        default_texture: &str,
    ) -> ArmorTextureRef {
        let _ = (host, stack, layer_type, layer, default_texture);
        ArmorTextureRef::DEFAULT
    }
    /// Choose the tooltip font for this item (`IClientItemExtensions.getFont` with
    /// `FontContext.TOOLTIP`).
    ///
    /// Used to measure and wrap text lines during `render_tooltip_gather_components` and to seed
    /// `render_tooltip_pre`'s `font`. Return `None` to use the default font (`minecraft:default`).
    fn tooltip_font(&self, host: &dyn ClientHost, stack: &ItemStack) -> Option<Identifier> {
        let _ = (host, stack);
        None
    }
    /// Whether this dropped stack bobs up and down as it spins
    /// (`IClientItemExtensions.shouldBobAsEntity`).
    ///
    /// Asked with the dropped stack each time it is drawn on the client, picked-up stacks flying
    /// to their collector included. `false` removes only the up-and-down offset: the spin, the
    /// height above the ground and the number of copies drawn stay as they are. If the callback
    /// panics, the stack bobs as vanilla's does.
    fn should_bob_as_entity(&self, host: &dyn ClientHost, stack: &ItemStack) -> bool {
        let _ = (host, stack);
        true
    }
    /// Whether a dropped stack of several copies draws them spread apart
    /// (`IClientItemExtensions.shouldSpreadAsEntity`).
    ///
    /// Asked with the dropped stack each time it is drawn on the client, picked-up stacks flying
    /// to their collector included. `false` draws every copy
    /// at the same place and in the same pose, so the stack looks like one item. If the callback
    /// panics, the copies spread as vanilla's do.
    fn should_spread_as_entity(&self, host: &dyn ClientHost, stack: &ItemStack) -> bool {
        let _ = (host, stack);
        true
    }
}

/// Client resource-load registration. Models, textures and item providers refresh together.
/// No registration carries over to the next resource load. An error refuses only that call;
/// successful registrations are still installed.
pub trait ClientItemExtensionsRegistrar {
    /// Register a named [`ModelPart`] tree, validated and baked once for this resource load.
    /// Names can use any valid namespace. Malformed trees and duplicate names are refused by name.
    /// Submission uses the baked geometry, not the caller's tree, and does not require a depth cap.
    fn register_model(&mut self, name: &str, model: &ModelPart) -> Result<ArmorModelRef, String>;
    /// Bind a requested atlas sprite and its UVs once for this resource load.
    /// An unknown texture refuses registration by name.
    fn register_texture(&mut self, name: &str) -> Result<ArmorTextureRef, String>;
    /// Attach the one provider for this item. Duplicates name both providers.
    fn client_item_extensions(
        &mut self,
        item: ItemKey,
        provider: Arc<dyn ClientItemExtensions>,
    ) -> Result<(), String>;
}
