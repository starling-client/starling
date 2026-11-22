//! Hook and helper capability notes, discovered with the rest of the hook catalog.
pub trait Notes {
    #[notes = r#"## Hook and helper capabilities

- `ClientHost::panel_items` shows an item panel beside container screens; `panel_event` supplies the player's
  clicks, searches and keys over its items. `Host::recipes_of_type` returns `Recipe` values with parsed ingredients,
  results and rules for supported vanilla types, and the original JSON for mod-defined types. `Recipe` does not
  define display pages, per-result chances or fluid input/output slots: a viewer interprets its own machine-recipe
  JSON and draws those details itself. `known_recipes`, `place_recipe` and `transfer_recipe` provide recipe-book
  integration, `transfer_recipe_check` its dry run on the server; they do not create a machine-recipe display record.
  `native_sprite` takes an optional `tint` (`0xAARRGGBB`) that multiplies every texel, as a fluid's colour does.
  `DrawContext::text(text, at, argb, shadow)` takes its colour as vanilla's `GuiGraphics` does, `0xAARRGGBB`
  (`0xFFFFFFFF` is opaque white), and draws nothing for a fully transparent one, as vanilla's does.
  `DrawContext::text_width(text)` measures what `native_text` draws, in GUI pixels, to lay text out before drawing
  it (`Font.width`). A sprite's own size is `gui_sprite(&id)`'s `size`, read once in `texture_atlas_stitched`.
  Diverges from upstream: there `native_text` takes an opaque `0xRRGGBB`.
  Slot clicks keep their `SlotClicked` numbering: inputs, then results.

- `GameLevel::native_send_particles` on authoritative servers. It sends supported
  vanilla particles to nearby players. Block particles require a valid block state; item particles
  require a validated stack. Trail and vibration remain unsupported. Count zero sends one directional
  particle. The return value says whether the call was accepted, not whether any player received it.

- Authoritative world lifecycle events through `world_lifecycle`: open failed, final close and
  close failed (contract version 2). **Migration (`WorldPhase::WorldOpenReady` → `server_started`):** a mod that matched
  `WorldOpenReady` in `world_lifecycle` implements `server_started` instead, fired at the same moment (after the worlds
  open, before the tick loop; one call per successful open, with the same `dedicated` flag as `server_about_to_start`);
  delete the match arm. Nothing else about the phases changed. Diverges from upstream: upstream still has the phase.

- Authoritative server tick events through `server_tick`: start (before any level ticks, like NeoForge's `ServerTickEvent.Pre`) and end of each non-paused
  server tick, with `world_running`. A frozen world reports false except during a requested step.

- Server commands and players: `register_commands` adds usages to the server tree with
  `Command::executes` to bind their executors, with helpers to answer
  ([Server commands and players](#server-commands-and-players)).

- A player's items for server mods: counting, taking and giving go through the player's
  [`ENTITY_ITEMS`](../concepts/capabilities/entity-items.md) handler.

- Server menus: a vanilla chest menu a server mod opens and fills, whose clicks and closing reach the mod
  (`menu_clicked`, `menu_closed`), and trades on them between the players'
  [`ENTITY_ITEMS`](../concepts/capabilities/entity-items.md) handlers in one transaction ([Server menus](#server-menus)).

- Item tooltips: unless the stack hides them, `ItemPart::tooltip` adds lines where `Item.appendHoverText` does,
  after the name and before the components, with the same `TooltipAppendContext` as appenders in `event.context`.
  `Hooks::item_tooltip` (NeoForge's `ItemTooltipEvent`, version 3) then gets every item's whole tooltip once it is
  complete, lending `stack: ItemStackView<'_>` (call `project()` to copy components on demand),
  `display: &TooltipDisplay<'_>`, live `lines: TooltipLines<'_>` validated per edit with `TextFormError`
  (one invalid edit leaves all other lines intact), the local player, and its `TooltipFlag` (`advanced`,
  `creative`), and may add, change or remove any line. It also fires for `ItemResource::tooltip`
  queries; reentrant queries return base lines. Creative search uses exported lines and does not fire it.

- Data component tooltip lines: `Registrar::register_tooltip_appender(component, placement, appender)` (NeoForge's
  `RegisterTooltipAppendersEvent`) gives one component's `TooltipAppender` a `TooltipPlacement`: `BeforeAll`, `AfterAll`,
  or `Before(id)` and `After(id)` for a component detail Sunrise lists (`minecraft:lore`, `minecraft:enchantments`...), where
  the lines go among the other components' lines. The game asks only that appender, only for a stack that carries the
  component (live, transient ones included) and whose `tooltip_display` shows it, and only while its mod is enabled. It gets
  the stack and a `TooltipAppendContext` (local player, `advanced`/`creative`/`nested` flags, and the client level's
  `tick_rate`, or 20 without a level). Its lines go after the item's own and before durability and advanced lines.
  Creative search doesn't read these lines or `item_tooltip`; mods reacting to every item use `item_tooltip`.
  `Before`/`After` accept only the 23 vanilla anchors listed in `tooltip_appender.rs`, not other registered
  components. The `registerAppender(TooltipLocation, ...)` overload is not represented. Several appenders may register
  for one component; all run in registration order at an equal placement. The player is always the local player,
  including for nested crossbow projectiles where vanilla passes no player; use `nested` to distinguish those calls.

- `ItemPart::use_on` and `use_item` run on both sides (`HostSide`), version 2, with
  `Option<InteractionResult>` answers. `None` defers; `Some` replaces the template.
  Consuming answers end the client's hand loop: `Success` swings client-side, `SuccessServer`
  server-side, `Consume` never. Client hosts permit reads/effects, not authoritative writes.

- A block's own item-use behaviour, on the server for every player: attach a `BlockPart` and implement
  `use_item_on(host, event) -> Option<InteractionResult>`. `event.state` keys the per-state table;
  `event.click` carries the level, position, hit, hand, stack and player. `Some` replaces the template's
  `Block.useItemOn`: `Success`/`SuccessServer` consume and swing, `Consume` consumes without swinging,
  `Pass`/`Fail` let the item try its `useOn`, and `TryWithEmptyHand` tries the block's `useWithoutItem`
  on the main hand only. `None` leaves the template in charge. This is the block's behaviour, not
  NeoForge's separate `UseItemOnBlockEvent` broadcast interceptor. The earlier `right_click_block`
  event still decides whether the block is used at all. Dispatch visits only the clicked state's parts.

- Global interaction interception on both sides: `Hooks::use_item_on_block(host, event)` is NeoForge's
  `UseItemOnBlockEvent`. `event.phase` is `UsePhase::Block` just before the block's item method/Part,
  or `ItemAfterBlock` just before the item's `use_on` method/Part. It fires only after that phase's
  sneak, `use_block`/`use_item`, cooldown and other eligibility gates select it. `event.click`
  (`BlockClick`) supplies the dimension, hit, hand, stack and player. Return `Flow::Cancel` to
  skip that phase's Part and template, using `*event.cancellation_result` as the phase's result
  (default `TryWithEmptyHand`); a main-hand block-phase TRY can still run the empty method.
  `UsePhase::ItemBeforeBlock` fires first, before the item's `on_item_use_first` Part and before
  the block is asked anything (ahead of the sneak gate, unless `right_click_block` denied the item);
  canceling skips that item's Part, but `Pass` continues to the block. Any non-`Pass` answer ends the block interaction.
  Client Parts predict hands only; server-only answers cannot retroactively suppress client offhand use.
  `Fail` ends only block interaction; `Pass`/`TryWithEmptyHand` fall through.
- `ItemPart::on_item_use_first(host, event)` is `IItemExtension.onItemUseFirst`, server side, for the
  held item (`event.click` is the same `BlockClick`). Return `Some(result)` to end the click:
  neither the block nor the item's `use_on` runs, and `Success` swings the hand. `None` or `Pass`
  goes on to the block. It has no global method: NeoForge's bus event for this moment is
  `use_item_on_block`'s `ItemBeforeBlock` phase.

- `Hooks::right_click_item(host, event)` is NeoForge's `PlayerInteractEvent.RightClickItem`, before
  `Item.use` and the item's `use_item` Part on both sides, after the same hand's target interaction falls through.
  `event.click` (`ItemClick`) contains the dimension, player's feet block position, player,
  hand and stack. `Flow::Cancel` skips the Part/template and returns `*event.cancellation_result`
  (default `Pass`). `Success`, `SuccessServer` and `Consume` end the client's hand loop. `Success`
  predicts a swing, `SuccessServer` swings on the server, and `Consume` has no swing. A block
  success, block FAIL, or entity success prevents item fallback; the server fires this on the separate UseItem packet
  rather than on UseItemOn.

- A block's empty-hand behaviour: `BlockPart::use_without_item(host, event)` replaces
  `Block.useWithoutItem` on the server, on the main hand only, after `useItemOn` returns
  `TryWithEmptyHand`. The event keeps the original state's key even if the item phase
  changed the block. `Some` replaces the template; `None` runs it, including jukebox ejection,
  pot wobble and bookshelf removal. Consuming results trigger DEFAULT_BLOCK_USE.
  `Pass`, `Fail` and `TryWithEmptyHand` continue to the item; the empty method is never
  retried. Dispatch visits only the clicked state's parts.

- Breaking and placing a block, on the server for every player: `break_block` hears a player break a
  block once the game allowed it (`ServerPlayerGameMode.destroyBlock`) and may cancel it, which keeps the block and its
  drops. Set `exp_to_drop` to `Some(amount)` to replace the vanilla XP sample later, after item drops are prepared;
  `None` retains vanilla sampling. The subsequent `block_drops` event can edit the resulting experience or cancel
  the drops, and disabled block drops suppress both items and XP. `entity_place` (`EntityPlaceEvent`, `EntityMultiPlaceEvent`) hears a block placed from an item
  (`BlockItem.place`), an enderman putting its block down or a snow golem's trail, before it is written, and may cancel it:
  nothing is placed and the stack is kept. For a player, `placed_against` is the state next to the primary placed
  block opposite the clicked face, even when the clicked block is replaceable; for mobs it is the state below.
  A bed, door or tall plant arrives as one `Placement::Multi`. Silverfish
  infesting, turtle eggs and falling blocks are not placements yet.

- Block entities for a mod's blocks (`block_entity`, `block_entity_ticks`,
  `block_entity_tick`): made when the block is placed, ticked on the server and saved with the chunk.
  `block_entity_tick` receives `&mut BlockEntityTickEvent<'_>` without a separate Host.
  `event.subject.with_view` lends the resident type ID, saved data, item slots, and transient
  data slots for the closure; it returns `None` if the subject is no longer available. To edit
  Part state, return the handle from `event.subject.with_view(|view| view.handle())`. After the
  view borrow ends, borrow `event.subject.host()` separately for `edit_state`.
  Use `event.subject.transaction` to commit edits to the subject and other objects together.

- Item slots for those block entities (`r.block_container(&block, container)`): hoppers, comparators and breaking the
  block use them as they use vanilla's containers, and the tick's resident view lends the slots.

- Menus for mod containers: the container names one of vanilla's slot-grid or furnace menus, which opens when the block is used.
  The tick's resident view lends the data slots the menu shows.

- A machine reaches neighbouring storage through the block's Parts and capabilities, not a raw mutable
  block-entity callback. `block_entity.data_slots(host)` reads a mod entity's menu data on the server or
  the client's own copy; `set_data_slot(host, slot, value)` writes a declared slot on the server under
  the current transaction, rolls back on abort, and uses the normal menu broadcast to viewers.
  `level.set_block` uses the level's normal update path.

- Screens a mod draws for menu types: `r.register_menu_screen(menu, factory)` registers the constructor directly by menu type, matching NeoForge's `RegisterMenuScreensEvent` and vanilla's `MenuScreens.SCREENS`. It returns a fresh `MenuScreen::new(layout)` for each client opening. Decode `MenuScreenOpen::extra` during this call; the bytes are not available afterwards. Draw GUI textures (`assets/<namespace>/textures/gui`) with `MenuScreen::with_draw` and read live values from `MenuScreenFrame::data`.
  The factory goes with the menu type whatever opened the menu: a block's use or `player_open_mod_menu` from an item, an entity or a command. Built-in vanilla menu types that already have a screen in `MenuScreens.SCREENS` and duplicate registrations are refused; any mod may register a factory for a custom menu type. `open.block` is `None` without a verified opener.
  `None` entries in `MenuScreenLayout::slots` (including omitted trailing entries) start inactive/hidden (`Slot.isActive()`) and can be activated or deactivated locally with `set_screen_slot` by any mod; local slot changes neither reach the server nor survive a replacement. A bound book needs a registered factory for its menu: set the book in the layout.

- Buttons and tooltips on mod screens: install client callbacks with `MenuScreen::with_click` and `MenuScreen::with_tooltip`. A button returned by the client is a request: the server still checks its window, menu validity and the owning part's `menu_button_valid` before calling `menu_button`, with the backing block entity if present. A mod inserts items through the block's item capability, `level.capability(host, &ITEMS, pos, side)`.
- `menu.click_button(host, button)`, on a client `MenuRef`, presses button `button` of the open menu as a click on one of its buttons does
  (`MultiPlayerGameMode.handleInventoryButtonClick`, `ServerboundContainerButtonClickPacket`); the server's menu decides what it does
  (`clickMenuButton`, and `menu_button` for a mod's menu). It is for a client mod's own input on a menu's screen other than a press, such as a key or the wheel (`screen_key_pressed_pre`, `screen_mouse_scrolled_pre`). It returns `false` for a stale
  handle and for the inventory menu, which no server opened.

- What a server mod needs to guard land and move players. `right_click_block` hears a player's right
  click on a block before the block's own use (a chest, a door, a lever) and the item's, and may cancel it
  with NeoForge's answer: `Fail` stops the click, and the player sees the block as it
  is. `host.players().acting_player()` returns the acting player handle;
  `host.commands().run_command` runs a command as the server's console (level 4, no output), so `execute in <dimension> run tp
  <player> <x> <y> <z>` moves a player across levels; `PlayerRef::action_bar` tells a player why, even inside their own
  hook; and the handles `host.players().server_players()` lists expose health through `player.entity().living(host)`.

- What a server mod needs to keep the world itself out of guarded land. `block_spread` hears a
  fluid flow or fire spread into a block, `native_piston_pre` a piston's push or pull, `explosion_detonate` the
  blocks an explosion is about to break (a mod takes out those it spares), `fill_bucket` a bucket filled or
  emptied, and `attack_entity` a player's blow on an entity (an item frame, an armor stand, an animal, a
  player). Each may cancel, and nothing happens.

- `level_tick_pre` and `level_tick_post`, both-sided observations around each
  level tick pass. Each receives the dimension ID; neither can cancel the tick. Paused and held passes fire
  neither hook, and an error skips the post hook.

- `native_set_block`, a server-side `Game` helper that replaces a state in the current dimension through
  `Level.setBlock`. It accepts vanilla's neighbour and shape flags and returns whether a block changed; a block with a
  block entity gets one. `UPDATE_CLIENTS` sends the update; `UPDATE_INVISIBLE` suppresses notification only on the client, as in `Level.setBlock`. Hooks holding only `GameLevel` cannot call it yet.

- `GameLevel::play_sound` for registered vanilla sound events on an authoritative server.
  It queues the sound for nearby players in the level's dimension, including during a guest's turn.
  Pass a `SOUND_SOURCES` index, position, volume and pitch. The return value says whether the call
  was queued, not whether a player heard it. The server discards unknown sound events when it dispatches
  the queue.

- `Host::config_files()` retains the calling mod's `config/<mod id>/`, `<mod id>.<ext>` and `<mod id>-<rest>` ownership spelling and 4 MiB `CONFIG_FILE_MAX`; a world's `serverconfig/` copy is read through `config_loading`.
  `Host::data_files()` on both sides instead owns `<game directory>/sunrise-mod-data/<mod id>/`; server-only `ServerHost::world_data_files()` owns `<world>/sunrise-mod-data/<mod id>/`, copied, deleted and backed up with the save (`NoWorld` without a world).
  All three share `ConfigFiles`, `ConfigPath`, `ConfigEntry` and typed `ConfigFileError`; bulk paths omit the mod id, relative to the implicit mod root. Settings remain separate, one file per key, with unchanged scanning; bulk files are never scanned.
  Sunrise's bulk cap is `DATA_FILE_MAX` (64 MiB), not the config cap. Reads, writes and stats enforce their root; writes replace bytes atomically, never leaving a partial file.
  Checked `/`-separated paths refuse traversal, backslashes and symbolic links; another mod's files, missing calling-mod context, oversize files and unavailable operations receive typed refusals.
  `list(&folder, after, limit)` returns lexicographically ordered names strictly after the cursor, at most `limit` entries; start with `None`, continue with the last name, and stop on an empty page.
  `ConfigPath::root()` lists the mod's own root; `limit` must be 1 through `CONFIG_LIST_MAX` (1024), otherwise `InvalidPage` ([config and bulk storage](../how-to/mod-settings.md)).

- To draw custom status rows, replace or wrap `minecraft:player_health` (hearts), `minecraft:armor_level`,
  `minecraft:food_level` or `minecraft:air_level` in `register_gui_layers`. Use the installed body's
  `DrawContext` (`gui_paint` accepts a `GuiDraw` list). `gui_layer_pre` can veto a slot without vetoing
  the other rows. Heart bookkeeping belongs to the vanilla heart body; bubble sounds belong to the
  vanilla air body.

- Which hearts the player's health bar draws through `player_heart_type` (NeoForge's `PlayerHeartTypeEvent`):
  when the vanilla player-health body draws its hearts, `kind` comes in as vanilla's pick (poisoned, withered,
  frozen or normal) and a mod may set another. Only the heart sprites change.

- Where each boss bar is drawn through `customize_boss_event_progress` (NeoForge's
  `CustomizeGuiOverlayEvent.BossEventProgress`): once a frame per bar, in the order the server added them, `x` and
  `y` come in as the bar's top-left corner (91 left of the centre; 12, then 19 lower for each bar) and `increment` as
  the spacing to the next (19). The bar and its title are drawn where a mod moved them, and the next bar comes in
  `increment` below where this one came in. A cancelled bar is not drawn and its increment still applies. Bars stop
  once the next would start a third of the way down the screen. With no listener nothing is built per bar.

- Where the in-game chat is drawn through `customize_chat_overlay` (NeoForge's `CustomizeGuiOverlayEvent.Chat`):
  each frame the chat layer draws, `x` and `y` come in as the chat's bottom-left corner (0 and the GUI height minus 40)
  with the GUI size and whether the chat screen is open, and the chat is drawn shifted to where a mod moved them. Not
  asked while the chat is hidden (F1 with the chat closed). Only the drawing moves; the input box and clicks on
  messages stay in vanilla's place, as with NeoForge.

- `entity_interact` and `ItemPart::item_interact_mob` run on both sides with native `HostSide`,
  version 2, answering `Option<InteractionResult>`; the latter includes the interaction hand.
  `None` defers; consuming results end the hand loop with the swing rules above; nonconsuming entity answers fall through.

- Left-clicking a block, on the server for every player: `left_click_block` hears the start of a dig
  (the block, the face hit) once the game allowed it, before the block is attacked (a note block plays) or broken in
  creative. A mod that cancels it keeps the block: on the server it isn't attacked, dug or broken, and the player's client
  gets it back. The clicking player's own client still predicts the crack until then.

- A player's tick: `player_tick_pre` and `player_tick_post` match NeoForge 26.3's
  non-cancellable `PlayerTickEvent.Pre`/`Post` on **both logical sides**, including
  ordinary single player's client window and server-thread players. Match the typed
  `HostSide`; the event's `player: PlayerRef` unambiguously names the ticked player
  (including other client-side players). On the server the helpers read that acting
  player. Pre precedes sleep/cooldown/body work; Post follows it, including item,
  arrow and XP pickups. Dead players still receive the pair without enabling dead
  actions. Players tick under `/tick freeze`, not while paused. With no listeners,
  the added per-player cost is only the indexed subscriber probe for each endpoint;
  listeners on the server pay an acting-guest enter/leave at each endpoint, reusing
  the already resolved guest index (no second connection lookup).

- Shape updates a mod can answer through `native_update_shape`: on an authoritative server, each time a
  block's state is worked out from a neighbour (`BlockBehaviour.updateShape`): neighbour shape updates after a
  `setBlock`, a block fitted to all its neighbours as a piston lands it or a chunk is post-processed, and the blocks at
  a feature's edge as it is placed in the live world, such as a tree grown from a sapling
  (`StructureTemplate.updateShapeAtEdge`). It gets the dimension, position, state, the neighbour's direction, position
  and state, before the block computes it. `state` may not be in the world yet (a piston landing it), so decide from
  `state`, not from the block at `pos`. Answer only for your own blocks and leave `new` empty otherwise: a state put
  there is the block's new state, applied as vanilla applies the result (air breaks the block, with its drops, on a
  neighbour update; post-processing and a feature's edge write air without drops), and the block's own `updateShape`
  doesn't run, nor the ticks it would schedule. The level is read-only, as vanilla's `LevelReader` is there; the hook
  schedules ticks with its `ticks` through the level's `native_schedule_block_tick(ticks, pos, block, delay)` and
  `native_schedule_fluid_tick(ticks, pos, fluid, delay)` (vanilla's `ScheduledTickAccess.scheduleTick`; false for an
  unknown name or a position that isn't loaded). It runs often, at least six times per `setBlock` that updates shapes
  (more for redstone wire's diagonals and knock-on updates): keep it short. A LAN guest shows vanilla's shape for a
  moment, until the server's update arrives. Chunk generation fires none.

- Random ticks a mod can handle through `random_tick`: on an authoritative server, each block
  `ServerLevel.tickChunk` random-ticks (`BlockState.randomTick`), with its dimension, position and state, before the
  block handles it. Only blocks that tick randomly fire it (a mod block as its vanilla template block does), once per
  draw that lands on one: it runs often, so keep it short. Bone meal's random ticks (`StemBlock.performBonemeal` calls
  `randomTick` itself) don't fire it. The hook may change the block with the level's
  `native_set_block_property(ticks, pos, key, value, flags)` and set `handled` to skip the block's own random tick, so a
  mod crop grows its own way. Without `handled`, the block then takes its own random tick as it is now (if it still
  ticks randomly), and lava's fluid random tick runs either way. It needs the level mutably, which only `random_tick` and `tick_block` give. It is `Game::native_set_block_property` with `Level.setBlock`'s flags (`native_hooks::flags`): a
  vanilla crop grows with `flags::UPDATE_CLIENTS` (neighbours' shapes update, so an observer fires, but no neighbour
  gets `neighborChanged` and comparators aren't told), a furnace sets its `lit` with `flags::UPDATE_ALL` (both). Sunrise
  sends updates when `UPDATE_CLIENTS` (2) is set; `UPDATE_INVISIBLE` (4) suppresses notification only on the client, as in `Level.setBlock`. `UPDATE_IMMEDIATE` (8) has no effect yet. It writes at any loaded position, returns false off an authoritative server (the `Game` one too: a window that isn't a server), for a property or
  value the block doesn't have, a position that isn't loaded, or a property that already has the value (nothing is written:
  vanilla's `Level.setBlock` returns false for an unchanged state, as `native_set_block` does), and true when the block changed. Elsewhere, `Game::native_set_block_property_with(pos, key, value, flags)` does the same, as a brewing
  stand's tick sets its bottles with `flags::UPDATE_CLIENTS`.

- Scheduled block ticks a mod can handle through `tick_block`: on an authoritative server, each due
  scheduled tick of a block that is still there (`ServerLevel.tickBlock` → `BlockState.tick`), with its dimension,
  position and state, before the block handles it. The hook may change the block with the level's
  `native_set_block_property` and the flags the block's own tick would write with (an observer's tick
  uses `flags::UPDATE_CLIENTS`) and set `handled` to skip the block's own tick. Without `handled`, the block then takes
  its own tick as it is now, if it is still the tick's block.

- The commands a mod owns on the client, as NeoForge's `RegisterClientCommandsEvent`: `register_client_commands` is asked for a `Command` per usage (`x <mode:word>`, the same usage form as `register_commands`), so the chat box colours, completes and lists them, arguments included, like the game's own, and `command` runs them. A name the game already has keeps that command; a nonzero `level` has no meaning on the client and the command is left out with a log line. It replaces `commands`, which gave bare names: a mod that implemented it pushes `Command::new("name <args:text>")` instead. Asked at connect, never per frame.
- The keys a mod registers on Options > Controls, as NeoForge's `RegisterKeyMappingsEvent`: call
  `Registrar::register_key_mapping(KeyRegistration::new("zoom", "key.keyboard.z").label("key.mymod.zoom"))`
  from `Hooks::register`. Ownership comes from the registrar; invalid or duplicate local ids refuse startup.
  `.category(...)` takes a translation key, `.modifiers(...)` sets shift/control/alt, and `.context(...)` takes
  `KeyConflictContext::{Universal, InGame, Gui}`. Only client startup collects the retained declarations for Controls;
  dedicated and integrated servers do not. One editor persists keys and modifiers in `sunrise-mods.txt`.
  Conflicts compare vanilla and mod bindings, context and modifiers, excluding unbound and both-default pairs.
  Resolve a `ModKeyMappingRef` once with `host.mod_key_mapping(owner, id)` during `client_setup`. Retain
  it for `binding(host)` (current name borrowed from the host, plus modifiers), `translated_key_message(host)`
  (current Controls label on demand), `is_down(host)` and `consume_click(host)` (`KeyMapping.isDown`,
  `consumeClick`). Any mod can `set_binding(host, key, modifiers)` or `reset_binding(host)` on a
  resolved mapping, which take effect from the next input and save by owner and id. `NotRegistered` is a missing owner/id at
  resolution; `Unavailable` means this host or phase has no client input access. `Stale` means a retained ref
  belongs to a replaced client or registration roster: resolve again there. Binding edits alone do not
  invalidate the ref. Persist names, not refs.
- The `command` hook runs a client command: the local player's typed or clicked chat command, without its slash, before the server and the game's own commands see it; set `ran` when the mod ran it. Client-side, with the `ClientHost`. Contract version `2`: a mod written for `1` was given the side-typed host, and was also asked on a server for commands run there (command blocks, LAN guests, the console, the integrated server thread); now only the client asks, and only for its own player's commands.

- Recipes of any type (`RecipeManager.getAllRecipesFor`), vanilla's (`minecraft:smelting`, `minecraft:campfire_cooking`) and a mod's own (`mymod:crusher`, for a machine or an altar) through one read: `host.recipes_of_type("minecraft:smelting")` gives each recipe in force, in file order, as a `Recipe`: its `id`, `kind`, `group`, typed `ingredients` and `results` (for shaped, shapeless and cooking recipes, as the game parsed them; a recipe the game cannot parse is not in force and not listed), `cooking_time` and `experience` for the cooking types, and its file's `json`, which is all a mod's own type has. The recipes are those of `data/<namespace>/recipe/` in vanilla, content mods (`.smod`) and the world's data packs, with data packs merged and load conditions held, so a pack's override or removal shows; a type nobody uses gives nothing. `Host::recipe_ingredient` and `Host::recipe_result` read the usual fields as JSON text, tags included (a bare id is read as `minecraft:`; items the game does not know are left out, and the helper returns `Ok(None)` if none is left). Invalid JSON returns `RecipeJsonError::InvalidJson` rather than panicking. The recipes are the server's table: on the server thread, a dedicated server or a LAN host; a client, the single-player window included, sees vanilla's and the content mods' recipes, not the world's data packs', unless a mod had the server send it the type (`native_datapack_sync`). All three give nothing outside a world. The game crafts the recipes of its own types itself, and shows none of a mod's own in the recipe book; `/reload` reads a data pack's again, so look them up again when you need them. The lookup borrows the table, with no copy.

- [mobs](#mobs) for server mods: spawn one, read the mobs of a level in place, and move, hurt or remove one by its id.

- Redstone from a mod's block, on authoritative servers. `block_signal` may change the signal a block gives on
  a face, weak or direct, and `is_signal_source` lets redstone wire connect to it. Both are asked very often, so check the
  block first and answer from the mod's own state. When the output changes, and once the block is placed (a direct signal past a conductor isn't read before),
  `level.update_neighbors(host, pos, state, NeighbourUpdate::Neighbours)` makes the blocks around it read it again
  (`Level.updateNeighborsAt`; a block that powers what it is attached to, as a lever does, also updates there). A
  diode's front takes `level.neighbor_changed(host, front, state)` then `NeighbourUpdate::NeighboursExcept(facing)`
  (`DiodeBlock.updateNeighborsInFront`). Both act on the level in place and are refused elsewhere.

- Item behaviour, on the server for every player: `hurt_enemy` hears a player's hit land on a mob with the
  held stack, `inventory_tick` runs each tick, on the `ItemPart`s of every stack a player carries (armour and off hand included), and
  the stack is the place `ItemPlace::PlayerSlot { player, slot }`, read and written as any item place, and
  `on_equip` hears an equipment slot take a different item.

- Hearing players and mobs on the server for every player: `player_died`, `mob_spawned`,
  `mob_killed` (a player's melee blow, a sweep or a spear thrust too),
  `item_toss` (`ItemTossEvent`: a throw, with the item entity made but not yet in the level, which the `item_entity_*` methods read and change; not the items scattered when a player dies; `canceled` keeps the entity out of the level and the items are lost) and `item_picked_up` (the ground item entity; `can_pickup`: `False` leaves the stack on the ground, `True` takes it despite its pickup delay or another player's target), then `item_picked_up_post` (`ItemEntityPickupEvent.Post`: the same item entity, a copy of the stack before the pickup and the live remainder on it, which a listener may set; heard on a whole or partial pickup, never when nothing went in). They name a player by UUID.

- [Server data reload listeners](#server-data-reload-listeners) read a mod's pack resources with datapack overrides, once per reload.
- [Client asset reload listeners](#client-asset-reload-listeners) read a mod's pack resources with resource-pack overrides, once per reload.

- The item entities near a block, for machines that pick up items as a hopper does:
  `native_item_entities(dimension, min, max)` lists those whose box meets `min`..`max` (vanilla's
  `Level.getEntitiesOfClass(ItemEntity.class, box)`), each with its entity ID, position, stack and pickup delay, and
  `ItemEntityRef::set_stack` changes one's nonempty stack; discarding an entity is a separate operation. The query acts on
  the level in place.

- `level.with_random(host, |rng| rng.next_int_bound(bound))` borrows the named server level's live
  stream (`Level.getRandom()`), including levels parked while the server ticks another. In a restricted
  level hook use `host.with_random(|rng| rng.next_int_bound(bound))` to borrow the hook's level stream.
  The borrow ends with the closure. Without an authoritative, resident stream to lend, including for a
  stale level handle, the call returns `None` and skips the closure. Draws advance the live stream
  directly and are not rolled back on panic/unwind or transaction abort, matching vanilla
  `Level.getRandom()`. A mod that sets its own seed uses a separate `LegacyRandomSource`.

- Server lifecycle hooks run once per start, on both the single-player server thread and a dedicated server.
  `server_about_to_start(dedicated)` runs before worlds load (`ServerAboutToStartEvent`).
  `server_starting(dedicated)` (`ServerStartingEvent`) follows world and pack loading and spawn preparation on dedicated servers, or immediately follows about-to-start on integrated servers. Both precede accepting play.
  `server_started(dedicated)` runs after every level loads but before the first tick (`ServerStartedEvent`).
  On every exit after startup began, including a failed open, failed tick, or caught panic,
  `server_stopping(dedicated)` (`ServerStoppingEvent`) runs before player cleanup and the final world save attempt.
  `server_stopped(dedicated)` (`ServerStoppedEvent`) runs last, after network teardown, flushing saves and releasing
  storage, even when the save failed or a shutdown listener panicked. An early callback panic (including
  `config_unloading`) still reaches the final adopted-world save attempt and full server/network teardown,
  not just unpublishing or flushing queued writes. The first failure is retained if cleanup also fails.
  These are server-side, mutable `ServerHost` notifications, not cancellable.
  A failed open never overwrites a world the server did not successfully adopt: the session lock is taken before the prepared level is installed, and only that installation allows the final save. `WorldOpenFailed` or `WorldCloseFailed`
  still reports the failure; `WorldCloseFinal` is only emitted after a successful orderly close. No tick-time cost:
  these hooks dispatch only at server startup and shutdown.

- Mod block entities are sent to clients. Only declared types receive wire IDs after vanilla's, and several valid
  blocks can share a type. Server and client use its declared identity; `block_entity` supplies only initial data.
  `block_entity_update_tag` gives what clients get of one (`BlockEntity.getUpdateTag`, read from the block entity
  alone): it is asked when a chunk holding it is sent and when its block changes, and the client merges it into the
  copy's `nbt`. A saved chunk sent past the view distance asks it on its own thread, so read only `entity`. Keep it to what clients show (a machine running, its facing, a tank's level), not a menu's progress:
  menus have their own data slots. `block_entity_data_received` notices merged client data with a `Chunk` or `UpdatePacket` source; empty chunk tags and absent packet tags are silent, while empty packet compounds notify. Related `IBlockEntityExtension.onDataPacket` and `handleUpdateTag` perform loading, not this post-merge notice; they are not counterparts.

- One-shot fields ride that tag. `view.set_client_update_fields(fields)` on a `BlockEntityView` queues them in
  the runtime under the entity's identity; they are not part of the plain-data entity,
  so saves, clones and comparisons never see them and its revision stays. Once a block change (`block_entity_sync`)
  reaches viewers, the game builds one tag, puts the fields over `block_entity_update_tag`'s and sends it to
  them all. A change nobody sees, a chunk send and a single player's resend leave them waiting; removal or unload drops them.

- `affect_neighbors_after_removal` is a mod block's `Block.affectNeighborsAfterRemoval`: the block was
  removed from `pos` (`moved`: a piston pushed it) with `UPDATE_NEIGHBORS` or the piston flag. There is no bus
  event for it, so it is a `BlockPart` method only. The block's own class has already run its updates, as an
  override's `super` call; the part then pushes onto `updates` what it wants, in order: `NeighbourUpdate::OutputSignal`
  (`updateNeighbourForOutputSignal`), `Neighbours` (`updateNeighborsAt`) and `NeighboursExcept(direction)`
  (`updateNeighborsAtExceptFromFacing`), and the level runs each at once with vanilla's own functions. It runs
  inside the level's write with the level host, so it reads but cannot write blocks or spawn entities; a mod that
  needs either reports the gap on #44. The cost with no part on the block is one `has_part_hook` read.

- `visuals`: a world-frame hook that appends block-rim and
  colored-light requests (`BlockRim::new(pos, argb, width)`,
  `ColoredLight::new(at, rgb, radius, intensity)`). Rims trace the client's existing block outline edges,
  depth-tested against terrain. Lights add camera-local RGB illumination to the
  normal terrain/entity shaders, not gameplay light. Only the first eight rims
  and eight lights are examined; positions must be within 32 blocks of the player,
  values must be finite and bounded, and source chunks must be loaded. At most
  256 rim edges and 16 colored lights are rendered. Requests are frame-local,
  so disabling a native mod or leaving the world clears them without a retained
  state. Helpers `native_lit_block_name` and `native_flame_particle` read loaded
  blocks and emit a nearby vanilla flame particle; native mods must rate-limit
  emission. It also adds `emissive`: a world-frame hook for at most 32 pairs of
  already-loaded terrain sprites. Transparent pixels in an overlay leave its
  base unchanged; opaque pixels are drawn full-bright without changing gameplay
  light or the surrounding rock. The source pack must supply the overlay texture
  under `assets/<namespace>/textures/emissive/`. When no mod requests overlays,
  the terrain shader skips the lookup.
  When a native mod bundles a Sunrise shader pack, naming its folder
  `native-<mod-id>` makes the Mods On/Off control pause that pack too (without
  changing the player's saved shader selection); other packs are unaffected.

### Custom ingredient metadata
`ingredient` receives its recipe metadata as an API `Compound`. Recipe JSON is
converted at load by the shared JsonOps-to-NbtOps boundary; mod code matches
`Tag` variants rather than JSON fields. Nested null refuses the recipe as an
unclaimed kind does; numeric and heterogeneous arrays remain lists. Read `event.data`
while claiming the kind and capture the parsed configuration in the predicate. Matching
borrows the live stack, not the recipe's JSON or its converted compound."#]
    fn capabilities();
}
