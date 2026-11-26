//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's registry-wide key reads of the `keys` area, implemented by the game
/// and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait KeysHost {
    /// Side: server. Resolve a biome by canonical namespaced id (`minecraft:plains`) in the
    /// installed world-data generation as a scoped [`BiomeView`]. Returns
    /// [`BiomeKeyError::Unavailable`] on a client or before world data is installed,
    /// [`BiomeKeyError::InvalidIdentifier`] for an invalid id, and
    /// [`BiomeKeyError::UnknownBiome`] for an id absent from the installed generation.
    fn biome(&self, id: &str) -> Result<BiomeView<'_>, BiomeKeyError>;

    /// Side: both. Exact startup game-event id; None before registry completion or for an unknown id.
    fn game_event_key(&self, id: &str) -> Option<GameEventKey>;
    /// Side: both.
    /// The key of the block named `id`, for a name that only arrives at run time (data, config,
    /// a command, a network message). `id` is the full registry name, `minecraft:stone` or
    /// `mymod:machine`. None for a name the registry does not hold, and before the registry is
    /// complete (during registration, use `Registrar::block_key`). Does not allocate.
    fn block_key(&self, id: &str) -> Option<BlockKey>;
    /// Side: both.
    /// The key of the effect named `id`: the full registry name, `minecraft:speed` or
    /// `mymod:rage`. None for a name the registry does not hold, and before the registry is
    /// complete (during registration, use `Registrar::effect_key`). Does not allocate.
    fn effect_key(&self, id: &str) -> Option<EffectKey>;
    /// Side: both.
    /// Resolve a canonical entry in the current side's live registry. None for an
    /// unknown registry or entry, or before that registry is available.
    ///
    /// `minecraft:worldgen/world_preset` has no published complete registry owner, so this
    /// returns `None` for it even when prepared preset files exist or a world selected that
    /// preset.
    fn registry_entry(&self, registry: &str, id: &str) -> Option<RegistryEntry>;
    /// Side: both.
    /// Visit the canonical entry IDs in the current side's registry, in the table's existing
    /// order: numeric IDs for protocol tables, lexical order for name-keyed local tables.
    /// IDs are borrowed for each call; use [`Identifier::parse`] if you need to retain a typed ID.
    /// Return `ControlFlow::Break(())` to stop, or `ControlFlow::Continue(())` to keep visiting.
    /// Returns `Ok(Continue(()))` on completion, including for an empty registry, or
    /// `Ok(Break(()))` on early stop. An unknown registry or one not exposed on this side
    /// returns [`RegistryReadError::UnknownRegistry`]. Without a live session, returns [`RegistryReadError::Unavailable`]. Neither calls `visit`.
    fn visit_registry_entries(
        &self,
        registry: &str,
        visit: &mut dyn FnMut(&str) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, RegistryReadError>;
    /// Side: both.
    /// Membership in the current side's merged tags. None for an unavailable registry or
    /// entry; an absent tag, or a tag not containing the entry, returns false.
    fn registry_entry_in_tag(&self, entry: &RegistryEntry, tag: &str) -> Option<bool>;
    /// Side: both.
    /// Every block key in the registry's registration order: vanilla's first, then
    /// mod blocks in the order they registered. A block registry lists a name once
    /// per state; this yields it once. Empty before the block table completes.
    fn block_keys(&self) -> Vec<BlockKey>;
    /// Side: both.
    /// Every item key in the registry's registration order: vanilla's first, then
    /// mod items in the order they registered. Empty before the item table completes.
    fn item_keys(&self) -> Vec<ItemKey>;
    /// Side: both.
    /// Every entity-type key: built-in and content types in registry order
    /// (`BuiltInRegistries.ENTITY_TYPE`), then native general types ordered by
    /// declaring mod and type id.
    fn entity_type_keys(&self) -> Vec<EntityTypeKey>;
    /// Side: both.
    /// Every built-in menu-type key in registry order (`BuiltInRegistries.MENU` order):
    /// vanilla's first, then mod types in the order they registered.
    fn menu_keys(&self) -> Vec<MenuKey>;
    /// Side: both.
    /// Every enchantment key in global sorted loaded-registry order, including
    /// vanilla and mod data-pack entries together (`RegistryDataLoader` sorts by id).
    fn enchantment_keys(&self) -> Vec<EnchantmentKey>;
    /// Side: both.
    /// The network id of `name` in `registry` (`Registry.getId`): the number a packet carries for
    /// it, for a mod that builds or reads raw packets (`minecraft:particle_type` and
    /// `minecraft:flame`). Built-in registries answer their fixed ids, mod entries included;
    /// data-driven ones (`minecraft:damage_type`, `minecraft:biome`, ...) answer the order the
    /// server synced them in; custom data component types are included. A client reads its own
    /// connection's registries, a server the ones it sends. The id holds for one connection and one
    /// data reload: do not keep it across a reconnect, a reconfiguration or `/reload`. None for an
    /// unknown registry or name, on a client before that registry was synced, and with no
    /// connection. Allocates nothing; each registry is searched in order. Use
    /// [`ItemKey::registry_id`] for the local prepared item number used by scatter rendering;
    /// that read is available without a connection and is not a packet lookup.
    fn registry_protocol_id(&self, registry: &Identifier, name: &Identifier) -> Option<i32>;
}
/// The handles' reads of the `keys` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait KeysBackend {
    /// Side: server. Resolve a persistent [`BiomeKey`] in the installed world-data generation
    /// as a scoped [`BiomeView`].
    fn biome_view(&self, key: &BiomeKey) -> Result<BiomeView<'_>, BiomeKeyError>;
    /// Side: server. Validate a scoped [`BiomeView`] against this host's installed world-data
    /// generation and borrow its canonical id.
    fn biome_view_id(&self, view: BiomeView<'_>) -> Result<&str, BiomeKeyError>;

    /// Side: both.
    /// Resolve a live typed key's load-owned slot, never its protocol numeric id.
    fn registry_entry_key_slot(&self, entry: &RegistryEntry) -> Option<u32>;

    /// Side: both. Pin the exact id of an installed game-event slot; invalid keys are refused.
    fn game_event_key_id(&self, key: GameEventKey) -> Result<RegistryName, GameEventKeyError>;
    /// Side: both.
    /// Borrows the canonical block id from the registry. `None` if the host has no
    /// backend or no name for the slot.
    fn block_key_id(&self, key: BlockKey) -> Option<&str>;
    /// Side: both.
    /// Borrows the canonical item id from the registry. `None` if the host has no
    /// backend or no name for the slot.
    fn item_key_id(&self, key: ItemKey) -> Option<&str>;
    /// Side: both. Current numeric item registry id, unavailable before completion.
    fn item_registry_id(&self, key: ItemKey) -> Option<u32>;
    /// Side: both.
    /// Borrows the canonical effect id from the registry. `None` if the host has no
    /// backend or no name for the slot.
    fn effect_key_id(&self, key: EffectKey) -> Option<&str>;
    /// Side: both. Registered MobEffect.getCategory(); unavailable before registry completion.
    fn effect_category(&self, key: EffectKey) -> Option<EffectCategory>;
    /// Side: both.
    /// Borrows the canonical fluid id, such as `minecraft:water`, from the completed registry.
    /// `None` if the host has no backend or no name for the slot.
    fn fluid_key_id(&self, key: FluidKey) -> Option<&str>;
    /// Side: both.
    /// Borrows the canonical block entity type id from the registry. `None` if the host
    /// has no backend or no name for the slot.
    fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> Option<&str>;
    /// Side: both.
    /// Pins the canonical entity type id without borrowing the host.
    /// Returns `None` if the host has no backend or no name for the slot.
    fn entity_type_key_id(&self, key: EntityTypeKey) -> Option<RegistryName>;
    /// Side: both.
    /// The category of a registered entity type with no entity spawned
    /// (`EntityType.getCategory`): a mob type's declared category, a mod mob's included,
    /// `Monster` for the ender dragon and `Misc` for every other type. `None` if the host has
    /// no backend or no name for the slot.
    fn entity_type_category(&self, key: EntityTypeKey) -> Option<MobCategory>;
    /// Side: both.
    /// Pins the canonical menu type id without borrowing the host.
    /// Returns `None` if the host has no backend or no name for the slot.
    fn menu_key_id(&self, key: MenuKey) -> Option<RegistryName>;
    /// Side: both.
    /// Pins the canonical enchantment id without borrowing the host.
    /// Returns `None` if the host has no backend or no name for the slot.
    fn enchantment_key_id(&self, key: EnchantmentKey) -> Option<RegistryName>;
}
