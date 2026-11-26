//! The game's side of the mod API: the installed tables, the reload machinery, the structs the
//! game fills for its hooks (`built`) and what it reads of handles and the API's own types
//! (`game`). None of it is part of the API a mod is written against.
use crate::*;
#[path = "__private/registry_installation.rs"]
mod registry_installation;

/// The argument of a handler trait's hidden marks method. Its field is private, so no mod can
/// make one: nothing outside this crate can call the method.
// exhaustive: its field is private; nothing builds one
// no wire: it never crosses; the method is not in the game's table and neither side calls it.
pub struct Sealed(());

/// Native libraries' launcher-to-game registration contract, not a mod-facing API.
/// The launcher generates manifest rows independently of hook factories. The game checks
/// them against the packages beside its executable (`sunrise_mod_package::runtime`).
/// These types live in the shared API closure so every library uses the same Rust ABI.
pub mod native {
    /// One dynamically loaded native library, registered before the engine seal.
    pub struct NativeMod {
        /// Hook factories owned by this library.
        pub entries: fn() -> Vec<crate::ModEntry>,
        /// Manifest identities (empty for the patch library).
        pub built: &'static [BuiltMod],
        /// Install patch copies and return their verification rows.
        pub installs: fn() -> Vec<(&'static str, &'static [&'static str], bool)>,
    }

    /// One native mod supplied by a library, with its manifest's `side`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct BuiltMod {
        pub id: &'static str,
        pub version: &'static str,
        pub side: crate::Side,
    }
}

/// Switch an installed mod's dispatches on or off, returning whether it was found.
pub fn set_mod_enabled(id: &str, on: bool) -> bool {
    crate::set_mod_enabled(id, on)
}

/// Index the `(registry key, mod ID)` pairs admitted to each startup registry.
pub fn install_content_owners<'a>(
    blocks: impl IntoIterator<Item = (&'a str, &'a str)>,
    items: impl IntoIterator<Item = &'a str>,
    block_entities: impl IntoIterator<Item = (&'a str, &'a str)>,
    menus: impl IntoIterator<Item = (&'a str, &'a str)>,
) {
    crate::runtime::install_content_owners(blocks, items, block_entities, menus);
}

/// Whether this block came from the admitted content declarations.
pub fn is_content_block(id: &str) -> bool {
    crate::runtime::content_block(id).is_some()
}

/// Whether this item came from the admitted content declarations.
pub fn is_content_item(id: &str) -> bool {
    crate::runtime::is_content_item(id)
}

/// Whether the engine has bound content ownership for the installed dispatch.
pub fn content_owners_published() -> bool {
    crate::runtime::content_owners_published()
}

/// Whether the declaring package of this block entity type is switched off.
pub fn block_entity_owner_disabled(id: &str) -> bool {
    crate::runtime::block_entity_owner_disabled(id)
}

/// Whether any installed mod is off; computed only at load/toggle time.
pub fn any_mod_disabled() -> bool {
    crate::runtime::any_mod_disabled()
}

/// Whether a block's owner has a disabled Part subscribed to this hook.
/// The scheduler uses the same pre-indexed table as dispatch, before consuming a tick.
pub fn block_hook_inert(hook: HookId, state: BlockState) -> bool {
    let table = crate::parts(hook, "block_hook_inert");
    table
        .for_hook(hook)
        .and_then(|table| table.block_entry(state))
        .and_then(|entry| entry.disabled_owner_slot())
        .is_some()
}

/// Resolve a listener's owner by id, for dispatch through [`mod_index_on`].
pub fn mod_index(id: &str) -> Option<usize> {
    crate::runtime::installed()?.by_id.get(id).copied()
}

/// Whether a resolved listener's owner is currently enabled.
pub fn mod_index_on(index: usize) -> bool {
    crate::mod_index_on(index)
}

/// Marks the thread as inside mod `id`'s `call` until the guard drops, as a hook's dispatch does,
/// so a panic in it names both.
pub fn enter(id: &'static str, call: &'static str) -> crate::content::crash::ModGuard {
    crate::mark_mod(id, call)
}

/// Install the process-wide ordered list of mods.
pub fn install(mods: Mods) {
    crate::install(mods);
}

/// Bind a native mod entry's declared package side before installation.
pub fn with_mod_side(mut entry: ModEntry, side: Side) -> ModEntry {
    entry.side = side;
    entry
}

