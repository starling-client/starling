//! Item-owned client behaviour for worn equipment. Registered anew at resource load.
use crate::{ClientHost, ItemKey, ItemStack, ModelPart};
use std::sync::Arc;

/// The equipment asset's material list used for this piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the humanoid renderer has these three layer kinds
pub enum EquipmentLayerType {
    /// Adult head, chest and feet.
    Humanoid,
    /// Adult legs.
    HumanoidLeggings,
    /// Baby humanoids, including their legs; not small armour stands.
    HumanoidBaby,
}

/// A model baked from a registered ModelPart tree on this resource load.
/// References from a previous load fall back to the vanilla model, with one diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
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
    /// Engine transport constructor; mods obtain references from the reload registrar.
    // hidden: engine transport only; mods use the resource-load registrar
    #[doc(hidden)]
    pub const fn __new(generation: u64, index: u32) -> Self {
        Self { generation, index }
    }
    /// Engine transport identity.
    // hidden: engine transport only; mods use the resource-load registrar
    #[doc(hidden)]
    pub const fn __parts(self) -> (u64, u32) {
        (self.generation, self.index)
    }
}

/// A texture resolved against this resource load's atlas.
/// A stale reference falls back to the supplied vanilla/CIT texture, with one diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: only the engine constructs resource-load references
pub struct ArmorTextureRef {
    generation: u64,
    index: u32,
}
impl ArmorTextureRef {
    /// Engine transport constructor; mods obtain references from the reload registrar.
    // hidden: engine transport only; mods use the resource-load registrar
    #[doc(hidden)]
    pub const fn __new(generation: u64, index: u32) -> Self {
        Self { generation, index }
    }
    /// Engine transport identity.
    // hidden: engine transport only; mods use the resource-load registrar
    #[doc(hidden)]
    pub const fn __parts(self) -> (u64, u32) {
        (self.generation, self.index)
    }
}

/// An API-owned snapshot of one material layer, in asset order.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct EquipmentLayer {
    /// The layer's default texture binding.
    pub texture: ArmorTextureRef,
    /// Whether this layer takes the stack's dye colour.
    pub dyeable: bool,
    /// Packed RGB used when no dyed colour is present. None skips an undyed dyeable layer.
    pub color_when_undyed: Option<u32>,
    /// Whether the equipment asset asks for the player's own texture.
    pub use_player_texture: bool,
}
impl EquipmentLayer {
    /// Build a layer snapshot at the engine boundary.
    // hidden: engine transport only; mods use the resource-load registrar
    #[doc(hidden)]
    pub fn __new(
        texture: ArmorTextureRef,
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

/// The single provider attached to one item for worn equipment rendering.
/// Only visible equipped pieces which pass the equippable slot/asset gate are queried.
/// Register trees and textures through the reload registrar and use the references it returns.
/// If the callback panics, the game discards every layer of the piece, draws it with vanilla
/// behaviour, and disables the provider until reload.
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
        stack: &ItemStack,
        layer_type: EquipmentLayerType,
        layer: &EquipmentLayer,
        index: u32,
    ) -> ArmorModelRef {
        let _ = (host, stack, layer_type, layer, index);
        ArmorModelRef::VANILLA
    }
    /// The stack's dyed colour as packed ARGB, or zero when undyed.
    fn default_dye_color(&self, host: &dyn ClientHost, stack: &ItemStack) -> u32 {
        let _ = host;
        stack
            .components()
            .and_then(|c| {
                // Component maps are small; UTF-16 comparison avoids allocating a lookup key each frame.
                c.as_nbt().iter().find_map(|(id, tag)| {
                    id.0.iter()
                        .copied()
                        .eq("minecraft:dyed_color".encode_utf16())
                        .then_some(tag)
                })
            })
            .and_then(|tag| match tag {
                crate::Tag::Int(rgb) => Some(*rgb as u32 | 0xFF00_0000),
                _ => None,
            })
            .unwrap_or(0)
    }
    /// Packed ARGB for this material; exactly zero skips texture and model callbacks.
    /// Asked before either callback. Nondyeable materials are white by default.
    fn armor_layer_tint_color(
        &self,
        host: &dyn ClientHost,
        stack: &ItemStack,
        layer: &EquipmentLayer,
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
    /// Choose this layer's texture. `default` already includes pack/CIT selection.
    /// A returned override is final; stale references fall back to `default`.
    fn armor_texture(
        &self,
        host: &dyn ClientHost,
        stack: &ItemStack,
        layer_type: EquipmentLayerType,
        layer: &EquipmentLayer,
        default: ArmorTextureRef,
    ) -> ArmorTextureRef {
        let _ = (host, stack, layer_type, layer);
        default
    }
}

/// Client resource-load registration. Models, textures and item providers refresh together.
/// No registration carries over to the next resource load.
pub trait ClientItemExtensionsRegistrar {
    /// Register a named ModelPart tree, validated and baked once for this resource load.
    /// Malformed trees and duplicate names are refused by name.
    fn register_model(&mut self, name: &str, model: &ModelPart) -> Result<ArmorModelRef, String>;
    /// Bind an atlas sprite by name. An unknown texture refuses registration by name.
    fn register_texture(&mut self, name: &str) -> Result<ArmorTextureRef, String>;
    /// Attach the one provider for this item. Duplicates name both providers.
    fn client_item_extensions(
        &mut self,
        item: ItemKey,
        provider: Arc<dyn ClientItemExtensions>,
    ) -> Result<(), String>;
}
