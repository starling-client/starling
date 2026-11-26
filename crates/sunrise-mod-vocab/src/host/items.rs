//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `items` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait ItemsHost {
    /// Side: both.
    /// Registered enchantments (`Registries.ENCHANTMENT`) in registry order, with `max_level`
    /// (`Enchantment.getMaxLevel`), selection `weight`, `anvil_cost` (the base anvil fee per
    /// enchantment level) and whether each is a curse. A server answers from its own registry,
    /// data packs included; a client from the server's synced entries when it has them, and
    /// the local table otherwise. The other enchantment reads use the same table.
    fn enchantments(&self) -> Vec<EnchantmentInfo>;
    /// Side: both.
    /// `Enchantment.getMinCost(level)`: the enchanting power needed, or None for an unknown id.
    fn enchantment_min_cost(&self, id: &str, level: u32) -> Option<i32>;
    /// Side: both.
    /// Whether `item` is in enchantment `id`'s `supported_items` (`Enchantment.isSupportedItem`);
    /// false for an unknown id. Does not consult Item Part overrides.
    fn enchantment_supports(&self, id: &str, item: ItemKey) -> bool;
    /// Side: both.
    /// The most of the item `item` one slot holds (`Item.getDefaultMaxStackSize`),
    /// or `None` for an unknown item: for a mod's container that fills or
    /// merges stacks itself.
    fn max_stack_size(&self, item: ItemKey) -> Option<u32>;
    /// Side: both.
    /// Default durability of the resolved item, or none if its metadata is unavailable.
    /// Stack-specific component overrides are checked at the game's write boundary.
    fn max_damage(&self, item: ItemKey) -> Option<u32>;
    /// Side: both.
    /// Resolve an item once at load, including vanilla items, to its process-local identity.
    /// None if the completed registry does not contain the identifier.
    fn item_key(&self, id: &Identifier) -> Option<ItemKey>;
    /// Side: both.
    /// Item `item`'s default value of component `component`, in its prototype
    /// (`Item.components().get`): `minecraft:max_damage` on an iron sword is `250`, data
    /// packs' and native mods' changes included. Read from the item report's JSON, so a
    /// number is an `Int`, `Long` or `Double` and a boolean a `Byte`. None where the item
    /// hasn't the component, and for an unknown item.
    fn item_default_component(&self, item: ItemKey, component: &str) -> Option<Tag>;
    /// Side: both.
    /// `ItemStack.getHoverName()` as a component. The custom name wins, then a written book's
    /// nonblank raw title. Otherwise the item's name is used: a mod item's `ItemPart::item_name`
    /// answer replaces its default; `None` keeps it. Vanilla item overrides (such as potion
    /// contents or a profiled player head) participate in the item-name step; the base item uses
    /// the stack's `item_name` patch or inherits its prototype's value. A removal in the
    /// stack's patch reads empty; if a mod item's prototype has no `item_name` (including
    /// when the mod removes it), the game uses its item or block translation key. Only a
    /// fallback written into a component is kept.
    /// It carries no rarity colour or italics (`getStyledHoverName` adds those). Each viewer
    /// renders a translatable in their own language; call `text_string` on it for a string in
    /// this side's language. An invalid stack is a [`TextFormError`].
    fn hover_name(&self, stack: &ItemStack) -> Result<Text, TextFormError>;
    /// Side: both.
    /// Item `item`'s cooking-fuel burn time in ticks (`FuelValues.burnDuration` of its `cooking_fuel`
    /// component, the data pack's context int provider resolved), None where it is no fuel. API
    /// @API:item_components@.
    fn item_burn_time(&self, item: ItemKey) -> Option<i32>;
    /// Side: both.
    /// Item `item`'s chance to raise a composter's level (its `compostable` component's layers provider
    /// resolved), None where it is not compostable. API @API:item_components@.
    fn item_compost_chance(&self, item: ItemKey) -> Option<f32>;
    /// Side: both.
    /// Whether enchantments `a` and `b` may share an item (`Enchantment.areCompatible`), by the
    /// world's enchantment definitions. API @API:item_components@.
    fn enchantments_compatible(&self, a: &str, b: &str) -> bool;
    /// Side: both.
    /// Every item ability the game asks `can_perform_action` about (NeoForge's
    /// `ItemAbility.getActions`): plain names such as `sword_sweep` and `shears_trim`,
    /// sorted. A mod's own abilities are ids it owns and are not listed.
    fn item_abilities(&self) -> Vec<String>;
    /// Side: both.
    /// `EntityType.is(TagKey)`: whether `entity_type` (`minecraft:player`) is in the entity
    /// type tag `tag` (`minecraft:cannot_be_pushed_onto_boats`).
    fn entity_type_in_tag(&self, entity_type: &str, tag: &str) -> bool;
    /// Side: both.
    /// An entity type's default value of an attribute, with no entity spawned
    /// (`DefaultAttributes.getSupplier(type).getValue(attribute)`): what a new entity of
    /// `entity_type` (`minecraft:zombie`) has for `attribute` (`minecraft:max_health`) before
    /// any modifier, its supplier's base inside the attribute's range, for vanilla's types and
    /// mods' (a mod type's declared row, NeoForge's `EntityAttributeCreationEvent` and
    /// `EntityAttributeModificationEvent` included). Both names are read as `Identifier.parse`
    /// reads them: `max_health` and `:max_health` are `minecraft:`'s. The Host and typed
    /// reload/setup contexts share this read on both sides; it never unwinds. `None` before
    /// publication, for an unregistered type or one with no supplier (such as `minecraft:item`),
    /// or for a missing attribute.
    #[published(entity_type_default_attribute)]
    fn entity_type_default_attribute(&self, entity_type: &str, attribute: &str) -> Option<f64>;
    /// Side: both.
    /// Parse one complete SNBT compound as a saved `ItemStack.CODEC` item:
    /// `{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5}}`
    /// (`stone` is `minecraft:stone`). `count` defaults to 1; a present number must be in
    /// 1..=99 after Java’s `Number.intValue`. Unknown fields such as `Count` are ignored.
    /// Trailing whitespace is allowed. The shared SNBT parser's structural rules apply,
    /// but no binary NBT byte quota is applied to the text. Bad syntax, an invalid item or
    /// component patch, and an unknown item return `None` and log the failure reason.
    #[forward_api_output]
    fn item_from_snbt(&self, snbt: &str) -> Option<ItemStack>;
    /// Side: both.
    /// Encode an optional item stack as JSON with vanilla's `ItemStack.OPTIONAL_CODEC`
    /// (`ItemStack.CODEC` wrapped in `ExtraCodecs.optionalEmptyMap`) over `JsonOps`:
    /// `None` (an empty stack) encodes as `{}`, and `Some(stack)` encodes as
    /// `{"id":"...","count":N}` with `"components":{...}` when the stack carries non-default
    /// persistent components or explicit removals (`"!minecraft:damage":{}`), resolved against
    /// this side's item and component registries.
    ///
    /// Each component uses its codec's JSON form: real JSON booleans, integers versus floats,
    /// `ComponentSerialization.CODEC` text components, canonical identifiers, and nested
    /// `ItemStackTemplate` objects. Transient components are omitted and prototype-equal
    /// components are elided. Returns [`ItemJsonError`] naming the failing field when the item
    /// is unknown or air, `count` is outside `1..=99`, or a component cannot be encoded.
    #[forward_api_inputs]
    fn item_to_json(&self, stack: Option<&ItemStack>) -> Result<String, ItemJsonError>;
    /// Side: both.
    /// Decode one complete JSON value as an optional item stack with vanilla's
    /// `ItemStack.OPTIONAL_CODEC` (`ItemStack.CODEC` wrapped in `ExtraCodecs.optionalEmptyMap`)
    /// over `JsonOps`: `{}` decodes as `Ok(None)`, and
    /// `{"id":"...","count":N,"components":{...}}` decodes as `Ok(Some(stack))`, resolving
    /// item identifiers (outer and nested) and persistent component types against this side's
    /// registries. `count` defaults to 1 and must be in `1..=99` after Java's `Number.intValue`;
    /// bare identifiers default to `minecraft:`; component removals use `"!id":{}`.
    ///
    /// Follows `JsonOps` type rules: booleans require JSON booleans, numbers require JSON
    /// numbers, `Unit` and removal payloads require JSON objects, and JSON `null` in optional
    /// record fields is treated as absent. Returns [`ItemJsonError`] naming the failing field on
    /// bad JSON, an unknown item or component, or a malformed value. Does not enforce resident
    /// `max_stack_size` limits beyond `1..=99`; write boundaries check those separately.
    #[forward_api_output]
    fn item_from_json(&self, json: &str) -> Result<Option<ItemStack>, ItemJsonError>;
    /// Side: both. Read the creative catalog against this world's active holders and flags.
    /// On a cache miss, build it with explicit `permissions` and dispatch
    /// `build_creative_mode_tab_contents`. It does not require an open creative screen.
    /// A same-parameter read from a listener sees tabs already built and the previous
    /// contents of tabs not yet built. Reentering with different parameters is refused.
    /// Published snapshots stay immutable. Permission is a build parameter, not a grant.
    /// During setup, reload, or without a live world, returns [`CreativeTabReadError::Unavailable`].
    fn creative_tabs(
        &mut self,
        permissions: CreativeTabPermissions,
    ) -> Result<std::sync::Arc<CreativeModeTabs>, CreativeTabReadError>;
    /// Side: both. The catalog this side last finished building, as
    /// `CreativeModeTab.getDisplayItems` reads it, for a reader that only has `&dyn Host`.
    /// It never builds and never fires `build_creative_mode_tab_contents`. `Ok(None)` before
    /// this side has built once (a read through [`creative_tabs`](Self::creative_tabs), or the
    /// creative screen, builds it); [`CreativeTabReadError::Unavailable`] in setup or reload
    /// and without a live world. The snapshot keeps the permissions of that build and stays
    /// immutable: a later build publishes a new one.
    fn built_creative_tabs(
        &self,
    ) -> Result<Option<std::sync::Arc<CreativeModeTabs>>, CreativeTabReadError>;
}
/// The client host's methods of the `items` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ItemsClientHost {
    /// Side: client.
    /// The creative search tab's final display order, for ranking stacks in inventory sorters.
    /// This requests the same catalog as the creative screen (using the player's permission and
    /// Operator Items Tab option). On a miss it dispatches the creative build hook. During setup,
    /// reload, a different build, or without a live catalog, this returns an empty list.
    fn creative_search_items(&mut self) -> Vec<ItemStack>;
    /// Side: client.
    /// Registered potions (`Registries.POTION`), in registry order, each by its registry id:
    /// vanilla's 46, from `minecraft:water` to `minecraft:infested` (content mods cannot declare
    /// potions yet). The game builds the list once; each call copies it. A mod builds the potion, splash,
    /// lingering or tipped-arrow stack itself with the `minecraft:potion_contents` component.
    fn potions(&self) -> Vec<PotionInfo>;
}
/// The handles' reads of the `items` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait ItemsBackend {
    /// Side: both. Normalize a creative membership key against the game's item prototypes.
    fn creative_stack_key(&self, stack: &ItemStack) -> Option<ItemStack>;

    /// Side: both. Effective damageability after defaults and the stack patch.
    fn item_resource_damageable(&self, stack: &ItemStack) -> bool;
    /// Side: both. Effective glint, including overrides and item-specific foil.
    fn item_resource_foil(&self, stack: &ItemStack) -> bool;
    /// Side: both. Effective repair ingredient membership, including item tags.
    fn item_resource_repairable(&self, stack: &ItemStack, material: ItemKey) -> bool;

    /// Side: both. ItemResource's effective use duration, including its Part.
    fn item_resource_use_duration(&self, stack: &ItemStack) -> Option<i32>;
    /// Side: both. `ItemStack.getUseAnimation` after the mods' `use_animation` answers; none
    /// for an unknown item.
    fn item_stack_use_animation(&self, stack: &ItemStack) -> Option<UseAnimation>;
    /// Side: both. The mod that made the stack, including its Part.
    fn item_resource_creator_mod_id(&self, stack: &ItemStack) -> Option<String>;
    /// Side: both. The effective vanilla bar, before client drawing hooks.
    fn item_resource_bar(&self, stack: &ItemStack) -> Option<ItemBar>;
    /// Side: both. Effective stack range for the supplied actor.
    fn item_resource_entity_attack_range(
        &self,
        stack: &ItemStack,
        attacker: LivingRef,
    ) -> Option<ItemAttackRange>;
    /// Side: both. Effective stack melee source for the supplied actor.
    fn item_resource_damage_source(
        &self,
        stack: &ItemStack,
        attacker: LivingRef,
    ) -> Option<ItemDamageSource>;
    /// Side: both. Item immunity to the supplied damage source.
    fn item_resource_can_be_hurt_by(
        &self,
        stack: &ItemStack,
        source: &ItemDamageSource,
    ) -> Option<bool>;
    /// Side: both. Attack bonus preview without consuming world RNG or equipping the stack.
    fn item_resource_attack_damage_bonus(
        &self,
        stack: &ItemStack,
        attacker: LivingRef,
        target: EntityRef,
        base_damage: f32,
    ) -> Option<f32>;
    /// Side: client. Inventory tooltip lines including mod edits, before drawing.
    /// A request during item tooltip line construction returns base lines without dispatching
    /// line callbacks again. No request gathers images or draws.
    fn item_resource_tooltip(
        &self,
        stack: &ItemStack,
        advanced: bool,
        creative: bool,
    ) -> Option<Vec<TooltipElement>>;
    /// Side: server.
    /// How many container slots `at`'s side reaches (none: every slot), by its
    /// `WorldlyContainer` rules: a furnace's top reaches its input only. A composter's up,
    /// down and unsided ports each expose one state-backed slot. None without a supported
    /// container/port, in a level not in place, and on a client's copy of the world.
    fn container_size(&self, at: BlockSide) -> Option<usize>;
    /// Side: server.
    /// Lend the resident stack once in side-local `slot` among those `at` reaches
    /// ([`Self::container_size`]). For an empty, unreadable or unavailable slot, return
    /// without invoking the reader. Never generates loot or loads chunks.
    #[forward_api_inputs]
    #[decline_unavailable]
    fn lend_container_stack(
        &self,
        at: BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    );
    /// Side: server.
    /// The backing slot of slot `slot` of those `at`'s side reaches (`container_size`):
    /// `WorldlyContainer.getSlotsForFace` picks the slot and `ChestBlock.getContainer` joins a
    /// double chest's halves, and each slot reports the half or block entity that owns it
    /// (matching `View::locate`, `CompoundContainer` and `MenuRef::slot_backing`), with that
    /// block entity's own unsided slot index. A furnace's sides that reach one slot report the
    /// same `StorageSlot`. A composter has no backing container: vanilla builds one per access
    /// (`ComposterBlock.getContainer`), so it returns `None` on every face. `None` as well where
    /// `container_size` is `None` or `slot` is past it.
    fn container_slot_backing(&self, at: BlockSide, slot: usize) -> Option<StorageSlot>;
    /// Side: server.
    /// How many of `stack` slot `slot` of those `at`'s side reaches holds when empty, as
    /// NeoForge's `isValid` and `getCapacityAsLong` answer: none where it may not go in (a
    /// furnace's output, `canPlaceItem`), else the slot's limit for it. `None` asks for the
    /// general slot limit before item-specific rules.
    fn container_capacity(&self, at: BlockSide, slot: usize, stack: Option<&ItemStack>) -> u32;
    /// Side: server.
    /// How much of `resource` the slots `at`'s side reaches hold in all: [`STORAGE`]'s
    /// amount for a container or composter port, in one pass. None where the total does
    /// not fit in 64 bits; 0 where no supported container or port exists.
    ///
    /// [`STORAGE`]: crate::STORAGE
    fn container_kind_amount(&self, at: BlockSide, resource: &ItemResource) -> Option<u64>;
    /// Side: server.
    /// How much of `resource` an insert through `at`'s side would take now: the room left in
    /// the slots holding it and in the empty slots it may go in, in one pass. None where the
    /// total does not fit in 64 bits.
    fn container_kind_space(&self, at: BlockSide, resource: &ItemResource) -> Option<u64>;
    /// Side: server.
    /// Visits the occupied slots `at`'s side reaches, in sided order, including duplicate
    /// kinds. Stops before reading another slot when the visitor breaks. If no supported
    /// container or composter port exists, there are no entries to visit. Refuses the read
    /// if a visited stack cannot be represented as an API value.
    fn container_visit_entries(
        &self,
        at: BlockSide,
        visitor: &mut dyn FnMut(&ItemResource, u64) -> std::ops::ControlFlow<()>,
    ) -> Result<(), LevelReadError>;
    /// Side: both.
    /// Whether an enchantment on `stack` has effect component `effect`, by the world's
    /// enchantment definitions (`EnchantmentHelper.has`), behind
    /// `ItemResource::has_enchantment_effect`, through the opaque backend. API
    /// @API:item_components@.
    fn item_has_enchantment_effect(&self, stack: &ItemStack, effect: &str) -> bool;
    /// Side: both.
    /// The slot `stack` is equipped to (`Equippable.slot`) after its component patch, behind
    /// `ItemResource::equippable_slot`, through the opaque backend. API @API:item_components@.
    fn item_equippable_slot(&self, stack: &ItemStack) -> Option<String>;
    /// Side: both.
    /// `Capabilities.Item.ITEM`, read: the slots of one item of `stack`,
    /// empty ones included, kept in its `minecraft:container`. None for an
    /// item without slots, and for an empty stack.
    fn item_container_slots(&self, stack: &ItemStack) -> Option<Vec<Option<ItemStack>>>;
    /// Side: both.
    /// `ResourceHandler.insert`: as much of `item` as fits into one item of
    /// `stack`, topping up the slots that hold the same item and components
    /// first, then the first empty ones (`SimpleContainer.addItem`'s order),
    /// no slot past its limit (99, or the item's own when less). A shulker
    /// box never goes in. The count that went in, and the single item it
    /// becomes (the item itself when none did). The stack given is not
    /// changed: shrink it by one and hand out the result, or leave the result
    /// unused to simulate. None where the item has no slots.
    fn item_container_insert(
        &self,
        stack: &ItemStack,
        item: &ItemStack,
    ) -> Option<(u32, ItemStack)>;
    /// Side: both.
    /// `ResourceHandler.extract`: up to `max` out of slot `slot` of one item
    /// of `stack`. What came out (none when the slot is empty or past the
    /// item's slots), and the single item it becomes. The stack given is not
    /// changed, as with `native_item_container_insert`. None where the item
    /// has no slots.
    fn item_container_extract(
        &self,
        stack: &ItemStack,
        slot: usize,
        max: u32,
    ) -> Option<(Option<ItemStack>, ItemStack)>;
    /// Side: both.
    /// `ResourceHandler.insert(index, …)`: as `native_item_container_insert`,
    /// into slot `slot` alone: topped up when it holds the same item and
    /// components, filled when empty. Nothing goes into a slot past the
    /// item's slots or holding another item.
    fn item_container_insert_slot(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
    ) -> Option<(u32, ItemStack)>;
    /// Side: both.
    /// `ResourceHandler.isValid` and `getCapacityAsLong`: how many of `item`
    /// slot `slot` of one item of `stack` holds when empty. None of a shulker
    /// box, nor into a slot past the item's slots.
    fn item_container_capacity(&self, stack: &ItemStack, slot: usize, item: &ItemStack) -> u32;
    /// Side: both.
    /// The stacks in one item of `stack`'s configured bundle component, front first (the
    /// stack the container gives back next is slot 0). `Ok(None)` for an item without the
    /// component; `Err(BundleReadError::UnsupportedFormat)` if the custom component (or a nested
    /// container carrying the same component) is not a `Tag::List` of `ItemStack.CODEC` entries;
    /// `Err(BundleReadError::ExcessiveWeight)` if its weight overflows vanilla's `Fraction`.
    fn bundle_slots(
        &self,
        stack: &ItemStack,
        config: &BundleConfig,
    ) -> Result<Option<Vec<Option<ItemStack>>>, BundleReadError>;
    /// Side: both.
    /// `ResourceHandler.isValid` and `getCapacityAsLong` for a bundle-like container: how many of
    /// `item` one item of `stack` still takes, by exact fraction weight under `config`'s
    /// multiplier, nested occupancy, container-fit rules, and optional `filter_tag`; `Ok(0)` for
    /// an item refused or without the component; `Err(BundleReadError::UnsupportedFormat)` if
    /// `stack`, a nested container inside it, or `item` carries a malformed custom component
    /// payload; `Err(BundleReadError::ExcessiveWeight)` if a weight or the room left overflows
    /// vanilla's `Fraction`.
    fn bundle_capacity(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
        config: &BundleConfig,
    ) -> Result<u32, BundleReadError>;
    /// Side: both.
    /// `ResourceHandler.insert` for a bundle-like container: as much of `item` as fits goes in at
    /// the front under `config`. Gives how many went in and the changed single container stack;
    /// `Ok(None)` without the component; `Err(BundleReadError::UnsupportedFormat)` without
    /// modifying the stack if `stack`, a nested container inside it, or `item` carries a malformed
    /// custom component payload; `Err(BundleReadError::ExcessiveWeight)`, likewise, if a weight
    /// overflows vanilla's `Fraction`.
    fn bundle_insert_slot(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
        config: &BundleConfig,
    ) -> Result<Option<(u32, ItemStack)>, BundleReadError>;
    /// Side: both.
    /// `ResourceHandler.extract` for a bundle-like container: up to `max` of the front stack
    /// (slot 0 only) under `config`. Gives the taken stack, if any, and the changed single
    /// container stack; `Ok(None)` without the component; `Err(BundleReadError::UnsupportedFormat)`
    /// without modifying the stack if the custom component (or a nested container carrying the same
    /// component) carries a malformed payload; `Err(BundleReadError::ExcessiveWeight)`, likewise,
    /// if a weight overflows vanilla's `Fraction`.
    fn bundle_extract(
        &self,
        stack: &ItemStack,
        slot: usize,
        max: u32,
        config: &BundleConfig,
    ) -> Result<Option<(Option<ItemStack>, ItemStack)>, BundleReadError>;
    /// Side: both.
    /// Exact fullness, full flag, vanilla-style fullness bar `(width, rgb)`, and stored stack
    /// count for one item of `stack` under `config`; `Ok(None)` without the component;
    /// `Err(BundleReadError::UnsupportedFormat)` if the custom component (or a nested container
    /// carrying the same component) carries a malformed payload;
    /// `Err(BundleReadError::ExcessiveWeight)` if its weight overflows vanilla's `Fraction`. All
    /// four come from the one weight computed as the contents are read; nothing is cached
    /// between calls.
    fn bundle_metrics(
        &self,
        stack: &ItemStack,
        config: &BundleConfig,
    ) -> Result<Option<BundleMetrics>, BundleReadError>;
    /// Side: server.
    /// How many slots the inventory of entity `entity` in the level in place has, as
    /// `entity_slots` reaches them: none for an entity without one, and on a client.
    fn entity_container_size(&self, entity: u32) -> Option<usize>;
    /// Side: server.
    /// How many slots the container entity `entity` of the level in place shows automation
    /// (`EntitySelector.CONTAINER_ENTITY_SELECTOR`: chest and hopper minecarts, chest boats; a
    /// chested mount is not one): none for any other entity, and on a client.
    fn entity_automation_size(&self, entity: u32) -> Option<usize>;
    /// Side: server.
    /// How many equipment slots living entity `entity`'s [`ENTITY_ITEMS`](crate::ENTITY_ITEMS)
    /// answers (NeoForge's `LivingEntityEquipmentWrapper`): a mob's or an armor stand's
    /// seven (main hand, off hand, feet, legs, chest, head and body, by `EquipmentSlot`'s
    /// ordinal; the saddle is not automation's to move), none for any other entity, and on a
    /// client.
    fn entity_equipment_size(&self, entity: u32) -> Option<usize>;
}
/// The handles' writes of the `items` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait ItemsBackendMut {
    /// Side: server. Set a persisted container limit in 1..=99 under the transaction.
    fn container_set_stack_limit(
        &mut self,
        at: BlockSide,
        limit: u32,
        tx: &TransactionContext,
    ) -> bool;

    /// Side: server. Wear a stack nobody wears, behind [`ItemStack::hurt_and_break`]
    /// (`ItemStack.hurtAndBreak(amount, level, player, onBreak)`).
    fn item_stack_hurt_and_break(
        &mut self,
        stack: &ItemStack,
        level: LevelRef,
        amount: i32,
        player: Option<PlayerRef>,
    ) -> Result<Option<ItemStack>, StackWriteError>;
    /// Side: server. Roll an enchantment result with the named level's random source.
    fn item_resource_enchant_randomly(
        &mut self,
        stack: &ItemStack,
        level: LevelRef,
        power: i32,
        pool: &Identifier,
    ) -> Result<Option<ItemStack>, StackWriteError>;
    /// Side: server. The durability `stack`'s enchantments repair for `points` experience points
    /// (`EnchantmentHelper.modifyDurabilityToRepairFromXp`), drawing from the active level's
    /// random where an effect's conditions do.
    fn item_resource_durability_to_repair_from_xp(&mut self, stack: &ItemStack, points: i32)
    -> u32;
    /// Side: server.
    /// Puts up to `amount` of `resource` into slot `slot` of those `at`'s side reaches, as a hopper
    /// would (`canPlaceItem`, the slot's limit, the same item and components), in transaction
    /// `tx`; what didn't fit.
    fn container_insert(
        &mut self,
        at: BlockSide,
        slot: usize,
        resource: &ItemResource,
        amount: u32,
        tx: &TransactionContext,
    ) -> Result<Option<ItemStack>, StackWriteError>;
    /// Side: server.
    /// Takes up to `max` out of slot `slot` of those `at`'s side reaches, in transaction `tx`;
    /// through a side, only what that side may take (`canTakeItemThroughFace`). What came out.
    fn container_extract(
        &mut self,
        at: BlockSide,
        slot: usize,
        max: u32,
        tx: &TransactionContext,
    ) -> Option<ItemStack>;
    /// Side: server.
    /// Puts up to `amount` of `resource` into the slots `at`'s side reaches, as hoppers fill
    /// them, the slots holding it first and then the empty ones, in transaction `tx`, in one
    /// pass: how much went in.
    fn container_insert_kind(
        &mut self,
        at: BlockSide,
        resource: &ItemResource,
        amount: u64,
        tx: &TransactionContext,
    ) -> u64;
    /// Side: server.
    /// Takes up to `amount` of `resource` out of the slots `at`'s side reaches and may take
    /// from, in transaction `tx`, in one pass: how much came out.
    fn container_extract_kind(
        &mut self,
        at: BlockSide,
        resource: &ItemResource,
        amount: u64,
        tx: &TransactionContext,
    ) -> u64;
    /// Side: server.
    /// Lend a stack in a native item place, once, as in [`ItemPlace::with_stack`].
    fn game_stack(
        &mut self,
        place: &ItemPlace,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()>;
    /// Side: server. Resolve an occupied player slot, hand, or mob or armour stand equipment
    /// slot before lending its component editor once, without lending the host. Refuse an
    /// unavailable or unreadable slot or a closed `tx` before calling `edit`. Validate all
    /// changed overrides before applying any of them to that slot. Journal the write in `tx`
    /// and let normal menu, inventory and equipment sync publish it without replaying equip
    /// effects. A refusal or a transaction abort leaves the original slot intact.
    fn game_edit_stack(
        &mut self,
        place: &ItemPlace,
        tx: &TransactionContext,
        edit: &mut dyn for<'a> FnMut(&mut ItemComponentEditor<'a>),
    ) -> Result<(), ItemPlaceEditError>;
    /// Side: server. Runs `ItemStack.onCraftedBy` for a stack held by a menu slot or its cursor,
    /// including the item's post-processing and the player's crafted statistic, in transaction
    /// `tx`.
    fn game_on_crafted_by(
        &mut self,
        place: &ItemPlace,
        tx: &TransactionContext,
        amount: i32,
    ) -> Result<(), CraftedByError>;
    /// Side: server.
    /// `ItemAccess.insert` at a player's place, an item entity's or an entity's slot: up to
    /// `amount` of `resource`, into the place where it's empty or holds the same item and
    /// components, up to the item's stack size. The rest goes where vanilla puts an item split
    /// off it: a player's into their inventory, else dropped as `Player.drop(stack, false)` at
    /// the root commit (a creative player's hand takes anything, and gives them one if they have
    /// none, as `ItemUtils.createFilledResult`); an item entity's dropped beside it at the root
    /// commit; an entity slot's into the entity's other slots (`forHandlerIndex`). How much
    /// went: 0 where the place is unavailable. In transaction
    /// `tx`, which keeps each place as it was. The player's screen follows with the tick's sync.
    fn game_insert(
        &mut self,
        place: &ItemPlace,
        resource: &ItemResource,
        amount: u32,
        tx: &TransactionContext,
    ) -> Result<u32, StackWriteError>;
    /// Side: server.
    /// `ItemAccess.extract` at those places: match the item and components of `resource`,
    /// without checking write limits, and take up to `amount`. An item entity emptied stays,
    /// empty, until the root commit removes it. A creative player's hand gives without losing
    /// any (`forInfiniteMaterials`).
    #[read_stack]
    fn game_extract(
        &mut self,
        place: &ItemPlace,
        resource: &ItemResource,
        amount: u32,
        tx: &TransactionContext,
    ) -> Result<u32, StackWriteError>;
    /// Side: server.
    /// Drops `stack` at the middle of the block at `pos`, as a block entity's handler gives up
    /// what it can't put back.
    fn drop_at(&mut self, pos: [i32; 3], stack: ItemStack) -> Result<(), StackWriteError>;
    /// Side: server.
    /// `DefaultDispenseItemBehavior.addToInventoryOrDispense`: inserts `remainder` into the
    /// dispenser at `pos`, or drops what does not fit with the click and smoke, journaling
    /// slot writes and deferring drops and events when a storage transaction is open.
    fn dispense_add_remainder(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        remainder: ItemStack,
    ) -> Result<(), LevelWriteError>;
}