// no wire: process-local diagnostic report, never crosses the wire.
// exhaustive: internal two-field report for the game client's toast queue.
/// One mod callback panic queued for the game client's error toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HookPanicReport {
    /// The `[mod] id` of the panicking mod.
    pub mod_id: &'static str,
    /// The hook or retained callback operation that panicked.
    pub hook: &'static str,
}

/// Drain queued mod callback panic reports for the game client's error toast.
pub fn take_hook_panics() -> Vec<HookPanicReport> {
    crate::runtime::take_hook_panics()
}

/// Active hook-dispatch panic-containment nesting depth on this thread.
pub fn hook_dispatch_depth() -> u32 {
    crate::runtime::hook_dispatch_depth()
}

/// Run one mod's startup `Hooks::register` callback under the unified hook-dispatch failure rule.
pub fn invoke_registration(mod_id: &'static str, call: impl FnOnce()) -> bool {
    crate::runtime::invoke_registration(mod_id, call)
}

/// Discard one startup batch's values independently under the owning `register` callback's policy.
pub fn discard_startup_registrations<T>(mod_id: &'static str, values: impl IntoIterator<Item = T>) {
    crate::runtime::discard_startup_registrations(mod_id, values);
}

/// Revision of the enabled mod set, for engine caches of hook answers.
pub fn mods_switched() -> u64 {
    crate::runtime::mods_switched()
}

/// Whether an installed mod subscribes to `hook`.
pub fn listened(hook: HookId) -> bool {
    crate::listened(hook)
}

/// How many listeners of `hook` are switched off for the session by an isolated dispatch after
/// they unwound.
pub fn poisoned_listeners(hook: HookId) -> usize {
    crate::runtime::poisoned_count(hook)
}

/// Whether at least one enabled mod subscribes to `hook`, visiting only its subscribers.
pub fn enabled_listened(hook: HookId) -> bool {
    crate::runtime::enabled_listened(hook)
}

/// A live listener query bound to the creating thread's installed dispatch table.
// no wire: a process-local view of the installed dispatch table, never sent.
pub struct EnabledListeners {
    installed: Option<&'static crate::runtime::Installed>,
    hook: HookId,
}

impl EnabledListeners {
    pub fn capture(hook: HookId) -> Self {
        Self {
            installed: crate::runtime::installed(),
            hook,
        }
    }

    pub fn any(&self) -> bool {
        self.installed
            .is_some_and(|installed| crate::runtime::has_enabled_listener(installed, self.hook))
    }
}

/// Whether any part subscribed to the scoped entity hook `hook` is attached to
/// entity type `kind`, so a family's walk can resolve its rows' slots once per
/// pass instead of naming each row's type.
pub fn entity_part_hooked(hook: HookId, name: &'static str, kind: &str) -> bool {
    crate::parts::has_part_hook(hook)
        && crate::parts::parts(hook, name)
            .for_hook(hook)
            .is_some_and(|table| !table.entity(|| Some(kind)).is_empty())
}

/// A read backend wrapper for a host the game builds narrowed: the mob tick's
/// goal view hands itself out as its own backend.
pub fn backend_ref(inner: &dyn Backend) -> crate::BackendRef<'_> {
    crate::BackendRef::of(inner)
}
/// Call `read` on `host`'s real backend, or `None` where the host has none:
/// the mob tick's goal view forwards the level's own reads — blocks, loaded
/// chunks, time, height, weather, block entities — through it; the
/// entity data its snapshot does not carry stays refused. Returned borrows stay tied to the host.
pub fn backend_read<'a, R>(
    host: &'a dyn crate::Host,
    read: impl FnOnce(&'a dyn Backend) -> R,
) -> Option<R> {
    host.backend().map(|backend| read(backend.open()))
}
/// The trait mods installed, in dispatch order: the game's own list, not part of the mod surface.
pub fn mods() -> &'static [ModEntry] {
    crate::mods()
}

/// Actual installed code identities, including mods with no hook subscriptions.
/// The engine uses this dispatch inventory to apply its saved code switches.
pub fn installed_code_ids() -> impl Iterator<Item = &'static str> {
    crate::mods().iter().map(|entry| entry.id)
}

