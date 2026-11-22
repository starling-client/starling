//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `items` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait ItemsHost {
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
    /// Decodes a saved-item compound from SNBT, for example
    /// `{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5}}`.
    /// A missing `count` defaults to 1, unknown fields such as `Count` are ignored, and
    /// `stone` means `minecraft:stone`. Returns `None` unless the input is one compound
    /// with only whitespace around it. Air, unknown items and patches naming a transient
    /// component are refused. A present `count` must be numeric and in 1 to 99 after
    /// Java's `Number.intValue` conversion. The text parser also enforces binary NBT's
    /// 2 MiB input and 512-level nesting bounds.
    fn item_from_snbt(&self, snbt: &str) -> Option<ItemStack>;
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
/// The server host's methods of the `items` area, implemented by the game and reached through [`ServerHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ItemsServerHost {
    /// Side: server.
    /// Drop one item as DefaultDispenseItemBehavior does. For a dispenser hook:
    /// return this remainder in its answer with `CLICK`; the caller plays the
    /// answer's event and smoke once. A client, another dimension or an unloaded
    /// chunk keeps the input stack unchanged.
    fn dispense_default(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Option<ItemStack>, StackWriteError>;
    /// Side: server.
    /// Holds `stack` outside the world, in the open transaction `tx` (`ItemAccess.forStack`):
    /// the [`ItemPlace::Scratch`] to run item capabilities against. It lives until the
    /// outermost transaction closes, and an abort of the one that made it drops it. None
    /// where `tx` is not open.
    fn scratch_stack(
        &mut self,
        tx: &Transaction,
        stack: ItemStack,
    ) -> Result<Option<ItemPlace>, StackWriteError>;
    /// Side: server.
    /// Takes the stack out of scratch place `place`, in `tx`, ending the place: none where it
    /// is empty, no longer held, or not a scratch place.
    fn scratch_take(&mut self, tx: &Transaction, place: &ItemPlace) -> Option<ItemStack>;
    /// Side: server.
    /// Holds `stack` outside the world in a two-slot copy whose item may change, in the open
    /// transaction `tx`: the [`ItemPlace::ScratchSlots`] a dispensed fluid container runs in
    /// (`DispenseFluidContainer`), `stack` in its first slot. It lives as a
    /// [`scratch_stack`](Self::scratch_stack) place does. None where `tx` is not open.
    fn scratch_slots(
        &mut self,
        tx: &Transaction,
        stack: ItemStack,
    ) -> Result<Option<ItemPlace>, StackWriteError>;
    /// Side: server.
    /// Takes both slots out of two-slot place `place`, in `tx`, ending the place: what the
    /// first holds and what the second caught. None where it is no longer held or not a
    /// two-slot place.
    fn scratch_take_slots(
        &mut self,
        tx: &Transaction,
        place: &ItemPlace,
    ) -> Option<(Option<ItemStack>, Option<ItemStack>)>;
    /// Side: server.
    /// `DefaultDispenseItemBehavior.consumeWithRemainder`: one of `stack` is used and
    /// `remainder` replaces it, a single `stack` by becoming it, else into the dispenser at
    /// `pos` (`DispenserBlockEntity.insertItem`) or, where it doesn't fit, out of its front with
    /// the click and smoke. What the slot holds then. A client, another dimension or an unloaded
    /// chunk keeps `stack` unchanged.
    fn dispense_with_remainder(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        stack: ItemStack,
        remainder: ItemStack,
    ) -> Result<Option<ItemStack>, StackWriteError>;
}
/// The client host's methods of the `items` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ItemsClientHost {
    /// Side: client.
    /// Creative mode only: gives the player `count` of `item`
    /// (`minecraft:diamond`) as taking it from the creative inventory does
    /// (`ServerboundSetCreativeModeSlot`): the selected slot first, then the
    /// hotbar, then the inventory. The count is cut to one stack.
    fn give(&mut self, item: ItemKey, count: u32) -> Result<(), String>;
    /// Side: client.
    /// The creative search tab's final display order, for ranking stacks in inventory sorters.
    /// This requests the same catalog as the creative screen (using the player's permission and
    /// Operator Items Tab option). On a miss it dispatches the creative build hook. During setup,
    /// reload, a different build, or without a live catalog, this returns an empty list.
    fn creative_search_items(&mut self) -> Vec<ItemStack>;
    /// Side: client.
    /// Registered enchantments, in registry order.
    fn enchantments(&self) -> Vec<EnchantmentInfo>;
    /// Side: client.
    /// Registered potions (`Registries.POTION`), in registry order, each by its registry id:
    /// vanilla's 46, from `minecraft:water` to `minecraft:infested` (content mods cannot declare
    /// potions yet). The game builds the list once; each call copies it. A mod builds the potion, splash,
    /// lingering or tipped-arrow stack itself with the `minecraft:potion_contents` component.
    fn potions(&self) -> Vec<PotionInfo>;
    /// Side: client.
    /// `Enchantment.getMinCost(level)`: the enchanting power needed, or None for an unknown id.
    fn enchantment_min_cost(&self, id: &str, level: u32) -> Option<i32>;
    /// Side: client.
    /// Whether enchantment `id` can go on `item` (`Enchantment.isSupportedItem`).
    fn enchantment_supports(&self, id: &str, item: ItemKey) -> bool;
}
/// The handles' reads of the `items` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait ItemsBackend {
    /// Side: both. Normalize a creative membership key against the game's item prototypes.
    fn creative_stack_key(&self, stack: &ItemStack) -> Option<ItemStack>;

    // The editor checks the active callback on both sides; setup and reload can only return Stale.
    /// Side: both. Read the current entries through the active creative tab editor.
    fn creative_tab_entries(
        &self,
        edit: CreativeTabEdit,
        list: CreativeTabList,
    ) -> Result<Vec<ItemStack>, CreativeTabEditError>;
    /// Side: both. Apply a change to the active creative tab editor.
    fn creative_tab_change(
        &self,
        edit: CreativeTabEdit,
        change: CreativeTabChange,
    ) -> Result<(), CreativeTabEditError>;

    /// Side: both. Effective damageability after defaults and the stack patch.
    fn item_resource_damageable(&self, stack: &ItemStack) -> bool;
    /// Side: both. Effective glint, including overrides and item-specific foil.
    fn item_resource_foil(&self, stack: &ItemStack) -> bool;
    /// Side: both. Effective repair ingredient membership, including item tags.
    fn item_resource_repairable(&self, stack: &ItemStack, material: ItemKey) -> bool;

    /// Side: both. ItemResource's effective use duration, including its Part.
    fn item_resource_use_duration(&self, stack: &ItemStack) -> Option<i32>;
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
    /// The stack in slot `slot` of those `at`'s side reaches (`container_size`).
    fn container_stack(&self, at: BlockSide, slot: usize) -> Option<ItemStack>;
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
    /// The stacks in one item of `stack`'s `minecraft:bundle_contents`, front first (the
    /// stack the bundle gives back next is slot 0). None for an item without the component.
    fn item_bundle_slots(&self, stack: &ItemStack) -> Option<Vec<Option<ItemStack>>>;
    /// Side: both.
    /// `ResourceHandler.isValid` and `getCapacityAsLong` for a bundle: how many of `item` one
    /// item of `stack` still takes, by `BundleContents.getMaxAmountToAdd` (weight left, nested
    /// bundles' weight); 0 for an item the bundle refuses, or without the component.
    fn item_bundle_capacity(&self, stack: &ItemStack, slot: usize, item: &ItemStack) -> u32;
    /// Side: both.
    /// `ResourceHandler.insert` for a bundle: as much of `item` as the bundle takes goes in at
    /// the front, as the bundle screen inserts. Gives how many went in and the changed bundle;
    /// None without the component.
    fn item_bundle_insert_slot(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
    ) -> Option<(u32, ItemStack)>;
    /// Side: both.
    /// `ResourceHandler.extract` for a bundle: up to `max` of the front stack (slot 0 only), as
    /// the bundle screen takes out. Gives the taken stack, if any, and the changed bundle; None
    /// without the component.
    fn item_bundle_extract(
        &self,
        stack: &ItemStack,
        slot: usize,
        max: u32,
    ) -> Option<(Option<ItemStack>, ItemStack)>;
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
    fn container_set_stack_limit(&mut self, at: BlockSide, limit: u32, tx: &Transaction) -> bool;

    /// Side: server. Roll an enchantment result with the named level's random source.
    fn item_resource_enchant_randomly(
        &mut self,
        stack: &ItemStack,
        level: LevelRef,
        power: i32,
        pool: &Identifier,
    ) -> Result<Option<ItemStack>, StackWriteError>;
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
        tx: &Transaction,
    ) -> Result<Option<ItemStack>, StackWriteError>;
    /// Side: server.
    /// Takes up to `max` out of slot `slot` of those `at`'s side reaches, in transaction `tx`;
    /// through a side, only what that side may take (`canTakeItemThroughFace`). What came out.
    fn container_extract(
        &mut self,
        at: BlockSide,
        slot: usize,
        max: u32,
        tx: &Transaction,
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
        tx: &Transaction,
    ) -> u64;
    /// Side: server.
    /// Takes up to `amount` of `resource` out of the slots `at`'s side reaches and may take
    /// from, in transaction `tx`, in one pass: how much came out.
    fn container_extract_kind(
        &mut self,
        at: BlockSide,
        resource: &ItemResource,
        amount: u64,
        tx: &Transaction,
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
        tx: &Transaction,
        edit: &mut dyn for<'a> FnMut(&mut ItemComponentEditor<'a>),
    ) -> Result<(), ItemPlaceEditError>;
    /// Side: server. Runs `ItemStack.onCraftedBy` for a stack held by a menu slot or its cursor,
    /// including the item's post-processing and the player's crafted statistic, in transaction
    /// `tx`.
    fn game_on_crafted_by(
        &mut self,
        place: &ItemPlace,
        tx: &Transaction,
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
        tx: &Transaction,
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
        tx: &Transaction,
    ) -> Result<u32, StackWriteError>;
    /// Side: server.
    /// Drops `stack` at the middle of the block at `pos`, as a block entity's handler gives up
    /// what it can't put back.
    fn drop_at(&mut self, pos: [i32; 3], stack: ItemStack) -> Result<(), StackWriteError>;
}