/// What the installed code mod `id` carries that a joining client must have, as the end of a
/// sentence about the mod: its first subscription to a client-side hook, its first part that
/// subscribes to one, a listener of a client mod event, a required client menu screen, a synced
/// attachment, a required datapack registry, or a required payload channel sent to clients. The
/// engine refuses a `side = "server"` mod that has one. Read once the parts are installed.
pub fn client_requirement(id: &str) -> Option<String> {
    let mine = || crate::mods().iter().filter(|entry| entry.id == id);
    if let Some(hook) = mine().find_map(|entry| entry.client_hook()) {
        return Some(format!("listens to the client hook {hook}"));
    }
    let installed = crate::parts::installed()?;
    if let Some(index) = mod_index(id)
        && installed
            .menu_screen_factories
            .values()
            .any(|reg| reg.index == index)
    {
        return Some("registers a client menu screen factory".into());
    }
    if installed
        .menu_recipe_books
        .values()
        .any(|book| book.owner == id)
    {
        return Some("binds a recipe book that requires a client menu screen".into());
    }
    if let Some(event) = mod_index(id).and_then(|index| installed.client_event(index)) {
        return Some(format!("listens to the client event {event}"));
    }
    if let Some((part, hook)) = installed.client_part(id) {
        return Some(format!(
            "attaches the part {part}, which listens to the client hook {hook}"
        ));
    }
    if let Some(attachment) = installed.first_synced_attachment(id) {
        return Some(format!("declares the synced attachment {attachment}"));
    }
    if let Some(registry) = installed
        .datapack_registries
        .iter()
        .find(|registry| registry.mod_id == id && registry.sync == DatapackRegistrySync::Required)
    {
        return Some(format!(
            "declares the required datapack registry {}",
            registry.key
        ));
    }
    let channel = installed.keys.payloads.as_ref()?.client_bound(id)?;
    Some(format!("sends the required payload {channel} to clients"))
}

/// Every client-required declaration registered by `id` during `Hooks::register`
/// (general entity types, attributes and bounds widenings, custom stats, synced
/// attachments, datapack registries, and required client-bound payloads).
pub fn client_required_declarations(id: &str) -> Vec<(&'static str, String)> {
    crate::parts::installed()
        .map(|installed| installed.client_required_declarations(id))
        .unwrap_or_default()
}

/// The command argument types mods registered at startup, by namespaced id, for the game's
/// command trees. Not part of the mod surface.
pub fn command_argument_types() -> Vec<(String, std::sync::Arc<dyn crate::CommandArgumentType>)> {
    crate::parts::installed_command_arguments()
        .iter()
        .map(|info| (info.id.to_string(), info.argument.clone()))
        .collect()
}

/// The selector tokens mods registered at startup, by name, for the game's command
/// registries. Not part of the mod surface.
pub fn entity_selector_types() -> impl Iterator<
    Item = (
        &'static str,
        &'static crate::callback_owner::CallbackOwner,
        &'static dyn crate::EntitySelectorType,
    ),
> {
    crate::parts::installed_entity_selectors()
        .iter()
        .map(|info| (info.name.as_ref(), &info.owner, info.selector.as_ref()))
}

/// Resolve an installed selector's owner and handler by reference, without allocating or cloning a resource.
pub fn entity_selector_type(
    name: &str,
) -> Option<(
    &'static crate::callback_owner::CallbackOwner,
    &'static dyn crate::EntitySelectorType,
)> {
    let selectors = crate::parts::installed_entity_selectors();
    selectors
        .binary_search_by(|info| info.name.as_ref().cmp(name))
        .ok()
        .map(|index| {
            let info = &selectors[index];
            (&info.owner, info.selector.as_ref())
        })
}

/// The startup package inventory at an owning engine load boundary.
pub fn loaded_mods() -> LoadedMods {
    LoadedMods::new()
}

/// The packages enabled and admitted at startup, as registration reads them: `ConfigHost::mods`.
pub fn startup_mods() -> LoadedMods {
    LoadedMods::enabled_at_startup()
}

/// The data-pack and resource reloads the game runs.
pub mod reload {
    include!(concat!(
        env!("OUT_DIR"),
        "/mod_api_exports_private_reload.rs"
    ));
}

/// The structs the game fills for its hooks, each turned into the API's own with `From`.
pub mod built {
    include!(concat!(
        env!("OUT_DIR"),
        "/mod_api_exports_private_built.rs"
    ));
    /// Wrap a slot in the persistent instance group store.
    pub fn instance_group_ref(generation: u64, slot: u32) -> crate::InstanceGroupRef {
        crate::InstanceGroupRef::from_parts(generation, slot).expect("published group generation")
    }
    /// Wrap a slot in the prepared item store.
    pub fn prepared_item_ref(generation: u64, slot: u32) -> crate::PreparedItemRef {
        crate::PreparedItemRef::from_parts(generation, slot)
            .expect("published prepared item generation")
    }
    /// Wrap a slot in the model store's published generation.
    pub fn model_ref(generation: u64, slot: u32) -> crate::ModelRef {
        crate::ModelRef(generation, slot)
    }
    /// Wrap a slot in the pipeline set's published generation.
    pub fn render_pipeline_ref(generation: u64, slot: u32) -> crate::RenderPipelineRef {
        crate::RenderPipelineRef::from_parts(generation, slot)
            .expect("published pipeline generation")
    }
    /// Wrap a slot in the render-type table's published generation.
    pub fn render_type_ref(generation: u64, slot: u32) -> crate::RenderTypeRef {
        crate::RenderTypeRef::from_parts(generation, slot)
            .expect("published render type generation")
    }
    /// Wrap a slot in the compute-pipeline table's published generation.
    pub fn compute_pipeline_ref(generation: u64, slot: u32) -> crate::ComputePipelineRef {
        crate::ComputePipelineRef::from_parts(generation, slot)
            .expect("published compute pipeline generation")
    }
    /// Wrap a registered pipeline modifier slot.
    pub fn pipeline_modifier_ref(slot: u32) -> crate::PipelineModifierRef {
        crate::PipelineModifierRef::from_slot(slot)
    }
    /// Wrap a slot in the retained GPU mesh store.
    pub fn mesh_ref(generation: u64, slot: u32) -> crate::MeshRef {
        crate::MeshRef::from_parts(generation, slot).expect("published mesh generation")
    }
    /// The renderer's [`crate::RenderCapabilities`].
    pub fn render_capabilities(
        float32_filterable: bool,
        read_write_storage: Vec<crate::RenderTargetFormat>,
        subgroup: bool,
        indirect_first_instance: bool,
        multi_draw_indirect_count: bool,
    ) -> crate::RenderCapabilities {
        crate::RenderCapabilities {
            float32_filterable,
            read_write_storage,
            subgroup,
            indirect_first_instance,
            multi_draw_indirect_count,
        }
    }
    /// Wrap a slot in the retained GPU storage buffer store.
    pub fn storage_buffer_ref(generation: u64, slot: u32) -> crate::StorageBufferRef {
        crate::StorageBufferRef::from_parts(generation, slot)
            .expect("published storage buffer generation")
    }
    /// Wrap a readback id in the pending readback table.
    pub fn readback_ticket(id: u64) -> crate::ReadbackTicket {
        crate::ReadbackTicket::from_id(id).expect("published readback ticket id")
    }
    /// Wrap a slot in the retained GPU uniform buffer store.
    pub fn uniform_buffer_ref(generation: u64, slot: u32) -> crate::UniformBufferRef {
        crate::UniformBufferRef::from_parts(generation, slot)
            .expect("published uniform buffer generation")
    }
    /// Wrap a slot in the retained GPU offscreen render target store.
    pub fn render_target_ref(generation: u64, slot: u32) -> crate::RenderTargetRef {
        crate::RenderTargetRef::from_parts(generation, slot)
            .expect("published render target generation")
    }
}

/// What the game reads of handles and calls on the API's own types.
pub mod game {
    /// Engine-side slot queries use the same handler as `ItemPlace::MenuSlot` transfers.
    pub fn menu_slot_place_handler(
        place: crate::MenuSlotPlace,
        host: &mut dyn crate::ServerHost,
        player: crate::PlayerRef,
    ) -> Option<(
        crate::Capability<dyn crate::ResourceHandler<crate::ItemResource>>,
        usize,
    )> {
        place.handler(host, player)
    }

    include!(concat!(env!("OUT_DIR"), "/mod_api_exports_private_game.rs"));
    use crate::*;
    use std::fmt::Debug;

    /// A type's fields the game reads, as `value.fields().name`.
    pub trait Fields {
        type Fields<'a>
        where
            Self: 'a;
        fn fields(&self) -> Self::Fields<'_>;
    }

    /// Borrow the attachment frame tracker exposed by a game-owned host or level.
    pub fn attachment_read_scope(
        scopes: &sunrise_mod_runtime::attachments::AttachmentQueryScopes,
    ) -> AttachmentReadScope<'_> {
        AttachmentReadScope(Some(scopes))
    }

    /// A block entity handle the game names.
    pub fn block_entity_ref(
        level: LevelRef,
        position: [i32; 3],
        generation: u64,
    ) -> BlockEntityRef {
        BlockEntityRef {
            level,
            position,
            generation,
        }
    }
    /// A client mod's sound handle the game names.
    pub fn sound_ref(id: u64) -> SoundRef {
        SoundRef { id }
    }
    /// A boss bar handle the game names.
    pub fn boss_bar_ref(epoch: WorldEpoch, uuid: u128) -> BossBarRef {
        BossBarRef { epoch, uuid }
    }
    /// A scoreboard objective handle the game names.
    pub fn objective_ref(epoch: WorldEpoch, id: u64) -> ObjectiveRef {
        ObjectiveRef { epoch, id }
    }
    /// A scoreboard team handle the game names.
    pub fn team_ref(epoch: WorldEpoch, id: u64) -> TeamRef {
        TeamRef { epoch, id }
    }
    /// A chunk handle the game names.
    pub fn chunk_ref(level: LevelRef, pos: [i32; 2]) -> ChunkRef {
        ChunkRef { level, pos }
    }
    /// A live watered-area handle issued by the level's runtime ticket index.
    pub fn water_ticket(
        level: LevelRef,
        id: sunrise_mod_runtime::water_tickets::WaterTicketId,
    ) -> WaterTicket {
        WaterTicket { level, id }
    }
    /// The runtime-issued identity of a live watered-area handle.
    pub fn water_ticket_id(
        ticket: WaterTicket,
    ) -> sunrise_mod_runtime::water_tickets::WaterTicketId {
        ticket.id
    }
    /// Resolve an issued ticket controller handle against the active controller table.
    pub fn ticket_controller_slot(controller: TicketController) -> Option<u32> {
        controller.active_slot()
    }
    /// The namespaced id of a controller slot for world-save serialization.
    pub fn ticket_controller_id(slot: u32) -> Option<&'static str> {
        crate::world::ticket_controller::id_for_slot(slot)
    }
    /// The installed controller slot for a saved namespaced id during world-load deserialization.
    pub fn ticket_controller_slot_by_id(id: &str) -> Option<u32> {
        crate::world::ticket_controller::slot_for_id(id)
    }
    /// Borrowed modifier view constructed at the living-entity read boundary.
    pub fn attribute_modifier_view<'a>(
        id: &'a str,
        amount: f64,
        operation: AttributeOperation,
    ) -> AttributeModifierView<'a> {
        AttributeModifierView {
            id,
            amount,
            operation,
        }
    }
    /// A non-blocking chunk request handle the game names.
    pub fn chunk_request_ref(level: LevelRef, id: u64) -> ChunkRequestRef {
        ChunkRequestRef { level, id }
    }
    /// One finished or failed non-blocking chunk request constructed by the game.
    pub fn completed_chunk_request(
        request: ChunkRequestRef,
        pos: [i32; 2],
        result: Result<ChunkSnapshot, ChunkLoadError>,
    ) -> CompletedChunkRequest {
        CompletedChunkRequest {
            request,
            pos,
            result,
        }
    }
    /// An open menu's handle the game names.
    pub fn menu_ref(player: PlayerRef, window: i32, generation: u64) -> MenuRef {
        MenuRef {
            player,
            window,
            generation,
        }
    }
    /// A player handle the game names.
    pub fn player_ref(entity: EntityRef, uuid: u128) -> PlayerRef {
        PlayerRef { entity, uuid }
    }
    /// The player a completion is for, as the game knows them.
    pub fn command_sender(uuid: u128, name: String, permission: PermissionLevel) -> CommandSender {
        CommandSender {
            uuid,
            name,
            permission,
        }
    }
    /// A stack's modifiers for `item_attribute_modifier`, as its component
    /// gives them.
    pub fn item_attribute_modifiers(entries: Vec<ItemAttributeModifier>) -> ItemAttributeModifiers {
        ItemAttributeModifiers {
            entries,
            changed: false,
        }
    }
    /// The modifiers the hook left, or `None` when no mod changed them.
    pub fn changed_item_attribute_modifiers(
        modifiers: ItemAttributeModifiers,
    ) -> Option<Vec<ItemAttributeModifier>> {
        modifiers.changed.then_some(modifiers.entries)
    }

    /// Move a validated sound key into the engine's sound event queue.
    pub fn sound_event_name(event: SoundEventKey) -> Arc<str> {
        event.0
    }

    /// A stack's enchantments for `get_enchantment_level`, as its
    /// component gives them.
    pub fn item_enchantments(entries: Vec<(Identifier, u32)>) -> ItemEnchantments {
        ItemEnchantments {
            entries,
            changed: false,
        }
    }
    /// The enchantments the hook left, and whether a mod changed them.
    pub fn item_enchantments_entries(
        enchantments: ItemEnchantments,
    ) -> (Vec<(Identifier, u32)>, bool) {
        (enchantments.entries, enchantments.changed)
    }

    pub struct ModelRefFields {
        pub generation: u64,
        pub slot: u32,
    }
    impl Fields for ModelRef {
        type Fields<'a> = ModelRefFields;
        fn fields(&self) -> ModelRefFields {
            ModelRefFields {
                generation: self.0,
                slot: self.1,
            }
        }
    }

    impl Fields for crate::BakedModelKey {
        type Fields<'a> = ModelRefFields;
        fn fields(&self) -> ModelRefFields {
            ModelRefFields {
                generation: self.0,
                slot: self.1,
            }
        }
    }

    impl Fields for RenderPipelineRef {
        type Fields<'a> = ModelRefFields;
        fn fields(&self) -> ModelRefFields {
            ModelRefFields {
                generation: self.0,
                slot: self.1,
            }
        }
    }

    impl Fields for RenderTypeRef {
        type Fields<'a> = ModelRefFields;
        fn fields(&self) -> ModelRefFields {
            ModelRefFields {
                generation: self.0,
                slot: self.1,
            }
        }
    }

    impl Fields for ComputePipelineRef {
        type Fields<'a> = ModelRefFields;
        fn fields(&self) -> ModelRefFields {
            ModelRefFields {
                generation: self.0,
                slot: self.1,
            }
        }
    }

    pub struct PipelineModifierRefFields {
        pub slot: u32,
    }
    impl Fields for PipelineModifierRef {
        type Fields<'a> = PipelineModifierRefFields;
        fn fields(&self) -> PipelineModifierRefFields {
            PipelineModifierRefFields { slot: self.0 }
        }
    }

    pub struct BlockEntityRefFields {
        pub level: LevelRef,
        pub position: [i32; 3],
        pub generation: u64,
    }
    impl Fields for BlockEntityRef {
        type Fields<'a> = BlockEntityRefFields;
        fn fields(&self) -> BlockEntityRefFields {
            BlockEntityRefFields {
                level: self.level,
                position: self.position,
                generation: self.generation,
            }
        }
    }

    pub struct SoundRefFields {
        pub id: u64,
    }
    impl Fields for SoundRef {
        type Fields<'a> = SoundRefFields;
        fn fields(&self) -> SoundRefFields {
            SoundRefFields { id: self.id }
        }
    }

    pub struct BossBarRefFields {
        pub epoch: WorldEpoch,
        pub uuid: u128,
    }
    impl Fields for BossBarRef {
        type Fields<'a> = BossBarRefFields;
        fn fields(&self) -> BossBarRefFields {
            BossBarRefFields {
                epoch: self.epoch,
                uuid: self.uuid,
            }
        }
    }

    #[non_exhaustive]
    pub struct ObjectiveRefFields {
        pub epoch: WorldEpoch,
        pub id: u64,
    }
    impl Fields for ObjectiveRef {
        type Fields<'a> = ObjectiveRefFields;
        fn fields(&self) -> ObjectiveRefFields {
            ObjectiveRefFields {
                epoch: self.epoch,
                id: self.id,
            }
        }
    }

    #[non_exhaustive]
    pub struct TeamRefFields {
        pub epoch: WorldEpoch,
        pub id: u64,
    }
    impl Fields for TeamRef {
        type Fields<'a> = TeamRefFields;
        fn fields(&self) -> TeamRefFields {
            TeamRefFields {
                epoch: self.epoch,
                id: self.id,
            }
        }
    }

    pub struct MenuRefFields {
        pub player: PlayerRef,
        pub window: i32,
        pub generation: u64,
    }
    impl Fields for MenuRef {
        type Fields<'a> = MenuRefFields;
        fn fields(&self) -> MenuRefFields {
            MenuRefFields {
                player: self.player,
                window: self.window,
                generation: self.generation,
            }
        }
    }

    pub struct PlayerRefFields {
        pub entity: EntityRef,
        pub uuid: u128,
    }
    impl Fields for PlayerRef {
        type Fields<'a> = PlayerRefFields;
        fn fields(&self) -> PlayerRefFields {
            PlayerRefFields {
                entity: self.entity,
                uuid: self.uuid,
            }
        }
    }

    /// Allocate a fresh delayed task id for `schedule_task`, without exposing its raw value.
    pub fn allocate_task_id() -> Option<TaskId> {
        TaskId::allocate()
    }

    pub struct ModEntryFields<'a> {
        pub id: &'static str,
        pub hooks: &'a dyn Hooks,
    }
    impl Fields for ModEntry {
        type Fields<'a> = ModEntryFields<'a>;
        fn fields(&self) -> ModEntryFields<'_> {
            ModEntryFields {
                id: self.id,
                hooks: &*self.hooks,
            }
        }
    }

    pub trait PartsLookup {
        fn for_hook(&self, hook: HookId) -> Option<&crate::parts::HookParts>;
    }
    impl PartsLookup for Parts {
        fn for_hook(&self, hook: HookId) -> Option<&crate::parts::HookParts> {
            Parts::for_hook(self, hook)
        }
    }

    pub trait CapabilityChecked {
        fn checked(&self, answer: u32, asked: u32, at: &dyn Debug) -> Option<u32>;
    }
    impl<T: ?Sized> CapabilityChecked for Capability<T> {
        fn checked(&self, answer: u32, asked: u32, at: &dyn Debug) -> Option<u32> {
            Capability::checked(self, answer, asked, at)
        }
    }

    pub trait FileToIdConverterPaths {
        fn directory(&self) -> &str;
        fn file_to_id(&self, file: &str) -> Option<Identifier>;
    }
    impl FileToIdConverterPaths for FileToIdConverter {
        fn directory(&self) -> &str {
            FileToIdConverter::directory(self)
        }
        fn file_to_id(&self, file: &str) -> Option<Identifier> {
            FileToIdConverter::file_to_id(self, file)
        }
    }

    pub trait RegistrationContextNew {
        fn new() -> Self;
    }
    impl RegistrationContextNew for RegistrationContext {
        fn new() -> Self {
            RegistrationContext::new()
        }
    }

    /// Engine-only per-listener checked declaration staging.
    // no wire: lends one registration sink for the duration of a callback.
    pub type DeclarationListener<'a, D> = dyn FnMut(&mut dyn FnMut(crate::Answered<D>) -> Result<(), crate::DeclarationError>) -> bool
        + 'a;

    /// Lend a registration snapshot to the named listener, including on scoped workers.
    pub fn bind_registration_context(
        context: &RegistrationContext,
        mod_id: &'static str,
    ) -> RegistrationContext {
        context.for_mod(mod_id)
    }

    pub trait ReloadContextNew {
        fn new(files: crate::world::reload::ReloadFiles) -> Self;
    }
    impl ReloadContextNew for ServerReloadContext {
        fn new(files: crate::world::reload::ReloadFiles) -> Self {
            ServerReloadContext::new(files)
        }
    }
    impl ReloadContextNew for ClientReloadContext {
        fn new(files: crate::world::reload::ReloadFiles) -> Self {
            ClientReloadContext::new(files)
        }
    }

    pub trait GlobalLootModifierManagerNew {
        fn prepared(
            entries: Vec<(Identifier, std::sync::Arc<dyn crate::GlobalLootModifier>)>,
            owner: std::sync::Arc<dyn std::any::Any + Send + Sync>,
        ) -> Self;
        fn bind_from(&self, source: &Self);
    }
    impl GlobalLootModifierManagerNew for crate::GlobalLootModifierManager {
        fn prepared(
            entries: Vec<(Identifier, std::sync::Arc<dyn crate::GlobalLootModifier>)>,
            owner: std::sync::Arc<dyn std::any::Any + Send + Sync>,
        ) -> Self {
            Self::prepared(entries, owner)
        }
        fn bind_from(&self, source: &Self) {
            Self::bind_from(self, source);
        }
    }
    pub trait ServerReloadListenersOrder {
        fn new() -> Self;
        fn finish(
            self,
        ) -> Result<crate::world::server_reload::OrderedServerReload, ReloadOrderError>;
    }
    impl ServerReloadListenersOrder for crate::world::server_reload::ServerListenerRegistry {
        fn new() -> Self {
            crate::world::server_reload::ServerListenerRegistry::new()
        }
        fn finish(
            self,
        ) -> Result<crate::world::server_reload::OrderedServerReload, ReloadOrderError> {
            crate::world::server_reload::ServerListenerRegistry::finish(self)
        }
    }

    pub trait ClientReloadListenersOrder {
        fn new() -> Self;
        #[allow(clippy::type_complexity)]
        fn finish(
            self,
        ) -> Result<
            Vec<crate::world::reload::RegisteredListener<dyn ClientReloadListener>>,
            ReloadOrderError,
        >;
    }
    impl ClientReloadListenersOrder for crate::client::client_reload::ClientListenerRegistry {
        fn new() -> Self {
            crate::client::client_reload::ClientListenerRegistry::new()
        }
        fn finish(
            self,
        ) -> Result<
            Vec<crate::world::reload::RegisteredListener<dyn ClientReloadListener>>,
            ReloadOrderError,
        > {
            crate::client::client_reload::ClientListenerRegistry::finish(self)
        }
    }

    pub trait ServerReloadResourcesInstall {
        fn install(
            &mut self,
            retained: Vec<crate::world::server_reload::RetainedEntry>,
            skipped: &std::collections::BTreeSet<Identifier>,
        );
    }
    impl ServerReloadResourcesInstall for crate::world::server_reload::RetainedListeners {
        fn install(
            &mut self,
            retained: Vec<crate::world::server_reload::RetainedEntry>,
            skipped: &std::collections::BTreeSet<Identifier>,
        ) {
            crate::world::server_reload::RetainedListeners::install(self, retained, skipped)
        }
    }

    pub trait BackendRefOf<'a> {
        fn of(inner: &'a dyn Backend) -> Self;
    }
    impl<'a> BackendRefOf<'a> for BackendRef<'a> {
        fn of(inner: &'a dyn Backend) -> Self {
            BackendRef::of(inner)
        }
    }

    pub trait BackendMutRefOf<'a> {
        fn of(inner: &'a mut dyn BackendMut) -> Self;
    }
    impl<'a> BackendMutRefOf<'a> for BackendMutRef<'a> {
        fn of(inner: &'a mut dyn BackendMut) -> Self {
            BackendMutRef::of(inner)
        }
    }

    pub trait ClientBackendMutRefOf<'a> {
        fn of(inner: &'a mut dyn ClientBackendMut) -> Self;
    }
    impl<'a> ClientBackendMutRefOf<'a> for ClientBackendMutRef<'a> {
        fn of(inner: &'a mut dyn ClientBackendMut) -> Self {
            ClientBackendMutRef::of(inner)
        }
    }

    mod reloadfilesnew {
        #[allow(unused_imports)]
        use crate::world::reload::*;
        #[allow(unused_imports)]
        use crate::*;
        #[allow(unused_imports)]
        use crate::{
            FileToIdConverter, Identifier,
            world::reload_order::{ReloadOrder, ReloadOrderError},
        };
        #[allow(unused_imports)]
        use std::{collections::BTreeMap, sync::Arc};
        pub trait ReloadFilesNew: Sized {
            fn new(files: BTreeMap<Identifier, Vec<u8>>) -> Self;
        }
        impl ReloadFilesNew for ReloadFiles {
            fn new(files: BTreeMap<Identifier, Vec<u8>>) -> Self {
                ReloadFiles::new(files)
            }
        }
    }
}

/// Construct a fluid state from load-resolved data supplied by the engine boundary.
pub fn fluid_state(
    fluid: crate::FluidKey,
    amount: u8,
    falling: bool,
    source: bool,
) -> crate::FluidState {
    crate::FluidState {
        fluid,
        amount,
        falling,
        source,
    }
}

#[path = "__private/ingredient_parser.rs"]
pub mod ingredient_parser;
#[path = "__private/platform.rs"]
pub mod platform;

/// Side- and override-filtered ordinary dependency rows, in their original direction.
/// Host-only: this is not the author-facing package inventory.
// no wire: the host keeps this resolved graph for package and Part admission; it never crosses.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct EffectiveDependencies(
    std::collections::BTreeMap<String, std::collections::BTreeMap<String, crate::DependencyKind>>,
);

impl EffectiveDependencies {
    /// Record a required or optional row from the resolver.
    pub fn insert(&mut self, dependent: &str, target: &str, optional: bool) {
        self.0.entry(dependent.into()).or_default().insert(
            target.into(),
            if optional {
                crate::DependencyKind::Optional
            } else {
                crate::DependencyKind::Required
            },
        );
    }

    /// The surviving direct dependency, including an absent optional target.
    pub fn kind(&self, dependent: &str, target: &str) -> Option<crate::DependencyKind> {
        self.0.get(dependent)?.get(target).copied()
    }

    /// A Part's own package or a surviving direct dependency may own its key.
    pub fn permits(&self, dependent: &str, owner: &str) -> bool {
        dependent == owner || self.kind(dependent, owner).is_some()
    }
}

include!(concat!(env!("OUT_DIR"), "/mod_api_exports_private.rs"));
