//! Notes that belong to no one hook or helper: the general catalog, counterpart and roadmap prose.
pub trait Notes {
    #[notes = r##"## Named hook catalog (draft)

The declarations map stable hook names and contract versions to source targets. A contract includes its signature,
timing and behavior: retain it or version incompatible changes.
Each listener's edited stack-list outputs are validated before leaving its scope. A refused output keeps the immediately
preceding accepted value; scalar outputs and cancellation still apply, and later listeners remain independent.
One warning per refused output names its owning mod, hook, output and conversion error.

### Reading server blocks (draft)

A native server tick hook can call `native_server_block` with its context, a dimension id and a block position. It returns the block name and state properties from a loaded chunk. It does not load a chunk or open a dimension. Reads outside that hook, from a stale context, from unavailable dimensions or outside the world bounds return an error.

### Neighbour observations (draft)

`neighbor_changed` observes each authoritative server `neighborChanged` before the block handles it. It supplies the dimension, position, current owned `BlockState`, changed `BlockKey` and moved flag. The changed block key comes from the source block's default state through the prepared state index, after the primary state's routing guard. No registry-name lookup or allocation occurs on this path. Use explicit immutable block metadata for property queries; the hook does not load chunks or change the update queue. Its host is the level with its ticks (`&mut dyn LevelTicksHost`), as vanilla's `neighborChanged` gets the level: a `BlockPart::neighbor_changed` can read its inputs there with `level.signal(pos, direction)`, `direct_signal`, `direct_signal_to`, `best_neighbor_signal` and `has_neighbor_signal`, the same reads as `LevelRef`'s (mods' parts included), and schedule its own tick with `level.schedule_block_tick`, as a diode does (`DiodeBlock.checkTickOnNeighbor`). The reads write nothing and draw nothing, so they leave the update as it was, and cost what the game's own call does. `tick_block` and the other hooks with a `LevelWriteHost` have them too. Contract version `2`: version `1` handed the hook a plain `&dyn LevelHost`. Diverges from upstream.

### Server commands and players

For mods that run where the world is simulated: a dedicated server, a LAN host, single player's server thread
(`crates/sunrise-client/src/game/modding/native_server.rs`).

- **`register_commands(commands)`**: push a `CommandRegistration` per usage with
  `Command::new(usage).executes(executor)` or `Command::op(usage, level).executes(executor)`.
  The server asks when a world opens, reloads data packs, or refreshes its commands.
  A usage is words, then arguments `<name:kind>`, optional ones last in brackets:
  `pay <player:player> <amount:integer>` or `balance [player:player]`.
  Kinds: `word`, `text` (the rest of the line, last), `integer`,
  `number`, `boolean`, `player`, `item`, `block_pos`, `vec3`, `id`, and any command argument type a mod registered
  (`Registrar::register_command_argument`): its namespaced id is the kind, as in
  `<machine:mymod:machine>`. A custom argument is one word (or the rest of the line when its type says
  `Greedy`) and delivers its type's canonical value to the executor. The usages of one command merge into
  one tree, which every player receives after vanilla's (filtered by their level, as vanilla's is), so their
  chat box completes and colours it. A client that does not know a custom type still parses and completes it:
  the tree sends the argument as a Brigadier string and asks the server for its suggestions. Fabric sends
  the registered argument type's id instead, which an unmodded client cannot read; the string keeps its
  tree valid. The server also parses player and console invocations against that tree, including each
  usage's permission and argument types, before calling the executor as the registering mod.
  Each executable usage keeps its own permission: an operator-only `foo` remains unavailable to ordinary players
  even if `foo bar` is public. Players can traverse the shared `foo` prefix to reach `bar`, but cannot execute
  `foo` itself. Optional arguments make their preceding prefixes executable at that usage's level; if several
  accepted usages execute at the same node, each later accepted registration overwrites that node's executor
  (matching Brigadier's `CommandNode.addChild`) together with Sunrise's paired execution permission level
  for that node. A rejected usage does not change the permissions of accepted usages.
  Clients receive the accepted command tree, never its executor bindings. When argument alternatives consume the same input without errors, an
  executable alternative takes precedence over a denied one; equally valid alternatives keep registration order.
  A name vanilla has, or a usage that doesn't parse, is left out with a warning in the log.

- **Command executor**: the closure receives `(&mut dyn ServerHost, &CommandSource, &CommandInvocation)`.
  The invocation has its literal path, its `dimension` and
  `position` after `execute in/at`, and named `CommandArgumentValue` values, not raw command text. Integer, number and boolean
  values retain the dispatcher's types; word and greedy text are strings. Player selectors resolve to a single live
  `PlayerRef`, items become component-bearing stacks of one, and block positions resolve source-relative/local
  coordinates without requiring loaded chunks. `mod_id` is a loaded package id as a string; `resource:<registry>`
  is `CommandArgumentValue::Id`, an id that registry holds. A custom argument is `CommandArgumentValue::Custom
  { kind, value }`: its registered type's id and what its parser returned. Missing optional arguments are absent.
  Parsing, permission and resolution failures produce feedback without calling the executor. The console resolves from the world spawn's
  block corner, in the level the spawn is in, with zero rotation and no source entity (`@s` fails).
  Return `Some(n)` for success, including `Some(0)`: `execute store result` receives `n`. Return `None` for failure;
  no other mod is asked. Clients get the command tree's display data, not the executors.
- **`armor_stand_manipulate(player, name, stand_id, slot, cancel)`**: a player takes an item from an armor stand or
  puts one on it, heard after the stand's disabled-slot checks and before anything moves. `slot` is `mainhand`,
  `offhand`, `feet`, `legs`, `chest` or `head`. Set `cancel` and nothing changes (the click is spent). The player's
  state is swapped in; `player` is their UUID. Not told of a click that changes nothing; breaking or blasting a stand and
  a dispenser armouring one are other ways its equipment moves and are not covered.
- **`vehicle_moved(player, kind, from, to)`**: a boat or minecart that carries a server player (`player`, its UUID)
  moved since the last tick, heard once a tick after the move with its positions before and after. The rider's state is
  swapped in, so `host.players().acting_player`, `PlayerRef::tell` and the item helpers reach them. A vehicle a player rode and left is
  still told of, with no `player` and nobody swapped in, until it has stood still for ten seconds. Mounts (horses...) and
  vehicles nobody ever rode are not covered. It only hears and costs one position compare a tick for each watched vehicle.
- **`container_opened(player, name, kind, pos, cancel)`**: a player is about to open one of the game's own menus
  (a chest, furnace, crafting table, villager, anvil, lectern, a horse's inventory...). `kind` is the menu type
  (`minecraft:generic_9x3`, `minecraft:merchant`...), `pos` the block it belongs to, none for a villager, a mount or a
  vehicle's chest. Heard after the checks that refuse the menu (a locked chest), before the open menu closes. Set
  `cancel` and nothing opens. The player's state is swapped in, so `host.players().acting_player`, `PlayerRef::tell` and the item
  helpers reach them. Menus a mod opens (`native_open_menu`) are not heard.
- **`creative_slot(player, name, slot, item, count, cancel)`**: a creative-mode player sets a slot from the creative
  screen (`slot` is the inventory menu's slot; below 0 is a stack thrown out of the screen), with the `item` (`minecraft:air`
  for an emptied slot) and `count`. Heard after vanilla's checks, before the slot changes. Set `cancel` and the packet is
  ignored. The player's state is swapped in. Stacks a creative player gets other ways (pick block, `/give`) are not heard.
- **`Hooks::player_game_mode_changed`**: a server player's game mode is about to change, from `event.from` to
  `event.to`, both `GameMode` (`Survival`, `Creative`, `Adventure`, `Spectator`): the `/gamemode` command, the
  F3+F4 switcher's packet and a hardcore world's respawn into spectator, once per change to a different mode. The mode a
  player joins with (`defaultgamemode`) is not heard. Write `*event.to` to send the player to another mode instead (each listener sees the previous one's choice); return `Flow::Cancel`, or leave `to` at the current mode, and the player keeps their mode. Inside the
  command and the switcher the player's state is swapped in; for the hardcore respawn it is not. The hook receives a
  `&mut dyn ServerHost`, and `event.player` identifies the player by UUID.
- **`player_changed_dimension(player, name, from, to)`**: a player changed dimension, once per move
  (`from` and `to` are ids such as `minecraft:overworld` and `minecraft:the_nether`), heard when the move has ended:
  the portal's exit is found and the player stands there, a command's teleport has landed, or a respawn in another
  level than the death's is placed for good. `to` is the level in place and the player's state is swapped in (as in
  a command executor), so `host.players().acting_player` and `PlayerRef::tell` reach them. Heard for Nether and End portals
  (also riding, and leaving the End as its winner), `/execute in ... run tp` and any command teleporting into another
  level, and death respawns, on dedicated and integrated servers (single player and LAN), where the world is simulated.
  Not heard by a remote client window, for a move within a level (`/tp`, an End gateway), for a player who leaves before the move ends, or for a move ending where it started (a respawn in the death's level).
- **`player_toggle_flight(player, name, flying, cancel)`**: a player's client started (`flying`) or stopped
  flying, once per change, for a player who may fly (creative or spectator), on a server. The player's state is swapped
  in, so `PlayerRef::tell` and the other helpers reach them. Set `cancel` to refuse it: the player keeps the state they
  had and gets their abilities back, so the client lands or flies again. Only what a client asks for is heard: not the
  flight a game-mode change starts or ends, and not a LAN host's own player.
- **`player_switch_hotbar_slot_pre(player, player_entity_id, old_slot, new_slot, cancel)`** and
  **`player_switch_hotbar_slot_post(player, player_entity_id, old_slot, new_slot)`**
  (`PlayerSwitchHotbarSlotEvent.Pre`/`.Post`): a server player switches to a different hotbar slot. Pre runs after
  the slot range check and before it changes; set `cancel` to keep the old slot, and the client is sent that slot
  again. Post runs after the change. The player's state is not swapped in.
- **`furnace_burn(pos, fuel)`**: a furnace consumed `fuel` and started burning at `pos`.
- **`furnace_smelt(pos, item, result)`**: a furnace finished smelting `item` into `result` at `pos`.
- **`player_item_cooldown(player, name, item, ticks, cancel)`**: an item cooldown is about to start for a
  server player, including a used item's cooldown and a shield disabled by a hit. `item` is the item id and `ticks`
  its duration. Set `cancel` to skip the cooldown; shield disable still stops the use and plays its configured sound.
  The player's state is not swapped in.
- **`player_recipe_discovered(player, name, recipe, cancel)`**: a recipe id is about to become known to a
  server player through an advancement reward, a knowledge book or `/recipe give`. Set `cancel` to keep it locked;
  the client is sent the matching removal. The player's state is not swapped in.
- **`native_player_set_spawn(player, player_entity_id, name, dimension, pos, forced, cancel)`**: a server player's
  respawn point is about to be set, at each `ServerPlayer.setRespawnPosition` call: every sleep in a bed, a new respawn
  anchor point, `/spawnpoint`, and the copy to a replacement after `player_clone` (`pos` none for a player with no point).
  Cancel to keep the replacement's current point, including one set by a clone listener.
- **`entity_travel_to_dimension(entity, dimension, cause, cancel)`**: a server entity is about to travel to
  the level `dimension`, before it moves (NeoForge's `EntityTravelToDimensionEvent`); `cause` is `nether_portal`,
  `end_portal`, `end_gateway` or `teleport` (a `/tp` or spectator teleport into another level). Set `cancel` to
  keep it where it is. Players and every other entity that crosses a portal or a command's teleport are asked;
  passengers are asked recursively, and a refused passenger stays behind, dismounted, with its own subtree.
  Discarding an entity prevents its travel. A player leaving the End is asked about the resolved destination, including the world spawn when the saved point is missing. Respawn after the End credits is not travel. The player's state is not swapped in.
- **`block_damage_abort(player, name, pos)`**: a server player stopped digging `pos` before it broke. It is
  heard after the server clears the dig and sends the block state back to the client. The player's state is not
  swapped in.
- **`player_pick_item(player, name, from_entity, pos, entity_id, cancel)`**: a server player's middle-click
  pick resolved a block or entity's pick stack and is about to put it in the hotbar. For a block, `pos` is its
  position and `entity_id` is zero; for an entity, `from_entity` is true, `pos` is `[0, 0, 0]` and `entity_id`
  is its network id. The player's state is swapped in. Set `cancel` to leave the inventory and client unchanged. A
  pick with no result is not heard.
- **`native_sign_changed(player, name, dimension, pos, front, lines, cancel)`**: a player finished a sign they were
  editing, heard once every check has passed (editing that sign, in reach, not waxed) and right before
  the text goes on it. `lines` holds the four lines as written, formatting codes still in; rewrite them and that is
  what the sign gets, or set `cancel` and it keeps its old text. The
  player's state is swapped in, so `PlayerRef::tell` reaches them. Not told: the host's own editing on a LAN or
  single-player world, and changes that are no text edit (dye, glow ink, wax, `/data`, `/setblock`).
- **`player_client_loaded_world(player, name)`**: the server heard that a guest's client finished loading
  the level around them after joining, respawning or changing dimension. The player's state is not swapped in.
- **`player_stop_using_item(player, name)`**: a server player released the use-item key, or a mod's
  `release_using_item` did, before the item finished being used. The player's state is swapped in.
- **`player_recipe_book_settings_change(player, name, book, open, filtering)`**: a server player's recipe
  book open or filter setting changed; `book` is the book type's protocol id. The player's state is not swapped in.
- **`player_command(player, name, command, canceled, replies)`**: any command a player typed (without the
  slash), vanilla's too and commands the player may not run, heard before anything looks at it, and each line of a
  quiet command batch. Set `canceled` and nothing runs; push lines to `replies` and the player gets them as system chat
  (a quiet batch drops them). The player's state is not swapped in, and `PlayerRef::tell` and the other helpers that take a
  UUID reach nobody inside this hook: answer through `replies`. Commands may carry secrets (`/msg`, a login plugin's
  password), so don't log them carelessly.
- **`player_respawned(player, name, end_conquered)`**: once after `PlayerList.respawn` places and synchronizes the replacement (NeoForge's `PlayerRespawnEvent`). The player's state is swapped in for the call. `end_conquered` is true after the End credits (health, food and experience are kept) and false after death. An ordinary End teleport does not fire it.
- **`player_login(player, name, cancel, reason)`**: a player the server has accepted (their name, the whitelist,
  bans and the player limit are checked) and who holds a place, heard before their saved state is loaded and before they
  are placed in a level (NeoForge's `PlayerNegotiationEvent`). Set `cancel` and they are disconnected with `reason` (a
  plain "Disconnected" when it stays `None`); no join message and no `player_logged_in` follow. It runs after the
  client's configuration, so the client sees the refusal as an in-game disconnect screen. The player is not placed yet,
  so no player's state is swapped in and `PlayerRef::tell` reaches nobody: the refusal text is the answer. Players the
  server refuses itself (name, whitelist, bans, a full server, the proxy's forwarding) are not heard.
- **`player_teleported(player, name, from, to, dimension)`**: a server player a command (`/tp`, `/teleport`,
  also what a mod's `run_command` reaches) moved, inside their level or into another; `from` and `to` are their feet
  before and after and `dimension` the level they are in now. Heard after the move has reached the player's state and
  their client (into another level: when that level is entered, before it has loaded around them), so it cannot stop it
  (NeoForge's `EntityTeleportEvent`). The player's state is swapped in, so `PlayerRef::tell` and the acting-player helpers
  work. Ender pearls and chorus fruit have their own hooks below; not heard yet: a spectator's teleport menu; portals
  and respawns are not teleports here.
- **`native_ender_pearl_teleport(owner, pearl, from, target, damage, cancel, hit_entity, hit_block, hit_face, hit_at)` (version 3)**: a server player's or mob's
  pearl landed and is about to teleport its live `EntityRef` owner. `target` may be moved; cancellation spends
  the pearl without teleport, sound, damage or endermite roll. Players retain acting-player context and
  mutable fall damage (5). Mobs preserve rotation, reset fall distance, take no player fall damage and
  make no endermite draw. The pending move refuses an owner changed by the listener. The hit fields say what the pearl hit (`getHitResult`): an entity (`hit_entity`, with the cell of `hit_at` and face `[0, 0, 0]`) or a block and its face normal.
- **`item_consumption_teleport(consumer, consumed, from, target, cancel)` (version 2)**: a server player or fox whose `teleport_randomly`
  consume effect (chorus fruit) is about to try a spot (NeoForge's `EntityTeleportEvent.ItemConsumption`,
  `TeleportRandomlyConsumeEffect.apply`): once per attempt, up to 16, after they stopped riding. A mod may move
  `target` (its y is kept inside the level's height); the game still looks down from it for ground to land on, inside
  the border. Canceling ends the effect: no
  teleport and no more attempts, but the food is consumed. `consumer` is a `LivingRef` and `consumed` is the
  captured stack before consumption (count and components included). A fox's listener receives the full server
  host before its living movement and before the next mob ticks; writes to actors or the level survive the
  continuation. Changing the consumer's mouth item, position or eligibility refuses the captured effect.
- **`player_clone(player, name, was_death, original)`**: after death or the End credits, after old-player
  cleanup and data copy but before the replacement is placed. `OriginalPlayer<'_>` lends the retained old player,
  including its source level and independently journaled attachments, only for this call. With no subscriber,
  no old player is retained, though attachment copy still runs. See `Hooks::player_clone` for the full contract.
- **`player_load_from_file(player, name, directory)`**: a joining player's saved state was read and applied
  (NeoForge's `PlayerEvent.LoadFromFile`, `PlayerList.loadPlayerData`), before they are placed in a level: once for each
  join on a server, for an integrated server's owner (loaded from the file `level.dat` names, else their own) as for
  anyone else. `directory` is the world's `players/data` folder, created if missing; read your own file there, named `<uuid>.<suffix>`. It
  is heard whether or not the player was in the world before (a new player has no file). No player's state is swapped
  in. It cannot change or stop anything.
- **`player_save_to_file(player, name, directory)`**: a placed player's data was saved (NeoForge's
  `PlayerEvent.SaveToFile`, `PlayerList.save`): when they leave or the world closes, when the dedicated server saves
  everyone, and for an integrated server's owner when the world saves their state into `level.dat`. `directory` is the
  world's `players/data` folder, created if missing; write your own file there, named `<uuid>.<suffix>`, and never the player's `.dat`.
  The player may be leaving, so no player's state is swapped in. It cannot change or stop anything.
- **`player_commands_sent(player, name, names)`**: the command tree a player is sent, when they join and when
  their level changes (`/op`, `/deop`, the LAN settings). `names` are the root commands their client would get, vanilla's
  they may use and the mods'; remove a name and that client's tree lacks it, so its chat box neither completes nor
  colours it. This hides the command from the client only, a typed one still runs (stop it with
  `player_command`). For a player already on, their state is swapped in and `PlayerRef::tell` works; a joining
  player has no world yet, so nothing can be told them. The tree the hub re-sends by itself (after a respawn or a change
  of level) reuses the last answer.
- **`player_sprint(player, name, sprinting)`**: a server player's client started or stopped sprinting (the sprint
  key, a double tap forward, letting go), once for each change, after the server's player took it. It only hears. It
  reports what the client asks, not what the server does: a sprint that ends by itself (a hit's knockback, hunger, a
  wall) is not heard. The player's state is swapped in, so `PlayerRef::tell` and the acting-player helpers work.
- **`resource_pack_status(player, name, status)`**: a server player's client answered a resource pack while
  playing (`ServerboundResourcePackPacket`): `status` is `Action`'s number (0 loaded, 1 declined, 2 failed download,
  3 accepted, 4 downloaded, 5 invalid URL, 6 failed reload, 7 discarded). It only hears. The pack a dedicated server
  offers during the configuration is answered before the player has joined and is not heard. The player's state is
  swapped in, so `PlayerRef::tell` works.
- **`tnt_primed(player, pos, cancel)`**: TNT at `pos` is about to be lit by a flint and steel or fire charge
  (`player` is whoever clicked, whose state is swapped in), fire, a redstone signal, an arrow or a dispenser (`player` is
  None). Set `cancel` and it is not primed: the block stays, or is put back if fire or a signal had already taken it
  out. TNT set off by a blast, a TNT minecart and a sulfur cube are not asked; `explosion_detonate` can spare
  the blocks of a blast.
- **`hanging_place(player, name, kind, pos, cancel)`**: a server player places an item frame, a glow item
  frame, a painting or a leash knot (tying a lead to a fence that has none; `pos` is the fence), once it fits and
  before it is placed or the item used. Set `cancel` and nothing happens; the item stays in the hand. The player's
  state is swapped in, so `PlayerRef::tell` answers them. `/summon` and a lead tied to a mob are not heard.
- **`hanging_break(player, kind, pos, cancel)`**: an item frame, a glow item frame or a painting (`kind`)
  hanging in `pos` is about to break: a server player's hit (`player`, whose state is swapped in), a blast, or the
  wall it hangs on gone (`player` None for both), before anything drops or sounds. Set `cancel` and it stays as it
  was. A hit that only takes a frame's item out, the game's own player and `/kill` are not asked.
- **`bucket_empty(player, name, pos, fluid, cancel)`**: a server player empties a bucket (`fluid` is
  `minecraft:water`, `minecraft:lava` or `minecraft:powder_snow`; fish and axolotl buckets pour water) into `pos`,
  right before the fluid is placed. Set `cancel` and nothing is poured. The player's state is swapped in, so
  `PlayerRef::tell` answers them. `fill_bucket` hears the same click without the player and the fluid; dispensers
  and cauldrons are not heard. A fluid or mob bucket is also heard at the block beside the clicked face whenever that
  block is not `pos`, since a block that refuses the fluid passes it there; the fluid may still go into `pos`.
- **`player_bed_enter(player, name, pos, cancel)`**: a server player passed the sleep checks and is about to
  enter the bed at its head position. Set `cancel` to keep them out.
- **`player_bed_leave(player, name, pos, wake_immediately, update_level)`**: A server player is getting out of a bed (`PlayerWakeUpEvent`), heard before their sleeping state is cleared, so they still read as sleeping at `pos`, the head position. `update_level` is false for the morning wake of all players and true when the player got up; `wake_immediately` is true for generic stop-sleeping, missing-bed and death wakes, false for morning, explicit leave and bed-rule refusal; the hook owns the full caller table.
- **`player_edit_book(player, name, pages, signing, cancel)`**: A server player book-and-quill edit or signing packet is about to be applied. `pages` is its page count and `signing` says it has a title. Set `cancel` to drop the packet.
- **`player_shear_entity(player, name, entity_id, off_hand, cancel)`**: A server player is about to shear a sheep, mooshroom, snow golem or bogged. The hook gives the mob entity id and whether the shears are in the off hand. Set `cancel` to leave the mob and shears unchanged.
- **`player_advancement_done(player, name, advancement)`**: A server player completed an advancement after its progress was recorded. `advancement` is its identifier, such as `minecraft:story/mine_stone`.
- **`player_advancement_progress`**: A criterion of a server player's advancement was granted or revoked, after its progress changed and before any completion rewards, including the `minecraft:tick` criteria granted on a network player's first tick or a reload. The server-side event supplies `player` (UUID), `player_entity_id`, `name`, `advancement`, the changed `criterion`, and `progress_type` (`ProgressType::Grant` or `ProgressType::Revoke`). Its `progress` is an `AdvancementProgress` snapshot of this player's complete state at that mutation: all criteria in definition order with their original grant dates (or `None`), the requirement groups, the number of satisfied groups, and `done`. Each group is an OR of criteria; all groups must be satisfied, and an empty requirements list is not done. Clone the snapshot to retain it without borrowing the game; subsequent grants, revokes and reloads cannot change it. This is a read-only observation, not a way to write progress. A grant or revoke that changes nothing is not heard. The snapshot is built only when a listener subscribes.
- **`player_stat_award(player, name, stat_type, stat, value, cancel)`**: A statistic is about to be awarded to a server player, before its count and scoreboard criteria change. `stat_type` and `stat` are writable canonical namespaced strings, such as `minecraft:mined` and `minecraft:stone`, or `minecraft:custom` and a registered custom statistic. `value` is a writable `i32` amount. Unknown types or entries, noncanonical ids, and negative amounts reject the entire rewrite (warned once), keeping the original identifiers and amount. Set `cancel` to award nothing. Integrated and LAN servers apply the edits to counters, scoreboard criteria and dirty statistics packets. In a transaction, the hook and award wait for the outermost successful commit; aborts emit neither.
- **`player_riptide(player, name)`**: a server player launched with a riptide trident, after its velocity and spin are applied. The player's state is swapped in.
- **`player_fish(player, name, state, cancel)`**: a player's fishing rod, on a server: `state` 0 the cast, 1 a fish
  biting, 2 the reel-in of a bite (the catch), 3 the reel-in with nothing on the line, 4 the reel-in of a bobber hooked
  to an entity or item. Set `cancel` and nothing is thrown (0), the fish gets away and the wait starts over (1), or the
  bobber stays out with no wear on the rod (2 to 4). The bobber ending by itself (too far, rod put away, owner dead) is
  not heard, nor is the host of a LAN world's rod. The player's state is swapped in, so `host.players().acting_player`,
  `PlayerRef::tell` and the item helpers reach them.
- **`block_dispense_armor(dimension, pos, item, target)`**: a dispenser equipped `item` on the first eligible player, mob or armor stand in front of its block at `pos`. The equip, item consumption, sound and game event have completed; `target` names the kind of entity equipped.
- **`block_shear_entity(dimension, pos, entity, kind)`**: a dispenser sheared the entity of `kind` and id `entity` at the block in front of the dispenser at `pos`, after applying the shearing effects.
- **`block_receive_game(dimension, pos, block, event, cancel)`**: a sculk sensor, calibrated sensor or shrieker received the vibration `event`. Set `cancel` to suppress its response.
- **`brewing_start(pos, ingredient)`**: a brewing stand at `pos` starts its timer for `ingredient`, after it consumes fuel. Server-side.
- **`campfire_start(pos, item)`**: `item` was placed on the campfire at `pos` and began cooking. Server-side.
- **`block_cook(pos, item, result)`**: a campfire finishes cooking `item` into `result` at `pos`, before the result is dropped. Server-side.
- **`compost_item(pos, item)`**: a player adds a compostable item to the composter at `pos`, after its layer is applied. Server-side.
- **`crafter_craft(pos, recipe)`**: a crafter completed `recipe` at `pos`, after consuming its ingredients and handling its output. Server-side.
- **`player_attack_cooldown_reset(player, name)`**: a server player's punch was handled: the main-hand swing and attack-strength reset have both happened. The player's state is not swapped in.
- **`player_bucket_entity(player, name, entity_id, entity)`**: a server player caught a bucketable mob after the mob was removed and the filled bucket was put in the player's hand or inventory. `entity` is the mob species id.
- **`player_change_beacon_effect(player, name, pos, primary, secondary)`**: a server player's beacon selection was stored. Effect values are `Option<EffectKey>`, `None` when none is selected.
- **`player_deep_sleep(player, name)`**: a server player reached 100 ticks asleep in a bed and now counts as deeply asleep for the night-skip check.
- **`player_egg_throw(player, name)`**: a server player threw an egg after its projectile was launched and its use applied.
- **`player_elytra_boost(player, name)`**: a server player used a firework rocket to boost while gliding, after the rocket was launched and consumed.
- **`player_flower_pot_manipulate(player, name, pos, plant)`**: a server player put `plant` into the flower pot at `pos`, after its state changed and the item was consumed.
- **`player_insert_lectern_book(player, name, pos, book)`**: a server player put `book` on the lectern at `pos`, after the block entity and inventory changed.
- **`player_take_lectern_book(player, name, pos, book)`**: a server player removed `book` from the lectern at `pos`, after its block entity changed.
- **`player_lectern_page_change(player, name, pos, page)`**: a server player changed the lectern at `pos` to zero-based `page`, after its page and redstone pulse changed.
- **`player_item_frame_change(player, name, entity_id, action, item, rotation)`**: a server player changed a frame (`put`, `rotate`, `remove_item` or `break`), after its contents and comparator output changed. `item` is empty when none remains.
- **`player_item_mend(player, name, item, repair, experience)`**: an orb actually repaired `repair` durability on `item` for a server player, after the item changed. `experience` is the orb value.
- **`player_trade(player, name, entity_id)`**: a server player completed a villager or wandering trader trade, after the merchant applied it. `entity_id` is the merchant network id.
- **`player_shield_disable(player, name, ticks)`**: a blocked melee hit disabled the server player’s shield for `ticks` ticks. It fires just before the cooldown is set.
- **`player_loom_pattern_select(player, name, pattern)`**: a server player selected loom pattern button `pattern`, after the menu accepted it.
- **`player_name_entity(player, name, entity_id, entity_name)`**: a server player used a named name tag on a mob, after it became persistent and was named.
- **`player_open_sign(player, name, pos, front)`**: a server player opened the sign editor at `pos` for the front or back side, after the editor state was made.
- **`player_pickup_arrow(player, name, entity_id, item)`**: a server player picked up an arrow after it was removed and its pickup was applied. `item` is the pickup item id.
- **`player_stonecutter_recipe_select(player, name, recipe)`**: a server player selected stonecutter recipe button `recipe`, after the menu accepted it.
- **`player_swap_with_equipment_slot(player, name, slot)`**: a server player swapped their main hand with `slot` (currently `offhand`), after both hands changed.
- **`player_toggle_entity_age_lock(player, name, entity_id, locked)`**: a server player used a golden dandelion on an ageable mob, after its age lock changed and the item was used.
- **`player_unleash_entity(player, name, entity_id)`**: a server player's action removed an entity's leash, after its leash state was cleared.
- **`player_velocity(player, name, velocity)`**: the server synced changed motion to a player, with `velocity` in blocks per tick.
- **`player_stop_spectating_entity(player, name, entity_id)`**: a spectator's camera returned to their own player after they stopped watching `entity_id`.
- **`player_map_filled(player, name, map_id)`**: a server player filled an empty map, after the filled map and its saved map data were created.
- **`player_teleport_end_gateway(player, name, gateway, cancel)`**: a player is about to travel through the End gateway at `gateway`. Set `cancel` to leave them at the entrance; the gateway does not start its cooldown. The player is not swapped in.
- **`player_recipe_book_click(player, name, recipe, all)`**: a server player requests a known recipe be placed in their crafting grid through the recipe book. `all` is true for the shift click form. The player is not swapped in.
- **`player_exp_cooldown_change(player, name, ticks)`**: the player’s XP orb pickup cooldown changed, after its new value is stored. This includes the two tick delay after an orb is picked up and each tick it counts down. The player is not swapped in.
- **`entity_enter_block(entity, kind, pos, block)`**: a bee was stored in the beehive or nest at `pos`.
- **`entity_change_block(entity, kind, pos, from, to)`**: a mob changed a block, after the new state was applied.
- **`entity_break_door(entity, kind, pos)`**: a mob finished breaking a door at `pos`, after the block was removed.
- **`creeper_power(entity, lightning)`**: a lightning strike charged the creeper.
- **`living_visibility(entity, looking, visibility)`**: how visible `entity` is to `looking` (NeoForge's `LivingVisibilityEvent`, last in `LivingEntity.getVisibilityPercent`). `visibility` comes in as vanilla's percent: sneaking 0.8, invisibility scaled by armour cover, and 0.5 for a worn mob head whose default `mob_visibility` names the looking mob's type (a zombie head fools zombies, not husks). Multiply it to scale the range a targeting check sees the entity from. A negative answer counts as 0, and the result is clamped to 10, as vanilla clamps. It asks for a player, or for a mob (no sneaking factor: vanilla reads it through `isDiscrete`), seen by a mob whose targeting check scales by visibility (`TargetingConditions.testInvisible`): hostile targeting, the sensors, the happy ghast's tempt, a mob hunting a mob. It asks after the alive, spectator and selector checks, as `TargetingConditions.test` does. With no listener, `MobWorld::mob_hooks` is `None` and nothing is asked.
- **`entity_mob_griefing(entity, can_grief)`**: may `entity` change the world (NeoForge's `EntityMobGriefingEvent`, `EventHooks.canEntityGrief`)? `can_grief` comes in as the `mob_griefing` rule. Sunrise asks with the entity responsible for the change: the exploding entity for a mob explosion, the owner for a mob's arrow or small fireball, the wither for its rose, otherwise the mob itself. A zombie breaking a door, the dragon breaking its walls, a ravager breaking pitcher crops, a burning mob melting powder snow and a weaving mob's cobwebs ask with that mob. Whether an explosion reaches items and decorations asks with the exploding entity, and whether a mob-caused blast harms decorations asks with that mob. A blast's vehicles and the ravager's roar on armor stands read the rule. The game asks through `Game::can_entity_grief` and the mob AI through `MobWorld::can_grief`. With no listener both read the rule: one `listened` check, no dispatch. A large fireball's fire asks for its owner: the ghast that shot it, or the player who sent it back. A fireball whose owner is gone reads the rule. The blast's block damage still reads the rule, because Sunrise's mob explosion has no source entity yet.
- **`enderman_anger(enderman, player, cancel)`**: `player` stares at `enderman` (NeoForge's `EndermanAngerEvent`, last in `Enderman.isBeingStaredBy`, once the look cone, line of sight and carved pumpkin checks passed). It is asked each time the enderman's goals check the stare: to get angry, to freeze while looked at, and to teleport away up close. Set `cancel` and that stare doesn't count. The mob AI asks through `MobWorld::mob_hooks`, which is set only on the server while a mod listens: with no listener the stare check costs nothing more.

  Mob AI asks mods through `MobWorld::mob_hooks`, an `Option<&dyn MobHooks>`: one trait in the client lib, implemented once by the game, whose methods only build the hook's arguments and call its generated `native_*` entry. It is `Some` only on an authoritative server while a mod listens to one of its hooks, and each method is gated by its own hook's `listened`, so adding a method costs nothing to mods that don't listen to it. The hook gets a read `Host` over the game while the mob tick runs: the mobs being ticked and the level's random and attachments are lent to the tick, so through that host every mob's handle reads as gone and level attachments read as none. Use the event's own fields; anything that must write waits for a deferred hook.
- **`creeper_ignite(entity, player, item)`**: a player used a creeper igniter on the creeper, after it entered its ignited state.
- **`exp_bottle(pos, experience)`**: an experience bottle broke and its orbs were created.
- **`firework_explode(entity, pos, bursts)`**: a firework rocket completed its damage and explosion effects.
- **`potion_splash(item, pos)`**: a thrown splash or lingering potion finished applying its hit effects at `pos`. Experience bottles are excluded.
- **`entity_enter_love_mode(entity, kind, player)`**: a mob entered love mode after a player interaction; `player` is the feeder UUID when known.
- **`teleport_command(dimension, entity, from, to, cancel)`**: a server entity is about to be teleported by `/tp` or `/teleport`. `dimension` is the level it arrives in, `entity` its network id, `from` its feet now and `to` where the command sends it; move `to` to redirect it, or set `cancel` to keep it where it is (the command still reports success).
- **`entity_teleport(dimension, entity, from, to)`**: a server entity completed a same-level teleport command. `entity` is its network id; `from` and `to` are coordinates in `dimension`. This reports the move after it happens.
- **`entity_breed(parent, partner, child)`**: two mobs completed a breeding action. `child` is absent when the vanilla action lays an egg instead of creating a baby.
- **`entity_tame(entity, kind, owner)`**: a mob became tamed after its owner and persistent state were set. `owner` is the server player's UUID when known.
- **`item_spawn(entity, item, count, pos)`**: a dropped item entity was successfully added to the server level. The count is its stack size and `pos` is its spawn position.
- **`entity_pose_change(player, name, from, to)`**: a guest player's synchronized pose changed. Pose names are `STANDING`, `CROUCHING`, `SWIMMING`, `FALL_FLYING`, `SPIN_ATTACK` or `SLEEPING`.
- **`entity_toggle_glide(player, name, gliding)`**: a guest player's fall flying state changed after the server accepted or ended a glide.
- **`entity_shoot_bow(player, name, projectile, force)`**: a player fired an arrow from a bow or crossbow, after the arrow entered the level. `force` is the shooting power. This reports the completed shot.
- **`arrow_nock(player, name, bow, hand, has_ammo, cancel)`**: a player is about to start using a bow, where `BowItem.use` runs on the server (NeoForge's `ArrowNockEvent`). `has_ammo` is whether `getProjectile` found ammunition, always true in creative. Set `cancel` to fail the use: the bow is not drawn. Crossbows and tridents do not ask, and the client's own prediction does not.
- **`living_get_projectile(player, name, entity, kind, weapon, projectile)`**: the ammunition `LivingEntity.getProjectile(weapon)` returns (NeoForge's `LivingGetProjectileEvent`). Every mod gets the event with `projectile` set to the vanilla ammunition (the stack found in the hands or inventory, a mob's held projectile or a new arrow, a creative player's default arrow, or empty) and may change or empty it. It is asked on the bow's start and release, the crossbow's start and draw, the client's prediction of those starts, a skeleton's or illusioner's shot, and a pillager's or piglin's crossbow load (`entity` and `kind` name the mob, `player` is empty); a mob's arrow takes the replaced arrow's item and effect, a mob's crossbow also fires a replaced firework rocket, and an emptied stack leaves its crossbow uncharged and silent. A replaced stack is not taken from the inventory: the hook owns it.
- **`projectile_launch(player, name, entity, kind)`**: a projectile was added to the server level. `kind` is its entity type; `player` and `name` identify a player launcher, and are `None` and empty for other sources.
- **`entity_portal(dimension, entity, pos)`**: a server entity finished its portal wait and is beginning to leave through the portal block at `pos`. The destination follows the existing portal rules; the hook reports the transfer without changing it.
- **`furnace_extract(player, name, pos, item, count)`**: a server player takes furnace output at `pos`, after the output is removed; the player state is not swapped in.
- **`furnace_start_smelt(pos, item)`**: a furnace at `pos` begins cooking `item`, when its cooking timer first advances. Server-side.
- **`brewing_complete(pos)`**: a brewing stand finished brewing at `pos`, after the bottle contents changed.
- **`brewing_fuel(pos, fuel)`**: a brewing stand at `pos` consumed `fuel` and gained brewing fuel. Server-side.
- **`enchant_item(player, name, pos, item, levels)`**: a player applied an enchantment at `pos`; `item` is the finished item and `levels` the levels charged. Guest state is not swapped in.
- **`prepare_item_enchant(player, name, pos, costs)`**: enchanting offers were calculated at `pos`; change the three offer costs before they are sent to the player's menu. Guest state is not swapped in.
- **`anvil_update(host, event)`** (`AnvilUpdateEvent`, both logical sides): fires live after `AnvilMenu.createResult` (with no answer cache), matching NeoForge `AnvilMenu.createResult` / `CommonHooks.onAnvilUpdate`. Read `event.inputs` and change `event.edits`'s output, XP cost or material cost. `StackResult::Keep` keeps vanilla's output and `StackResult::Set(None)` clears it. Cancellation leaves no output and zero costs; an invalid replacement output is logged and ignored (keeping vanilla's output while still applying the cost edits). `xp_cost` stays signed across listeners (`AnvilUpdateEvent.setXpCost(int)`) and clamps to `0` when applied (`AnvilMenu.setCost`); a zero cost prevents taking the output (`AnvilMenu.mayPickup` requires a positive cost). Negative material costs clamp to zero, which consumes the entire right stack when a positive-cost output is taken.
- **`anvil_craft_pre(host, event)`** (`AnvilCraftPreEvent`, server only): before an anvil take charges levels, uses the inputs or wears the anvil, `event.inputs` lends the pre-take left and right stacks and result. Set `event.cancelled` or return `Flow::Cancel` to return early before charging levels, consuming inputs, damaging the anvil or firing `anvil_craft_post`, without undoing the click's item move (matching NeoForge `AnvilMenu.onTake`). The client's predicted take does not fire it.
- **`anvil_craft_post(host, event)`** (`AnvilCraftPostEvent`, server only): after an accepted anvil take charges levels, uses the inputs and applies anvil wear, `event.inputs` lends the same pre-take snapshot as Pre. A broken anvil is already air. Post does not fire on a veto.
- **`prepare_smithing(player, name, pos, result)`**: a smithing result was prepared for a player at `pos`; change it before it is sent to their menu. Guest state is not swapped in.
- **`prepare_item_craft(player, name, pos, result)`**: a crafting-grid result was prepared; `pos` is the crafting table, or None for the player's inventory grid. Change it before it is sent to the menu. Guest state is not swapped in.
- **`trade_select(player, name, villager, index)`**: a player selected offer `index` in a villager's trading menu; `villager` is its entity id when known. The menu has updated and the player's state is not swapped in.
- **`hopper_pickup_item(dimension, pos, item, count, cancel)`**: a hopper at `pos` is about to pick up a dropped stack; set `cancel` to leave it in the world.
- **`weather_change(dimension, raining, thundering, cancel)`**: the server's `/weather` command is about to set
  its final rain and thunder states in `dimension`. Set `cancel` to keep the previous weather parameters, including
  their timers.
- **`thunder_change(dimension, thundering, cancel)`**: the server's `/weather` command is about to change
  thunder. Set `cancel` to keep the previous thunder state and timer.
- **`lightning_strike(player, pos, cause, cancel)`**: a weather, Channeling-trident, or `/summon` lightning bolt
  is about to be created. Set `cancel` to prevent the bolt and its effects; `cause` names the creation path.
- **`entity_struck_by_lightning(entity, bolt, cancel)`**: a lightning bolt is about to hit `entity`
  (NeoForge's `EntityStruckByLightningEvent`), asked before vanilla's `Entity.thunderHit`, once per strike for each mob, player,
  item, vehicle or other entity whose box meets the strike box. Set `cancel` to spare it: no fire, damage or
  conversion, and `creeper_power` is not called. It still counts as struck for Channeling's advancement, as
  in NeoForge. A visual-only bolt asks nothing. With no listener the cost is one `listened` check per struck entity.
- **`time_skip(dimension, from, to, cause, cancel)`**: the server clock is about to jump after enough players
  sleep or through `/time set`, `/time add`, or `/time set <marker>`. Set `cancel` to keep the old time.
- **`sleep_finished_time(dimension, current, new_time, adjustment)`**: enough players slept and `advance_time`
  is on (NeoForge's `SleepFinishedTimeEvent`). `adjustment`, a `ClockAdjustment`, starts as vanilla's
  `wake_up_from_sleep` marker; a mod may set `Absolute`, `Relative` or another `Marker`, read with
  `level_clock_marker_time`. `new_time`: the morning, read only (contract 2). Level's own clock, resolved after every mod;
  a time before `current` or past `i64::MAX`, or a marker that clock lacks (only the Overworld's has any), moves to the morning.
- **`world_border_bounds_changed()`**: `/worldborder set` or `/worldborder add` changed the border size or started a timed size change.
- **`world_border_center_changed()`**: `/worldborder center` moved the border center. It hears successful commands only.
- **`spawn_change(dimension, pos, angle)`**: `/setworldspawn` moved the world's spawn point to `pos` in level `dimension`, after its level, position and rotation are applied.
- **`portal_create(dimension, pos)`**: a Nether portal built for dimension travel or the End arrival platform was completed in `dimension`; `pos` is its lower corner or platform origin.
- **`raid_spawn_wave(raid, pos, wave, raiders)`**: a raid wave is about to be spawned at `pos`; `raiders` is the count Sunrise will spawn.
- **`raid_state_change(raid, pos, state)`**: a raid at `pos` was triggered or changed to `victory`, `loss`, or `stopped`. Its `RaidRef` identifies the owning level, not necessarily the level currently in place.
- **`world_saved(wait)`**: the server completed its world save cycle; `wait` is true when chunk writes were flushed before returning.
- **`entities_load(dimension, chunk, count)`**: saved entity records for `chunk` finished loading into `dimension`.
- **`entities_unload(dimension, chunk, count)`**: entities in `chunk` left the active world and were queued for saving.
- **`tab_complete(player, name, command, range, suggestions)`**: a guest's server completion request was evaluated; mods may add or filter suggestions. `range` is the replaced byte range. Guest state is not swapped in.
- **`broadcast_message(line, cancel)`**: a system broadcast line is about to be logged and sent to every player. A mod may change `line` or set `cancel` to drop the broadcast.
- **`whitelist_state_update(enabled)`**: the dedicated server's player whitelist was enabled or disabled.
- **`player_bundle_item_select(player, name, slot, index)`**: a server player selected an item in a bundle;
  `slot` is the packet's menu slot and `index` is the chosen item index.
- **`player_ready_arrow(player, name, arrow)`**: a server player released a drawn bow, by the use-item key or a
  mod's `release_using_item`, with `arrow` naming the selected projectile item. The player's state is swapped in.
- **`prepare_hopper_transfer(pos)`**: a hopper at `pos` begins a transfer attempt, after its cooldown and enabled checks and before pushing or pulling items.
- **`inventory_move_item(source, destination, item, cancel)`**: a hopper is about to move one item between containers (a composter's bone meal too). Set `cancel` to leave it in the source slot. A chest boat or minecart is reported at the block it is in.
- **`hopper_inventory_search(hopper, container, entity)`**: a hopper selects a block container or container entity while searching for items.
- **`vault_loot(pos, item)`**: a vault is about to eject a loot item, before the item entity spawns.
- **`world_border_bounds_change_finish()`**: a timed world border size change finished on the server, after the border reached its final size.
- **`player_launch_projectile(player, name, item)`**: a server player launched a snowball, egg, ender pearl,
  potion or trident. It fires after the projectile is created and only for guest players in a LAN world.
- **`entity_unleash(entity, kind)`**: a mob's leash was removed, before any lead drop is handled.
- **`entity_drop_item(entity, item, count, pos)`**: a mob is about to drop `count` of `item` at `pos`.
- **`item_despawn(entity, item, count, pos)`**: a dropped item entity reached its 6000-tick lifetime and is
  about to be removed.
- **`explosion_prime(center, radius)`**: a server explosion is about to begin at `center` with `radius`.
  It is listen-only, so the explosion proceeds as usual; it also fires on the single-player server.
- **`vehicle_damage(kind, damage, cancel)`**: a player is about to damage a boat or minecart. Change `damage`
  or set `cancel` to leave its damage and wobble unchanged; the attack still completes. `kind` is `boat` or the
  minecart type. Server-side.
- **`vehicle_destroy(kind, pos)`**: a boat or minecart is about to be destroyed at `pos`. TNT minecarts that
  prime instead of breaking are not heard. Server-side.
- **`vehicle_create(kind, pos)`**: a boat or minecart joined the level at `pos`, after being placed by a
  player or dispenser. Server-side.
- **`player_toggle_sneak(player, name, sneaking)`**: a guest's player input changed sneaking, after the hub applied the state. The guest's state is swapped in; only actual changes are reported.
- **`player_input(player, name, flags)`**: a loaded guest sent a `PlayerInput` packet, after its flags were applied to the hub. `flags` is the packet's bit field, and the guest's state is swapped in.
- **`player_arm_swing(player, name, hand)`**: a guest's arm swing was handled and their attack strength reset. The packet currently swings `mainhand`; the guest's state is swapped in.
- **`player_jump(player, name)`**: accepted guest movement made the server player jump from the ground, after movement simulation finishes. The guest's state is swapped in.
- **`player_kicked(player, name, reason)` (version 2)**: a network or fake guest was kicked after the hub recorded the rendered disconnect reason; a network guest also receives the disconnect packet. The guest's state is swapped in, unless the guest was already removed in the same poll: the hook still fires, without it.
- **`player_level_change(player, name, from, to)`**: a server player's experience change altered their level, after the new level was applied. The guest's state is swapped in when the player is a guest.
- **`player_swap_hands(player, name, to_main_hand, to_off_hand, cancel)` (version 3)**: a player pressed the swap-hands key (not a spectator), heard
  before the main and off hand items swap. `to_main_hand` starts as the off hand's stack and `to_off_hand` as the main hand's; the hands get what the listeners leave, and the client gets both slots when they changed. Set `cancel` and they stay where they are; the player's client
  is sent both slots again. The player's state is swapped in, so `host.players().acting_player`, `PlayerRef::tell` and the item
  helpers reach them. Heard for a server's players, the single-player owner and a LAN host. Not heard: the same swap from an open inventory (hovering a slot and pressing the key).
- Services: `host.players().server_players()` returns online player handles in connection order;
  `player.tell(host, text)` sends a system line to one player, also the acting player inside their hook;
  `host.messages().broadcast(text)`, to every player and the server's
  log; `native_server_event(name, uuid, message)`, an `announce` event on a dedicated server's event stream, which the Discord
  bridge relays when `[discord] events` lists `announce`; `host.commands().feedback(text)`, the command's answer to the player whose command it is; `native_world_dir()`, the
  open world's folder, where a server mod keeps its data in a folder of its own so the world's backups carry it.
- Items: a player's inventory is their [`ENTITY_ITEMS`](../concepts/capabilities/entity-items.md) handler, from any server hook.
- Experience: `native_experience_orbs(dimension, min, max)`, the orbs in a box (id, position, `value`, and `count`
  for merged orbs), read before a mod cancels an orb in `entity_join_level`; and a player's `give_experience_points(host,
  points)` and `give_levels(host, levels)` through their `PlayerRef`: points given as touching an orb gives them, Mending
  repairing their damaged items first, the rest going to their experience with the level-up sound; levels added or,
  negative, taken off (`Player.giveExperienceLevels`, as the anvil and the enchanting table charge; below 0 the level
  and points are 0; no level-up sound). Both return false for a player not here, dead, or swapped out by another
  player's hook, and where the server's player list is busy (some world ticks hold it): give first, and cancel the orb
  only when the give returned true. A merged orb is worth `value` × `count`.
- **`player_totem(player, name)`**: a totem of undying saved a player whose hurt would have killed them, on a server
  (`checkTotemDeathProtection`), heard just before it is used up. Listen only: the health state machine decides before a
  mod is asked, so a mod cannot veto it. The player's state is swapped in, so `host.players().acting_player`, `PlayerRef::tell` and the
  item helpers reach them. Not heard: the host of a LAN world.
- `host.players().acting_player()`, the player whose state is swapped in (inside a command executor,
  `right_click_block`, `native_use_on`, `break_block`...; None elsewhere);
  `host.commands().run_command(line)`, a command run as the server's console (level 4, no output), including during a guest's
  turn. The acting player and other live guests are reachable; displaced outer players are excluded only on this
  native path, without changing ordinary player or console command resolution. A teleport to another level
  (`execute in minecraft:the_nether run tp Alex 0 70 0`) lands at the next tick's start. The returned player handles read health through their living handle (0 once dead).
  `player.action_bar(host, text)`, a line over a player's hotbar, which also reaches the acting player inside their
  hook.
- **`food_level_changed(player, name, from, to, cancel)`**: the food level of a player is about to change, on a
  server (eating, hunger and saturation effects, exhaustion, starving), before the new level reaches them. Change `to`
  (0 to 20) or set `cancel` to leave it at `from`, saturation included. It is heard once for what one turn of the
  player changed, not for the full bar a respawn gives, the food a player joins with, or the host of a LAN world. The
  player's state is swapped in, so `host.players().acting_player`, `PlayerRef::tell` and the item helpers reach them.
- **`right_click_block(click, use_block, use_item, cancellation_result, canceled)`** : NeoForge's `PlayerInteractEvent.RightClickBlock`, a player's right click on a
  block, once per hand where `useItemOn` starts, on the clicking client and on the server (`click.side`). `click`
  (`BlockClick`) names the level, block, hit, hand, held stack and the player's UUID and entity id. `use_block`
  and `use_item` (`TriState`) force the block's use or the item's `useOn` off, or on (the block even while
  sneaking with something in either hand, the item even with an empty hand or while it cools down).   Default skips sneak with either hand full (both-stacks bypass remains open in #2934) and requires a held item off cooldown; forced empty use reaches `ItemAfterBlock` with no stack or Part.
  Adventure restrictions still apply. Cancelling skips both and answers `cancellation_result` (`InteractionResult`, default `Pass`, which goes on to the item's own use and the other
  hand); with `break_block`, a cancel with `Fail` keeps players out of claimed land, and the player's client is
  sent the block back.
- **`use_item_on_block(click, phase, cancellation_result, canceled)`**: NeoForge's
  `UseItemOnBlockEvent`, on both sides, immediately before the selected block or item-on-block phase.
  Canceling replaces that phase with `cancellation_result` (default `TryWithEmptyHand`), skipping its
  Part and template. `UsePhase::ItemBeforeBlock`, `Block` and `ItemAfterBlock` all fire; `ItemBeforeBlock` precedes the item's
  `on_item_use_first` Part.
- **`right_click_item(click, cancellation_result, canceled)`**: NeoForge's
  `PlayerInteractEvent.RightClickItem`, on both sides, before `Item.use` and its Part after target
  fallthrough. Canceling answers with `cancellation_result` (default `Pass`). `Success`,
  `SuccessServer` and `Consume` end the client's hand loop. Only `Success` predicts a swing.
- **`block_spread(dimension, kind, from, to, canceled)`** and **`native_piston_pre(dimension, piston,
  direction, extending, push, destroy, canceled)`** and **`piston_post(dimension, piston, direction, extending, push, destroy)`** (on `GameLevel`, like the block observers): a fluid flowing or fire
  spreading (`kind` `fluid` or `fire`) from `from` into `to`, and a piston moving blocks once its structure resolved (`native_piston_pre`, NeoForge's `PistonEvent.Pre`; `piston_post`, `PistonEvent.Post`, hears the move done).
  Canceled, the block at `to` stays as it is, and the piston moves nothing. Fluids ask on every flow: answer from
  memory, fast.
- **`block_ignite(dimension, player, pos, cause, cancel)`** (on `GameLevel`): fire is about to be placed.
  It covers fire spread, lava's random fire placement, lightning strikes, and dispenser flint-and-steel. Set `cancel`
  to leave the position unchanged. Direct player flint-and-steel/fire-charge placement is not covered.
- **`block_burn(dimension, pos, cancel)`** (on `GameLevel`): fire's burnout check is about to remove the block at
  `pos`. Set `cancel` to keep it.
- **`crop_grow_pre(dimension, pos, state, result)`** and **`crop_grow_post(dimension, pos,
  original_state, state)`** (on `GameLevel`): NeoForge's `CropGrowEvent.Pre` and `.Post`, at the random-tick growth of
  crops (beetroot and torchflowers after their own 1 in 3 skip), stems, nether wart, cocoa, sweet berries, bamboo,
  cactus, chorus flowers, sugar cane, kelp and the vines. `result` is a `TriState`: `Default` keeps vanilla's
  chance, `True` grows, `False` doesn't. Pre fires at the position that grows (the block above for cactus and chorus,
  the block grown into for kelp and vines). Vanilla's random roll is drawn first, in vanilla's order, and passed as
  the default, so crops keep vanilla's random sequence. Pitcher crops, bamboo saplings and mangrove propagules
  don't fire.
- **`dragon_egg_form(dimension, pos, cancel)`** (on `GameLevel`): the first Ender Dragon fight is about to place
  its egg at `pos`. Set `cancel` to leave the block unchanged.
- **`fluid_level_change(dimension, pos, fluid, from, to, cancel)`** (on `GameLevel`): flowing `water` or `lava`
  at `pos` is about to change level. Levels run from 0 through 8. Set `cancel` to keep the old level.
- **`neighbor_notify(dimension, pos, state, skipped_side, cancel)`** (on `GameLevel`): NeoForge's
  `NeighborNotifyEvent`. The block at `pos` is about to tell its neighbours it changed: `ServerLevel.updateNeighborsAt`
  (all six sides, `skipped_side` is `None`) or `updateNeighborsAtExceptFromFacing` (all but `skipped_side`). Set
  `cancel` and no neighbour hears, on both paths. Asked for every such update, redstone
  included: answer fast.
- **`create_fluid_source(dimension, pos, state, can_convert)`** (on `GameLevel`): NeoForge's
  `CreateFluidSourceEvent`. When the game works out a block's new fluid (`FlowingFluid.getNewLiquid`), it asks once
  for each horizontal neighbour that is a source of the same fluid, with that neighbour's `pos` and `state`; two
  counted sources over a solid block or a source make a new one. `can_convert` starts as the fluid's rule
  (`water_source_conversion` or `lava_source_conversion`). Spread checks work out new fluids too, so this is asked
  often: answer fast.
- **`leaves_decay(dimension, pos, cancel)`** (on `GameLevel`): a non-persistent leaf block at distance 7 is
  about to decay. Set `cancel` to keep it.
- **`inventory_block_start(player, name, pos, cancel)`**: a guest is about to open an inventory supplied by a
  block at `pos`. Set `cancel` to leave it closed. The guest's state is not swapped in.
- **`moisture_change(dimension, pos, from, to, cancel)`** (on `GameLevel`): farmland at `pos` is about to
  change moisture. Set `cancel` to keep its old moisture.
- **`sculk_bloom(dimension, pos)`**: after a sculk catalyst at `pos` takes experience, its block has entered
  the blooming state.
- **`sponge_absorb(dimension, pos)`** (on `GameLevel`): a sponge at `pos` has absorbed water and become wet.
- **`target_hit(dimension, pos, strength)`**: a projectile hit the target block at `pos`, which now has the
  reported redstone output `strength`.
- **`vault_change_state(dimension, pos, state)`**: the vault block at `pos` changed its state to `state`
  (`inactive`, `active`, `unlocking`, or `ejecting`).
- **`block_failed_dispense(dimension, pos, item)`**: a dispenser at `pos` failed to dispense `item`.
- **`beacon_state_change(dimension, pos, active)`**: the beacon at `pos` gained or lost its complete beam
  pyramid; `active` says whether it is active now.
- **`anvil_damaged(dimension, pos, from, to)`**: the anvil at `pos` took damage, changing from `from` to
  `to`; `to` is air when it broke.
- **`block_break_progress_update(player, name, pos, progress)`**: a guest's crack animation at `pos` reached
  progress stage 0 through 9.
- **`block_lock_check(player, name, pos, locked)`**: a guest checks the lock on the container at `pos`;
  `locked` starts with vanilla's result and may be changed. The guest's state is not swapped in.
- **`block_fade(dimension, pos, from, to, cancel)`** (on `GameLevel`): ice or snow melts, or coral dies. `from`
  and `to` are block state IDs. Set `cancel` to keep the old block.
- **`block_form(dimension, pos, from, to, cancel)`** (on `GameLevel`): precipitation is about to place ice or
  snow, or add a snow layer at `pos`. `from` and `to` are block state IDs. Set `cancel` to keep the old state.
- **`bell_ring(dimension, player, pos, cancel)`** (on `GameLevel`): a bell is about to ring. Set `cancel` to
  stop the ring and its raider reveal.
- **`native_cauldron_level_changed(dimension, player, pos, from_level, to_level, reason, cancel)`** (on `GameLevel`):
  precipitation, dripstone, or a burning player being extinguished is about to change a cauldron's level. Set `cancel`
  to leave it as it was.
- **`random_tick(ticks, dimension, pos, state, handled)`** (on `GameLevel`): a block `tickChunk`
  random-ticks, before its own `randomTick`. Change the block with the level's `native_set_block_property(ticks, pos,
  key, value, flags)`; set `handled` to skip the block's own random tick.
- **`tick_block(ticks, dimension, pos, state, handled)`** (on `GameLevel`): a due scheduled tick of a block
  that is still there (`ServerLevel.tickBlock`), before its own `tick`; the same contract as `random_tick`.
- **`explosion_detonate(dimension, center, blocks, entities)`**: the positions an explosion is about to
  break and the entities in its box, before any is hit; take out the ones to spare (not hurt, not pushed).
- **`explosion_knockback(dimension, center, blocks, entity, knockback_velocity)`**: a server explosion's
  calculated velocity, before it is added to the entity. Replace `knockback_velocity` to change the push;
  `entity` is None for a player, whom `host.players().acting_player` names. The player packet uses the same replacement. Wind bursts are heard too.

- **`fill_bucket(stack, pos, canceled)`** and **`attack_entity(entity, kind, pos, canceled)`**:
  a player's bucket at a block, and their blow on an entity, with the acting player swapped in. Canceled, the bucket
  does nothing and the blow does no harm. A fluid or mob bucket also asks about the block beside the clicked face
  whenever that block is not its target, since a block that refuses the fluid passes it there; the fluid may still
  stay in the target.

### Mobs

`crates/sunrise-client/src/game/modding/native_mobs.rs`: helpers for server mods, for the level in place, on a server only (on a
client they do nothing, and the readers give nothing). Each takes the `dimension` (`minecraft:overworld`) of the level it means, as
`native_set_block` does, and is refused when another level is in place. The per-level tick hooks run with a defined level in
place; `server_tick` does not define one, so name the dimension and expect a refusal there.

- `LevelRef::spawn_mob(host, kind, pos, reason)` spawns a mob and returns an `EntityRef`, or `None` when refused.
  `SpawnReason` controls finalization; the new mob is not persistent, as for a natural spawn. Spawn limits and natural
  spawn rules do not apply, as for `/summon`. A non-mob kind, invalid or out-of-bounds position, unloaded chunk,
  stale or unavailable level, or cancellation during finalization or level entry refuses the spawn. Configure the
  resulting entity through its handle rather than a spawn-options record.
- `native_mob(dimension, id)` and `native_mobs(dimension)` read the living mobs: id, dimension, kind, position, yaw, health and maximum health.
- A mob's health is set with `living.set_health(host, health)` on its `LivingRef`.
- `native_kill_mob(dimension, id)` kills as `/kill` does, with its loot. `EntityRef::discard` removes an entity without death or loot: from the call on selectors and handles no longer find it, its passengers are off and `entity_leave_level` has fired, in the same hook.
- While the game ticks its mobs they are out of the level: from a hook inside that tick, a spawned mob joins after the tick, and the others
  read as gone.

### Loot tables

Changing loot a mod didn't make, as NeoForge's `LootTableLoadEvent` and global loot modifiers and Fabric's
`LootTableEvents.MODIFY` and `MODIFY_DROPS` do. A mod's own loot function and condition kinds are #5451.
- **`living_death(entity, kind, source, weapon, cancel)` (version 2)**: server-only, before loot, experience and death messages. `source: &HurtSource` retains the accepted killing blow's damage type, real causing entity (including players), direct entity (including projectiles) and original optional position; `weapon: Option<&ItemStack>` retains its full killing stack and components. `LivingDeathEvent` exposes `entity`, `kind`, `source`, `weapon`, `cancel`; the old top-level `damage_type`/`attacker` fields and callback signature are removed, with no shim. Cancel suppresses death loot and leaves health exactly where the listener left it: health 5 stays 5; an unhealed body at health <= 0 is removed after 20 ticks without redispatch or loot.
- **`sunrise_world::loot::loot_table_load(context, id, table)`**: each table's JSON as it loads, after the data packs'. `Hooks::loot_table_load` gets the owning load's `&dyn DataLoadContext` first (see "Items' default components"); its `origin()` is always `ServerDataLoad`, since only the server loads loot tables. Push a pool
  onto `table["pools"]` or an entry onto a pool's `entries`; chests, block and mob drops, fishing and the rest then roll the
  changed table. Keep a replaced table's `random_sequence`: a mob's table without one drops nothing. It runs once per table on
  each server data load and `/reload`, not on client preloads. It is a free function in
  `crates/sunrise-world/src/loot.rs`, so a helper injected there compiles inside `sunrise-world`, without `crate::game`.
- **`loot_drops(context, drops)`**: the items a broken block or a dead mob is about to drop, after its tables rolled (a
  mob's worn, held and carried items and inventory too, not an enderman's carried block or a copper golem's preserved equipment).
  Add, change or take out stacks; empty ones don't drop. `LootDrops` says what dropped them: its own loot table (empty when
  none rolled), the dimension, the origin, the block's position and state or the mob's id and type, the player who broke or killed
  it (100-tick player kill credit), the tool and explosion radius. Deaths carry `damage_source` (type, causing/direct IDs), `tool` is None, and `weapon_item(host)` reads the recorded projectile weapon or live melee weapon. It hears blocks
  that a player, an explosion or a mob breaks, not other breaks (leaves decaying, a torch losing its wall, a piston,
  `/setblock ... destroy`), nor a creative break or a tool that can't harvest the block. A block's stacks get its block entity's data (a
  shulker box's items) after the hook, and a container's contents spill on their own, not through it. A player's use drops
  through it too, with `used` set and that player named: shears on a sheep, a mooshroom, a snow golem or a bogged (its shearing
  table), a brush on an armadillo (`minecraft:brush/armadillo`) or a suspicious block (its item, no table), shears carving a
  pumpkin, picking sweet berries or cave vines (their harvest table), and a player's hit taking an item frame or its item (the
  frame's entity, no table). A dispenser's shears and an explosion's frames aren't told. Leads: untying one or cutting it with
  shears is a use (the leashed mob, no table), and a mob a player killed while leashed drops its lead on its next tick as a call
  of its own, naming the killer (`used` false, no table). A lead that snaps or loses its holder names no player and isn't told.
- **`native_living_drops(entity, kind, player, source, recently_hit, drops, cancel)`**: NeoForge's
  `LivingDropsEvent`, the whole of what a dying mob, server player or armor stand drops, after `loot_drops` and with the
  rest of `dropAllDeathLoot` in: an enderman's carried block, a copper golem's preserved equipment, and a player's inventory
  (unless `keepInventory`). Each `ItemDrop` holds the stack, position, motion and pickup delay its item entity gets, and
  its thrower, target (who alone may pick it up), no-gravity and starting age; an unedited drop spawns unchanged. `source` is a `HurtSource` (the damage type,
  `causing` and `direct` entity ids); the id of a projectile isn't kept, so `direct` is 0 for it. Edit the list or set `cancel`
  for no drops; experience orbs are separate (`living_experience_drop`). An armor stand fires it from `brokenByAnything`
  with an empty list, and its stand item and equipment pop outside the list, as in vanilla. `loot_drops` stays for loot that
  is not a death (blocks, shearing, leads); for a mob's death it is heard first (it edits the stacks a table rolled), and this hook
  then fires once on the whole collected list, with nothing spawned until it returns.
- **`native_get_equipment_drop_chance(entity, kind, slot, source, recently_hit, stack, original_chance, preserved, chance)`**:
  NeoForge's `GetEquipmentDropChanceEvent`. Change the chance a dying mob rolls for the item `stack` in `slot`; `original_chance`
  is the recorded chance before enchantments and `preserved` says the slot drops without a player kill. Each slot is asked at its
  own point of `Mob.dropCustomDeathLoot`, just before its roll, and the roll is drawn whatever the chance is.
- **`enchanted_entity_loot(entity, source, attacker, enchantment, level)`**:
  NeoForge's `EnchantedEntityLootEvent`. Change the level an entity's loot table uses of `enchantment`, in the middle of the roll:
  `EnchantedCountIncreaseFunction` and `LootItemRandomChanceWithEnchantedBonusCondition` ask each time they are evaluated, on a death and on
  `/loot ... kill`, whose `source` is `minecraft:magic` and whose `attacker` is the command's source. `entity` and `attacker` are live handles;
  `attacker` is None without one and may not be living, and `level` then starts at 0, otherwise at the living attacker's level. It is unsigned, so
  the negative levels `setEnchantmentLevel` throws on can't be written, and above 1024 it counts as 1024, so a mod's value can't size a loop. Without a listener the roll asks nothing, and its random's order is vanilla's.

### A mod block entity's data on its item

A mod's block entity carries its saved data on the item as vanilla's do: Ctrl + Pick Block, a loot function that copies the block entity (`minecraft:copy_components`-style) or a `/give` with `minecraft:block_entity_data:{id:"mod:type",...}` puts the compound (without `x`, `y`, `z`) in the item's `minecraft:block_entity_data`, and placing the item merges it into the new block entity before `native_place_block` runs, the counterpart of NeoForge's `BlockItem.updateCustomBlockEntityTag` and `BlockEntity.saveToItem`. Data tagged with another type is ignored. `/give` and `item replace` accept a type id a mod declared (its content's block entity types) and refuse any other as vanilla refuses an unknown one. The data is the compound the block entity saves, without its position and its components (`saveCustomOnly`): a component patch, a custom name say, rides as the item's own components and comes back as the block entity's on placing. A mod reads the data back in its own load as it does for a chunk.

### A block's items

A server mod reaches a block's items through its `ITEMS` capability ([parts](../concepts/parts.md)): `level.capability(host, &ITEMS, pos, side)` gives the block entity's slots as that side reaches them, by the rules hoppers follow (a furnace's sides, a double chest's two halves, a mod's `ModContainer` with its `faces`, `inputs` and `outputs`), and `resource_util::move_resources(host, &from, &to, filter, max, Some(tx))` moves items between two of them in a transaction. `side` is the side of the block being reached, so a pipe above a chest uses `Some(Direction::Up)`. A level not in place, a stale `LevelRef` and a client's copy of the world give none. `native_dimension()` names the level in place (`minecraft:overworld`) for a hook that isn't told it, such as `block_entity_tick`.

During `block_entity_tick`, use `event.subject.with_view` to read or edit the machine's own item slots in place. Return the view before borrowing the Host to reach another block. A composter has no block entity, so it has no `ITEMS`: hoppers fill and empty it. Container entities (minecarts, chest boats) aren't reached this way yet. A pipe keeps a neighbour's capability with `BlockCapabilityCache` instead of looking it up every tick ([keeping a block capability](../concepts/capabilities/keeping-a-block-capability.md)). A provider whose answer changes with no block change (a machine's side configuration) calls `level.invalidate_capabilities(host, pos)` (NeoForge's `invalidateCapabilities`): the capability generation there moves on, at the root commit inside a transaction and not at all when it aborts, and every cache there looks up again at its next `get`. `level.capability_generation(host, pos)` reads it; only its change means anything.

### Entities

`crates/sunrise-client/src/game/modding/native_entities.rs`: helpers for server mods, for the level in place, on a server only. Each
names the level it means by its `dimension` (`minecraft:overworld`), as `native_set_block` does.

- `native_summon(dimension, kind, pos, nbt)` spawns an entity of `kind` from its compound, as `/summon <kind> <pos> <nbt>`
  does (passengers included), and returns its id. None on a client, for another dimension, outside `/summon`'s bounds, an
  unloaded chunk, an unloadable kind, or a mod's `entity_join_level` veto (in a transaction its id goes stale).

A model that moves (a turning gun barrel, a thrown projectile) is an `item_display` showing an item whose model comes from
the mod's `.smod` assets: summon it once, then move it and pose it with `DisplayRef` from a tick hook. Easing between ticks happens on each
client, so a call a tick looks smooth.

### Entity and level handles

`crates/sunrise-client/src/game/modding/native_entity_refs.rs` (#7387): an `EntityRef` names one entity of the open world and a
`LevelRef` one level. They are small `Copy` values a mod keeps, and their methods take the host, so a mod can hold
several at once: `host.entity(id)` makes one from the id an event carries, `host.level("minecraft:the_nether")` one from
a dimension.

An entity held out of storage while it is changed refuses handle reads and writes: reads return `None` and writes return
`false`. This includes cart and ridden-cart ticks, boat ticks (including ridden-boat leash ticks), and a copper golem turning into a
statue. A block entity also refuses its handle while it is being ticked. Tick hooks before and after that work can use
the handle normally.

The client also owns a world epoch, separate from the server's. It begins when a play login creates the
client level, changes when a dimension change or later login replaces that level, and retires after the
client logout hook or when reconfiguration clears the level. A same-level respawn keeps the epoch.
Client epochs do not fire server world lifecycle or server tick hooks. This bookkeeping does not make
the server entity and level reads available on a client; the reads listed under "On a client" below are.

On a client (`host.local_player()`, a client hook's `host`), these reads answer from the client's own copy, which is
what the server sent it (vanilla's `LocalPlayer`, `ClientLevel` and `Entity` fields), and are `None` for a stale
handle, an entity the client doesn't track and a level it doesn't show:

- `host.entity(id)` and `host.level(dimension)`: the local player or an entity the server told the client of, and the
  one level the client shows. `host.living(entity)` is the local player only.
- `entity.position(host)`, `rotation(host)` and `velocity(host)`: the client's copy, interpolated as it draws it.
- `entity.name(host)`: the synced custom name or type name, and a player's profile name, from either side's host.
- `level.block_state(host, pos)`, `block`, `block_property` and `is_loaded`: blocks of the chunks the client holds.
  A chunk it doesn't hold reads as unloaded; no read loads one.
- `level.day_time(host)`: the synchronized Overworld clock in ticks (counting past 24000), advanced at the
  server-supplied rate while the client level runs. This remains the Overworld clock in the End, even though
  the sky uses the End's own default clock. Only a handle for the level the client shows can read it.
- `living.with_equipment(host, slot, read)`, `with_hand_item`, `effect`, `effects`, `potion_affected` (`hasEffect`), `health` and `max_health`, for the local player.
  Another entity's equipment, effects and health stay server reads, and so does every write.
- `player.last_death_location(host)`, for the local player: the position the login and respawn packets carried.
- `player.with_inventory(host, read)` lends a read-only inventory view in vanilla slot order (0 to 35 main, 36 to 42 equipment).
  Use `view.get(n)` to read slot `n`, or `view.iter().take_while(|(slot, _)| *slot < 36)` for just the main inventory.
  Read item, count and damage inside `read`. Use `stack.project()` only for components or to retain an owned stack;
  a failed projection does not mean the slot was empty. An unavailable player returns `None` without calling `read`;
  on a client only the local player's inventory is available.
- For the local player, its own state as the server sent it (`LocalPlayer`): `player.game_mode(host)`, `is_creative`,
  `is_spectator` and `may_build` (the game mode of the login, respawn and `ClientboundGameEvent` packets); `food`
  (level and saturation from `ClientboundSetHealth`; the client keeps no exhaustion, so it reads 0), `needs_food`,
  `has_enough_food` and `can_eat`; `abilities` (what the game mode gives, then `ClientboundPlayerAbilities`' flags and
  speeds); `selected_slot` (the hotbar slot held); `experience` (`ClientboundSetExperience`) and
  `xp_needed_for_next_level`. Another player reads `None` there: the client is not sent their state.
- The local player's open menu: `host.local_player()` then `player.container_menu(host)` makes a `MenuRef` for the menu
  the client has open (the player's own inventory when none is), and `menu.slot_count(host)`, `slot(host, index)` and
  `carried(host)` read it as the server's `ClientboundContainerSetSlot` and `SetCursorItem` packets left it. Indexes are
  the menu's own slot order (the same `slot_backing` names), not network slot ids. Another player's menu is `None`, and
  so are `container_id` and `kind` on a client (server reads). A slot reads `Some(None)` when empty and the outer `None`
  when it doesn't exist or the menu is unavailable; a menu type the client has no slot map for reads `None` throughout
  rather than guessed contents. A handle goes stale, and reads `None`, when its menu is closed or replaced (a reused
  window id included), on a respawn, a dimension change or a logout, and after a reconnect. Nothing here sends a packet
  or changes the menu.

- `entity.position(host)`, `block_position`, `rotation` (yaw and pitch), `kind` (`minecraft:zombie`), `uuid` (`&mut`:
  an entity with no UUID yet gets its own) and `level` read the entity.
- `level.block_state(host, pos)` returns a typed `BlockState` without allocating a state string. For text such as
  `minecraft:oak_stairs[facing=east,…]`, pass the ID to `host.block_state_string(id)`. `block` (the name alone),
  `block_property(host, pos, key)`, `is_loaded`, `height` (lowest `y` and height), `game_time` and `day_time` read any
  level the server has loaded, not only the one in place. A block in a chunk that isn't loaded or a `y` outside
  the world's height reads as `None`. `level.brightness(host, LightLayer::Block | Sky, pos)` reads the stored light
  (`getBrightness`) the same way, as the game's own light query holds it, and is `None` where the block read is.
  `level.height(host)` (`LevelHeightAccessor.getMinY`, `getHeight`) also works with `&dyn Host` in client
  hooks, including render, screen and mouse hooks. Obtain the handle with `host.level(dimension)`; on a
  client only its current dimension is available, after login. It reads that client's synchronized level
  shape, without loading chunks or allocating. A stale handle, logout or a different dimension returns
  `None`, not guessed Overworld bounds. On a server it still reads parked loaded levels as well.
  `level.fluid_state(host, pos)` reads the fluid there the same way.
  `level.fluid_flow(host, pos)` (`FluidState.getFlow`) reads its current as a `Vec3`, through the function that pushes
  entities, so a `fluid_flow` listener's change is what it reads: zero where no fluid flows, `None` where the block read
  is. It reads on a client too, from the client's own copy of the level. It costs what one entity's push by that block
  does: up to twelve block reads around `pos` (the four neighbours and below them; falling fluid also checks
  their faces), with no allocation, plus the `fluid_flow` listeners.
- `level.weather(host)` reads rain and thunder levels on **both sides**, using the shared `Host` from server,
  render, screen or mouse hooks. It returns `Some((rain, thunder))`, each 0.0 to 1.0, at partial tick 1.0
  (`Level.getRainLevel` / `getThunderLevel`, with thunder scaled by rain). Both are 0 where the level cannot have
  weather. A server can read any open level, including parked levels; a client reads only its current synchronized
  level after login. Stale handles and unavailable levels return `None`; the read never loads a level.
- `level.seed(host)`, `difficulty` (`"normal"`, as `/difficulty` names it), `game_rule(host, "keep_inventory")` (a
  `GameRuleValue`, `Bool` or `Int`; None for an unknown rule), `game_rules` (every rule as a `minecraft:` id and its
  value), `is_raining` and `is_thundering` read its server settings.
  `level.is_raining_at(host, pos)` (`Level.isRainingAt`) asks about one position of any open level: it rains there, the
  position has full sky light with nothing motion-blocking or fluid above it, and its biome's precipitation at that
  height is rain (not snow or none). It is `None` where the block read is, an unloaded chunk or a `y` outside the
  world. `level.enabled_features(host)` is the world's `FeatureFlags` (`LevelReader.enabledFeatures`:
  `contains("minecraft:redstone_experiments")`, `iter()`); every level of a world shares them. If the save
  omits its flag list, the world enables `minecraft:vanilla`.
- `level.chunk_ref(host, [x, z])` makes a `ChunkRef` for any chunk of an open level (None past 30 million blocks).
  `chunk.forced(host)` and `level.forced_chunks(host)` query `/forceload` tickets. `chunk.set_forced(host, bool)` adds
  or removes one and returns whether the saved set changed. A successful add waits for the chunk to load, even in a
  parked level. If streaming is paused, the add records the ticket to take effect when its level starts streaming.
  Adds after generation stops return false; repeated adds and removals do not load chunks.
- Ticket controllers (NeoForge's `TicketController`, registered in `RegisterTicketControllersEvent`): a mod registers
  each one once at startup, from `Hooks::register`, with `r.register_ticket_controller("quarry", validates)` and keeps the
  `TicketController` it returns (it is `Copy`; `Hooks` methods take `&self`, so a `OnceLock` holds it). The id is the
  calling mod's: `quarry` and `mymod:quarry` both become `mymod:quarry`, and another mod's namespace refuses the mods at
  startup, as does an id registered twice (the error names the mod and the id). `validates` is whether the controller
  validates its saved tickets (NeoForge's `LoadingValidationCallback`, below). The handle is only made by the registrar
  and indexes the installed table, so a ticket call looks up no string. Another mod's controller does nothing.
- `chunk.set_mod_forced(host, controller, owner, forced, natural_spawning)` (NeoForge's `TicketController.forceChunk`)
  holds or releases a chunk through `controller` and returns whether its tickets changed. `owner` is a
  `TicketOwner`, `Block([x, y, z])` or `Entity(uuid)`. A ticket is scoped by controller, owner, chunk and
  `natural_spawning`, so two owners in one chunk release separately, one owner holds a plain and a natural-spawning
  ticket on a chunk apart, and two controllers of one mod never touch each other's. The chunk stays loaded, entity
  ticking and saved, and its level keeps ticking, while any ticket or `/forceload` holds it. With `natural_spawning`, the
  chunk joins the spawning chunks (`ChunkMap.collectSpawningChunks`, within 8 chunks of
  a player): it gets inhabited time, thunder and a spawn attempt. The local mob cap then lets mobs spawn there only
  while a non-spectator player is within 128 blocks of the chunk's centre, as vanilla's
  `LocalMobCapCalculator` decides. `level.mod_forced_chunks(host, controller)` lists the controller's tickets as
  `(owner, [x, z], natural_spawning)`. Tickets are saved in `data/sunrise/mod_chunk_tickets.dat`, never in vanilla's
  `chunk_tickets.dat`, under the controller's id. When a world loads, the tickets of an id no loaded mod registers are
  dropped with a warning in the log, as NeoForge documents for a controller nobody registers; so are tickets from a
  file written before controllers, which are keyed by mod id. Outside a mod's hook, for another mod's controller, and for
  a stale handle or a chunk past the world's bounds, the calls return `false` or `None`.
- `validate_tickets` (NeoForge's `LoadingValidationCallback.validateTickets`, server) fires for each level that opens,
  after `level_load` and before its first tick, once for each controller registered with `validates` that has saved
  tickets in the level, and only to mods that implement it. `event.controller` is the controller; its owning mod's
  `event.validation` shows that controller's tickets read from the save when the level loaded, before they take hold
  (NeoForge's `TicketHelper` is per controller): a ticket a mod adds in `level_load` is not among them and is never
  removed by it, and a ticket of another controller is not shown. Every mod that implements the hook is called, but
  only the owner's view holds anything; a controller without a validator keeps all of its tickets. Removals take
  effect in the callback's view immediately but only publish if the callback completes without panic or phase refusal.
  The tickets that survive then force their chunks. `block_tickets()` gives each block position and `entity_tickets()` each entity UUID
  with a `TicketSet`, the chunks it
  holds apart by kind (`non_natural_spawning` and `natural_spawning`, as NeoForge's `TicketSet`). A plain and a
  natural-spawning ticket on one chunk are two tickets. `remove_block_tickets(pos)` and `remove_entity_tickets(uuid)`
  remove everything an owner holds, both kinds (and return how many); `remove_block_ticket(pos, chunk, natural_spawning)`
  and `remove_entity_ticket(uuid, chunk, natural_spawning)` remove the one kind they name and leave the other. A removed ticket never
  forces its chunk and is left out of the next save.
- `host.living(entity)` gives a `LivingRef` for a mob, a player, an armor stand or the dragon (vanilla's
  `LivingEntity`); `living.entity()` goes back. `living.with_equipment(host, EquipmentSlot::Head, read)` and
  `living.with_hand_item(host, Hand::Main, read)` lend a single stack. An empty or unavailable slot returns `None`
  without running `read`. Use `stack.project()` in the callback only for components or to keep an owned copy.
  `living.with_weapon_item(host, read)` lends the living attacker's weapon on the server (the active Riptide stack
  when present, otherwise its main hand). `set_equipment(host, slot, stack)` puts a stack in a slot as
  `LivingEntity.setItemSlot` does (None or air empties it), and `set_hand_item(host, hand, stack)` puts one in a hand
  as `LivingEntity.setItemInHand` does (the main hand's or the off hand's slot, written as `set_equipment` writes it).
  Mobs store all eight slots; players (the host and guests) keep body and saddle beside their hand and armor fields. A player's
  `body` and `saddle` are kept and saved in the `equipment` compound as vanilla does, and dropped on death, but nothing
  renders or sends them. Armor stands and the dragon hold all eight slots in the same `Equipment`, saved in `equipment`
  (omitted when empty). A stand keeps a body or saddle a mod writes (`setItemSlot` stores it; `ArmorStand.canUseSlot`
  only stops a player equipping it), and pops it with the rest, after the other six slots, when it breaks. A write to a
  stand or the dragon runs `LivingEntity.onEquipItem` like a mob's: the equip sound and the `equip` or `unequip` game
  event, unless the entity has not ticked yet or the item did not change. `living_equipment_change` hears all
  eight slots of a stand. `/item replace` keeps vanilla's predicate for a stand's or the dragon's chest, legs, feet, body
  and saddle (an empty stack, or an item made for that slot; a stand's `canUseSlot` refuses body and saddle), while
  `set_equipment` stays permissive. This is not full dragon equipment parity: the dragon keeps its equipment for mods and
  saves, and does not drop it on death (the death drops are built on `Mob`, which the dragon is not).
- `living.swing(host, hand, send_to_self)` (`LivingEntity.swing`) starts an arm swing, shortened by haste and lengthened
  by mining fatigue, unless one is under way and past its first half (it returns whether one started). The players around
  see it at the end of the tick, the swinger too with `send_to_self`; clients see a mob swing the hand asked for. Mobs and
  players work; armor stands and the dragon return false, and so does a player other than the acting one while another
  player acts.
- `living.arrow_count(host)` / `set_arrow_count` and `stinger_count` / `set_stinger_count` (`DATA_ARROW_COUNT_ID`,
  `DATA_STINGER_COUNT_ID`) read and set what is stuck in a player or a mob; the entity data carries it to the players
  around, so they see it. Armor stands and the dragon keep no such count (None, false). `set_arrow_removal_ticks` and
  `set_stinger_removal_ticks` set the ticks until the next one falls out (`removeArrowTime`, `removeStingerTime`; at 0 or
  less the timer restarts at `20 * (30 - count)`); a mob's timers run in its tick as a player's do.
- `living.item_use_remaining(host)` / `set_item_use_remaining(host, ticks)` (`useItemRemaining`) read and move the end of
  the item a player uses: at 1 the use completes on the next tick, at 0 or less it lasts until the player lets go, as in
  vanilla. It moves the deadline the use-item Tick hook moves, so shield, bow, trident, spear, instrument and bundle uses,
  which don't count it, ignore it, and a call inside that Tick hook is overwritten by the hook's result. Nothing in use reads 0 and refuses a write. A mob's use never runs out: its read is the item's use duration less the ticks its use counter has run, and a write moves that counter to where `ticks` are left, which the stabs of a spear and the draw of a bow follow. A piglin admiring gold and a drinking witch use no item, so they read 0 and refuse a write.
- `living.set_riptiding(host, ticks, damage)` (`startAutoSpinAttack`) starts a player's Riptide spin: for `ticks` ticks
  the player turns, the first mob its box touches takes `damage` and ends the spin, and the players around see it
  (living flag 4). It adds no velocity (a Riptide launch does that itself), and a `ticks` of 0 or less ends the spin.
  Mobs, armor stands and the dragon return false: Sunrise runs no spin for them.
- `living.set_waypoint_style(host, &style)` and `set_waypoint_color(host, rgb)` (`Waypoint.Icon.style`, `.color`) change
  the locator bar icon of a living entity as `/waypoint modify` does, untracking and tracking its waypoint again, so the
  players it transmits to get the new icon. A colour of `None` shows a player in their team's colour. Only a player's
  waypoint reaches other players (`waypoint_transmit_range` is 0 for every other entity); a mob's or an armor stand's
  icon is kept for its waypoint and `/waypoint list`. The dragon returns false.
- `living.used_item_hand(host)` (`getUsedItemHand` while `isUsingItem`) reads the hand of the item a player or a mob uses,
  None when it uses none; on a client only the local player answers. `living.start_using_item(host, hand)` is
  `LivingEntity.startUsingItem`: the item's use timer and the use-item Start hook, not the item's own `use` (no ammo check,
  no sound). A player's client lets go of a use whose button isn't down, as in vanilla, so that use is released on its next
  tick. `living.release_using_item(host)` is `releaseUsingItem`, what letting go of the button runs: the use-item Stop
  hook, the item's own release (a player's bow or trident shoots), a mod's `release_using`, once. A mob's use is its goal's
  counter: it shows and its goals see it, but it never runs out, so only a release or a goal ends it, and vanilla's bow and
  trident have no release for a mob. A piglin with something in its off hand and a drinking witch refuse a start and have no
  use to read or release: the counter is their own timer.
- `living.effects(host)` lists a living entity's active effects (vanilla's `activeEffects`) as `ActiveEffect`s — the
  load-owned `EffectKey` (borrow its canonical name, if available, with `key.id(host)`), the ticks left (-1 infinite),
  the amplifier, and the `ambient`, `visible` and `show_icon` flags (`show_icon` is vanilla's `showIcon`, apart from the particles). `living.effect(host, key)` reads one (`getEffect`) and `living.potion_affected(host,
  key)` answers `hasEffect`; request the key once through `Registrar::effect_key`. `living.add_effect(host, &effect)` is
  `LivingEntity.addEffect` by the game's own path: a player's or a guest's goes through the potion machinery (its attribute
  modifiers, absorption, and the added-effect hooks for a content mod's effect), a mob's past `canBeAffected` (a spider
  shrugs off poison) and with the same hooks. It returns true when accepted — whether it replaced one already there or only
  extended it is `MobEffectInstance.update`'s business (a stronger one replaces with its own duration, an equal one extends,
  a weaker one changes nothing). `living.remove_effect(host, key)` and `living.remove_all_effects(host)` take effects away
  the way their running out does: the attribute modifiers they gave recompute and absorption's hearts go with theirs. The
  added-effect hooks hear a player's add without an entity id, so a guest's reaches them as the acting player (`None`) — a
  limitation of the hooks themselves, unchanged here. Armor stands and the dragon keep no effects, as `/effect` skips them:
  `effects` and `effect` read `None` for them and their writes return false. On a client,
  `living.effect_blend_factor(host, key, partial)` (`getEffectBlendFactor`) reads how far the local player's effect has
  faded in at the partial tick: darkness fades over 22 ticks and nausea in over 150 and out over 20. Other effects start
  at 0 after a blended addition and interpolate to their post-countdown target each client tick (1 while active, 0 once
  expired). An unblended addition starts at its target immediately; a replacement packet preserves the old blend.
  Partial ticks are not clamped. A missing effect reads 0. The client keeps no other entity's effects, so they read `None`.
- `host.fishing_hook(entity)` gives a `FishingHookRef` for a cast hook (`FishingHook`); `hook.entity()` goes back.
  `player_owner(host)` is the player who cast it, `hooked_in(host)` what it holds (None when nothing, or when that is
  gone) and `open_water(host)` is `isOpenWaterFishing`. `item_fished` exposes mutable catch drops, cancel and absolute
  rod damage, and names a hook by id. A server box query with `LevelRef::visit_entities_in` can find fishing hooks.
  Discarding one takes it out. It is not a `ProjectileRef`, and selectors do not find it yet.
- `host.projectile(entity)` gives a `ProjectileRef` for a projectile in the level's projectile list (arrows, a mob's
  trident, thrown items, fireballs, wind charges, skulls, shulker bullets, mods' thrown types), a trident a player
  threw or a firework rocket; `projectile.entity()` goes back. A player's trident and a rocket have an owner and a
  deflection only: `pickup`, `base_damage` and `critical` read None and their setters return false, and a player's
  trident always has a player owner (`set_owner` or `deflect` with none or a mob returns false). `owner(host)` resolves the owner to a current `EntityRef` (None when gone
  or in another level) and `set_owner(host, owner)` takes any entity but a fishing hook (`Projectile.setOwner`). An owner that is neither a
  player nor a mob is kept by its UUID: `owner` resolves it and a save keeps it, but a hit credits no one for it. `deflect(host, deflection, by, new_owner, by_attack, power)` is `Projectile.deflect`: `Deflection::Reverse`
  draws one float for the turn from the projectile's own random (`Entity.random`, never the level's), `AimDeflect`
  and `MomentumDeflect` use the look or the known speed (last tick's move, `Entity.getKnownSpeed`) of `by` and do
  nothing without it; the new owner is set and a fireball's or wind charge's acceleration power becomes 0.1 by an
  attack, else halves. A wind charge refuses for its first five ticks (`WindCharge.noDeflectTicks`): false, with no
  change and no draw. `set_owner` on an arrow also allows pickup for a player owner (`AbstractArrow.setOwner`);
  `deflect` does not (vanilla's `EntityReference` overload). Arrows (and a mob's trident for
  `pickup`) also have `pickup`/`set_pickup` (`ArrowPickup`), `base_damage`/`set_base_damage` and
  `critical`/`set_critical` and `pierce_level`/`set_pierce_level`. The pierce level is a signed byte:
  vanilla stores all values from -128 through 127 unchanged, and nonpositive values do not pierce. The
  typed result distinguishes a stale handle, a non-arrow projectile and a wrong-side host. The critical
  flag reaches clients through entity data; these handle methods operate on the server.
- `living.drop(host, stack, throw_randomly, include_thrower_name)` throws a stack the mod holds from the eyes
  (`LivingEntity.drop`) and gives the item entity; it never touches a slot. `is_tamed(host)` and `owner(host)` read a
  tamable mob or a horse (`TamableAnimal.isTame`, its owner's UUID).
- `entity.velocity(host)` (a player's is the move its client last reported), `set_velocity(host, v)` (not for players,
  whose client moves them), `on_ground`, `bounding_box` (lowest and highest corner), `eye_height`, `eye_position`,
  `view_vector` and `distance_to_sqr(host, pos)` read and move it.
- `entity.up_vector(host)` is `Entity.getUpVector`, the view vector turned toward the zenith. `on_block(host)` is
  `Entity.getOnPos`: the supporting block of an entity on the ground (the solid block under its box nearest it),
  else the block under its feet. `pick(host, distance)` clips `Entity.pick`'s ray from the eyes along the view
  against block outlines, no fluids, and returns the `ClipHit` (its `miss` set for a miss); entities are
  clipped by `LevelRef::ray_trace_nearest_entity` or `ray_trace_entities`, not `pick`.
  `is_multipart(host)` and `parts(host)` are `Entity.isMultipartEntity` and `getParts`:
  the ender dragon's eight `EnderDragonPart`s (not the Parts of this API), head, neck, body, three tail segments and
  two wings, each an `EntityRef` of its own entity id, resolved as any handle is. A part's `kind` is the dragon's, its position, rotation (none: zero), velocity (zero)
  and `bounding_box` are its own, `level` is the dragon's, and it is no living entity, mob or dragon handle; it
  stops resolving when its dragon does, and removing one is refused.
- `entity.nbt(host)` and `merge_nbt(host, compound)` read and change its data as `/data get` and `/data merge entity`
  do (merging works on mobs, dropped items, armor stands and item frames; never on players, as in vanilla);
  `tags`, `add_tag`, `remove_tag` work as `/tag` does, and `name` is its name as plain text.
  `set_custom_name(host, Some(text))` names it and `None` takes the name away (`Entity.setCustomName`);
  `custom_name_visible` and `set_custom_name_visible` read and change whether the name shows at any distance. Both
  writes are saved and reach the players tracking a mob or an armor stand; they return false for a player and for the
  kinds whose name Sunrise does not retain (projectiles, primed TNT, falling blocks, experience orbs, ominous item
  spawners, the dragon and crystals).
  `classification` is the type's `MobCategory`, and `added_to_level` is true from when `entity_join_level` let the
  entity in until it leaves or is removed.
- `entity.play_sound(host, sound, volume, pitch)` (`Entity.playSound`: at the entity, under its `sound_source`, for the
  players in range; a silent mob plays nothing and a player doesn't hear their own), `game_event(host, event)`
  (`Entity.gameEvent`: the entity is the source of a vibration for sculk sensors, wardens and allays),
  `broadcast_event(host, id)` (`Level.broadcastEntityEvent`: entity event `id` to the players tracking it and to
  itself, for `handleEntityEvent` to act on), `sound_source(host)` (`Entity.getSoundSource`, a `SoundSource`) and
  `spectator(host)` (`Entity.isSpectator`) work for every kind. `game_event` and `broadcast_event` act on the level in
  place and return false for a player in a parked level. There is no `sprint_particles`: vanilla spawns those on the
  client only.
- `player.stop_sound(host, sound, source)` sends `StopSoundCommand`'s packet to that player (`None` for any sound or
  source), `player.play_sound(host, sound, source, volume, pitch)` plays a sound for them only at their position, and
  `entity.play_bound_sound(host, sound, source, volume, pitch)` plays one that follows the entity for the players in
  range, as `Level.playSound(null, entity, ...)` does. `/stopsound` and `/playsound` send through the same code. They
  return false for a stale handle or a player who isn't online. Looping, fading and a handle to a playing sound need
  client handles.
- `living.health(host)` is `LivingEntity.getHealth` (Paper's `Damageable#getHealth`), `max_health` is
  `getMaxHealth` (players and guests read their attribute map; an armor stand the fixed 20 the game seeds it
  with — lava does not hurt it, and splash water reads the very field — and the dragon its 200, from their own
  fields until live attribute maps cover them) and `alive` is `isAlive`
  (Paper's `Entity#isDead` is its inverse; a dying mob, mid death animation, is not alive). During a nested
  player swap an outer player's displaced record is unavailable, and during a player's turn the other
  players' handles read `None` until the hub is back.
- `living.attribute_value(host, id)` is `AttributeMap.getValue` and `attribute_base` is `getBaseValue`, for a
  player, a guest, a mob, an armor stand and the dragon (`None` for an attribute the entity type lacks or a stale
  handle). A mob, a stand and the dragon answer from a live attribute map built from the type's
  defaults, the saved `attributes` list (bases and permanent modifiers, with their ids), and the attribute modifiers
  of the items they hold and wear, put on and taken off slot by slot in `EquipmentSlot` order as
  `LivingEntity.collectEquipmentChanges` does (transient: never saved). The map is built when first needed. Mob
  behavior still reads the type's defaults, so a modifier does not yet change how a mob moves or hits. An item's
  modifiers are the stack's own `attribute_modifiers` component or its item type's, as `item_attribute_modifier`
  leaves them. On a client the reads answer for the local player only, from the client's own copy of the synced
  instances (`ClientboundUpdateAttributes`, the type's defaults before the server sends any); every other entity
  reads `None` there, the client keeping no attribute map of another entity's, only the few fields it draws
  (`handleUpdateAttributes` for a mob extracts its maximum health and scale for the renderer).
- `living.attributes(host)` lists the ids of the attributes the entity's type has (the holder's attribute set the
  value and base reads answer from), in registry order. `living.attribute_modifiers(host, id)` is
  `AttributeInstance.getModifiers`: the attribute's modifiers in the order they were added — a mod's transient and
  permanent ones, the saved list's, and what the worn and held items grant. An attribute no modifier is on answers
  an empty list; `None` for an attribute the entity type lacks, a stale handle, or on a client for any entity but
  the local player. Diverges from upstream: new helpers.
- The writes take players, guests (in their own turn), mobs and armor stands, and answer false for an attribute the
  type lacks, the dragon (`/attribute` refuses it, and nothing reads its attributes yet), a stale handle, or on a
  client. `living.add_attribute_modifier(host, id, modifier, lifetime)` is `AttributeInstance.addTransientModifier`
  or `addPermanentModifier` (`ModifierLifetime`), refused when the attribute already has a modifier with that id
  (vanilla throws) or the amount is not finite. `replace_attribute_modifier` puts the new one in place of the old
  (`addOrUpdateTransientModifier`, `addOrReplacePermanentModifier`), and the new lifetime wins.
  `remove_attribute_modifier(host, id, modifier_id)` is `removeModifier` (false when there was none), and
  `set_attribute_base(host, id, base)` is `setBaseValue`. `AttributeModifier` holds the modifier's id, amount and
  `AttributeOperation`. A permanent modifier and a base are saved with the entity; a transient modifier lasts until it
  unloads. The writes take the one path `/attribute` takes, so they go where the game and the attribute sync read:
  - a player's own attribute map;
  - for a mob, the store of the nine attributes its behavior uses (max health, movement and flying speed, follow
    range, attack damage, knockback resistance, jump strength, burning time, reinforcements), so it moves and hits by
    them at once; its armor map for armor and toughness, which its damage reads; and for its other attributes, as
    for an armor stand, the generic map `/attribute` reads and the sync sends.

  The live map behind `attribute_value` takes the same write, so the reads agree with `/attribute`, and the entity's
  save keeps it. A lower max health cuts a mob's health (`LivingEntity.onAttributeUpdated`).
- `item_attribute_modifier` (`ItemAttributeModifierEvent`) edits what one stack grants: `ItemAttributeModifiers`
  holds `ItemAttributeModifier` entries (the attribute's id, an `AttributeModifier`, an `EquipmentSlotGroup`) and
  edits them as NeoForge's event does: `add` (`addModifier`, refused when the attribute has that id already),
  `replace`, `remove`, `retain` (`removeIf`), `remove_all_for` and `clear`. Every reader of a stack's modifiers asks
  through one place (`sunrise_client::item_attributes`):
  - the equipment of a player, a mob, an armor stand and the dragon, when it changes;
  - mobs' gear knockback resistance and their pickup choices;
  - the attribute sync;
  - both tooltips.

  A worn stack is asked again only when it changes (`ItemStack.matches`), so the answer must be the same for the
  same stack. With no mod listening, or none changing a stack's list, nothing is copied. A player's attack speed
  still reads its held item type's `attack_speed`, not the stack's (see #549).
- `get_enchantment_level` (`GetEnchantmentLevelEvent`) edits the enchantments gameplay sees on a stack:
  `ItemEnchantments` is NeoForge's `ItemEnchantments.Mutable` (`level`, `set`, `upgrade`, `retain` for `removeIf`,
  levels at most 255), and `target` is the one enchantment a read asks for, `None` when every one is read. The
  game's reads follow NeoForge's split:
  - through the hook: `ItemStack::enchantment_level` (`getEnchantmentLevel`, every gameplay level read: Looting,
    Fortune and the other loot functions, Respiration, Silk Touch, Mending, Thorns, the trident's and crossbow's
    enchantments, fishing), and `ItemStack::all_enchantments` (`getAllEnchantments`: every enchantment effect
    component, the enchantments' attribute modifiers, the decorated pot's `hasTag` check, the killer's gear for
    entity loot);
  - the stack's own component: `ItemStack::enchantments` and `tag_enchantment_level` (`getTagEnchantmentLevel`)
    for tooltips, the anvil, grindstone and enchanting table, item predicates, saves, the network, and
    `EnchantmentHelper.has` (Curse of Binding), which this hook doesn't change.

  It is asked on every such read, many times a tick, so keep a listener cheap. With no mod listening nothing runs;
  with one, ids are parsed once and a read the mods leave alone allocates nothing.
- `living.set_health(host, health)` is `LivingEntity.setHealth` (Paper's `Damageable#setHealth`), cut to the
  range from zero to the maximum, with no hook; a non-finite value is refused. This raw write can set zero
  or change a dead entity's health without running the death pipeline.
  `living.heal(host, amount)` is `LivingEntity.heal`, through the `living_heal` hook: a signed amount changes
  health only when current health is above zero. Both write players, guests
  (in their own turn) and mobs, and answer false for armor stands and the dragon.
  `living.last_attacker(host)` is `LivingEntity.getLastHurtByMob`: who last hurt the entity, within a hundred
  ticks of the hurt, as an `EntityRef` of the current open; `None` once the window has passed or the attacker
  is gone, and for armor stands, the dragon and clients. A mob attacker that changed dimension is found in its
  loaded level under the id it arrived with, never another entity holding its UUID.
  `living.kill_credit(host)` is `LivingEntity.getKillCredit`: it returns the remembered player first, or the
  last hurting mob if there is no remembered player. A remembered player that cannot be resolved gives `None`,
  not the mob. Both resolve across loaded dimensions; a credited mob there answers only `kind`, `uuid`, `level`.
- An `EntityRef` holds the world's open (`WorldEpoch`) and the entity's id, which no other entity of that open
  gets. A handle of an earlier open, of an entity that is gone or moved to another level (it gets a new id there, as in
  vanilla), or used on a client reads as `None`.
- The entities are those of the level in place (the one a hook runs in) and the server's players, whatever their level.
- `host.block_entity(level, pos)` finds a block entity in a loaded chunk, including in parked levels;
  `kind`, `position`, `level` and `data` read it. A removed or temporarily taken-out entity reads `None`;
  a replacement at the same position resolves to the new entity, unlike vanilla's object identity.
- `player.open_menu(host, block_entity)` opens its menu on the server and returns true only when opened. It returns
  false for stale or wrong-level handles, non-providers, cancelled opens, and players without a connection (including
  fake players), since no connection can receive the menu.
- `block_entity.viewers(host)` lists the players with its container open on the server, read from their open menus
  when called: players in its level, not spectators or fake players, a double chest's menu on both halves. `None` for
  a stale handle or a client.
- `level.block_shape(host, state, pos, kind)` reads a supplied state's block-local outline or collision boxes with an empty context, without changing the world's state; pass `level.block_state(host, pos)` for the stored state.
  `None` for a client, unknown state, stale handle or closed level; parked levels can be read.
- `level.visit_entities_in(host, min, max, &mut visit)` visits readable entities whose boxes intersect the given box
  in the active server level or the client's displayed level. The visitor can read them through the same host.
  `ControlFlow::Break(())` stops the visit; `Continue(())` keeps it going. An empty query completes successfully,
  whereas a stale or unavailable level returns `LevelReadError` before the visitor runs.
- `level.visit_all_entities(host, &mut visit)` visits the active server level's lookup lazily in admission order
  (`ServerLevel.getAllEntities`), with no box filter, skipping any member that cannot resolve and stopping
  immediately on `ControlFlow::Break(())`.
- `level.pop_resource(host, pos, stack)` drops a stack as a broken block does (`Block.popResource`), `spawn_item(host, x, y, z,
  stack, pickup_delay)` and `spawn_item_with_motion` spawn a plain item entity and give its handle, with the pickup delay
  vanilla's `ItemEntity.setPickupDelay` sets at construction (10 a block's drop, 40 a player's throw, 32767 never), and
  `award_experience(host, x, y, z,
  amount)` splits experience into orbs (`ExperienceOrb.award`), all in the level in place.

`level.set_block(host, pos, state, flags)` and `level.remove_block(host, pos, moved)` change the level in place. They return false for a stale handle, client, parked level, unloaded chunk or out-of-height position; parked writes wait for a safe level swap. Removal preserves the fluid's legacy block; `moved` marks a piston move. `BlockUpdateFlags` combines supported vanilla update flags with `|`. `CLIENTS` sends the update, while `INVISIBLE` suppresses notification only on the client, as in `Level.setBlock`. Skipping block-entity side effects suppresses container-content drops when replacing a chest; without it, the contents spill.

`level.destroy_block(host, pos, drop, tool, collect)` uses that same level's destruction and loot path.
`tool: None` means an empty hand. With `collect`, the returned stacks are not spawned as item entities;
without it, a successful break returns an empty list after spawning them. The normal block-drop hook
still runs once before either outcome; tool enchantments and vanilla random ordering are unchanged.
`drop: false` suppresses resources. `None` means nothing broke, including an invalid write or air.
`level.set_break_progress(host, breaker, pos, stage)` shows stages 0..9 or clears the crack otherwise;
a machine uses a stable negative breaker id.

A helper whose catalog entry names a `receiver` (`entity`, `level`, `block_entity`) is called on that handle (for example `entity.position(host)`), rather than as an ID-based `Host` method.

### Client menu input

Client input uses `menu.click(host, &[mod_api::MenuClick::QuickMove { slot }])` on a mutable `ClientHost`.
Actions use menu-order slots; only `Pickup` accepts `None` for outside. Typed `MouseButton`,
`SwapWith` and `DragKind` values replace vanilla's wire codes, and `Drag { kind, slots }` expands
to a complete gesture. All actions are validated before the first is predicted or sent through the
normal `ContainerClick` path. A stale handle, invalid slot/hotbar index, creative-only action in survival,
spectator, non-client host or creative inventory screen returns `false` without side effects.
`true` is submission, not server acceptance; authoritative slot/cursor corrections still win.
`menu.close(host)` closes the menu's open screen as Escape does (`AbstractContainerScreen.onClose`,
`LocalPlayer.closeContainer`, which sends `ServerboundContainerClosePacket`), and `menu.click_button(host, button)`
sends `ServerboundContainerButtonClickPacket`. Both return `false` for a stale handle or a non-client host; `close` also
while the menu's screen is not open, and `click_button` for the inventory menu.
See [A menu's slots](../concepts/capabilities/menu-slots.md) for actions and an example.
Diverges from upstream: new helpers.

### Mob AI goals

`crates/sunrise-client/src/game/modding/native_mob_goals.rs` (#29's "movement and goals"): a mob's AI is vanilla's two goal
selectors, and a `MobRef` reads and changes them. `mob.goals(host)` and `mob.target_goals(host)` list each selector's
goals as a `MobGoalEntry` — the priority (a lower number preempts a higher one), the kind id (`melee_attack`) and
whether the goal runs now. `mob.add_goal(host, priority, &goal)` is `GoalSelector.addGoal`: the `GoalSpec` names one of
the vanilla goal kinds Sunrise itself runs, with the vanilla constructor's parameters
(`MeleeAttackGoal(speed, followUnseen)`), and `mob.add_target_goal(host, priority, &goal)` adds to the target selector.
`mob.remove_goal(host, id)` and `mob.remove_target_goal(host, id)` are `removeGoal`: every goal of that kind goes, a
running one stopped first, and the count comes back; as vanilla's `removeAllGoals` with a `getClass()` predicate, a
species variant of a vanilla goal (a bee's `BeeHurtByOtherGoal`, a labeled stroll) keeps its own name, is read under it
and is removed by it.

The kinds, by their id: `float`, `climb_on_top_of_powder_snow`, `panic` (with `water_range` for the turtle's),
`stroll` (`water_avoiding`, `swim` and `flying` pick RandomStroll's variants), `look_at_player`, `random_look_around`,
`melee_attack`, `leap_at_target`, `avoid_entity`, `tempt` (item ids, `minecraft:wheat`), `breed`, `follow_parent`,
`follow_mob`, `restrict_sun`, `flee_sun`, `eat_block`, `breath_air`, `try_find_water`, and the target selector's
`nearest_target` (with `prey` and `interval`) and `hurt_by_target` (with `alert`). Species ids in `prey`, `what` and
`only` are namespaced (`minecraft:zombie`); a bare id reads as `minecraft:`'s.

An add refuses what the mob's vanilla class does not hold: a `PathfinderMob` goal for a bat, ghast or phantom,
and `breed` / `follow_parent` for a mob that is no animal. Slimes, magma cubes and shulkers are `PathfinderMob`s.
`follow_mob` accepts ground or flying navigation, but not water-bound or amphibious navigation. It also refuses
non-finite numbers, negative speeds and distances, a probability outside 0..=1, a
negative interval, an empty `items` list, and a species id no mob type carries. Reads and removes answer `None`/`false`
for a stale handle, a client, the dragon, and a mob whose copy is held out — nothing in a mob's own tick resolves, as
`Host` is not there yet: change goals in `entity_join`, which fires for a loaded chunk's mobs too. Goals are not saved,
as in vanilla: after a reload the mob has its species' set again, and a mod's `entity_join` adds its own back.

These methods reuse built-in goal kinds; they do not define arbitrary goal callbacks or run at construction's
`Mob.registerGoals` moment. A typed goal registrar and callbacks on `MobRef` and Parts are not part
of this API yet. NeoForge has no bus event at this moment. The upstream raw-`Mob`
`register_goals` interface is not part of this fork's API.

### Mob AI controls

`crates/sunrise-client/src/game/modding/native_mob_refs.rs` and `native_mob_navigation.rs` (#938's mob AI controls): a `MobRef`
steers a mob through the engine's own controls, each a momentary wish the mob's own AI may replace next tick, as
`move_to`'s path is. Reads take the host by `&dyn Host` and writes by `&mut dyn ServerHost`.

- `mob.look_at(host, [x, y, z])` is `LookControl.setLookAt`: the mob's look control rotates its head toward the position
  over the next ticks. Non-finite coordinates are refused. `mob.jump(host)` is `JumpControl.jump`: the mob jumps on its
  next own tick by its own jump path — no velocity is written. No species refuses either: vanilla's brains themselves
  write through these controls (a brain species' `LookAtTargetSink` calls the same `lookAt`), so a mod's wish and the
  mob's own are one flag, not a brain memory a write could miss (which is why `set_target` refuses brain species, and `set_brain_target` writes their `ATTACK_TARGET` memory instead). A
  creaking being looked at, an angry bee and an axolotl playing dead hold their look control, and a held creaking its
  jump control, as in vanilla: the wish waits until the control runs again.
- `mob.navigation_speed(host)` / `set_navigation_speed(host, speed)` read and set `PathNavigation.speedModifier`, the
  multiplier the current path is followed at (1.0 is the mob's normal speed; 0 until a path sets one) — not the mob's
  movement-speed attribute. `move_to`'s speed argument sets the same modifier, and the mob's own next `moveTo` (a goal's
  or a brain species') sets its own again, as in vanilla. Non-finite speeds are refused.
- `mob.restrict_to(host, [x, y, z], radius)` is `Mob.restrictTo(pos, radius)`: the home area the mob keeps to — its own
  stroll and wander goals pick their positions inside it, as a lead's and a guardian's home do. It is saved with the mob
  (`home_pos`, `home_radius`). `restrict_center(host)` (`getRestrictCenter`; the last live one after a clear, `[0, 0, 0]`
  for a mob that never had one), `restrict_radius(host)` (`getRestrictRadius`; `-1` without a restriction),
  `has_restriction(host)` (`hasRestriction`), `clear_restriction(host)` (`clearRestriction`; the center stays) and
  `is_within_restriction(host, pos)` (`isWithinRestriction`; true everywhere without a restriction) read and change it.
  Radius `-1` in `restrict_to` clears the restriction while keeping the position as the center.
- `mob.left_handed(host)` / `set_left_handed` (`Mob.setLeftHanded`, the mob flags in its entity data, saved as
  `LeftHanded`) and `can_pick_up_loot` / `set_can_pick_up_loot` (`Mob.setCanPickUpLoot`, saved as `CanPickUpLoot`;
  an allay and a sulfur cube decide for themselves) moved here from the flat `living_*` helpers of #938's first round:
  vanilla keeps both on `Mob`, so `MobRef` is their home. The flat helpers are gone, with no aliases.
- All of these answer `None`/`false` for a stale handle, a client, the dragon (no `Mob` state), and a mob whose copy is
  held out of the level.

### Mobs drawn by a mod

`crates/sunrise-client/src/game/modding/native_mob_draw.rs`: `render_living_pre(entity, draw, model, canceled)` (client) is asked
once a frame for every mob and player in view, before its model (NeoForge's `RenderLivingEvent.Pre`). Set `canceled` and nothing
more of its own drawing happens this frame: model, layers, name tag, leash, outline and `render_living_post`. Its flames and shadow still draw (#1285).

- `event.model` borrows the selected body's hierarchy in bind pose. To leave out a part and
  its descendants after animation, hide its model-local index for this draw only:

  ```rust,ignore
  if let Some(index) = event.model.part("nose").map(|part| part.index) {
      event.model.hide(index);
  }
  ```

  A missing name doesn't refuse the draw. Layers using the same body respect the hide;
  independent models (armour, held items, the cape) are unaffected.
- `entity` (`MobRender`) is the frame's `LivingEntityRenderState`: the id, kind, custom name and `baby`, the partial tick,
  `age` (ticks lived plus the partial tick), `body_yaw`, `head_yaw` (from the body) and `pitch` in degrees, the walk cycle
  (`walk_position`, `walk_speed`: a vanilla quadruped's legs swing `cos(walk_position * 0.6662) * 1.4 * walk_speed` radians), `hurt_time`,
  `death_time` and `on_ground`.
- `draw` is a block entity's frame (`block_entity_submit`): draw with `native_draw_cube`, `native_draw_quad`,
  `native_draw_block` and `native_draw_item` under its pose stack, which starts at the mob's feet turned with its body: +z
  is the way it faces, +x its left, +y up, in blocks. Turn a head or swing a leg with `rotate_about` and a pivot. It holds
  the mob's light.
- A mod's own creature is an ordinary mob (`native_spawn_mob`) drawn this way: a quiet villager, an allay or a bat
  gives it movement, sounds and saving, and the mod gives it a model and animations of its own. Tell yours apart by the
  ids the server half sends with mod messages, or by its custom name.
- Players wrap the living hooks with `render_player_pre(player, who, draw, model, canceled)` (`RenderPlayerEvent.Pre`)
  before the living pre, and `render_player_post(player, who, model, draw)` after the living post, once a frame for each
  other player in view and for this window's player in third person when it holds the camera, with the same frame and
  `who` (the profile name, and `local`). The pair stays at the player's join slot, interleaved with general entity
  renderer submissions. Living Pre sees edits already made in Player Pre. Canceling Player Pre skips Living Pre and both
  Post hooks; canceling only Living Pre still calls `render_player_post` with `model: None`.
- `render_living_post(entity, model, draw)` (`RenderLivingEvent.Post`; a type's own layers are `register_entity_layers`)
  adds after the entity's model and layers, in the same frame. Use the optional final `event.model`
  to find a part by name or follow the parent, child and sibling links with `part_at(index)`. Read
  `part.pose()` and check `part.visible`, which includes its ancestors. A successful replacement
  renderer supplies no body pose to the layers or Post. Don't keep a model view or index after the callback.

### Advancement triggers

`crates/sunrise-client/src/game/modding/native_criteria.rs`: a mod's own criteria triggers, as a NeoForge mod registers a
`CriterionTrigger` and calls its `SimpleCriterionTrigger.trigger`. The advancement is an ordinary file of a content mod
or a world's data pack (`data/mymod/advancement/connect_a_pipe.json`) whose criterion names the mod's trigger and
whatever conditions the mod reads:

```json
{
  "criteria": {
    "copper_pipe": {
      "trigger": "mymod:pipe_connected",
      "conditions": {"pipe": {"items": "#c:ingots/copper"}}
    }
  }
}
```

- `player.trigger_criterion(host, &trigger, test)` fires it on a `PlayerRef`, with a mutable `ServerHost`,
  a mod-owned `Identifier` such as `mymod:pipe_connected`, and an `Arc<dyn CriterionTest>`. Each criterion with that
  trigger the player hasn't been granted, of an advancement not done yet, goes to `test`, then to its `player`
  predicate, as vanilla tests one. Those that pass are granted; an advancement they complete is awarded as by the game's
  own triggers: the toast, the chat line, the rewards and the player's saved progress.
- `CriterionTest::test(&self, criterion: &dyn CriterionContext)` returns `Result<bool, CriterionTestError>`.
  The context is a scoped game resource, borrowed only for this test; neither it nor its private world can be retained.
  `criterion.conditions_json()` gives owned JSON (`"null"` when conditions are absent).
  `criterion.item(key, &stack)` tests the item predicate under `key` as the game's own triggers read one: `items`
  (an id, a list or a `#tag`), `count`, the `potion_contents` and `damage` components, and the `enchantments` and
  `damage` predicates. Other components and predicates aren't checked. A missing key passes. Tag membership is
  evaluated in the target player's world, not from a detached conditions snapshot.
- It reaches the acting player inside their hook, and another player on the server in their own level, including
  during a guest's turn. A displaced outer player is unavailable. `Ok(())` means the player was reached, not that
  anything was granted. `TriggerCriterionError` distinguishes an invalid trigger (`minecraft:` is reserved; empty
  namespace/path is refused), a remote client, an unavailable player, a missing game backend, and a failed test.
  A failed test stops further tests; earlier matching criteria may already be granted. The test itself is not retained.
- Both resource interfaces and their glue are generated from the traits. Predicates run synchronously with the
  criterion context, which expires on return. `CriterionTestError::Unavailable` propagates as
  `TriggerCriterionError::Test` rather than treating the criterion as a nonmatch.

Diverges from upstream: this replaces the unscoped `Host` closure helper with a player receiver and generated
criterion resources; the private advancement world remains solely in the native implementation.

### Explosions

`crates/sunrise-client/src/game/modding/native_explosions.rs`: for server mods, in the level in place; the full contract,
with the causing-entity table, is the book's "Setting off an explosion" (how-to/world/block-experience-and-explosions.md).

- `LevelRef::explode(host, source, center, radius, fire, interaction, settings)` is vanilla's full `Level.explode`. The
  causing entity comes from `source`, never the caller (a primed TNT's owner, a living entity itself, a projectile's
  living owner); with both the damage is `minecraft:player_explosion`. `interaction` is `None`, `Block`, `Mob`, `Tnt` or
  `Trigger`; `fire` works with any. `ExplosionSettings` names a damage type, an `ExplosionCalculator` (blocks, entities,
  knockback, an immune tag that replaces the resistance rule), particles and sound, each `None` vanilla's. No radius
  limit. Refused with an `ExplodeError`, doing nothing; the book lists when.
- A blast set off inside `explosion_detonate` runs that hook again, so a mod that chains blasts keeps its own guard.

### Camera shake

`camera_shake(partial, shake)` (client) asks every frame for the camera shake a mod wants: `pitch`, `yaw` and `roll`
in degrees and an `offset` in blocks along the camera's right, up and back. Add to `shake` rather than set it, so two
mods' shakes add up. Sunrise reads it only while playing, scales the sum by Distortion Effects (a player who turns
that to 0% sees no shake), bounds it to 10 degrees and half a block on each axis, and multiplies it into the view
after the hurt tilt and view bobbing. A mod keeps its own shake's time and decay, from `partial` and its tick hooks;
it moves only what the player sees, not where they are or aim.

### A player's motion

`crates/sunrise-client/src/game/modding/native_motion.rs`: a push on a client.

- `native_push_player([x, y, z])` (client) adds a push to the player's own velocity, in blocks a tick, as an
  explosion's knockback does: the player's own game moves them. The server didn't send it, so a server's movement checks
  may see a move they can't explain; for a push the server should stand behind, have the server send it. Each component is
  cut to 4 either way; an upward push lifts the player off the ground. A spectator, no player, or a value that isn't
  a number is left alone. A server mod that wants a client pushed sends its client half a message.

### Values for a mod's shader pack

`shader_params(partial, params)` (client) asks every frame, while a Sunrise shader pack is on, for eight
`[f32; 4]` that every stage of the pack reads as `frame.mods` (`array<vec4<f32>, 8>`, docs/api/shaders-reference.md). What
they mean is the mod's own: a mod that bundles its pack as `native-<mod id>` writes the slots that pack reads, such as an
explosion's centre (camera-relative, from `frame.camera_position`) and its age in seconds, for a cloud or a shock wave
ray marched in a post pass against `scene_depth`. A value that isn't finite reads as 0, and all are 0 when no mod sets
them. The mod's server half can send what happened to each client half with mod messages.

### Biome colours

`crates/sunrise-client/src/game/modding/biome_colors.rs`: the grass, leaf and water colours a client mod changes while the
game runs, for seasons (a golden autumn, a pale winter) or a world that greens as it heals.

- `biome_colors(biome, colors)` (client) changes a biome's `BiomeColors`: `grass`, `foliage` (leaves, vines),
  `dry_foliage` (leaf litter) and `water`, 0xRRGGBB, starting from the biome file and the colormaps. The dark forest's
  modifier still darkens the grass set here; the swamp's noise colours replace it.
- It is asked for every biome when a mod calls `native_refresh_biome_colors()`. When the colours differ from those in use,
  every loaded section and the far terrain are rebuilt with them, as changing Biome Blend does. Call it when your colours
  change and once when the player joins, not every tick: a rebuild takes the workers a few seconds. A slow fade is a
  call a day, not a frame.
- Block-breaking and falling-leaf particles keep the biome's own colours.
- A season's mood in a shader pack (warmer grading, a lower sun, more haze) comes from the same mod through
  `shader_params` (`frame.mods`); its server half can send the season to each client with mod messages.

### Sounds a client mod plays

`ClientHost::play_local_sound(sound, after)` (client) plays `sound`, a `SoundPlay`, for this player only. Nothing is sent
to the server, and other players don't hear it. It takes `&self`, so the screen, key and mouse hooks, where interface
sounds belong, can call it: the sound is queued and starts at the next audio update (once a client tick), `after` ticks
counted from then (0 to 1200; `SoundEngine.playDelayed`). The answer is exact: the checks run at the call.
- `SoundPlay::ui(id)` is an interface sound (`SimpleSoundInstance.forUI`): at the listener, on the `Ui` source, not
  fading with distance, as the game's own button clicks play.
- `SoundPlay::at(id, source, position)` plays at a place in the client's level, fading with distance. A volume past 1
  carries farther, as vanilla's attenuation does.
- `with_volume`, `with_pitch` and `looping(delay)` (repeating, `delay` ticks between repeats: vanilla's
  `SoundInstance.getDelay`) change either.

It's the same record the `play_sound` hook edits, and it goes through the same mapping into the engine. The event is
vanilla's (`minecraft:entity.generic.explode`) or the mod's own: a content mod's `assets/<id>/sounds.json` and
`assets/<id>/sounds/*.ogg` load with the rest of its assets. A client half that hears from its server half (mod
messages) where something happened can time the sound to the distance, such as a blast heard when its shock wave reaches
the player: `after = distance / speed`. It answers `None` on a server and for a value out of range (an event id that
isn't namespaced, a position or volume that isn't finite, a negative volume, pitch or repeat gap, `after` past 1200);
an event the resources lack plays nothing. Diverges from upstream: horizon-rust/horizon's `native_play_local_sound` takes
`(sound, at, volume, pitch, delay)`.

Otherwise it answers a `SoundRef`, a handle on this one play, never reused. Its methods take the `&dyn ClientHost`, so
any client hook can call them. Like the play, the writes are queued for the next audio update, while the answers are
computed at the call:
- `sound.state(host)` is `Some(SoundState)`: `Waiting` (queued, waiting out `after`, or between a looping sound's
  repeats), `Playing` (`SoundEngine.isActive`) or `Stopped`. It is `None` on a server.
- `sound.stop(host)` stops it for good: a waiting sound never starts and a looping one never repeats. A stop asked
  since the last update already reads `Stopped`.
- `sound.fade(host, volume, ticks)` moves its volume to `volume` (at or above 0) in equal steps over the next `ticks`
  unpaused ticks it plays, or at once for 0. A fade to 0 stops it when the fade ends, as `LoopSoundInstance.fadeOut`
  does. A `SoundPlay` at volume 0 faded up fades in, and a later fade replaces one in progress. While the game is
  paused the fade waits, as vanilla's tickable sounds do (`SoundEngine.tick(paused)`): an interface or music sound
  keeps playing at the volume it had (`pauseAllExcept(MUSIC, UI)`), and any other sound pauses with the game.

`stop` and `fade` answer false, changing nothing, once the sound is over. Once it ends, is stopped or faded out, or is
cleared (leaving or changing levels stops every sound), the handle reads `Stopped` and its writes do nothing. It stays
valid across a change of output device, which reopens the output on the same mixer and keeps the sounds playing.
Dropping or overwriting the handle doesn't stop the sound: keep a looping sound's handle while it plays (stop it before
starting another in its place), or nothing can stop it until the player leaves the level.

`ClientHost::sound_events()` lists every sound event the client's resources have (`SoundManager.getAvailableSounds`).
`audio_devices()` lists the output devices by name (`SoundEngine.getAvailableSoundDevices`), and `audio_device()` names
the one the options ask for, empty for the system's default. Both lists are built anew at each call, and listing the
devices probes each one: ask from setup or a settings screen, not every frame.

### Falling blocks

`crates/sunrise-client/src/game/modding/native_falling.rs`: a helper for server mods, for the level in place, on a server only. It
names the level it means by its `dimension` (`minecraft:overworld`), as `native_set_block` does.

- `native_spawn_falling_block(dimension, state, pos, velocity, falling)` throws a falling block of `state` from `pos` (its
  bottom centre) at `velocity` in blocks a tick, each axis cut to 3.9. It flies, lands and places itself as sand does, or
  drops as an item where it can't be placed. `Falling` holds `drop_item` (default true) and `hurts` (hurts what it
  lands on as an anvil does, 2 a block fallen and 40 at most; default false).
- The block at `pos` stays: take it away first with `native_set_block` to make a block fall out of the world, as an
  explosion's debris or a collapsing tower would.
- It answers false, throwing nothing, on a client, for another dimension, air or an unknown state, a chunk that isn't
  loaded, a value that isn't finite, or with 1024 falling blocks in the level already.

### Bulk block edits

`crates/sunrise-client/src/game/modding/native_world_edit.rs`: a helper for server mods, for the level in place, on a server only.
It names the level it means by its `dimension` (`minecraft:overworld`), as `native_set_block` does.

- `native_edit_blocks(dimension, edit)` makes many blocks at once: `BlockEdit` holds a `palette` of block states and
  a list of typed `StatePredicate` filters (exact blocks or tags), and `runs`, each
  `height` blocks up column (`x`, `z`) from `y`, set to palette entry `block`, only where filter `only` passes the block
  there. With `surface` (`Heightmap`: `WorldSurface`, `MotionBlocking`, `MotionBlockingNoLeaves` or `OceanFloor`),
  `y` counts from that height of the column, read once a column before anything changes. Runs are made in order, each
  seeing the changes of those before it.
- The blocks go straight into the chunks, a section packed once however many of its blocks change, as `fill ... strict`
  does: no neighbour is told, nothing drops, and no block updates or ticks; light is worked out again for the chunks
  changed, and they go to the players whole. A block replaced by another loses its block entity, and a new block gets
  the one `native_set_block` would give it (a mod block's own, included).
- It answers the number of blocks changed. Runs in chunks that aren't loaded, and the parts of runs outside the level's
  height, are left out. It answers `Err`, changing nothing, on a client, for another dimension, a palette entry or filter
  or predicate that fails validation (NBT is not representable), a run naming a palette entry or filter past
  the end, or more than 65536 filters.

### Column reads

`crates/sunrise-client/src/game/modding/native_world_columns.rs`: helpers for server mods, for the level in place, on a server
only, that read many columns in one call. They name the level they mean by its `dimension` (`minecraft:overworld`), as
`native_set_block` does, and answer `Err` with the reason, reading nothing, on a client, for another dimension or for
a read too big, so a mod can tell "ask for less" from "wrong level". They run on the server tick: read a big area in
pieces.

- `native_surface_heights(dimension, columns, heightmap)` answers each column's height on `heightmap` (`Heightmap`,
  as `native_edit_blocks` reads it: one above the highest block that counts), `None` for a column whose chunk isn't
  loaded; at most 65,536 columns a call.
- `native_read_columns(dimension, columns, heightmap, (below, above))` reads, for each column, the blocks from `below`
  under its height to `above` - 1 over it, bottom first: `ColumnBlocks` holds the heights (`bases`), the block
  complete `BlockState` values seen (`palette`), the blocks as indexes into it, and each column's biome JSON water
  colour (`water_colors`, `0xRRGGBB`, at the column's top). Blocks outside the level's height, and every block of a
  column not loaded, read as air (its water colour as 0). `below + above` must be 1 to the level's height, and the
  read at most 1,048,576 blocks. Read blended grass and foliage tints on the client with
  `BlockAndTintGetter::block_tint` inside tint callbacks or `ClientHost::biome_tint` in other hooks.

### Chat and commands a client mod sends

`ClientHost::send_chat(message)` sends what the player could type in chat, to any server, a vanilla one included:
`ClientPacketListener.sendChat` and `sendCommand`. A leading `/` sends the rest as a command, signed where its
arguments are messages; anything else goes as signed chat. The message is trimmed first, as the chat screen trims it.
It fires no `client_chat` (vanilla's `ClientChatEvent` comes only from the chat screen) and isn't added to the chat
history. It is an error when the player isn't in a server's play state (not connected, not joined yet or being
reconfigured), when the trimmed message is empty, and when the Chat option blocks it: Commands Only blocks chat,
Hidden blocks chat and commands. The server's own spam limits apply as they do to typed chat.

### Server menus

`crates/sunrise-client/src/game/modding/native_menus.rs`: menus a server mod builds from ordinary items, which a
stock client shows as a chest (`generic_9x1` to `generic_9x6`); nothing is installed on the player's side.

- `host.open_menu(uuid, rows, title, items, replace)` opens one in place of the player's open menu (its window id, a new one each time, as `ServerPlayer.openMenu` counts them); the old menu closes on the server as a close does, its cursor stack back to the inventory, and its window's packets are ignored. `MenuReplace::CloseFirst` first sends the client a close, vanilla's default; `MenuReplace::InPlace` doesn't (`MenuProvider.shouldTriggerClientSideContainerClosingOnOpen`), so the title and rows change without the mouse centring again. A title changes only this way. `host.close_menu(uuid)` closes it.
  `host.menu(uuid, window)` is the open one's `MenuRef` (None for another window or player): its slots are [menu slots](../concepts/capabilities/menu-slots.md), written in a transaction, and the tick's container sync sends each changed one (`ContainerSetSlot`). Change its items that way, not by opening it again.
- A display menu's default click moves nothing. `MenuPart::menu_clicked` v3 runs on parts registered for the
  menu type. It shares one transaction with the default click unless the part handles the click and returns
  `Flow::Cancel`. An override or refusal sends the committed menu, cursor and any omitted swap target.
- `menu_closed` v2 names the player and window and passes any opening hand and detached owned slot values.
- A trade on these menus leaves the offered stacks in their owners' inventories until both sides are settled. Then,
  in one `host.transaction`, each offered inventory index is checked to still hold exactly what was offered, taken
  out of the player's [`ENTITY_ITEMS`](../concepts/capabilities/entity-items.md) handler and put into the other's with `insert`
  ([the recipe](../concepts/capabilities/entity-items.md)). A changed offer or no room returns an error, and every move is put back.

### Render functions

`crates/sunrise-mod-render/src/render.rs`: native-only, unstable raw frame places, outside `mod_api`.
A native mod replaces one with a `HEAD` mixin at `sunrise_mod_render::render::<name>`. They run on the render thread with the frame's wgpu objects;
what a mod draws there shows in that frame. Draw contexts remain the API-typed drawing route.

- `world_drawn(world)`: the world is drawn (sky, terrain, entities, particles, weather and the world post pass),
  the first-person hand, the HUD and screens not yet. `WorldDrawn` holds the device, queue and encoder, the frame's
  colour target (copy it out before sampling it), the world's reversed depth and its size, the camera's position and
  its matrices with and without TAA's jitter, the frame's camera-relative entity quads with their runs by entity, the
  entity atlas, the shared quad index buffer and the block sprite arrays ([block textures](#block-textures)). Not
  called on the title screen's panorama. `motion` says which mob, player or dropped item each run of the entity quads
  is, by its entity id, so a mod can match an entity's vertices with last frame's (a motion blur's own motion
  vectors); `spans` only name the type.
- `hand_drawn(hand)`: the first-person hand and held item are drawn, after `world_drawn` and before
  entity outlines, post effects, screen effects and the HUD, on every frame `world_drawn` runs. `HandDrawn`
  holds the device, queue and encoder, the frame's colour target with the hand in it, the hand's camera-relative
  quads (empty when no hand drew: third person, F1, a shader pack's own hand), the hand's clip matrix (its own field
  of view and the view bob, no jitter), the camera, the entity atlas and the quad index buffer. The hand's depth isn't
  kept: draw its quads into a depth of your own. A motion blur can match the hand's quads with last frame's for the
  swing, attack, equip and bob motion, and blur the picture with the hand in it.
- `render_needs(needs)`: set `entity_spans` to have the entity geometry split into runs by entity without a
  shader pack (each run's id is stable from frame to frame: a hash of the entity type's name, a block entity's state),
  and `world_depth` to keep the world pass's depth when nothing else needs it. Set what you need and leave the rest:
  several mods can ask. It's asked as each frame is built, so keep it cheap.

### Block textures

`crates/sunrise-client/src/game/modding/native_block_sprites.rs` and `WorldDrawn::block_sprites`: for a client mod drawing
blocks in its own pass (flying debris as voxels, say) with the player's own block textures, resource packs included,
as the terrain samples them.

- `WorldDrawn::block_sprites` (`BlockSpriteArrays`) holds the block sprite arrays as `D2Array` views: the terrain's
  eight, a 1-layer stand-in past the ones in use, each with the mip levels the Mipmap Levels option leaves, animated
  layers at the frame shown. Each texture is a whole square layer of `size` texels, its UVs 0 to 1 across it,
  gamma-encoded as the world uses it. `split` layers go to an array (2048 on most GPUs, fewer where the GPU holds
  fewer): every vanilla texture is usually in array 0. `nearest` samples texels sharp (nearest, repeating, the
  nearest mip level), `filtered` is the terrain's own sampler.
- `native_block_sprites(blocks)` says, for each block (`minecraft:grass_block`, `oak_log[axis=x]`; properties left
  out take the default), what its model draws on each side, in `Direction` order (down, up, north, south, west,
  east): its faces in the order drawn, each with its `sprite` (`array`, `layer`, `animated`, `cutout`), its `rect`
  on the side and `inset` from the side's plane (a whole face is 0, 0, 1, 1 and 0), its `uv` map, its `tint` source
  and whether it is `translucent`; the model's `particle` sprite; whether it is a plain `cube`. None for a block the
  game doesn't know. A side's axes run along x then z for down and up, x then y for north and south, z then y for
  west and east, from the block's corner nearest 0, 0, 0 (`block_sprites::side_axes`); the point (a, b) of a side
  shows the sprite at `(uv[0], uv[1]) + a * (uv[2], uv[3]) + b * (uv[4], uv[5])`, rotations and mirrors included
  (for a plain block's north side, `u = 1 - x, v = 1 - y`). A tinted face is multiplied by its source's colour:
  `Grass`, `Foliage`, `DryFoliage` and `Water` are the biome's where the block is, `Fixed` a constant (spruce
  leaves, redstone at its power). A grass block's side is two faces, the side and then its grey overlay tinted with
  grass. The model is the state's first random variant; faces that don't lie flat on a side (a flower's crossed
  planes) are left out, and the `particle` sprite stands for them. Fluids give their still texture on every side.
- `native_texture_sprites(textures)`: a texture's sprite by name (`block/stone`), none for one no model uses.
- `native_gui_sprite(sprite)`: a GUI atlas sprite's `AtlasRegion` by `Identifier` (`minecraft:container/slot`,
  `minecraft:particle/flame`, `mymod:panel`), its UV rect and its size in its own pixels, none for one the atlas
  lacks. Plain data, right until the next `texture_atlas_stitched` for `TextureAtlas::Gui`.
- `native_block_sprites_version()` changes each time the resources load (a resource pack turned on or off moves the
  layers, and the arrays are new): ask again then.

Look blocks up once and keep what you need (a few words a block for your own shader), not every frame.

### Menu slot rules

`menu_slot_rules` asks the registered vanilla or mod menu's scoped `MenuPart` whether a slot takes a stack, whether it gives up what it holds and how many it holds, on both client prediction and server. Mount inventories use `sunrise:mount_inventory`; unregistered player inventory and creative picker menus do not dispatch. `slot` is the backing container index. The slot's own answers arrive first; the final maximum is bounded to 0–99 and the item's stack limit (matching vanilla `Slot.getMaxStackSize` and `Container.getMaxStackSize`). Rules also apply to quick-move top-ups. The hook takes the read-only `Host` (it is a query: it cannot write or cancel). The part can read the menu's current slots and the player's inventory (`Player`: `0..36` inventory, `36..40` armor feet-to-head, `40` offhand, matching vanilla `Inventory`) with `context.slot(container, index)`. Each live `mayPlace`, `mayPickup` and `getMaxStackSize` check during a click asks the parts directly, matching vanilla `AbstractContainerMenu.doClick` and `Slot`. On a LAN host's own click the game is lent to the click with the LAN host lifted out of it, so what the `Host` reads of LAN players answers as if no LAN were open. Inside a slot-rule Part, nested `MenuRef::slot_holds` and `slot_gives` reads keep the slot's own answers without dispatching menu Parts again, even on another slot or menu. `may_pickup` (`Slot.mayPickup`) arrives true and is read only on a registered mod menu type, for its own slots and the player's inventory slots in it, asked with the stack the slot holds; a vanilla menu type keeps its slots' own answer. The slot's own rule comes first (a slot it refuses is not asked), and one ask answers `may_place` and `may_pickup` together. It is asked by a pickup click on a slot that holds an item (after `item_stacked_on_other`, before any take, merge or exchange), by quick move once on the clicked slot before the move (moving into a slot asks only `may_place`), by a hotbar or offhand swap that takes the slot's item (with both stacks present `may_place` must hold too), by throw inside the take (a refusal drops nothing and stops the repeat), by pick-all (which gathers when the clicked slot is empty or refuses, and skips each source that refuses) and by every take the menu makes. A drag spread, a creative clone, an insertion into an empty slot and a click outside the window never ask it; a drag that ends on one slot is a pickup click on it. A refusal leaves the slot and the cursor as they were, and the server corrects the slots and cursor the client predicted otherwise. A slot that refuses placement gives all of its stack or none. A display-only slot (a recipe preview) answers both `may_pickup` and `may_place` false. A shift-click a part answers with `menu_quick_move` is asked the pickup rule first, as vanilla asks it before the menu's `quickMoveStack`.

The hook runs during clicks, drags and shift-clicks with the read-only `Host` on server and client prediction; answer from the stack and host alone, and fast. If a part panics, its answers are discarded and its callback is disabled for the session; the shared log and error toast name the mod. Other parts still answer the check. Parts that read `context.setting(..)` or `side()` have no counterpart on the Host.

### The loop context

`dyn LoopContext` is the read-only view for a hook fired inside a menu, recipe or
player-list loop, where no game is at hand (a catalog entry names it with
`"context": "loop"`; `player_name_format`,
`player_tab_list_name_format`, `living_jump_power`, `living_speed`, `living_flying_speed`, `living_heal`, `living_drown`, `living_fall`, `living_jump`, `ender_entity_teleport`, `fluid_flow`, `player_stays_on_ground_surface`, `block_jump_factor`, `block_speed_factor`, `friction` and the sound hooks below take it). `side()` is `LoopSide::Server` or
`LoopSide::Client`, `registry(name)` borrows names in registry ID order,
`setting(mod_id, key)` returns a loaded mod's setting captured when the context was
built, and `mods()` is the `LoadedMods` view of registration. It has no game Host,
handle construction or transaction, and its answers never change while it is held.
The side that owns the loop builds it, never a hook: the client builds one at window
start and again after each resource reload, and a server builds one when it starts its
mods and again on each `/reload`, so a reload captures fresh settings. A LAN or dedicated server's hub holds the server's
current one, so its Tab list and chat names, and the clicks of its players, see it too.

`item_stacked_on_other(host, event, result, canceled)` is NeoForge's `ItemStackedOnOtherEvent` plus vanilla's `Item.overrideStackedOnOther` / `Item.overrideOtherStackedOnMe` on `ItemPart`, fired inside a pickup click on
the authoritative side only (the server: single player's server thread, a LAN host and its guests, a dedicated server) with the game's `Host` and the click frozen in `event` (`event.direction` distinguishes `Event`, `CarriedOnOther`, and `OtherStackedOnMe` in vanilla `AbstractContainerMenu.tryItemClickBehaviourOverride` order); see its hook documentation. `player_name_format(context, username,
displayname)` and `player_tab_list_name_format(context, player, name, display_name)` format a player's name for chat and
commands (both sides) and for the Tab list (server). The name hooks are contract version `2`: a mod written for `1` takes the
context as a new first argument of the `Hooks` method, and reads the side from it instead of `event.side`.

The predicate supplied by `IngredientDefinition` takes the matching side's read-only loop context and an `IngredientStack` borrowed for that call. `Ingredient::test(host, stack)` uses the host's loop context. There is no hook dispatch during matching: the predicate runs as the mod that defined it, or rejects the stack without running if that mod is disabled.

### Sound hooks

Four client hooks take the `LoopContext` from the sound engine, which has no game at hand and plays sounds from its own loops:

- `play_sound` is `PlaySoundEvent`. It fires at the start of vanilla's `SoundEngine.play`, before the engine's own checks, for every sound the client plays. `original` is the sound as asked for and `sound` is `&mut Option<SoundPlay>`, a copy of it that later mods see edited: change its volume, pitch, source, position, identifier, looping, delay, attenuation, `relative` or `effects`, or set it to `None` to play nothing. `effects` (`SoundEffects`) shapes the sound in the mixer: its own gain, a low-pass and a high-pass on the direct sound and sends to the shared `room`, `hall` and `cavern` reverbs; every change glides over one output buffer, so a listener that moves them on each refresh never clicks. A ticking sound's live refresh (`starting` false) takes the new volume, pitch and effects; a value out of range (`SoundEffects::valid`) refuses the edit as a bad volume does. A changed identifier is resolved again, with its own weighted pick. Only the fields of `SoundPlay` carry over; the engine's random, ticking and resolved sound never leave it. Setting `None` is the mod's own answer (there is no constant): a sound nobody edits plays as before.
- `play_sound_source` is `PlaySoundSourceEvent` and `play_streaming_source` is `PlayStreamingSourceEvent`. A sound fires exactly one of them, after the engine started it on a channel: streamed sounds (music, records) the second, every other sound the first. They are notices, and `ChannelRef` is an id only: a mod shapes a sound through `play_sound`'s `effects`, not through the channel.
- `sound_engine_load` is `SoundEngineLoadEvent`. It fires when the output is open: at window start, after each resource reload (with the new generation's context) and when the output device changes. With no audio (no device, or `SUNRISE_NO_AUDIO`) the engine never loads: this hook and the two source hooks do not fire, while `play_sound` still does, since it comes before the engine's check that it loaded.

All four early-out on the mod's subscription before building a `SoundPlay`, so a game with no listener pays one branch per play. They are client-side only: a server has no sound engine and builds no context.

### Items' default components

`Hooks::modify_default_components` receives `&dyn DataLoadContext` before its
`ModifyDefaultComponentsEvent`. The engine supplies this read-only phase context:
`origin()` is `DataLoadOrigin::ServerDataLoad` or `ClientPreload`, `registry(name)`
gives the names in registry ID order as a shared `Arc<[String]>` (no copy), `setting(mod_id, key)` returns a loaded mod's
setting captured when the owning context is built (including missing keys), and
`mods()` gives the same `LoadedMods` view used during registration (shared; cloning it
copies no package). A later owning
load captures fresh settings even when it reuses registry inputs; runtime Host
settings still read live storage. There is no game Host, handle construction or
transaction. The origin describes the actual owning load, not its thread name or
the side of a later reader.

The lower firing target is `modify_default_components(context, item, components)`
in `crates/sunrise-world/src/item_defaults.rs` (NeoForge's `ModifyDefaultComponentsEvent`).
`components` is the item's entry in `items.json`, vanilla's default-component report:
insert or replace a key to set a component (`"minecraft:max_stack_size": 16`), remove it to take the component away.
Any item can be changed, vanilla's included. Write values as vanilla's codec writes them: nothing decodes them with it.
Each table reads the fields it understands and takes a default for a field it can't read, but keeps the component, so a
malformed `minecraft:food` still makes the item edible, with no nutrition. A stack size above vanilla's 99 is kept: the
data pack's own is held to `ExtraCodecs.intRange(1, 99)`, a mod's to 1 to `i32::MAX` (the count the wire and the save
carry), and a value that is not a number leaves the data pack's own. Keep a stack above 99 to a server whose clients run
the mod: a vanilla client decodes the count with vanilla's 1 to 99 and cannot show it.

- One owning load decodes the defaults, asks once per item, then publishes one immutable snapshot. The item
  table, loot, enchanting, item models, banners, pot sherds and rarity readers borrow that snapshot; workers
  retain shared ownership. The renderers also share its canonical component prototypes. Stacks are saved and
  sent as their difference from the changed defaults, as from vanilla's.
- Reusing client-preloaded item data on the integrated server is not another load. Opening a world or running
  the data-pack `/reload` does not replay this hook. A dedicated resource load supplies `ServerDataLoad`;
  the client preload supplies `ClientPreload`. The hook may run on a load worker, before a game Host exists.
- Client and server each apply their own mods, so both need the mod. A client without it (a vanilla Java client, or
  Sunrise without the mod) fills in vanilla's defaults for the same stacks: a stone stack the server caps at 16 still
  looks like it stacks to 64 there.
- The creative search tab matches vanilla's exported tooltip text, so a changed `item_name` is not searchable yet."##]
    #[notes = r#"## Counterparts

A hook's optional `neoforge` catalog key names its NeoForge 26.3 counterpart;
that counterpart sets the hook's name, side and cancellation contract. The optional
`fabric` and `paper` keys name Fabric and Paper events at the same moment with the
same contract. Each key is a string or a list of strings; omission is valid. One
hook serves each matching moment; near matches with different timing, side or
cancellation are gaps, not counterparts. A hook with none of these keys names the
mod mixin or use it replaces in its design or PR."#]
    #[notes = r#"## Left to do

- A list of every native build, to delete the ones no instance needs.
- A prebuilt horizon-mixins from its CI instead of building it on the player's computer."#]
    #[notes = r##"## More hook capabilities

- `entity_join_level(dimension, entity, kind, pos, loaded_from_disk, canceled)`, an entity joining a
  server level (NeoForge's `EntityJoinLevelEvent`, vanilla's `PersistentEntitySectionManager.addEntity`):
  `loaded_from_disk` is true when it comes back with its chunk, false for a fresh add (a drop, an orb, `/summon`, an
  arrival through a portal into another level). The entity is already in the level's lists when the hook runs; `canceled` takes it back
  out, and a `/summon` of it fails with vanilla's "duplicate UUIDs" message. It tells every kind in the game's entity
  lists (items, orbs, mobs, boats, minecarts, armor stands, primed TNT, falling blocks, projectiles, rockets, tridents),
  once each, in creation order, so check `kind`. A drop, an orb, a mob, a `/summon` and a chunk's entities are told at
  once; other gameplay spawns when the next tick starts, and a cancelled one is removed then.

- [A block's items](#a-blocks-items) are available to server mods through the item capability and
  `move_resources`, so a pipe or machine can move items transactionally.

- What a server mod needs for a public server's screens: `native_sidebar(uuid, sidebar)` shows one
  player a sidebar of up to 15 lines (a text, and one where the score would be) that only their client holds (the
  server's scoreboard never sees it; a mod sends it again when they rejoin), or takes it away with `None`;
  `player.tab_list(host, header, footer)` sets the lines over and under their Tab list; `native_tick_stats()` reads the
  server's tick rate and average tick time in ms (`/tick query`'s); and `player.set_sleep_ignored(host, ignored)` leaves a
  player out of `players_sleeping_percentage`'s count until they are in bed (players away from the keyboard). It refuses
  when the server list is unavailable. Read where they look through each handle's entity.

- Damage a mod can change or cancel through `native_living_incoming_damage` (NeoForge's
  `LivingIncomingDamageEvent`): where `LivingEntity.hurtServer` starts, once its guards passed (invulnerable, dead or
  dying, fire with fire resistance; Sunrise's mobs don't check the fire resistance effect there yet), on every damage
  route, with the hit's level host (its blocks, random and own attachments), the hurt entity's id and type
  (`minecraft:player` for a player), the hit's source (`HurtSource`: the damage type, `minecraft:fall`; the causing
  entity, `causing`, vanilla's `DamageSource.getEntity`: a skeleton for its arrow; the direct entity, `direct`,
  `getDirectEntity`: the projectile's entity id when the damage type names one, for hits on players, mobs and
  the ender dragon, including an ownerless projectile, or the attacker itself in melee; `magic()` and any other
  source with no direct entity stay 0 even during that projectile's impact; and `position`, `getSourcePosition`), the `victim`'s
  position and view vector, the direct entity's `weapon` (`getWeaponItem`) when Sunrise knows it (a player's or mob's
  melee hit or projectile's saved weapon), `causing_weapon` (the causing living entity's current main hand; None for absent,
  unknown, non-living or gone owners), and a live mutable borrow of the engine's `DamageContainer` before reductions
  (no snapshot conversion). Entity ids are 0 for none and for the untracked sources listed below. A mod may change the
  container's new damage or the invulnerability ticks a fresh hit leaves (vanilla's 20), or set `canceled`: nothing of
  `hurtServer` itself happens then (no damage, hurt sound, knockback, invulnerable time, statistics or criteria), and
  the caller acts as on a refused hit, so a blast still pushes a mob whose hit was canceled, as vanilla pushes one that
  refused it. A canceled hit on the ender dragon still counts as landed, as vanilla's `EnderDragon.hurt` returns true
  whatever `reallyHurt` did: the arrow is spent and the melee effects run. A ghast's canceled hit by a fireball it was
  sent back counts as landed (`Ghast.hurtServer`). A negative amount hurts by 0 and a NaN or infinite one by
  `Float.MAX_VALUE`, as vanilla clamps them. It fires for the players, mobs and the ender dragon the game hurts, and
  runs on the thread that ticks the world (the server thread), so state a mod shares with its render thread hooks needs
  synchronising. `/kill` and `native_kill_mob` don't reach it yet: they set the health to 0 instead of hurting with
  `minecraft:generic_kill` as vanilla does. A mod may also install owned reduction modifier closures on the live
  container (`container.add_reduction_modifier(Reduction::Armor, |view, amount| amount * 0.5)`, NeoForge's
  `addModifier` / `IReductionFunction`): each runs in registration order when its reduction stage executes
  (the damage cooldown, armor, Resistance, protection enchantments, a witch's innate resistance, the absorption hearts),
  receiving a read-only `DamageContainerView` of the live container at that stage and the current reduction value
  (vanilla's or the preceding modifier's output). Matching NeoForge's `DamageContainer` semantics, the modified
  reduction is recorded verbatim without clamping to `[0, before]` or substituting for `NaN`, and only the resulting
  damage is floored at `0.0` (`Math.max(new_damage - modified, 0.0F)`). For `Reduction::Absorption`, modifiers receive
  `Math.min(hearts, damage)`, the modified value is recorded as `Reduction::Absorption`, remaining damage is
  `Math.max(damage - modified, 0.0F)`, and the absorption pool deducts `Math.min(damage, modified)` floored at `0.0`
  (`Math.max(0.0F, hearts - absorbed)`). Guarded stages the hit does not enter (for example, `#bypasses_effects`, zero
  Resistance reduction, no Protection enchantments, a non-matching Witch hit, or a Wolf hit absorbed by wolf armor) run
  no modifier; `Reduction::Armor` and `Reduction::Absorption` run at their enclosing `actuallyHurt` sites even with a
  `0.0` starting reduction. Modifiers last for the hit. A panic inside a reduction callback is caught through the
  shared hook panic boundary (without writing a crash report), disables that mod's reduction
  callbacks for the rest of the session, reports loudly via `log::error!` and an error toast naming the mod, and leaves
  the incoming reduction value unchanged so other mods and the hit continue.

- `living_damage_pre` (NeoForge's `LivingDamageEvent.Pre`) runs in `LivingEntity.actuallyHurt` after armor and magic
  reductions and before absorption, lending the same live mutable `DamageContainer` (no snapshot conversion). A listener
  may read earlier reductions, edit `new_damage` or `post_attack_invulnerability_ticks`, or call
  `add_reduction_modifier` for subsequent stages (such as `Reduction::Absorption`, following the same unclamped
  recording, `max(damage - modified, 0)` remaining damage, `max(hearts - damage.min(modified), 0)` absorption pool
  deduction, and per-mod panic-disabling semantics as `living_incoming_damage`).

- `living_damage_post` (NeoForge's `LivingDamageEvent.Post`) observes every hit that passed `living_incoming_damage`
  and the cooldown, at the end of `hurtServer` (after the sounds and that hit's death or totem protection),
  before the hurt caller resumes, even for a hit that took no health (`container.health_damage()` 0).
  The next hazard, poison application or rider's hit sees the preceding hit completed; no callback waits for
  the next tick or turn exit. A bee's drowning hurt finishes before stinger expiry is checked,
  and an expiry hurt finishes before nectar and anger bookkeeping and the server AI controls.
  Post lends a read-only live `DamageContainerView` (no snapshot conversion). Its `container.should_cause_side_effects()` is `hurtServer`'s
  fresh-hit decision (`shouldCauseSideEffects`), taken before the damage: true for a fresh hit, false for a stronger hit
  inside the invulnerability window, and true for a source that bypasses the cooldown. The flag is not inferred from
  final damage or invulnerability ticks, so a damage listener changing those leaves it. It accompanies the reductions
  and damages and describes the decision to cause fresh-hit effects, not whether every effect occurred.

A player's damage type is the source's own. A mob's and the dragon's come from the kind of damage Sunrise keeps for
the hit, which doesn't tell every vanilla type apart yet:

- A trident's hit on a mob is `minecraft:arrow`, not `minecraft:trident`
- A wither skull's hit on a mob is `minecraft:magic`, not `minecraft:wither_skull`
- Evoker fangs on a mob are `minecraft:magic`, not `minecraft:indirect_magic`
- A fireball nobody shot is `minecraft:fireball`, not `minecraft:unattributed_fireball`
- A shulker bullet's hit has no `minecraft:mob_projectile`
- A bed or respawn anchor blast on a mob is `minecraft:explosion`, not `minecraft:bad_respawn_point`
- A blast from TNT a player lit is `minecraft:explosion` on a mob, not `minecraft:player_explosion`
- A bed or respawn anchor blast on the dragon is `minecraft:explosion`, not `minecraft:bad_respawn_point`
- A blast a mob caused (an end crystal a skeleton shot, a creeper's) is `minecraft:explosion` on the dragon, not
  `minecraft:player_explosion`

A registered damage type Sunrise keeps no kind of its own for keeps its id and its tags on a mob and the dragon: a
datapack's `demo:heat` hurt through `ApplyEffectTickEvent::hurt` or `entity_hurt` reaches the hurt hooks as
`demo:heat`.

The causing and direct entities are 0 for now in these cases:

- `magic()` dealt to a player during a projectile hit (an ownerless wither skull, instant harm from a
  tipped arrow): vanilla's source names no entity
- A player's causing entity, except for a mob that shot the projectile, the mob `/damage` names, an evoker that summoned
  the fangs, a slime or magma cube that touched the player, and a mob that exploded (a sulfur cube's is guessed until
  its blast names its source)
- A mob's causing and direct entities when a player is behind a hit outside a scoped attack (for example TNT
  a player lit); a projectile's hit does run inside its owner's attack
- A mob's direct entity when it isn't the causing one outside a projectile hit (for example a TNT blast)
- A falling block on a mob, whose causing and direct entity is the block in vanilla
- The dragon's direct entity for a blast
- The dragon's causing player when it isn't the acting one on a LAN guest's turn
- `/damage` on a mob `by` an entity with a damage type that isn't melee: the direct entity is 0, not that entity

And `/damage` on a mob `by` one entity `from` another with a melee damage type gives the `from` entity as the direct
one, where vanilla gives the `by` one.


- `native_block_entity_sync(pos)`, which sends the block and its update tag to clients again
  (`Level.sendBlockUpdated`): with the level's block changes, to the players who see its chunk, once per position
  however often it is called in a tick. A change made in a block entity's tick reaches clients at the start of the
  next tick, as in vanilla. Call it when what clients show changes, not for every tick's progress: a menu's data slots
  carry that.

- `entity_leave_level(dimension, entity, kind)`, an entity leaving a server level (NeoForge's
  `EntityLeaveLevelEvent`, vanilla's `onTrackingEnd`): removed for any reason or its chunk unloading, not the entities
  still loaded when the server stops. It only hears. It tells item entities, experience orbs and mobs (`kind` is the mob type) so far; check `kind`.

- Fluid tanks on a mod block's block entity, in millibuckets (a bucket is a thousand). `r.block_fluids(&block, tanks)` registers `ModFluidTanks`, as `r.block_container(&block, container)` registers slots: each tank's capacity, the
  tanks each face reaches, and which a fill may reach and a drain may take; the tanks' contents live in the entity's saved compound under
  `fluids`, so they survive restarts, and the block's own tick reads them there. Other blocks and mods reach them, and a
  cauldron's, through the [`FLUIDS` capability](../concepts/capabilities/block-fluids.md).

- A content effect's tick behaviour belongs to its [`MobEffectPart`](../how-to/entities/effect-ticks.md), not global listeners.
  `on_effect_added` hears a content effect given to an entity that did not have it; `mob_effect_added` (NeoForge's `MobEffectEvent.Added`) hears every addition.
  The three moments around them cover every effect, vanilla's included: `mob_effect_applicable`
  (NeoForge's `MobEffectEvent.Applicable`, vanilla's `canBeAffected`) answers whether an effect can land — the first
  mod to answer `Allow` or `Deny` overrides the vanilla tag check, and an unanswered hook falls
  back to it; `mob_effect_expired` (`.Expired`) hears one count down to nothing — cancelling
  refuses the expiry, and the effect stays, kept at no ticks and asked again next tick;
  `mob_effect_remove` (`.Remove`) hears one about to be removed by any path — a command, milk or a
  mod's removal helper — and cancelling keeps it. All three name their entity as a `LivingRef`, the player's own included.

- Chunks and players coming and going, on the server: `chunk_load` (with whether the chunk is new), `chunk_unload`, and
  `player_logged_in` and `player_logged_out` (version 2), which come in pairs for network players however they leave, the server stopping included. A kicked fake also produces logout without a preceding login; silent `remove_fake` produces neither. Changes made to the departing chunk in `chunk_unload` aren't saved: as in vanilla, the chunk is saved before it unloads.

  One tick's streaming step (`update_world`) runs in this order:

  1. The chunks that come and go are worked out. A chunk saved while that is planned is copied for its save there.
  2. The arriving chunks join the level and the departing ones leave it.
  3. Each departing chunk that changed since its last save is copied, with its block entities and save data, and the
     copy is queued to save.
  4. `chunk_load` runs for each arriving chunk (`native_chunks_loaded`).
  5. `chunk_unload` runs for each departing chunk.
  6. The departing chunks' entities, block entities and block names are dropped.

  So an edit a `chunk_load` listener makes to a chunk departing in the same tick (a neighbour's block entity,
  say) comes after that chunk's save copy (step 3): it is not saved, and step 6 drops it. That is the difference from an
  ordinary load, where the edit lands in a chunk that stays and is saved with its next save, and it is the same rule as
  `chunk_unload`, whose edits also come after the copy. Change a chunk you may lose from a later tick instead,
  after checking it is still loaded.
- Sounds the server plays to its players: `native_play_level_sound_at_position` and `native_play_level_sound_at_entity`
  (NeoForge's `PlayLevelSoundEvent.AtPosition` and `.AtEntity`, in `ServerLevel.playSeededSound`). As in vanilla, a mob's own
  sounds (`Entity.playSound`) are at its position; only `playSound(except, sourceEntity, ...)` sounds are at an entity: vanilla's
  sites go through `Game::play_bound_sound` (the mobs' through `MobEvent::BoundSound`), which queues them for the end of the
  tick, after the tick's other sounds; the goat horn and a mod's bound sound ask directly. Two sites keep a difference: dye on a
  sheep and an allay's sounds reach the player who caused them too, since Sunrise's client plays them from the packet rather
  than predicting them. The packet stays positional (horizon-rust/horizon#9284). A mod may change the sound, its source, volume and pitch, or cancel it. Every
  server sound packet but `/playsound`'s goes through `Game::send_level_sound` (`game/interface/level_sounds.rs`), or the native-sound
  flush for a mod's own, so a new sound site asks the mods by calling it. The tick's sounds are asked as they go out, at the end
  of the tick. With no listener, the cost is one `listened` check per sound and nothing is allocated.
- Chunks a player watches, on the server: `chunk_watch(player, chunk)`, `chunk_sent(player, chunk)` and
  `chunk_unwatch(player, chunk)` (NeoForge's `ChunkWatchEvent.Watch`, `Sent` and `UnWatch`). Each chunk that comes into a
  player's view is watched, then sent; it's unwatched when it leaves the view, when the player leaves the level (a portal, a
  teleport, a respawn) and when they disconnect, in the level it was watched in. Vanilla watches a chunk when it queues it; Sunrise
  picks a chunk only as it sends it, so the watch comes just before the send. The hub records them only while a mod listens and
  the game tells them after the hub's network step for that level, so a handler sees the state after that step; the order per
  player and chunk is vanilla's. The server stopping doesn't unwatch.
- A chunk's ticket level changing, on the server: `chunk_ticket_level_updated(chunk, old, new)` (NeoForge's
  `ChunkTicketLevelUpdatedEvent`). The level is the chunk's loading level from its tickets (a player, a forced chunk, a portal
  ticket, the dragon fight's arena), on `ChunkLevel`'s scale: 31 and lower entity ticking, 32 block ticking, 33 loaded without
  ticking, and `ChunkTicketLevel::Unloaded` past it, so a non-ticking or unloaded chunk is never clamped to the ticking range.
  Sunrise keeps no level for a chunk partway through generating, and no chunk holder to hand over. The level's tickets keep each
  change, in the order they were made, only while a mod listens: with no listener an update of the tickets looks at no chunk
  and allocates nothing, so the tick costs what it did. A level is told in its own tick, after the tickets were updated for its players.

- [the renderer's functions](#render-functions): `world_drawn`, called once the world is drawn with its colour, depth, matrices and entity geometry, and
  `render_needs`, where a mod asks for the entity runs and the world's depth without a shader pack.

- [entities](#entities) for server mods: `native_summon(dimension, kind, pos, nbt)` spawns any entity from its compound
  as `/summon` does.

- `item_bar(stack, bar)`: the small bar `GuiGraphicsExtractor.itemDecorations` draws under an item,
  wherever the game shows one. `bar` arrives holding
  vanilla's choice (the damage bar, or a bundle's fullness, already applied); set it to a width `0..=13` and an
  `rgb` colour, or to `None` to hide it. Asked each frame a stack is drawn. Client-side (NeoForge has no event for
  this; vanilla's `Item.isBarVisible`/`getBarWidth`/`getBarColor`).

- Mods' own sound events in `play_sound`: an event of another namespace than `minecraft` (`mymod:machine_hum`) is sent to players by
  name, as vanilla sends a sound that isn't in its registry, and they hear it when their resources define it (a content mod's `assets/mymod/sounds.json`
  and its `.ogg` files). `minecraft:` events must still be registered.

- Nicknames for server mods: `player.set_nickname(host, Option<&str>)` shows a name in place of a player's in chat (their messages, `/me`, `/msg`) and the Tab list, the real name on hover and everywhere else. Obtain a `PlayerRef` from `host.players().server_players()` or `.acting_player()` and retain that value, not the borrowed service. The handle is tied to its world epoch and entity identity: nickname writes refuse stale or offline players, but can reach a connected player displaced by another player's turn. A nickname already set persists across reconnects until changed or the server stops.

- Tag and fluid reads from any hook that has a `Host`: `host.block_in_tag(oak_log, "minecraft:logs")`, `host.item_stack_in_tag(&stack, tag)` (`ItemStack.is(TagKey)`), `host.state_fluid_state(state)` (`BlockState.getFluidState`: water for a waterlogged state, `None` for an id no state has) and `host.fluid_in_tag("minecraft:water", "minecraft:water")`. The tag may be written with or without `minecraft:`, with no `#`. They read the world's merged tags, data packs and content mods included; an unknown tag is false and an empty stack tests `minecraft:air`. On a client they read the tags its server sent. The tag reads are `None` before those or the world's tags are loaded. `host.state_outline(state)` (`BlockState.getShape` with an empty getter at the origin) gives a state's outline boxes in the block's own space, with no random offset and no world context; empty for air, `None` for an id no state has. `host.tags()` gives `items`, `blocks` and `fluids` in vanilla’s order; see the [registry-key contract](../concepts/registry-keys.md#item-values-and-tag-names) for missing, empty and unavailable tags. `level.fluid_state(host, pos)` (`BlockGetter.getFluidState`) reads the fluid at a position, `None` where the block read is (an unloaded chunk or a `y` outside the world).
- Fluid facts from any hook that has a `Host`, on either side and with no world: `host.fluid_is_same("minecraft:water", "minecraft:flowing_water")` (`Fluid.isSame`: a source and its flowing form are one fluid; false when either id names no fluid), `host.fluid_bucket(fluid)` (`Fluid.getBucket`: `minecraft:water_bucket` or `minecraft:lava_bucket`), `host.fluid_pickup_sound(fluid)` (`Fluid.getPickupSound`: `minecraft:item.bucket.fill` or `minecraft:item.bucket.fill_lava`) and `host.fluid_empty_sound(fluid)` (`BucketItem.playEmptySound`: `minecraft:item.bucket.empty` or `minecraft:item.bucket.empty_lava`). Fluids are named by id, with or without `minecraft:`. The bucket and the sounds are `None` for `minecraft:empty`, which has neither (vanilla's bucket there is air), and for an id no fluid has; the empty sound is `None` too for a tank-only fluid, which no bucket pours.

- A block part's `is_pathfindable` changes whether a path of a `PathComputationType` (`Land`, `Water`, `Air`) may go through the block's states (`BlockBehaviour.isPathfindable`); `event.pathfindable` starts with the block's own answer. The answer depends on the state and the type alone: the game asks once per state and type when parts resolve (at world load and on every `/reload`) and path finding, spawning and the goals read the stored answer, so a part cannot answer from a position, a level or the time. Server-side.
- A block part's `should_check_weak_power` adjusts whether a signal query reads indirect power through that block, for its position and input direction. Block entity neighbour notices use `on_neighbor_change`; `weak_changes` opts the farther block into notices through a conductor (see the block entity neighbours concept page).

- A block part's `should_redstone_wire_connect_to` adjusts whether redstone wire connects to that block for the given position and optional direction.

- A block part's `connects_to` says whether a fence, a wall, or iron bars or a glass pane of that block connects a side toward a neighbour state. `event.kind` names vanilla's rule and `event.connected` starts with its answer: `ConnectionKind::Fence` is `FenceBlock.connectsTo` (a solid face, a fence of the same kind, a gate across the arm), `Wall` is `WallBlock.connectsTo` (a wall, a solid face, bars or a pane, a gate across the side; a connected side is still low or tall from the block above) and `Bars` is `IronBarsBlock.attachsTo`, for iron bars and panes, which are one block class (a solid face, bars or a pane, a wall). Set it to false to deny a connection as well as to true to make one. The neighbour answers first, in its own `can_be_connected_to`, and `event.connected` starts from that answer: the connecting block has the last word. `pos` and `state` are the block being shaped, `neighbour` the state beside it, `direction` the way from the block to the neighbour (vanilla's bars rule takes none) and `face_solid` whether the neighbour's face toward it is sturdy. It is asked on placement and on every shape update toward a side, on authoritative servers only, and only the block's own part answers, so vanilla blocks cost nothing. The three rules stay separate in the library; only the question a mod is asked is shared, and `kind` leaves room for fields one kind alone needs. Diverges from upstream: new hook.

  **Migration (`fence_connects_to` → `connects_to`, contract version 2):** a part that implemented `fence_connects_to(&self, level, event: &mut FenceConnectsToEvent)` implements `connects_to(&self, level, event: &mut ConnectsToEvent)` instead. The event's fields are the same plus `kind`; a part that only answered for fences returns early unless `event.kind == ConnectionKind::Fence`. The answer, its timing and the fence's own rule are unchanged.

- A mod block's comparator output, as vanilla's `BlockBehaviour.hasAnalogOutputSignal` and `getAnalogOutputSignal`: `has_analog_output` says a block of yours has one,
  and `analog_output` gives it (0 to 15) for its state and position. A state change with neighbour updates tells the comparators itself; when the output
  changes otherwise, `level.update_neighbors(host, pos, state, NeighbourUpdate::OutputSignal)` makes them read it again (`Level.updateNeighbourForOutputSignal`).

`Hooks::component_types`, `blocks`, `items` and `standalone_models` take
`&RegistrationContext` before their event; `Hooks::register` takes it after
its `Registrar`. `context.mods()` returns a read-only `LoadedMods` view whose
`ids()` lists the packages enabled and admitted at startup in dispatch order; reloads keep it.
`context.registry("minecraft:item")` borrows a built-in registry's names in ID order as
`Option<&[String]>`, `context.setting(mod_id, key)` borrows a captured setting as `Option<&str>`,
and `context.instance_dir()` borrows the startup instance directory as `Option<&InstanceDirectory>`,
so a mod can validate the keys it declares before its Parts are finalized. All three read one view
the engine captures before the first registration hook runs: every registration step sees the same
registries (as `registries.json` lists them, without what mods declare during registration) and the
settings on disk at that moment, whatever the dispatch order. Missing registries, unset keys, and a
context with no engine snapshot return `None`. These contexts are supplied by the engine and have no
game `Host`, handle constructors or transactions. Declarations still go on each event or registrar.

- `component_types(context, event)` runs once before content loads. Call `event.register(id)` with each owned namespaced ID. Duplicates, invalid IDs and the `minecraft` namespace stop startup. `native_component`, `native_set_component` and `native_remove_component` read, set and remove registered components, including item defaults. Values use NBT and must fit the network reader limits.

- `create_skull_models`, `map_decoration_types`, `register_entity_spectator_shaders`, `register_fluid_models`, `register_render_pipelines` and `register_texture_atlases` take one entry per `event.register(entry)` call. `block_entity_type_add_blocks` adds memberships with `event.modify(kind, blocks)` or `event.modify_matching(predicate, blocks)`; `event.types` lists registered types, not additions. These registration methods stage entries in listener order. A listener cannot read, edit or remove earlier registrations; a phase refusal discards only that listener's additions. `register_render_pipelines` (version 4) accepts any valid pipeline `Identifier` without a namespace ownership gate. Mod pipeline IDs are unique across required and optional registrations; collisions with vanilla are checked in the matching registry as in NeoForge `RegisterRenderPipelinesEvent`. Fluid models and texture atlases are collected again on resource reload. Atlas timing and duplicate handling are unchanged.

- `register_layer_definitions(context, event)` (version 4, matching NeoForge `ClientHooks.registerLayerDefinition` and vanilla `LayerDefinitions.createRoots`) binds each `<namespace>:<path>#<layer>` ID to an owned `Send + Sync + 'static` supplier with `event.register(id, supplier)`. Both delimiters are required; the namespace and path follow vanilla `Identifier` rules and may be empty, and the layer can be any string. Duplicate mod registrations keep the last supplier per ID before invoking suppliers, and an exact collision with a vanilla layer key in `entity-models.json` fails client startup. Each winning supplier runs once in registration order at client startup when the layer table freezes. The game keeps their definitions, not the suppliers. Baking through `ClientHost::bake_layer` and reloading resources reuse the definitions.

- [explosions](#explosions) for server mods: `LevelRef::explode(host, source, center, radius, fire, interaction, settings)`
  sets one off as `Level.explode` does, with an optional source entity, `ExplosionInteraction::None`, `Block`, `Mob`, `Tnt`
  or `Trigger` for what it does to blocks, and `ExplosionSettings` for its damage type, calculator, particles and sound.

- Dispenser behaviours are chosen at `DispenserBlock.getDispenseMethod`, before any vanilla behaviour, for mod or vanilla items, on the authoritative server only (droppers are unaffected). The block the dispenser faces is asked first: `dispense_method` gives its part the dimension, dispenser position, facing, the selected stack and `state`, the block at `pos` moved by `facing` (air included), and the part is asked only for that block. Then, unless that part answered, the item's own part is asked by `dispense_item` with the same dimension, position, facing and stack. Either answer is `Some((remainder, event))`: a `None` remainder empties the slot, `Some(stack)` writes that remainder without consuming again, and `event` is the level event played with the smoke (1000 click, 1001 fail, 0 nothing, as `NOOP`). The first answer wins: the other part is not asked and vanilla's behaviour doesn't run. `host.dispense_default(dimension, pos, facing, stack)` drops one item normally, including items with a special vanilla behaviour: answer its remainder with 1000. Leave the answer `None` to let vanilla decide, as before.

- Image tooltip components: `tooltip_component(stack, size)` answers finite positive width/height in GUI pixels, and `tooltip_component_draw(stack, batch, origin, size)` draws that component with `native_sprite`, `native_fill` and `native_text`. The stack supplies its data. Space and background are reserved after the text and any vanilla bundle component; the component width participates in tooltip positioning. Neither hook changes ordinary tooltips without an answer. Client-side.

- `ClientHost::modifiers()` returns a `ClientModifiers` snapshot of the window's shift, control, alt and super keys, using the same GLFW bits as the input hooks. It is available outside a world and clears on focus loss. Reading it copies the current modifier state; it does not allocate or dispatch hooks.

- `Hooks::render_tooltip_gather_components` (NeoForge's `RenderTooltipEvent.GatherComponents`, client-side, cancellable): an item's tooltip as one list of `TooltipElement`s, a `Text` line with its colour or an `Image` (a `TooltipImage`: its `TooltipImageSource` selects the built-in bundle renderer or an item part, carrying the stack and measured size in GUI pixels), after `item_tooltip` and before anything is wrapped or measured. Add, change, reorder or remove elements. `max_width` starts at -1, no limit, which leaves the lines as vanilla draws them; a mod that sets it wraps every line wider than that. Cancel and nothing is drawn. An image with a non-finite or non-positive size is dropped.

- `Hooks::render_tooltip_pre` (NeoForge's `RenderTooltipEvent.Pre`) is asked for each tooltip about to be placed and drawn, after `render_tooltip_gather_components`: item tooltips and text tooltips (a button's, a book's, the creative tab names), not the selected-item name a bundle draws beside its grid. `place` is the point the tooltip hangs from (the cursor, or the point its screen gives), before the positioner adds its offset and keeps the box on screen; set its `x` and `y` to move it. `screen_width` and `screen_height` are the screen's size. `elements` are the list `render_tooltip_gather_components` left, text and images (the built-in bundle grid included) in that order, read-only (change them in `render_tooltip_gather_components`), and `stack` is the item it describes, `None` for a text tooltip. Set `cancel` and nothing is drawn and no `render_tooltip_texture` is asked. It runs each frame a tooltip shows, only while a mod listens. Diverges from upstream: new hook.

- `Hooks::component_hover` (vanilla's `GuiGraphicsExtractor.componentHoverEffect`; NeoForge has no event for it) is asked for the hover of the text component under the cursor in the chat, a book or a dialog, before the game draws it. `hover` is the typed hover action (`TextHover`: its text, an item or an entity), `x` and `y` the cursor. Set `cancel` and the game draws nothing for it, and a mod can draw its own with `draw`. Left alone, a text hover is split at half the screen's width (at least 200), an item hover draws its stack's tooltip, which asks `item_tooltip` and the `render_tooltip_*` hooks as any item tooltip does, and an entity hover shows its name, type and UUID while advanced tooltips are on. It runs each frame a hovered component shows, only while a mod listens. Diverges from upstream: new hook.

- [advancement triggers](#advancement-triggers) of a mod's own: `player.trigger_criterion(host, trigger, test)` fires a trigger
  an advancement of a content mod or a data pack names (`mymod:pipe_connected`) for one player, as a NeoForge mod's
  `SimpleCriterionTrigger.trigger`.

- Where a mod block can stay. `can_survive(dimension, pos, state, survives)` answers
  `BlockState.canSurvive` before the block does: as it is placed (`BlockItem.canPlace`: `Some(false)` refuses the spot),
  from the shape update of a block whose template checks it (a plant, a torch, a sign: `Some(false)` breaks it with its
  drops when a neighbour changes), and wherever else vanilla asks. `state` may not be in the world yet, so decide from
  `state`. `may_place_on(dimension, state, ground, ground_pos, may)` answers `VegetationBlock.mayPlaceOn` for a
  mod plant, whose block declaration opts into vegetation or whose template inherits it. The soil's `can_sustain_plant`
  decides first, then the plant; an empty answer keeps the plant's rule. Placement refuses unsupported ground and
  neighbour updates pop the plant off. Answer only for your own blocks. The level is read-only; both run on shape
  updates, so keep them short. With `can_sustain_plant` and `state_for_placement` they are asked on both sides (their docs).

- Drawing mod block entities on the client. `block_entity_submit(pos, block, state, entity, draw)` runs once a frame
  for each mod block entity in view, as vanilla's `BlockEntityRenderer.submit` does (within its view distance, 64 blocks
  unless `block_entity_view_distance` answers otherwise, asked once per mod block and kept, again after a resource reload or a world change; Diverges from upstream: new hook; in the frustum,
  its chunk faded in), with the copy the server sent (`block_entity_update_tag`). It draws into the entity batch
  under `draw`'s pose stack (`push`, `pop`, `translate`, `rotate_about`, `scale`, in blocks from the block's corner, as
  vanilla's `PoseStack`): `native_draw_quad` (four corners and a sprite's UVs), `native_draw_cube` (a model element's box,
  its faces baked as a model's are, each a `CubeFace::new(sprite)`, `.with_uv(uv)` for UVs of its own), `native_draw_block` (a block
  state's model, as `submitBlockModel`: a moving part is often a block of the mod's own) and `native_draw_text`
  (`BlockEntityDrawContext::text(text, at, rgb, glowing, outline)`: a component in the game's font, as a sign's line or a
  text display's, in font pixels with y down as `Font.drawInBatch` lays it out under the pose, with its styles   (bold, italic, underline, strikethrough, obfuscated, fonts, `§` codes), opaque `0xRRGGBB`, in the frame's light or full bright when
  `glowing` (a nixie tube), ringed with `outline` when given; `text_width` measures it, styles included, to centre it).   Parts drawn after `spin` turn on the client's animation clock (`animation_time`) and are lit where they turn to. Sprites are the atlas's:
  `<ns>:block/...` and `block/...`, and a content mod's `assets/<ns>/textures/entity/` as `<ns>:entity/...`. Culling uses the bounds
  `block_entity_render_bounding_box` gives (NeoForge's `getRenderBoundingBox`): the block with a block of margin
  unless the mod widens them, or never culled when a bound is infinite. Both run every frame for every one in view: keep
  them cheap.

- Bone meal on a mod block, the `BonemealableBlock` three questions a block nothing in Sunrise knows one for
  answers itself: `is_valid_bonemeal_target(state, valid)` says whether the block takes bone meal (`Some(false)` refuses it even where its template takes it, `None` leaves the
  template's; asked on both sides, as vanilla asks it, so a client predicts the use); `is_bonemeal_success(state, success)` is
  `isBonemealSuccess`, drawn through the level host's `with_random` to keep vanilla's order; and `perform_bonemeal(state)`
  is the growth itself, with `native_set_block`/`native_set_block_property`. The meal is used up and the growth
  particles and sound follow as vanilla's do, whether or not the roll grows the block, from a player's bone meal, a
  dispenser's and, on a crop, a villager's. A block that answers nothing keeps the `BonemealableBlock` its template gives it.
  Authoritative servers only, except the first question.

- Bone meal on any block: `Hooks::bonemeal` (NeoForge's `BonemealEvent`) hears every `BoneMealItem.growCrop`, before
  the block is asked anything: a player's use on both sides, a dispenser's and a villager's. It names the player (none for a
  dispenser or a villager), the block, the `source`, the stack (what the mod changes stays) and `valid_bonemeal_target`, the
  block's own answer. Cancel it and the growth, the block's bone meal Parts and the meal's use are skipped: `growCrop`
  answers `successful` (`setSuccessful`), false unless the mod sets it, so a player's use still tries the water plant
  beside the clicked face. A mod growing only its own blocks answers the three questions above instead.

- Mod-block landing is a block Part method, not a global hook (`block_fall_on` is removed; it diverges from upstream): `BlockPart::fall_on` (`Block.fallOn`) receives writable fall distance and damage multiplier before calculation; a zero multiplier prevents fall hurt and generic fall sounds. Creative and fall-immune entities notify with zero multiplier and retain immunity. Non-finite inputs are ignored; negative multipliers are clamped to zero. `BlockPart::bounce_on` (`Block.bounceOn`) fires only when the entity bounced, just before vanilla sets its movement, and may change the movement the bounce leaves. Both run synchronously inside the entity's move on the level random host (reads blocks, draws the level's random in vanilla's order, attachments; no writes); players and mobs fire them, and dropped items fire `fall_on` (kind `minecraft:item`, zero multiplier, read and act on the item through `ItemEntityRef`; an item's `bounce_on` is not fired yet). An item's fall distance grows by its moves down out of water and resets in water and on landing, as `Entity.checkFallDamage` has it; with no Part on the block it lands on, the landing costs one `fall_listens` lookup. `fall_on` asks for writes with `event.request(LandingWrite::SetBlock(state))` or `LandingWrite::DestroyBlock { drop }` (farmland to dirt after its random draw, a turtle egg breaking): they apply at the batch end through the same drain as `entity_inside`, in the order asked, if the block is still the state the entity landed on.
- `Block.entityInside` is `BlockPart::entity_inside(host, event)` (`EntityInsideEvent { state, pos, entity, precise }`; `precise` is `Entity.checkInsideBlocks`' flag), the Part form of what `block_entity_inside` was; there is no global hook (NeoForge has no bus event for it). It fires mid-sweep, inside the entity's move, on the level random host (`&mut dyn LevelRandomHost`, like `fall_on`): a Part reads blocks and draws the level's random in vanilla's order, interleaved with the vanilla blocks of the same walk; only writes are restricted to the event. The Part queues its writes: `event.apply(InsideEffect::…)` is one of the five `InsideBlockEffectType`s, `event.run_before`/`run_after(effect, EntityWrite)` a typed write to the swept entity (applied inside its own tick, in `StepBasedCollector` order, stopping when it dies), `event.request(InsideWrite::…)` a level write at the touched block, applied at the batch end, guarded on the state (the `fall_on` contract: the complete batch is checked, an invalid state refuses all of the callback's writes). Only a block state a Part listens for fires it; with no listening Part nothing is recorded and the walk allocates nothing. `EntityWrite::Hurt` takes a registered damage type without naming a causing or direct entity. Players, mobs, items and projectiles all fire it on the shared walk, inside their own tick or batch; the level writes a contact asked for still land at the batch end, through the drain the landings use. The block's `getEntityInsideCollisionShape`/`insideBlock` shape that decides "inside" is not exposed (tracked on #44). Contact Parts cannot hurt boats or minecarts yet (#4177).

- [loot table hooks](#loot-tables), for changing loot a mod didn't make. `loot_table_load(context, id, table)`, in `sunrise-world`'s `loot.rs`, gets each loot table's JSON as it loads, after the data packs' (NeoForge's `LootTableLoadEvent`, Fabric's `LootTableEvents.MODIFY`): add pools or entries, change or replace it, and chests, block and mob drops, fishing and the rest then roll the changed table. It runs on the server for each data load and reload, not client preloads. `loot_drops(context, drops)` gets the items a broken block or a dead mob is about to drop, to add, change or take out (global loot modifiers, `MODIFY_DROPS`), with `LootDrops`: the table rolled, the dimension, the origin, the block or the mob, the player who broke or killed it, the tool and an explosion's radius. It hears blocks a player, an explosion or a mob breaks, not other breaks such as leaves decaying.

- A `minecraft:post_attack` enchantment effect of a mod's own kind, on a mob a player's weapon hit: `enchant_effect` (see
  [A mod's own enchantment effect kinds](../how-to/items/enchantment-effect-kinds.md)).

- `screen_render_post`, the point of `ScreenEvent.Render.Post`: a native mod draws over the open screen after it and its tooltips, given its id, the server-opened menu's vanilla type (`menu`) and title, its container panel, its slot count, the mouse in GUI pixels and the partial tick (`ScreenView`). Every container, station and mod window draws on the inventory screen and so reports `inventory`: tell them apart by `menu` and `title` (None on a window the client opened) and by `slots`. It does not fire on the title screen, nor for the prompts, books and sign editors drawn over gameplay after it.

- `native_screen_exclusion()` answers the rectangles (`[x, y, width, height]`, GUI pixels) the open screen keeps clear for the things drawn beside its window: the recipe book's panel, the creative tab rows above and below the window, and the active effects right of the inventory, then every rectangle a mod added in `screen_exclusion_areas(screen, areas)` (a side panel, a button column; a mod that draws over a screen reports where). Compute it once per screen layout and cache it, not per frame: the rectangles move only when the window, the recipe book or the effects change. A mod that lays out panels of its own, as a recipe viewer does, shrinks its rows and columns around them. Empty and non-finite rectangles are dropped. Empty with no screen up. Vanilla has no counterpart; NeoForge recipe viewers take the same from their own API (EMI's `EmiExclusionArea`, JEI's `IGuiContainerHandler.getGuiExtraAreas`). Client-side.

- `native_item_burn_time(item)` answers the item's cooking-fuel burn time in ticks (`FuelValues.burnDuration`: its `cooking_fuel` component, the data pack's context int provider resolved), None where it is no fuel. `native_item_compost_chance(item)` answers the chance the item raises a composter's level (its `compostable` component's layers provider resolved; the `compostables` data map), None where it is not compostable. A mod cannot resolve either from the item's default component, which holds only the provider's name. Client-side, API next.
- `native_screen_hovered_item()` answers the id of the item in the slot under the pointer on the open container screen (`AbstractContainerScreen.getHoveredSlot()`'s stack), None over no slot, an empty one, or with no window. A recipe viewer reads it for its look-up keys. Client-side, API next.
- `menu.slot_boxes(host)` (`menu_slot_boxes`) answers the open inventory screen's slot rectangles (`[x, y, width, height]`, GUI pixels) in menu slot order: `AbstractContainerScreen.leftPos`/`topPos` plus each `Slot.x`/`Slot.y`, 16 by 16, so a mod that highlights or searches slots in `screen_render_post` draws where vanilla's own slot highlight does. `None` entries are slots the screen does not show; the whole answer is `None` for a stale menu, another screen, or an inventory covered by the recipe book. Client-side.

- `modify_default_components(context, item, components)`, in `sunrise-world`'s `item_defaults.rs`: an item's default
  components in `items.json`'s form, for any item, vanilla's included (NeoForge's `ModifyDefaultComponentsEvent`). Insert
  or replace a component to set it (`"minecraft:max_stack_size": 16`), remove it to take it away. The item table,
  loot, enchanting and item models all read the changed defaults, and stacks are saved and sent against them as
  against vanilla's. A `minecraft:max_stack_size` above vanilla's 99 is kept, up to `i32::MAX`. Asked for every item
  each time the game reads the defaults: answer the same each time. Client and
  server each ask their own mods, so both need the mod.

- The open screen hears of a mouse button going down (`screen_mouse_button_pressed_pre`) or up (`screen_mouse_button_released_pre`) before it acts, as NeoForge's `ScreenEvent.MouseButtonPressed.Pre` and `.MouseButtonReleased.Pre`: the screen, the GLFW button (0 left, 1 right, 2 middle, 3 back, 4 forward; a button past those is the platform's own number, which differs between systems) and GLFW's modifier bits (shift 1, control 2, alt 4, super 8), with the pointer in `screen.mouse`. A real press also carries `double_click`, true when the last handled press was on this screen instance with this button less than 250 ms ago. A cancelled press skips the screen handler, Post and gameplay mappings, but counts as handled for double-click detection. Cancel a release only when its press was cancelled, or a drag it ended stays held. These hooks cover the game's and title's screens, not the connect and loading screens, prompts, books and sign editors drawn over gameplay, or a gamepad or keyboard-focus click.

- [falling blocks](#falling-blocks) for server mods: `native_spawn_falling_block(dimension, state, pos, velocity, falling)`
  throws a block that flies, lands and places itself as sand does, for debris and collapsing structures.

- Camera shake for client mods: `camera_shake(partial, shake)` asks each frame for turns in degrees and an
  offset in blocks, which add up across mods and multiply into the view after its bobbing. Sunrise scales the sum by
  Distortion Effects and bounds it to 10 degrees and half a block on each axis.

- Client sounds: `ClientHost::play_local_sound(sound, after)` plays a `SoundPlay`, vanilla's event or the mod's own
  from its `assets/<id>/sounds.json`, for this player only: an interface sound (`SoundPlay::ui`) or one at a place
  (`SoundPlay::at`), now or up to a minute later, so a client mod can click its own buttons or time a blast's sound
  to its shock wave. Its `SoundRef` answers whether it plays, and stops or fades it.

- Values for a mod's own shader pack: `shader_params(partial, params)` asks each frame for eight `vec4`s that
  every stage of a Sunrise shader pack reads as `frame.mods`, so a mod can tell the pack it bundles where and when
  something happened (an explosion's centre and age for a cloud drawn in a post pass).

- Menus of content mods' own menu types (vanilla's `MenuType`): a mod container's `menu` may name a menu type its content mod declares in `data/<id>/sunrise/menu/` with as many slots ([making a content mod](../how-to/content-mods.md#menu-types)). It opens when the block is used, its data slots are the block entity's `data`, and a Sunrise client shows it through the mod's registered screen factory. Server collect hooks `recipe_sources` and `menu_links` name lent ingredient and linked-slot containers; see the menu-types guide for ordering, caps, skips, write-back and the no-listener tick path.

- `block_highlight(pos, state, partial, canceled)` runs before building the eligible targeted block's selection outline, after hidden-HUD and game-mode restrictions. Set `canceled` to suppress only that outline; default behavior is unchanged otherwise. Position is block coordinates and state is the current block state ID. No entity outlines are affected.

- `render_hand(render, draw, canceled)` before each visible first-person hand, including maps, the empty main-hand arm and the empty off hand, where vanilla draws nothing. `render` carries the player UUID and entity id, hand, displayed stack, partial tick, interpolated pitch, light, swing and inverse equip height including swap animation scale. Adjust its translation (blocks), rotation (pitch, yaw, roll in degrees) and scale; the pose is applied in that order before the ordinary hand and item poses. Non-finite or out-of-range components are ignored individually: offsets must be within -16..=16 blocks, angles within -3600..=3600 degrees and scale within 0.01..=100. Set `canceled` to skip default geometry. `draw` draws in the hand, as the submit hooks' draws do: its pose stack starts in the hands' view space (blocks, +x right, +y up, -z into the screen) under the hand sway, before `render`'s pose and vanilla's per-item transforms, and both hands share it (`render.hand` says which one is asked). What a mod drew joins the hand's own vertices ahead of vanilla's, in a run of its own with no rendered item id, as the arm's: it gets the hand's field of view and the view bob, a shader pack's hand pass draws it, `hand_drawn` sees it, and it stays when the hand is cancelled. `HandDrawContext::render_arm` submits either physical arm with the local avatar’s effective skin, model width and that side’s sleeve, under the current draw pose and fixed light. Explicit submission does not fire `render_arm`. So cancel and draw to replace a hand, or draw without cancelling to add to it; it covers a hand that holds an item, where `render_arm` (vanilla's arm only) never fires, and the empty off hand, for a two-handed pose's second arm. Unlike NeoForge's `RenderHandEvent`, the pose stack is `draw`'s own: turning it moves what the mod draws, `render`'s pose moves the default geometry, and player render state is not exposed.

`screen_opening(current, new, outcome)` (`ScreenEvent.Opening`), `screen_closing(screen)` (`ScreenEvent.Closing`) and `screen_init_post(screen)` (`ScreenEvent.Init.Post`) take a `ScreenView`. Every change of the game's screen (all of the game's screen assignments) goes through `Game::set_screen`, which mirrors `Gui.setScreen`: when a different screen would open, `screen_opening` fires first with the screen that is open (`current`) and the one about to open (`new`; the id `playing` is vanilla's null, no screen), and answers `ScreenOpening::Cancel` (the current screen stays), `ScreenOpening::ModScreen` (the answering mod's own screen opens instead, if it can open there: over the world or the title's screens, not over a book, a prompt or a loading world) or `ScreenOpening::Vanilla(ScreenTarget)` (another vanilla screen that takes no arguments opens instead, as it opens on its own: `Pause`, `Options`, `Inventory`, `Advancements`, `Stats` and `Chat` in a world, `Title`, `WorldSelect`, `WorldCreate`, `Multiplayer`, `AddServer` and `DirectConnect` on the title; a target that can't open there, a title screen in a world or a world's screen on the title, keeps the screen asked for and the game logs it); the replacement fires nothing again; the first mod to answer wins, and the dispatch records its id, so a mod can open only its own screen. Then `screen_closing` fires for the screen being left (never for `playing`), and the new screen opens. The same screen again fires nothing, and a game where no mod listens to `screen_opening` or `screen_closing` assigns the screen with no event. `screen_init_post` is still observed once per frame (an open or a resize). The screens of the game's `Screen` list (inventory, containers, pause, options, chat, creative, stats, advancements, mod screens, dead, win…) announce themselves in `set_screen`, before they change, and honour every answer. The overlays drawn over gameplay (sign editors, book edit/sign/view and lectern screens, dialogs, and the command, link, code-of-conduct and server-pack prompts) open through `Game::open_screen_overlay`, which fires `screen_opening` before the overlay is set and honours the answers in the same way (a canceled server-pack prompt leaves its packs unanswered, a canceled code of conduct leaves the server waiting, as in NeoForge), then `screen_closing` for an overlay it replaces. The title's own screens (`world_select`, `world_create`, `world_edit`, `multiplayer`, `onboarding`…) open in the same way: `Title::set_mode` asks for the screen while a mod listens to `screen_opening` or `screen_closing`, and the game fires `screen_opening` (from the title screen shown) and `screen_closing` for the one left before it applies it, after the window event or control command that asked; `Cancel` keeps the screen shown, `ModScreen` opens the mod's own screen over the title (drawn and fed input by the screen hooks as over the world; closing it returns to the title screen), and `Vanilla` opens another title screen. The multiplayer list's own steps (`add_server`, `edit_server`, `direct_connect`, `server_delete`, `friends_join`, and back to `multiplayer`) are title screens too and go the same way. The title appearing as a whole (at startup, when a world closes, after a disconnect or a failed join) and going (when a world opens) goes through `Game::show_title` and `Game::hide_title`: `screen_opening` (from no screen to the title's screen, or back) and `screen_closing` for what it leaves fire before the change, and `screen_init_post` follows an appearance; a `Cancel` doesn't keep the title shut when there is no world to show instead, nor keep it open over a world that opens; an appearing title honours `ModScreen` and `Vanilla`. Still announced by the per-frame observer, through the same function and in the same order but after the change, so a mod's answer can't keep it shut: an overlay's closing. Overlay views use the full window as their rectangle and have no menu or title; anvil rename remains part of its inventory screen. Dialog states share the `dialog` id. A menu swapped inside the open inventory screen is not a change of screen.

- `client_resources_reloaded()` fires on the render thread after successful client resource-pack application: the atlas, models, renderer and client caches already use the new resources. It covers selection/F3+T and server-pack reloads. Failed staging or application and initial startup do not fire it. This is a completion notification, not a reload-listener registration or staging API.
- `texture_atlas_stitched(atlas)` (NeoForge's `TextureAtlasStitchedEvent`) fires on the render thread once per atlas the resources publish, `TextureAtlas::Blocks` and then `TextureAtlas::Gui`: on the first load before `client_setup`, and before `client_resources_reloaded` on later reloads. The atlas notifications run at the model manager's apply step, before listeners ordered after that anchor. A refused prepare fires nothing, and the old sprites stay right. Sunrise packs vanilla's many atlases into these two: the block sprite arrays (`texture_sprites` answers a `SpriteRef`) and the GUI atlas of GUI sprites, particles and flat item and block textures (`gui_sprite` answers an `AtlasRegion`). Look up the sprites a mod draws here, once, and keep them until that atlas is stitched again; never by name every frame. `TextureAtlas` is `#[non_exhaustive]`. A dedicated server never fires it.

- `client_stopped` fires after the client shutdown finishes. `game_shutting_down` fires after that on clients and after the dedicated server stops, with a side-typed host. Both fire once on orderly process exit. There is no separate `client_started`: `client_setup` covers that moment.

- `client_stopping()` (Fabric's `ClientLifecycleEvents.CLIENT_STOPPING`; NeoForge has no event for it) fires once per game process, on the render thread, when the game shuts down: after the world the player was in is left and saved, with the settings and resources still held, so a listener can read them through the `ClientHost` and save its own client state. Closing the window, a quit shortcut and the stop signals reach it; a crash does not. A dedicated server has no client and never fires it. Listeners are pure functions of the `ClientHost` reads and the mod's own state. Tick-time effect: none, it fires once at exit. Diverges from upstream: new hook.

- `client_setup` (NeoForge's `FMLClientSetupEvent`) fires once per game process on the render thread, after the first client load has applied and before the title screen's first frame. It receives `&mut dyn ClientSetupContext` and `&mut ClientSetupEvent`, not a `ClientHost`; setup can use published resources and client setup operations, but it cannot open worlds or transactions. `client_resources_reloaded` skips initial startup and runs after later reloads; returning to the title or opening another world does not repeat setup. NeoForge's `DeferredWorkQueue` has no counterpart (the listener already runs on the thread that would drain it). A dedicated server has no client and never fires this hook; `server_about_to_start` covers its setup.

- `vanilla_game_event(dimension, event, pos, cause, affected_state, cancel)` (NeoForge's `VanillaGameEvent`) fires on the server for every vanilla game event before the sculk sensors, wardens and allays hear it; canceling keeps them from hearing it. `cause` is the network id of the entity that caused it (`GameEvent.Context.sourceEntity`, None for none) and `affected_state` is the block state the event touched (`GameEvent.Context.affectedState`): the block a player broke or placed, the block a mob stepped on, and None where vanilla passes none (`Context.of(entity)`). Contract version `2`: version `1` had no `affected_state`. With no listener the cost is one `listened` branch per event. Diverges from upstream.

- `get_arm_pose(entity_id, hand, stack, humanoid, pose)` (NeoForge's `IClientItemExtensions.getArmPose`) sets the pose of the arm that holds `stack` in `hand`, as a humanoid model draws it: the window's own player, remote players and humanoid mobs. `humanoid` is a borrowed `HumanoidRenderState` exposing both held stacks, the main arm, use hand, use ticks, use animation and the selected main/off-hand poses before listeners answer (including item declarations). It replaces what the item would give, whatever the use animation, a loaded crossbow or an `arm_pose` declared on the item say (a content mod's data key, see the content-mods how-to, is the answer when no mod listens or none sets one). Return one of vanilla's `ArmPose` values (`empty`, `item`, `block`, `bow_and_arrow`, `crossbow_hold`, `crossbow_charge`, `throw_trident`, `toot_horn`, `spyglass`, `spear`, `brush`); the first mod to answer wins. It is asked once a tick for each humanoid model and non-empty hand, only while a mod listens, and a two-handed pose still moves the other hand as vanilla's does. Custom poses are registered through `Registrar::register_arm_pose` and returned as `ArmPose::Custom`; `arm_pose_transform` writes their API-owned `ArmAngles` in radians. Unknown slots and non-finite angle components are ignored. Mob-specific animations run after the humanoid base pose, as in vanilla. API stack snapshots are converted only with subscribers; there is no allocation when no mod listens. Client-side, with the `ClientHost`. Diverges from upstream: new hook.

- `item_decorations(stack, x, y, batch)` appends custom GUI drawing after the default bar, cooldown shade and count have been painted for an item icon. The stack is the displayed stack, x/y are its icon's top-left in GUI units, the units the batch takes (no item-local translation remains active). Existing GUI helpers can draw badges, text and overlays. The shared path covers the hotbar, carried stacks, menu slots, recipe book and item tooltip icons, and the item icons of the advancement, beacon, stats and trading screens. `item_bar` still controls the bar separately; this hook does not suppress vanilla decorations.

- Using an item over time, as vanilla's `Item.getUseDuration`, `getUseAnimation` and `finishUsingItem` and NeoForge's `LivingEntityUseItemEvent.Start` and `.Finish`. `use_duration` answers a stack's use time: a mod's own item with no use of vanilla's that answers above 0 is used over time on a right click, until it is let go or the time runs out, on single player and LAN alike. `use_animation` changes the item's vanilla use animation, including specialised item poses, each first-person frame and each local or remote player model tick; a spear override without kinetic data is refused. `can_eat` decides whether a food can be eaten now. `living_entity_use_item_start` may change a use's duration or refuse it, on the server and where the window's client predicts the use, and `living_entity_use_item_finish` may replace what the hand holds once the time ran out (consumables, potions and mods' items), on the server and on the client that predicts it. Both take the using entity, None for a player, so mobs can fire them later, and the hand in use. Change the world only where this game runs the world, not on a server's client.

- Items and block state properties for drawing mod block entities: in `block_entity_submit`, `native_draw_item(draw,
  stack, display, owner_yaw, seed)` draws an item stack under `draw`'s pose as a campfire's or a shelf's
  (`ItemModelResolver.updateForTopItem` and `ItemStackRenderState.submit`, in the model's `display` transform; with `owner_yaw`, the block
  entity is the item's owner, as a shelf is, so a compass on it points from there). The client's copy of a block entity has
  no item slots: send the stacks you draw in `block_entity_update_tag` and read them from `entity.nbt`.
  `BlockState::property(metadata, "facing")` reads an owned state's property without a world lookup.
  Host callbacks supply `host.block_states()` as metadata; tint workers receive their explicit
  immutable metadata resource and never need a game host.

- The world's clock for client mods: `native_clock()` is the shown world's game time and time of day in ticks (`i64`; the day time counts on past 24000), none on the title screen.

- The open screen hears of the wheel (`screen_mouse_scrolled_pre`) before it scrolls, as vanilla's `ScreenEvent.MouseScrolled.Pre`: the screen, the scroll's x and y as the game's scroll handler scales them (a line times the wheel sensitivity: with Discrete Scrolling on, a notch is ±1, its sign only; a touchpad's pixels are divided by 30), GLFW's modifier bits (shift 1, control 2, alt 4, super 8) and the pointer in `screen.mouse`. A cancelled turn scrolls nothing: no bundle under the pointer, no list, slider or scrollbar, and no hotbar slot. It fires for the window's wheel (sunrise-ctl's `scroll-at` included), never on the title screen itself (Options and the pack list opened from it do fire), the connect and loading screens, the prompts, books and sign editors drawn over gameplay, or while a world loads or saves, and never for a wheel a gamepad or the keyboard's focus sends.

- Keeping a mod block entity's geometry: `draw.keep()` in `block_entity_submit` keeps what that call drew, and the
  block is drawn the same again without calling `block_entity_submit` or `block_entity_render_bounding_box`
  until its state, its block entity (what the server sent), its light or the resource packs change, so idle machines cost
  no call into the mod each frame. Keep only geometry that reads none of `partial`, `game_time`, the blocks around it or other state of yours:
  kept geometry stands still (an enchanted item's glint, a clock or compass drawn with `native_draw_item`).

- Any command a player types: `player_command(player, name, command, cancel, replies)` is heard before the command is looked at (NeoForge's `CommandEvent`), without its slash, for vanilla's commands, the mods' own and commands the player may not run, once for each command a player sends, on a server. Also each line of a quiet command batch. A mod sets `canceled` to stop it: nothing runs and nothing is answered but the lines it pushes to `replies`, which reach the player as system chat.

- Redstone signals as a block reads them, on a level handle (`LevelRef`) on a server, through the game's own `SignalGetter`, mods' `block_signal` and `should_check_weak_power` parts included: `level.signal(host, pos, direction)` (`getSignal`, how strongly the block at `pos` powers its neighbour on the `direction` side, conducted power included), `level.direct_signal(host, pos, direction)` (`getDirectSignal`, the strong power alone), `level.direct_signal_to(host, pos)` (`getDirectSignalTo`), `level.best_neighbor_signal(host, pos)` (`getBestNeighborSignal`, the strongest power reaching `pos`, as a dispenser or a piston reads it) and `level.has_neighbor_signal(host, pos)` (`hasNeighborSignal`, stopping at the first powered side). Each costs what the game's own call does. They take `&dyn Host` and read the live signal state without a transaction or mutable level borrow. Block signal and weak-power Part callbacks receive the restricted `LevelHost`, so they cannot recursively invoke these signal helpers, just as on the write path. They read the level in place: None (or false) for another level, a stale handle or a client.

- `level.weather(host)` (rain and thunder levels, 0 in the Nether and the End), `level.day_time(host)` (the time of day), `level.height(host)` (min y and height) and `native_max_stack_size(item)`, with `level.brightness(host, layer, pos)` for light and `level.is_raining_at(host, pos)` for rain at a position: what a mod's solar panel, wind turbine, quarry or own container needs without reaching into Sunrise's internals.

- Giving a whole stack (damage, name, enchantments kept) to any player from a server hook: `insert` into
  the player's [`ENTITY_ITEMS`](../concepts/capabilities/entity-items.md) handler places it as `Inventory.add` does, and answers how many went.

- A mod's command executor handles every source of its registered usage. Players retain their swapped-in state; the console has no acting player. Functions are silent, and block/cart feedback follows track-output. Returning `None` fails as an unknown command.

- Player-use drops to `loot_drops`: shears on a sheep, a mooshroom, a snow golem or a bogged, a brush on an
  armadillo or a suspicious block, carving a pumpkin, picking sweet berries or cave vines, and a player taking an item frame or
  its item pass through it, with `LootDrops::used` set and the player named.

- Server experience: `native_experience_orbs(dimension, min, max)` reads the experience orbs in a box; `PlayerRef::give_experience_points` gives points as an orb they touch does (Mending first), and `PlayerRef::give_levels` gives or, for a negative amount, takes levels.
- Server player death location: `PlayerRef::last_death_location(host)` reads where the player last died as a `GlobalPos`, the dimension's key and the block (`Player.getLastDeathLocation`, saved as `LastDeathLocation`, which the recovery compass follows). The dimension is data: a death in a dimension that is not open still reads, and `host.level(&pos.dimension().to_string())` gives the level when it is. `GlobalPos::new` takes an `Identifier` and `dimension()` returns `&Identifier`, so dimension keys are validated before they enter the typed API. Players resolve as in the other `PlayerRef` reads, so the acting player works too. It is `None` for a player who never died, a stale handle, and a player who is not online or whose record is swapped out. `set_last_death_location(host, location)` stores a location or, for `None`, forgets it (`Player.setLastDeathLocation`); the client hears of it with the player's next respawn, as in vanilla. It returns false in the same cases and for a dead player (so inside `player_died` the read works and the write does not).
- Server player rules: `PlayerRef::may_use_item_at(host, pos, face, stack)` is `Player.mayUseItemAt`: a player who may build always may, anyone else only with a stack whose `can_place_on` matches the block `pos` lies against (the check the game's own item use asks), `None` for a player in another level than the one in place. `can_harm(host, other)` is `ServerPlayer.canHarmPlayer`: the `pvp` rule and the two teams (attacks, arrows and tridents ask the same). `respawn_config(host)` reads the respawn point as `Some(Some(PlayerRespawnConfig { position, yaw, forced }))`, `Some(None)` for a player with none (the world spawn), and `set_respawn_config(host, config)` is `/spawnpoint`'s write, `None` forgetting it: the yaw wraps, `player_set_spawn` may cancel (false), and a dead player (except the removed original during `player_respawn_position`), a dimension the game doesn't have and a yaw that is not finite are refused. `text_filtering(host)` is the client's `ClientInformation.textFilteringEnabled` (false for the player running the server); it is the client's own option, so there is no write. The sleep reads are `sleep_ignored(host)` (what `set_sleep_ignored` left), `counts_for_sleep(host)` (the night's count: not a spectator, not a fake, not ignored while out of bed) and `sleep_timer(host)` (`Player.sleepCounter`: 100 once asleep long enough, 101 to 109 on the way up); `set_sleep_timer(host, ticks)` writes 0 to 109 through the sleep writes' guard (no fake player, no guest in another level). Writes take a `ServerHost`.

- Lead drops to `loot_drops`: untying a lead or cutting it with shears (`used`), and the lead a mob a
  player killed while leashed drops on its next tick, naming the killer.

- Native mod particle types to `native_send_particles`: `ParticleOptions::simple(spark_key)` with a load-resolved `ParticleKey` for `mymod:spark` sends a particle of the mod's own type, drawn by Sunrise clients from the sprite set in the mod's `assets/<namespace>/particles/<path>.json` (`"textures": ["mymod:spark/1", ...]`, resolved as `particle/<texture>` in the atlas). The name must be namespaced; a client without that file draws nothing for it, vanilla clients (whose brand isn't Sunrise's) aren't sent it, and the call still returns true. The type behaves as vanilla's `SimpleParticleType` with a sprite set: a plain billboard, no tint, 20 to 40 ticks, its motion the game's default spread (the speed sets its direction, not how fast it goes).

A mod particle may opt into declarative behaviour in that same particle JSON (no new hook API):

```json
{
  "textures": ["mymod:spark"],
  "sunrise:behaviour": {
    "velocity": [0.0, 0.15, 0.0],
    "velocity_jitter": [0.02, 0.0, 0.02],
    "animation": "loop",
    "frame_ticks": 3,
    "gravity": 0.5,
    "friction": 0.96,
    "lifetime": 40,
    "size": [0.1, 0.0],
    "alpha": [1.0, 0.0]
  }
}
```

`animation` chooses how the particle walks through its sprites: `age` (first to last over its
lifetime), `loop` (cycles every `frame_ticks` ticks, 1..200, only valid with `loop`), `random` (one random
sprite, kept) or `first` (the first sprite, kept). Without `animation` the particle keeps the sprite it was
spawned with.

Every field is optional. Omit `sunrise:behaviour` (or use `{}`) to keep the existing behaviour,
including its random draws, unchanged. `velocity` replaces the initial randomized velocity with
an exact vector in blocks/tick, plus the velocity the spawn call passed; omit it to retain the spawn's default spread. `velocity_jitter` adds a random
offset per axis, uniform in `-j..=j` (0..16 each, drawn from the particle's seeded generator, so the same
seed gives the same particle); declaring it alone starts from zero plus the spawn velocity. Each component must
be finite and within -16..16. `gravity` is the vanilla multiplier: each tick subtracts
`0.04 * gravity` from vertical velocity before moving (negative rises; range -100..100).
After movement, `friction` multiplies all velocity components (0..1); normal block collisions
and the vanilla on-ground horizontal multiplier of 0.7 still apply. Omitted gravity remains 0,
omitted friction remains the engine default (0.98), and omitted lifetime remains 20..39 ticks.
`lifetime` is an integer from 1 to 12000; removal uses the existing base tick: it moves that many
times and is removed on the following tick.

`size` and `alpha` are `[start, end]` linear curves over `age / lifetime`, clamped to their
endpoints. Size is billboard half-width/half-height in blocks (0..16), not a multiplier;
alpha is opacity (0..1). The start value applies immediately at spawn, then the curves update
once per tick, reaching the end after `lifetime` motion ticks. Omitted size keeps the random
spawn size; omitted alpha stays 1. Fading particles use the blended particle layer.
Malformed declarations, unknown behaviour fields, non-finite or out-of-range numbers reject the
whole behaviour object with a log warning, falling back to the existing particle behaviour;
the sprite set still loads. Vanilla particle JSONs are unaffected. Resource reloads apply new
declarations to newly spawned particles; live particles keep their spawn-time values.
The engine copies fixed-size numeric data at spawn and performs no new allocations per particle
tick. Declarations support linear size/opacity curves and constant gravity/friction; for per-particle
code, register a provider with `register_particle_providers` ("Particle providers"). A provider replaces the
declared motion (its tick moves the particle) but the declared spawn values still seed it.

`register_particle_groups(context, event)` binds a namespaced `Identifier` (any nonempty
namespace and path, including `minecraft`) to a factory with `event.register(id, factory)` once
at client startup. Duplicate IDs fail client startup without invoking any factory (matching
NeoForge's `RegisterParticleGroupsEvent`, version 3). The factory captures owned data
(`Send + Sync + 'static`) and returns a `ParticleGroup` with its blend and particle types,
not its ID. The engine calls an accepted factory once per particle engine construction.
A sprite reload resolves particle types against those groups without running factories again.


- the open screen hears of a key press (`screen_key_pressed_pre`) and of each character it types (`screen_char_typed_pre`), as vanilla's `ScreenEvent.KeyPressed.Pre` and `.CharacterTyped.Pre`: the screen, vanilla's key name (`key.keyboard.e`; a key with no name fires nothing), whether the press is an auto-repeat, the character, GLFW's modifier bits (shift 1, control 2, alt 4, super 8), and `screen.mouse` for the pointer. A cancelled press goes no further — no focus move, no Escape-close, no inventory or hotbar key, no typing by the key itself — while its release still arrives and the characters no mod cancelled still reach the text box, as vanilla's separate `charTyped` comes for a cancelled `keyPressed` too. A cancelled character drops only that character and never reaches a text box, and the character hook's `field` is a copy of the focused box whose edits the box takes through its own filter, length limit and responders; control characters (Enter, Backspace, Tab, Delete, Escape, a Control letter) are keys and fire nothing. A key's characters fire after the key has acted, as GLFW's char callback comes after its key callback, so a key that closes the screen (the inventory key) reports no character (the closed screen hears none) and one that moves the focus types into the box it moved to. The input method's committed text fires the character hook too. They fire for the window and the control socket's `key` and `type` (not its `\b` backspace), after the fullscreen, screenshot, debug and narrator keys, never on the connect and loading screens, the prompts, books and sign editors drawn over gameplay, or while a world loads or saves.

- `native_push_player([x, y, z])` on a client, added to the local player's velocity as an explosion's knockback is.

- `data_pack_files(files)`, a native mod's own pack files in the data the server loads: while the tables are assembled (a world's first open, and again on every `/reload`), push `(path, bytes)` and they join that load. `path` is the overlay's key, `<kind>/<namespace>:<path>` without `.json`, over the kinds the tables read: `recipe/mymod:thing`, `tags/mymod:block/thing`, `loot_table/…`, `advancement/…`, `trade_set/…`, `villager_trade/…`, `dialog/…`; a file that isn't JSON is skipped with a warning. Asked once per load on both paths, so it is where a mod recomputes them from its config or the world. Contributed files sit below the world's own data packs: a pack's file for the same path still overrides a mod's, and a later contribution overrides an earlier one.

- Biome colours for client mods, for seasons: `biome_colors(biome, colors)` changes a biome's grass, foliage, dry
  foliage and water tints, asked for every biome when a mod calls `native_refresh_biome_colors()`, which rebuilds the
  loaded terrain with them.
- Hidden blocks for client mods (ore scanners, cutaways): the `block_render_visibility` Part method (`state`, `hidden`)
  makes a state draw nothing: the mesher sees it as air, so its neighbours' faces toward it show and it stops hiding what
  is behind it. Its fluid layer and block entity are hidden too; light, collision and the server are untouched. It is
  asked once per state when the client joins a world and again when a mod calls `refresh_render_visibility()`, which
  remeshes the loaded terrain. Attach it to vanilla blocks with `block_part_tag`. With no part answering, meshing is unchanged.

### Screenshots

`Hooks::screenshot` is the client-thread counterpart of NeoForge 26.3's
`ScreenshotEvent`. It runs with a `ClientHost` after F2's GPU readback and before
any file is written. `event.image` is the in-memory frame (`getImage`): RGBA8,
top to bottom, `width * height * 4` bytes. Change the pixels, or replace the
image, and the normal PNG is that image. A buffer whose length is not
`width * height * 4` writes neither file and shows the translated screenshot
failure line. The HDR copy is not this image; it stays the read-back frame.

Return `Flow::Cancel` to write neither the normal PNG nor its HDR companion.
The chat then shows `event.message` when it is set, and otherwise the translated
cancellation line (`screenshot.failure` with `sunrise.screenshot.canceled`).

Change `event.file_name` to a relative path under the player's `screenshots/`
folder (for example `album/view.png`). Missing parent folders are created.
Absolute paths, empty, `.` or `..` components, backslashes, drive prefixes, NUL
and symbolic links, including the HDR destination, are refused with the normal
translated screenshot failure line. The player may make the screenshots folder
itself a symlink. As with other local file operations, this does not isolate a
native mod or defend against a concurrent external filesystem replacement.

`event.message` starts as `None`: after a successful save the game builds the
normal translated, clickable success line using the final file name. Set it to
`Some(Text)` to replace that success line. Save errors always show the failure,
not a listener's success message. A no-op listener preserves ordinary saving.
With HDR enabled the existing tagged 16-bit copy follows the chosen name as
`<stem>-hdr.png`; its encoding and best-effort error handling are unchanged.
The hook does not intercept control-socket captures or automatic world icons.
It adds no tick work; dispatch runs only when an F2 image has been read back.

### Server data reload listeners

Trait mods implement `Hooks::add_server_reload_listeners` (server only), the counterpart of
NeoForge 26.3's `AddServerReloadListenersEvent`. Registration runs on world open, `/reload`
and `/datapack` changes, after pack selection. The `&dyn Host` beside the event gives registry
access at registration, as `AddServerReloadListenersEvent.getRegistryAccess` does. Register an
`Arc<impl ServerReloadListener>`
with a unique `Identifier` and `FileToIdConverter::json("machines")`, or `::new("machines", ".bin")`
for another suffix. The listener's `ServerReloadContext` lends converted IDs and bytes from
`data/*/machines/`, including a mod's packaged files and later selected datapack
overrides. A file `data/other/machines/heavy/press.json` becomes `other:heavy/press`: the directory
and extension are removed, subdirectories preserved, and all namespaces included. It cannot
read assets, other directories, or filesystem paths. Converter directories have no leading or
trailing `/`, or empty, `.` or `..` components. Converter fields and reload-view fields are private.

The prepare context is the pack view only, as vanilla's `ResourceManager`. Like vanilla's
`SimplePreparableReloadListener`, `ServerReloadListener::prepare` parses and validates without
changing live state and returns a `Box<dyn FnOnce() + Send + 'static>` using
`Box::new(move || { /* publish parsed state */ })`. The action owns its prepared state and the
runner consumes it once on the server's owning thread. All listeners prepare before **any** apply
runs. Do not read another listener's new published state in prepare: dependencies order both
phases but do not remove this barrier. Listener actions cannot report an error. A refusal
while preparing world tables comes before any listener applies. As vanilla's
`SimpleJsonResourceReloadListener.scanDirectory`, a listener that parses its files logs and
skips one that will not parse, carrying the rest through. Returning `Err` from `prepare` — or
introducing a dependency cycle — aborts the whole reload: all pending actions are dropped
without running and the old mod data and vanilla function library are kept. A handled
registration error from `add_listener`, `add_retained_listener` or `add_dependency` returns
`Err(ReloadOrderError)` without mutating or poisoning the registry and is logged each time.
A panicking or phase-refused registration contributes no listener declarations or edges.
A panic in prepare or apply logs the mod and listener, shows the mod error toast on the local
client, and skips that listener for the session without stopping the other applies. The game does
not admit that listener's new retained object. An apply action must publish any private state
it owns atomically: the game cannot undo writes to native mod memory.
Wrap the original error in `ReloadResourceError::new(id, cause)` to keep its source chain;
`resource()` borrows the resource ID and `std::error::Error::source` exposes the cause.
`Display` gives just the resource ID. Reload diagnostics walk the source chain once, marking
a circular reference and stopping there. The log and `/reload` reply name the listener, the
resource and the causes. A
failed `/datapack` change also preserves the selected packs on disk.

`event.listeners.add_dependency(first, second)` orders named listeners. `ReloadOrderError` retains the
IDs as typed variants: unknown names fail immediately without latching a failure, and cycles fail
after registration. `VanillaServerListeners::functions()` names Sunrise's function-library
listener, with no synthetic `minecraft:first` or `minecraft:last` boundary nodes or automatic
boundary edges. This anchor and dependency ends use `Identifier`. Mod listeners without a path to
or from a vanilla listener run after vanilla by default, in registration order. Dependencies
before or after `VanillaServerListeners::functions()` order both the real function-file read and
the function-library admission. Use the named anchor rather than its string value. `event.listeners.get_registry().iter()` borrows the registered names in registration
order, starting with the vanilla anchor. `event.listeners.get_graph().iter()` borrows
`(first, second)` name pairs in edge insertion order. These views borrow the registration in
progress: copy only the IDs you need to keep or use after registering another listener or
dependency. `event.listeners.get_last_vanilla_listener()` always borrows
`VanillaServerListeners::functions()` (matching NeoForge's
`SortedReloadListenerEvent.getLastVanillaListener`); the built-in global loot modifier listener
(`neoforge:loot_modifiers`) is registered before mod hooks without replacing it or adding a
pre-wired edge.
The reads give names, not the listener objects NeoForge exposes (a mod keeps its own listener's
`Arc`, and its name is the one it registered it under). `prepare` returns a boxed `FnOnce() + Send + 'static` action. Sunrise does not expose
NeoForge's condition context here.

`event.listeners.add_retained_listener(&key, converter, listener)` also keeps the listener
object for other hooks, the counterpart of NeoForge's `addRetainedListener(key, listener)`: the
caller builds the typed key — `let key: ListenerKey<MyListener> = ListenerKey::new(id)` — and
reads the object back from any server hook through
`host.reload_resources().listener(&key)` — an `Arc<T>` of the live object the last successful
reload installed, as NeoForge's `ReloadableServerResources.getListener`. Registration rules and
prepare/apply semantics are `add_listener`'s. Retention follows the reload: a new reload
registers fresh objects and the store is replaced whole by each successful reload, so a key
held across a reload addresses the new object unless the owning listener was skipped. A skipped
listener keeps its previous object, when one exists. Before the first successful reload, or
for a key whose id or type does not match a registration, `listener` gives `None`.

```rust,ignore
use std::sync::{Arc, RwLock};
use mod_api::{
    self, AddServerReloadListenersEvent, FileToIdConverter, Host, Hooks, Identifier,
    ReloadResourceError, ServerReloadContext, ServerReloadListener,
};

#[derive(Default)]
struct MachineData(Arc<RwLock<serde_json::Value>>);
impl ServerReloadListener for MachineData {
    fn prepare(
        &self,
        context: &ServerReloadContext,
    ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
        let id = Identifier::parse("my_mod:press").unwrap();
        let next = match context.read(&id) {
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|error| ReloadResourceError::new(id, error))?,
            None => serde_json::Value::Null,
        };
        let live = self.0.clone();
        Ok(Box::new(move || *live.write().unwrap() = next))
    }
}
#[derive(Default)]
struct MyMod(Arc<MachineData>);
#[mod_api::sunrise_mod]
impl Hooks for MyMod {
    fn add_server_reload_listeners(
        &self,
        _context: &dyn ReloadContext,
        event: &mut AddServerReloadListenersEvent<'_>,
    ) {
        event.listeners.add_listener(
            Identifier::parse("my_mod:machines").unwrap(),
            FileToIdConverter::json("machines").unwrap(),
            self.0.clone(),
        ).unwrap();
    }
}
```

Reload work only: graph ordering, listing selected files, reading the winning bytes and one
boxed apply closure per listener. No new work or filesystem reads run on the tick path.
The existing post-success server-data notice below is a different moment, not a second
reload registration path.

- `tags_updated(cause)` fires once tags changed (NeoForge's `TagsUpdatedEvent`), on both sides. `TagsUpdateCause::ServerDataLoad { reload }` fires once the server's data tables are built: the recipes, tags, loot and advancements of the world's data packs and of `data_pack_files` are what the helpers read from then on. `reload` is false on a world's first open, before its chunks and players are loaded, and true after `/reload`, once the players were sent the new data. `TagsUpdateCause::ClientPacketReceived { integrated_server }` fires on a client once per set of tags from its server: at login, and for every update in play, on the game thread, with the new tags in the tag helpers. It is where a mod drops caches derived from the data and rebinds what depends on it. In single player both fire, each on its own thread; `cause.should_update_static_data()` is false only for an integrated server's client packet, so data the two share is updated once, on the server. A remote client's packet returns true.

### Client asset reload listeners

Trait mods implement `Hooks::add_client_reload_listeners` (client only), the counterpart of
NeoForge 26.3's `AddClientReloadListenersEvent`. Registration runs when a resource reload is
assembled — the player changed resource packs, pressed F3+T, or a server sent its packs —
after the selection is known and before the reload's assets are staged. The `&dyn ClientHost`
beside the event is the client read host. Register an `Arc<impl ClientReloadListener>` with a
unique `Identifier` and the same converters as the server side
(`FileToIdConverter::json("ui")`). The listener's `ClientReloadContext` lends converted IDs and
bytes from `assets/*/ui/` with resource-pack overrides already resolved: a selected
pack's file wins over the mod's packaged one, and a pack added later wins over both. A file
`assets/my_mod/ui/theme.json` becomes `my_mod:theme`. It cannot read data files, other
directories, or filesystem paths. Files resolve like vanilla resource packs: every file
except `lang/*.json` is delivered as the exact bytes of the winning pack's file, while
language files arrive key-wise merged across the packs, as vanilla loads them.

Prepare and apply semantics, error handling, crash attribution and dependency ordering match
those of [server listeners](#server-data-reload-listeners). Client registry reads return owned IDs.
`ClientReloadListener::prepare` parses without changing live state and returns a boxed
`FnOnce() + Send + 'static` action. All prepares finish before any apply; a prepare error or
dependency cycle aborts the reload and drops pending actions without running them, while a
handled registration error leaves the registry unmutated and does not poison the reload.
A panic follows the same named log, toast, and session-disablement path as on the server.
`VanillaClientListeners::model_manager()` names Sunrise's single modeled vanilla client
reload anchor; `first()` and `last()` alias `model_manager()` (the Sunrise counterparts of
NeoForge's `VanillaClientListeners.FIRST`/`LAST`), with no synthetic boundary nodes or automatic
edges. Only dangling non-`minecraft` listeners are linked after the anchor during topological
sorting (matching NeoForge's `ReloadListenerSort`). Dependencies on that anchor run mod
prepares before or after the actual build, and applies on the corresponding side of
model publication on the game thread. First, the atlas uploads fire `texture_atlas_stitched`
with the previous model store (none on startup). `model_baking_completed` fires in the model
manager's apply once the new store is installed. All applies precede `client_resources_reloaded`.
On the initial load, registration uses an empty host because no game exists yet; a host
read not admitted by the phase gate is a refusal. The first load, F3+T, pack changes and server packs all
respect the same prepare/apply ordering. A failed initial prepare refuses the load,
rather than publishing a generation with its listener state missing. NeoForge's client event offers no
retained listeners, and Sunrise has none: the `Arc` the mod registers with is the listener's
own live state.

```rust,ignore
use std::sync::{Arc, RwLock};
use mod_api::{
    self, AddClientReloadListenersEvent, ClientReloadContext, ClientReloadListener,
    ClientReloadListenerContext, FileToIdConverter, Hooks, Identifier, ReloadResourceError,
};

#[derive(Default)]
struct Theme(Arc<RwLock<serde_json::Value>>);
impl ClientReloadListener for Theme {
    fn prepare(
        &self,
        context: &ClientReloadContext,
    ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
        let id = Identifier::parse("my_mod:theme").unwrap();
        let next = match context.read(&id) {
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|error| ReloadResourceError::new(id, error))?,
            None => serde_json::Value::Null,
        };
        let live = self.0.clone();
        Ok(Box::new(move || *live.write().unwrap() = next))
    }
}
#[derive(Default)]
struct MyMod(Arc<Theme>);
#[mod_api::sunrise_mod]
impl Hooks for MyMod {
    fn add_client_reload_listeners(
        &self,
        _context: &dyn ClientReloadListenerContext,
        event: &mut AddClientReloadListenersEvent<'_>,
    ) {
        event.listeners.add_listener(
            Identifier::parse("my_mod:theme").unwrap(),
            FileToIdConverter::json("ui").unwrap(),
            self.0.clone(),
        ).unwrap();
    }
}
```

Reload work only: graph ordering, listing selected files, reading the winning bytes and one
boxed apply closure per listener. No new work or filesystem reads run on the tick path. The
existing post-success client-resources notice is a different moment, not a second reload
registration path.

- `compute_fov(partial, camera, degrees)` receives the computed world FOV after settings, ordinary FOV effects and zoom, before projection/Nausea. `camera` is its world position and `partial` the interpolation fraction. Mods can narrow or widen the angle; only finite values in 1..=179 degrees are accepted, otherwise the original computed angle is used. It runs for world rendering and near-plane overlays, not just once per frame. Hand/HUD FOV and the title panorama keep their existing paths.

`movement_input_update` (NeoForge's `MovementInputUpdateEvent`) lends the local player's input after sampling, before auto-jump and sprint. `event.input.move_vector` is `[left, forward]`; `event.input.key_presses` holds all seven independent flags, including opposing directions. Vector edits do not rewrite flags, and flag edits do not recalculate the vector. The sprint flag is a key press, so clearing it does not stop latched sprint or veto a vanilla double tap. The server receives the final key flags after auto-jump, not the move vector, and can correct modified movement.

- Mobs drawn by client mods: `render_living_pre(entity, draw, model, canceled)` is asked each frame for every mob and player in
  view, with its id, kind, custom name and the frame's body and head turns and walk cycle; a mod draws a model of its own with the
  block entity drawing helpers under `draw`'s pose (at the feet, turned with the body) and sets `canceled` to skip the rest of its own drawing (its flames and
  shadow still draw).

- A screen of a client mod's own: `native_mod_screen_open(mod_id)` opens one over the world (every mod's own screen has the `ScreenView.id` `mod`: check `native_mod_screen()` is yours; it doesn't pause a single-player game; it opens only on the window's own client, over the world with no overlay up; a gamepad can close it but not click on it), the screen hooks fire over it as over any other, so the mod draws it in `screen_render_post` and takes its clicks, scroll, keys and typing from them, and Escape closes it (`native_mod_screen()` says whose is open, `native_mod_screen_close(mod_id)` closes it); and two drawing helpers beside `native_fill`: `native_quad(batch, points, argb)` and `native_item_model(batch, item, model, at, size)`, an icon drawn with another `item_model`.

- `camera_distance(partial, mirrored, entity, distance)` receives the final scaled maximum distance for a third-person camera, before the existing eight visual-shape collision rays shorten it. `entity` is the camera entity view, including optional living scale/distance and mount scale/distance; this covers player and spectator-mob cameras. `mirrored` identifies front-facing third person. Only finite values in 0..=1024 blocks are accepted, keeping mod requests within the collision-ray budget; other values retain the original distance. First-person views and title panoramas do not fire it.

- `compute_fog_color(partial, camera, fog_type, rgb)` adjusts the final world fog RGB after normal biome, weather and effect calculation and before GPU upload. Camera is its world position; fog_type distinguishes atmosphere, water, lava and powder snow. Only finite components in 0..=1 are accepted; invalid requests retain the original RGB. Alpha and all distance fields are unchanged. Title panoramas skip this hook.

- `mouse_button_pre(button, pressed, modifiers, canceled)` sees real button events on any screen, before normal dispatch. Button and modifiers use the screen hooks' GLFW encoding. Physical button bookkeeping already ran. Cancellation skips normal dispatch and Post. Accepted events call `mouse_button_post(button, pressed, modifiers)` after dispatch, including screen handlers' early returns. Synthetic pad/focus clicks skip both hooks. Existing screen-specific hooks remain.

- A player respawning: `player_respawned(player, name, end_conquered)` is heard once at the end of `PlayerList.respawn`, with the replacement placed (`end_conquered` is true after the credits, when health, food and experience are kept; false after death) (NeoForge's `PlayerRespawnEvent`), on a server, with their state swapped in so the helpers reach them.
- Where a player respawns (`PlayerRespawnPositionEvent`, version 3): `native_player_respawn_position(player, original_level, original_position, missing_respawn_block, from_end_fight, level, position, yaw, pitch, copy_original_spawn_position)`, on a server, once the respawn point is found and before `player_clone` or placement. The listener receives the original player and can change the replacement's destination level, position and facing, or whether its respawn point is copied (default: yes, unless the bed or anchor was missing). The destination the listener chooses is not block-checked and the hook does not fire again. Runs for death and after the End credits (`from_end_fight`), never for ordinary End teleports.

- A client mod's view: `native_raycast(distance)` and `native_view_hit()` (the first block along the view, up to 1024 blocks, in the loaded chunks).
- A client mod's main-hand input: `register_client_item_use` binds a single `ClientItemUseBehavior` to an item key at startup. It consumes the input before the crosshair target, without a use or release packet or a change to living-item use or blocking. Each callback receives the live typed `ItemStack` from its hotbar slot while the same item kind remains there (rebound on tick and release like `LivingEntity.useItem`, or the last valid stack of that kind if emptied or replaced) and the hotbar slot; `release_using` and `on_stop_using` also receive elapsed held ticks. Releasing the use input (including keybinds that open a screen and release key mappings, as in `Minecraft.handleKeybinds`) calls `release_using`, then `on_stop_using`; a reset (switching slots, replacing the item kind, dying or disconnecting) calls `on_stop_using` alone. Disabled owners are not called. Ordinary item use still goes through `ItemPart::use_item` and `ItemPart::release_using` on both sides.

- `render_fog(partial, camera, fog_type, fog)` adjusts final FogData after normal environment/effect/render-distance calculation and Sunrise's fog option, before GPU upload. Title panoramas skip it. The whole request is accepted only when color components are finite in 0..=1, all distances are finite, environmental and render-distance ends exceed their starts, and sky/cloud ends are nonnegative. Negative near planes remain valid (water/lava). Invalid requests retain the original whole fog.

- `screen_key_released_pre(screen, key, modifiers, canceled)` runs before a named key's release reaches an open screen, with the same screen exclusions and GLFW modifier encoding as the existing key-press hook. It does not fire for presses/repeats or unnamed keys. Cancel bypasses screen handling, but physical key removal and mapped keyboard/toggle releases still happen to prevent stuck inputs. Mouse mappings retain their existing gameplay-only gate.

- Music for client mods: `music(music, volume)` changes the track the music manager plays and its volume each
  tick, and `music_layers(layers)` lists looping tracks played over it, each easing to its own volume (stems that
  grow fuller).

- A server's recipes reach its Sunrise clients, as NeoForge's `OnDatapackSyncEvent.sendRecipes` and `RecipesReceivedEvent`: on the server, `native_datapack_sync(player, recipe_types)` takes the recipe types (`minecraft:smelting`, `mymod:crusher`) a mod's client half needs, asked once the joining player exists, after admission and before their recipes are sent; `player` is a required `PlayerRef`. Reload asks once per online player, at most once per player per reload, before re-sending their recipes; with nobody online it is not asked. Configuration tags precede admission; Sunrise sends their final recipes (data packs merged, load conditions held) to that Sunrise client when it joins and after each `/reload`, in payloads within the clientbound cap, and nothing to a vanilla client. On the client the recipe helpers (`native_recipes`, `native_recipes_of_type`) then give the server's recipes of those types in place of the window's own, single player included, and `recipes_received(recipe_types)` says they came. Vanilla recipe serializers send resolved item candidates, not native ingredient predicates; mod serializers send their own writer's output. Unsent types retain their already-loaded local recipes. No type asked: nothing is sent and nothing changes; none asked after a load that asked some: the clients go back to their own recipes and `recipes_received` comes with no types.
- `level.recipe_for(host, &recipe_type, &inputs)` matches through the game's crafting, cooking, stonecutting and smithing matchers, including data-pack recipes of those vanilla types. `RecipeInput` has Crafting, Single and Smithing variants. A `RecipeMatch` carries recipe id, typed output, ordered remainders, experience and processing time. Valid non-matching input is `Ok(None)`; malformed input, invalid items and unavailable levels are typed errors. Mod-owned types return `RecipeLookupError::NotMatchable`: cache `recipes_of_type` once per load/reload and match in mod memory, with no per-tick host crossing. Dynamic custom outputs/contextual predicates stay mod-owned. Lookup does not consume inputs; cache the result until the input changes.

- `input_key(key, layout_key, raw_code, action, modifiers)` (version 2) runs after a real window keyboard event finishes handling, including shortcuts, screen consumption and repeats (`InputEvent.Key`). key is vanilla's name for the physical key (`key.keyboard.a`; `key.keyboard.unknown` for one it doesn't name; `getKey`), layout_key its name for the key the layout produced (`getKeycode`: a letter or digit names itself, so AZERTY's key at QWERTY's W is `key.keyboard.z`; a named key keeps the physical name; `None` otherwise), raw_code the platform scan code, or winit's native code for a key it doesn't identify, so unnamed keys stay apart (`getScanCode`; stable within a platform only), action is `KeyAction::Press`, `Repeat` or `Release`, and modifiers use the screen hooks' GLFW bits, captured before handling. Not cancellable. Foreign-window and OS synthetic focus events skip it. Window callback only: pad, focus and control-socket input are not window key events.

- `mouse_scrolling(scroll, accumulated, buttons, cursor, modifiers, canceled)` fires with a player in gameplay and no open screen/prompt/sleeping UI, after accumulation and before hotbar/spectator handling, including when no whole step has accumulated. scroll is x/y after discrete/sensitivity scaling (touchpad pixels normalized by 30), accumulated is the whole x/y pair, buttons are physical left/middle/right, cursor is the GUI position, and modifiers use GLFW bits. Cancel skips gameplay scrolling; accumulated whole steps remain consumed. Synthetic input follows the same gameplay scroll path; screen-scroll hooks are unchanged.

- `screen_mouse_dragged_pre(screen, button, delta, modifiers, canceled)` runs before screen drag handlers for real nonzero pointer movements with a held button. The most recently pressed button is active until it releases; focus loss clears it. Same screen exclusions and GLFW button/modifier encoding as the other screen input hooks. screen.mouse is the new GUI position; delta is movement in GUI pixels. Cancel skips screen drag handlers while retaining the new cursor position. Synthetic input and hover-only moves skip it.

- The screen render stages (`screen_render_pre`, `screen_render_background`, `screen_render_foreground`, then `screen_render_post`) are NeoForge's `ScreenEvent.Render.*`, on the screens `screen_render_post` covers. `Render.Pre` cancel skips the screen's own drawing and with it Background and Foreground; Post still fires. Background fires after the screen's background panel and before its contents; screens with no panel fire it before their contents. Foreground fires after the contents, before tooltips, the dragged stack and the preedit overlay. Lower client renderers report these boundaries to the game, which dispatches with its client host and draw context. `ScreenEvent.Init.Pre` has no hook: Sunrise has no screen init method to veto or skip; `screen_opening` (`ScreenEvent.Opening`) is the veto point. With no mod subscribed to any of the four, nothing is built per frame.
- Screen input Post hooks do not fire if the matching Pre cancelled. The button Posts (`screen_mouse_button_pressed_post`, `screen_mouse_button_released_post`, version 2) use the press or release's original receiver, even if it changed screens. They fire whether the receiver handled or declined the event and expose its original answer (`was_click_handled` or `was_release_handled`). Press Post carries the same `double_click` as press Pre. Listeners share `result`: `TriState::Default` keeps the original answer, `TriState::True` forces handled and `TriState::False` forces unhandled. The final answer determines whether a press enters double-click history and whether the event can reach gameplay mappings with no screen open.
- Drag and scroll Posts remain notices on the screen still open after handling, whether that screen took the input or not. Key and character Posts remain unhandled-input hooks: `screen_key_pressed_post`, `screen_key_released_post` and `screen_char_typed_post` fire only when no screen handler took the input, and their `canceled` marks it handled. A cancelled key press or release never reaches the game's key bindings. The char Post does not fire on Options and ordinary title screens, whose text is routed in place.

- The preedit hooks (`screen_preedit_pre`, `screen_preedit_post`) are NeoForge's `ScreenEvent.Preedit.Pre` and `.Post`, fired from the window's input method preedit event while a screen is open. Pre cancel bypasses the screen's handler and Post. Sunrise's handler is the focused text field taking the composition, and its real result is what gates Post: Post fires only when no text field took the preedit (no constant). An empty `full_text` is a composition that ended. Nothing fires with no subscriber.
- `screen_render_inventory_mob_effects` is `ScreenEvent.RenderInventoryMobEffects`: once a frame in the survival and creative inventories' effect panels, after vanilla's early-outs (no effects, or under 32 GUI pixels free), with `available_space`, `compact` (vanilla's default is under 120) and `horizontal_offset` to change, and cancel to draw nothing. The answer is a screen-build input: the exclusion areas (`screen_exclusion_areas`) follow it in the same frame, with no lag behind a change, and a second build in one frame reuses the frame's answer rather than asking again; no subscriber leaves vanilla's layout.
- `gather_effect_screen_tooltips` is `GatherEffectScreenTooltipsEvent`: fires for every hovered rendered effect panel beside the survival and creative inventories. `tooltip` starts empty when the panel's information is fully visible, or as the full name then duration for a compact panel or a name cut short. A listener can add a tooltip where vanilla has none, or change, reorder or remove lines; an empty list draws nothing. `effect` is the panel's effect (a `ActiveEffect`). It is separate from `screen_render_inventory_mob_effects`, which picks the layout; hidden panels never dispatch. Screen and effect metadata are built only with a subscriber.


- Knockback a mod can change or cancel through `native_living_knockback` (NeoForge's `LivingKnockBackEvent`): at the
  start of `LivingEntity.knockback`, on the raw strength, before the entity's knockback resistance takes its share and
  its motion changes, with the hit's level host (its blocks, random and own attachments), the pushed entity's id and
  type (`minecraft:player` for a player), the push's `source` (`HurtSource`, named as `native_living_incoming_damage`
  names a hit's, with the same entity ids left at 0; a goat's ram pushes with `minecraft:mob_attack` and hurts with
  `minecraft:mob_attack_no_aggro`, as in vanilla), the `attacker` (the attacking entity's network id, falling back to a
  known last hurt-by mob, or 0 when unknown) and what it holds (`attacker_main` and `attacker_off`: a player's selected
  and off-hand stacks, a mob's main hand; none when unknown, recorded only while a native mod is loaded), the `entry`
  (`Short` via `knockback(power, x, z)`, else `Full`), the `strength`, the direction (`x`, `z`; vanilla points it at the
  source of a hit, the attacker's side, and the entity is pushed the other way) and a `canceled` flag. A mod may change
  the three numbers or set `canceled`, and the push follows: canceled, the entity's motion is left alone. A `strength`
  of 0 or less after the hook is no push, and a direction too short to tell draws a random one after it, as in vanilla;
  the resistance scales the strength the hook leaves, so a full knockback resistance still stops the push. A value a mod
  leaves that is not a finite number is put back as it was. It fires for the players, mobs and armor stands the game
  knocks back: a hit's own push, a sprinting or Knockback hit, a sweep, a spear, a shield's push on its holder and a
  goat's ram. It does not fire for a sulfur cube with a block that a hit with a causing entity knocks back
  (`SulfurCube.knockback` does not call `super` there), for the pushes vanilla doesn't make with
  `LivingEntity.knockback` (the ender dragon's wings, Punch arrows, explosions, wind charges, a mace smash's area, a
  hoglin's throw), nor yet for the ones Sunrise makes another way: the ender dragon's. The hit is in the middle of its
  work: the level host reads and draws random but can't write the entity or the world. It runs on the thread that ticks
  the world (the server thread), so state a mod shares with its render thread hooks needs synchronising.


- `native_edit_blocks(dimension, edit)` for server mods: runs up columns, from a height or a heightmap's, of block states and filters written as commands write them, made at once straight into the chunks of the level being ticked with what `fill ... strict` does for block entities; the chunks changed go to the players whole.

- Column reads for server mods, in the level being ticked: `native_surface_heights(dimension, columns, heightmap)` and `native_read_columns(dimension, columns, heightmap, (below, above))`, the blocks round each column's height as a palette and indexes with each column's biome JSON water colour (`water_colors`). Client-side blended biome tints are read with `BlockAndTintGetter::block_tint` in block tint callbacks or `ClientHost::biome_tint` in other client hooks.

- `experience_orb_can_merge(dimension, into, from, into_value, from_value, can_merge)` is asked for each pair of orbs whose boxes meet, when an orb spawns (`ExperienceOrb.tryMergeToExisting`; `from` is 0, the new orb is no entity yet) and every 20 ticks as an orb scans (`scanForMerges`); `can_merge` holds vanilla's `canMerge` answer, which a mod may flip either way. `experience_orb_merge` follows a yes and may cancel it (its own notes). A merged orb counts as `count` orbs of the keeper's value. `native_set_experience_orb(dimension, id, value, count, age)` writes an orb's `value`, `count` and `age` (each optional; false for a `count` of 0 or an orb that isn't there; signed `value` is preserved, including zero and negative values).

- `block_entity` gives a block a block entity whatever its template: a mod's stone casing or thin part can use any template. The engine removes it when its block is broken or replaced, no piston moves it, and a mod that draws it (`block_entity_submit`) is asked for it.

- Slot rules for registered vanilla and mod menu types: `menu_slot_rules` starts with the slot's own answers and may change both, as a NeoForge mod overrides `Slot.mayPlace` and `Slot.getMaxStackSize`.

- An item's use tick and release: `living_entity_use_item_tick` replaces the ticks left after the may-still-use check. Effects run only above 0, then the counter decrements. On the server, canceling or setting 0 or below finishes the use without release-using; setting 1 finishes in that same tick after effects. A use-on-release item's counter goes negative until release instead. On the client, canceling sets the counter to -1 before decrement and skips effects; the negative countdown continues. `on_use_tick` is what a mod's own item does then (cook, charge). Canceling Start prevents beginning (duration 0 still begins); canceling Stop skips only release-using, while the use still clears. Finish cannot be canceled, but its result stack can be replaced. `release_using` acts with the ticks left; setting `used` starts the item's cooldown. Change the world only where this game runs the world, not on a server's client.

- `native_inventory(uuid, kind)`: a player's inventory (`Player`: 41 slots, main, armour, off hand; `Ender`: 27), live when they are on the server, else from their file, never while they log in, are dead, or have their inventory swapped out. Other guests' live inventories remain available during a turn; `native_move_stack((from, kind, slot), expected, (to, kind))`: one stack moved, checked against `expected`, where it fits whole, or nothing moves (at most one side a file, written first); `native_send_map(uuid, map_id, colors)`: a 128 x 128 map picture sent to a player, for a filled map with that `map_id`. What an operator's `/invsee` and a price chart on a map need.

- A loot table's custom function or condition uses a codec registered by id through `Registrar::register_loot_function_type` or `Registrar::register_loot_condition_type`. Data validates before publication; unknown types refuse the load. Callbacks receive read-only context facts, authoritative tags and the actual evaluation random. Entry, pool and table order is preserved in both loot evaluators.

- A server's levels opening and closing, on the single player server thread and on a dedicated server: `level_load(dimension)` when a level opens, before any of its chunks (NeoForge's `LevelEvent.Load`), and `level_unload(dimension)` once it has closed, after its last save (`LevelEvent.Unload`). The Overworld, Nether and End fire `level_load` in that order during startup, before `server_started`. The Nether and End can still close when idle and fire `level_load` again when reopened. Every level still open closes when the server stops, before `WorldCloseFinal`, or when its open or its loop fails, so each load has one unload.

- The client's level made and dropped, on the window: `client_level_load(dimension)` when the client makes a level, on joining a server (single player's own included) and on changing dimension (NeoForge's `LevelEvent.Load` from the `ClientLevel` constructor), and `client_level_unload(dimension)` when it drops one, replaced by another dimension's (told after the new one's load, as `Minecraft.setLevel` does) or on leaving the server (`LevelEvent.Unload`). Each load has one unload, except when the game itself quits.

- A projectile's impact, which a mod can cancel, through `native_projectile_impact` (NeoForge's `ProjectileImpactEvent`): it fires when a projectile has found what it meets, before the hit acts, with the projectile's id, type (`minecraft:arrow`) and owner, the level, and what it hit: an entity (its id and type) or a block (its position and the face's normal) and the point. Set `canceled` and the impact does not happen: no damage, knockback, sound or block reaction, an arrow does not stick, a pearl does not teleport, a potion or a fireball does not burst. The projectile flies on if the mod only cancels. Resolve its live handle with `host.entity(event.projectile)` and call `EntityRef::discard` to consume it. A removed projectile cannot apply or replay the hit, even if another listener clears cancellation. It covers arrows, tridents, thrown items, fireballs, wither skulls, wind charges, llama spit and shulker bullets, on the single player server thread and on a dedicated server. Version 2 includes flying fishing bobbers and fireworks, never misses or landing on water. Listeners keeping the earlier family set filter out `minecraft:fishing_bobber` and `minecraft:firework_rocket`. A canceled bobber skips hooking and hit-point redirection but its own move still stops on solid blocks; it notifies before vanilla's entity deflection and world-border bounce. Rockets notify after the deflection decision; cancellation skips their impact explosion, without stopping their normal lifetime. Other projectile families still omit breeze deflections and hits on wind charges. A mod hooks `hears_projectile_impacts(hears)` too and sets `hears`: without it the server keeps no copy of its flying projectiles and the impact hook is not called.

- An entity getting on or off a vehicle, which a mod can cancel, through `native_entity_mount` (NeoForge's `EntityMountEvent`): the rider's id and type, the vehicle's id and type, whether it is mounting, and a `canceled` flag. Destructive removal asks the departing rider to get off first, then asks its direct passengers in passenger-list order. The removed entity no longer has a valid handle or selector result during those calls. Cancellation keeps an edge to a removed boat, cart or mob. A resident Mob retries before its next eligible entity tick, after the tick's cancellable Pre event; a server player retries on its next vehicle tick. A rejected, nonresident passenger does not autonomously retry. Explicit later dismounts ask again. A mob riding a player is compulsorily ejected on save, logout, death or dimension change; the dismount notifies the hook but ignores cancellation. Loading a save does not notify. On the single player server thread and on a dedicated server.

- `interaction_key_mapping_triggered(key, hand, attempt, swing_hand, canceled)` fires while playing when an interaction key is about to act on the crosshair (NeoForge's `InputEvent.InteractionKeyMappingTriggered`): `key` is `InteractionKey::Attack` (when an attack is allowed, and each tick a held key keeps digging a block), `Use` (once for each hand tried, main first, before the hand's target and item) or `PickBlock` (only with a block or entity aimed at). `attempt` distinguishes a queued press from a held mapping. Set `canceled` to skip the action; a cancelled use ends the click, so the other hand gets no turn. Attack and use honour `swing_hand` (true on entry) as a swing request independently of cancellation: false suppresses natural swing requests, cancellation with true requests a swing without acting, and uncancelled true preserves natural no-swing outcomes. A request does not guarantee a new animation: an item can veto it, and an animation already in progress can prevent a new swing from starting during held mining. Pick only vetoes its packet and never requests a swing. The window client on any server, including single player.

- `key_mapping_pre(mapping, pressed, canceled)` is asked while playing, before the game acts on a physical key or mouse button bound to a gameplay mapping (`key.jump`, `key.attack`...), once for each mapping bound to it. `pressed` is true for a press (a key's repeats too) and false for its release. Set `canceled` on a press to drop it for every mapping bound to that input: the mapping is not held, nothing acts on it, and a mouse press is not counted as a click. A cancel on a release is ignored, so what the game holds is always let go. A release is asked too, while playing: a screen that opens lets go of what it holds without asking. A key bound to two mappings (forward and sprint) is canceled as one. It is asked for the window's keys and mouse buttons, agent control's included, after the window's own shortcuts (fullscreen, screenshot, debug keys) and with no screen open. A controller's presses are not asked, nor are `VanillaKeyMappingRef::set_down` and `VanillaKeyMappingRef::click`: they change a mapping's held state or clicks, not a physical key or button.

- `gui_layer_pre` and `gui_layer_post` fire around each dispatched vanilla or mod HUD slot. Paint below the slot in Pre or above it in Post. Returning `Flow::Cancel` from Pre skips that slot's body and Post, including any vanilla bookkeeping and audio in that body: `minecraft:player_health` owns the heart blink and `minecraft:air_level` owns the bubble state and pop sound. Vetoing one slot does not veto its neighbours. `register_gui_layers` installs, wraps or replaces bodies at client startup. A wrapper's `draw_previous()` paints into the same batch and doesn't fire these hooks again; omit the call to skip the previous body. Replacing or wrapping a body keeps the slot's ID, order and dispatch. The status slots fire even under F1, in creative or off a mount; the vanilla visibility checks apply to the original body only, including when a wrapper delegates to it.

- `map_decorations(map, decorations)` is asked each time a map is drawn (held, in an item frame, or on the cartography table's screen) with `MapInfo` (id, centre block, scale, dimension, locked, whether it is in a frame); push `MapDecoration { sprite, x, y, rot, shown_on_frame }` (an atlas sprite, a position in half map pixels from -128 to 127, a rotation 0 to 15, and whether it shows in frames and on the screen like vanilla's markers) and it is drawn after the vanilla decorations with the same pose; a sprite the atlas lacks is left out. Nothing is saved in the map and vanilla's decorations are not passed in. This is the client half of NeoForge's `MapDecorationType`; types registered on the server and saved in map data need code-side registration.

- A player's client starting and stopping tracking an entity, on the single player server thread and on a dedicated server: `player_start_tracking(player, name, entity, uuid, kind)` when the entity is sent to the player (NeoForge's `PlayerEvent.StartTracking`, vanilla's `ServerEntity.addPairing`) and `player_stop_tracking(...)` when it stops (`PlayerEvent.StopTracking`, `removePairing`): out of range, gone from the level, or the player left, changed level or respawned. They name the entity by ID, UUID and type and run with no player's state swapped in, so a mod sends what the player needs with `PlayerRef::send_payload`, and it reaches the player right after the spawn bundle or the removal packet. Each start has one stop. Both are heard in the tick's tracking pass (version 2), once the pass has built every player's packets, because the pass builds them on several threads: a listener sees the player tracking every entity the pass paired, not only those paired before it. A leaving player's stops follow `player_logged_out`; a respawn's or a level change's come at the next pass. While they run the engine is walking its tracker, so a listener reads the game and sends payloads but doesn't spawn, remove or teleport entities.

- An energy buffer on a mod block's block entity. `r.block_energy(&block, buffer)` registers a `ModEnergyBuffer` as `SimpleEnergyHandler` is built: its capacity, the most one insert and one extract moves, and the faces a fill may come through and a drain may leave by. The amount lives in the entity's saved compound as the int `energy`, so it survives restarts and the block's own tick reads it there. The game's own energy capability and registration-time type coverage share that declaration (see "A block's energy").

- Data maps (`RegisterDataMapTypesEvent`). `native_data_map_types(types)` pushes the types a mod registers: a namespaced `id` (`namespace:path`) for a `registry`, any the game names; asked once per load of the server's data (a world's first open and every `/reload`). The values come from `data/<namespace>/data_maps/<registry folder>/<path>.json` (the registry's path for a `minecraft` registry, `<namespace>/<path>` otherwise) in the world's data packs and from files a mod pushes through `native_data_pack_files` under the key `data_maps/<namespace>:<registry folder>/<path>`: `{"replace": false, "values": {"minecraft:coal": ..., "#minecraft:logs": ...}, "remove": [...]}`, an entry's value being the data itself or `{"value": ..., "replace": false}`, optionally with its own `neoforge:conditions`; every entry value is converted to NBT before applying `replace`, inserts or `remove`, so a file with an unconvertible value is skipped atomically with an error log; an id the registry doesn't hold is skipped with one warning per file; `replace` clears what earlier files set, a tag gives every entry in it the value, `remove` applies after the file's own values, and a newer value overrides an older one. Push a typed map (`DataMap`: its id, its entries' registry and how a value decodes) with `DataMap::ty`. A key's `data(host, &map)` returns an owned decoded value. `data_map_value(host, registry, entry, map)` borrows a raw tag, and `data_map_entries(host, registry, map)` borrows the whole slice in entry ID order. Both raw reads borrow from the host; copy only what must outlive that borrow. A point read returns `Ok(None)` when the map gives the entry no value; a known empty map yields an empty slice. Missing publication and an unknown map type are errors, not absence; `DataMapError::Undecodable` is returned when a typed `DataMap::decode` refuses a tag. Server side only and not synced to clients; advanced mergers are not here yet. The six builtin mob and item maps are registered by the engine in the same store with canonical codec NBT values, including Float distances and Int frequencies. This adds no query key families; remaining query coverage is deferred to #4199. `native_data_maps_updated(registry, cause)` (NeoForge's `DataMapsUpdatedEvent`) fires once for each registry that has a type, in registry id order, right after `native_tags_updated`'s `ServerDataLoad`, on a world's first open and after every `/reload`, with `DataMapsUpdateCause::ServerReload`: the place to build caches from the new values. `DataMapsUpdateCause::ClientSync` is for a client applying synced data maps, which does not exist yet.

`living_fall(context, entity, kind, distance, multiplier, cancel)` corresponds to NeoForge's `LivingFallEvent` at `LivingEntity.causeFallDamage`. Mods may change the distance and damage multiplier or cancel fall handling; cancellation does not reverse physical movement. This free function receives the entity ID and type and cannot call game helpers. Player landings, mounted player falls and non-equine mob falls are wired. Equines use their own fall-damage override rather than `LivingEntity.causeFallDamage`. `context` is the loop context of the side that ticks the entity; the hook fires inside the landing, so it takes no `Host` (a deferred call would reorder the fall damage and block effects). `ender_entity_teleport` takes the context the same way, and so does `living_jump`, which fires inside the jump. `farmland_trample` takes the server host in v3 (see its own entry). Contract version `2` for `living_fall` and `ender_entity_teleport`: a mod written for `1` takes the context as a new first argument of the `Hooks` method.

`living_jump(context, entity, kind, velocity)` listens for NeoForge's `LivingEvent.LivingJumpEvent` at the end of `LivingEntity.jumpFromGround`, once per jump of a player or a mob, when the jump power is above vanilla's `1.0E-5` threshold. It receives the entity ID and type and `velocity`, the entity's motion right after the jump (`[x, y, z]`, blocks per tick). Like NeoForge's event it cannot be cancelled; the velocity is read-only (a write to `event.velocity` is not applied), because a NeoForge listener changes the jump by setting the entity's motion, which needs a `Host` this hook does not have yet. Jump height is edited with `living_jump_power`. The player's jump fires before its jump statistic and exhaustion, as `Player.jumpFromGround` calls its parent first; a rabbit's and a sniffer's fire before their own push from standing. The player motion and mob ground-jump paths, including rabbits, cubes and ridden horses and camels, are wired; a mod's simulation of a server player's tick (`native_simulate_player_move`) does not fire it, the server's own jump of a guest does. Contract version `3`: `LivingJumpEvent` gains `velocity`; mods rebuild.

`living_heal(context, entity, amount, cancel)` corresponds to NeoForge's `LivingHealEvent` at `LivingEntity.heal`. The amount can be changed and cancellation prevents the heal. It covers player effect, food and peaceful regeneration healing and the wired mob healing paths. Other healing paths and client-side coverage remain unverified. `entity` is the affected player or mob ID. `context` is the loop context of the side that ticks the entity; the hook fires inside the health update, so it takes no `Host` (a deferred call would reorder vanilla's regeneration timers). Contract version `2`: a mod written for `1` takes the context as a new first argument of the `Hooks` method.

`living_drown(context, entity, kind, is_drowning, damage_amount, bubble_count, cancel)` asks on the server when a player or mob cannot breathe and has no air (NeoForge's `LivingDrownEvent`, `LivingEntity.baseTick`); water animals, axolotls and nautiluses are asked too, from their own air logic, and show no bubbles. Mods may change the drowning decision, damage and bubble count, or cancel. Damage at or below zero does not hurt. The normal eight bubbles use the entity event; other positive counts are sent in a particle packet, and nonpositive counts suppress bubbles.

- A player's or mob's breathing, on the single player server thread and on a dedicated server: `living_breathe(host, event)` every tick for a player and for a mob (NeoForge's `LivingBreatheEvent`, vanilla's `LivingEntity.baseTick`), with Sunrise's answers as the defaults: set `can_breathe` and how much air `consume` takes or `refill` gives (up to `entity_max_air_supply`); at -20 the entity is set to 0 air and takes 2 drowning damage. Version 3 gives the listener the server's host and the entity as a `LivingRef`, so it can read the air (`entity_air_supply`), equipment and blocks, edit items through `ItemPlace::edit` and grant criteria, in one transaction with the air update: if the entity is gone, dead or has other air when the listener returns, the update is refused (the air is never recomputed) and the transactional writes are rolled back; a granted criterion stays granted. Axolotls, nautiluses and `WaterAnimal`s are asked with their own air logic's answers (full air where they breathe, 1 lost elsewhere) and dry out or drown at -20 by it.

- Blocks and items a native mod declares in code, as NeoForge's `DeferredRegister.Blocks` and `.Items` drained by `RegisterEvent`, instead of one content file each: `blocks(context, blocks)` and `items(context, items)`, asked once at start on every side, take `mod_api::Block` and `Item`, the declarations a content mod's `data/<id>/sunrise/block/` and `item/` files hold (a vanilla template and what differs; a block has its item unless `item` is false, in code as in a file). Ids are in the mod's namespaces or in a content package it serves (`served_by`). They join the content mods' blocks and items, inside the content mod whose id is that namespace when it is on (its models, textures, loot and recipes), else on their own with the template's look, so their network ids, saves and the joining client's content check are the files' (docs/mods.md). They pass the files' checks; a namespace whose declarations don't is logged and left out.

- `entity_tick_pre` and `entity_tick_post` observe the entity families described in the hook catalog on both logical sides. Pre and Post carry matching `entities` and `kinds` arrays. Push a current ID onto `canceled` in Pre to skip the corresponding tick work and Post. Individual delivery applies to minecarts, boats and general rows and to observed types requested through `entity_tick_each`; other calls are batched by family. The server's player entity tick is separate from the player connection body. The local client avatar is not observed by these hooks. Set `listened` in `entity_tick_listened` to enable delivery.

- A mob's, an armor stand's or a player's equipment changing, on the single player server thread and on a dedicated server: `living_equipment_change(entity, kind, slot, from, to)` once for each slot (`mainhand`, `offhand`, `feet`, `legs`, `chest`, `head`, `body`, `saddle`) that holds another stack than the mods last heard (NeoForge's `LivingEquipmentChangeEvent`, vanilla's `LivingEntity.collectEquipmentChanges`). A mob new to the level counts from empty on its first tick. A mod that hooks it also hooks `living_equipment_listened(listened)` and sets `*listened = true`: otherwise the game does not read every mob's and connected player's equipment each tick, and only armor stands are told. Players connected to the server (a dedicated server's, a published world's guests) are told the same way with `kind` `minecraft:player`, for their six slots `mainhand` (the selected hotbar slot, so a hotbar selection counts), `offhand`, `feet`, `legs`, `chest` and `head`; their `body` and `saddle` are never told. The world host's own player isn't told here: `on_equip` hears it. It only hears.

- `render_player_pre(player, who, draw, model, canceled)` fires once a frame for each other player in view and for this window's player in third person, before its `render_living_pre`, with the same frame (`MobRender`, a `BlockEntityDraw` at the feet turned with the body) and `who` (profile name, whether it is the local player); set `canceled` to skip the player's own drawing this frame (its flames and shadow still draw) (`RenderPlayerEvent.Pre`). `render_living_post(entity, model, draw)` and then, for a player, `render_player_post(player, who, model, draw)` fire once a frame after the vanilla model and layers of every mob and player in view whose pre hooks weren't canceled (but a player whose `render_living_pre` alone was canceled still gets `render_player_post`, with no model), and what the mod adds in that frame is drawn with it (`RenderLivingEvent.Post`, `RenderPlayerEvent.Post`; layers of one type's renderer are `register_entity_layers`, `EntityRenderersEvent.AddLayers`). The draw starts in the entity's frame; the optional model lends the final poses and hierarchy to anyone attaching geometry to bones. The game asks once a frame whether any mod listens to the four, and with none builds nothing for them.

- A player finishing a sign: `native_sign_changed(player, name, dimension, pos, front, lines, cancel)` is heard when a player sends the text they wrote on a sign they were editing, right before it goes on the sign, once the checks have passed, on a server, with the player's state swapped in (`PlayerRef::tell` reaches them). A mod may rewrite `lines` (four lines of plain text) or set `cancel` so the sign keeps its old text; a waxed sign is not told.

- Models no blockstate names, as in NeoForge's `ModelEvent.RegisterStandalone`: `standalone_models(context, event)` runs once at client startup (not on integrated or dedicated servers). Call `event.register_model(id)` for a plain file, or `event.register(id, dependencies, baker)` to assemble declared model files during resource loads. The baker owns its captures (`Send + Sync + 'static`) and may run on a resource worker; the `StandaloneModelBakeContext` is borrowed only for that invocation. Use its `append_dependency(id, quarter_turns, uvlock)` to append quads in call order with the blockstate baker's centered negative XYZ quarter-turns and inverse-face UV locking. A failed `append_dependency` leaves the assembly unchanged and returns `Err`; the baker's returned `Result` controls whether the model is published (`Ok(())`) or discarded (`Err(_)`). A missing dependency or `Err` return drops the result for that load and logs the registering mod, not the source namespace, and the baker runs again on later resource reloads; only a panic disables the callback for the session. A successfully baked file wins over a baker with the same ID; otherwise the first successful baker wins. Resolve each result's `Identifier` through `host.render_assets().model(&id)` in `model_baking_completed`, retaining its opaque `ModelRef` between frames. `BlockEntityDrawContext::model(model, tint)` consumes that handle under the draw pose in `block_entity_submit` or `render_living_pre`, with `tint` (`0xAARRGGBB`) on tinted faces, alpha included; use `0xFFFFFFFF` for no tint. Missing or unbaked names return `ModelError::UnknownModel`. Successful resource replacement invalidates old handles with `ModelError::StaleModel`; failed reloads keep them usable. Drawing errors append no geometry. Lookup never reads or bakes files, and results are quads rather than arbitrary mod-defined baked types.

- A player opening and closing any menu on a server, on the single player server thread and on a dedicated server: `player_container_open(player, name, dimension, menu_type, pos, menu)` and `player_container_close(...)` (NeoForge's `PlayerContainerEvent.Open` and `.Close`), with the menu type's name and, for a block's menu, its position. They run with the player's state swapped in. Unlike the viewer count of a mod container (`mod_container_viewers`) they name the player and cover every menu: chests, furnaces, crafting tables, trades, a mod's own. The open is heard synchronously after construction, the open packet is queued and full contents sent, with the new menu already current; failed construction and the player's own inventory are excluded. Even inside one tick, open precedes close; replacement closes the old menu before the new open. Each open has one close, also when the player leaves. The synchronous timing is unchanged; version 2 adds `menu: Option<MenuRef>` during open and names the type string `menu_type` on both notices. Crafting tables, merchants, workstations, lecterns and mount inventories have no handle.

- A player starting or stopping flight: `player_toggle_flight(player, name, flying, cancel)` is heard once for each change a player's client makes to its flying (vanilla honours it only for a player who may fly), on a server, with the player's state swapped in. A mod sets `cancel` to refuse it: the player keeps the state they had and the server sends their abilities back, so the client lands or flies again. Only what a client asks for is heard, not the flight a game-mode change starts or ends, and not a LAN host's own player.

- `native_physical_window_size()`, the window's width and height in physical pixels, independent of GUI scale. It returns no size on a server or when no window exists.


- A player changing dimension: `player_changed_dimension(player, name, from, to)` is heard once after a cross-dimension portal or command teleport lands, with `from` and `to` as dimension ids and the player's state swapped in. It is server-only. Same-level moves and replacement by `PlayerList.respawn` are not heard; use `player_respawned` for death or the return after the End credits.

- `pickup_xp(player, dimension, orb, value, count, canceled)` runs before a server player takes an orb; with no listener the server skips swapping a guest in for it (the shared `listened` probe). Canceling leaves the orb and the pickup delay unchanged; otherwise the signed value is preserved, including zero and negative values, and the count is clamped to at least one, then one count is taken. It runs on the integrated and the dedicated server, guests included.

- Whom an experience orb follows (`XpOrbTargetingEvent`): `xp_orb_targeting(orb, scan_distance, following)`, on a server, for each orb as it ticks. `following` starts as the nearest living player within `scan_distance` (8) blocks who isn't a spectator, or `None`; set another player, or `None` to follow no one. The orb goes only for a living player who isn't a spectator and is in its level, and moves only once they are within 8 blocks. An orb that follows no one gets vanilla's push out of a block it is stuck in. Sunrise looks for the nearest player on every orb tick, where vanilla keeps the player it follows while they stay within 8 blocks, so the hook is asked every tick for each orb. Clients still predict the orb's flight towards the nearest player; the server's position wins.

- Where a loot roll happens, for a mod's own loot types: `LootTypeContext::snapshot().place`
  brings `LootContextParams.ORIGIN` (a broken block's centre, a dead mob's position), `BLOCK_ENTITY` (the broken
  block's entity as it was, saved with its id, position and `Items`) and the level (its dimension, game time, time of day, rain and
  thunder), on a block broken by a player or an explosion and on a mob's death loot; None on the other rolls. The game sets it
  around a roll with `with_native_loot_place`.

- How a mod block ignites on either side and burns on the server. `fire_odds(dimension, state, pos, face, odds)` answers
  `FireBlock`'s flammability table (`FireBlock.setFlammable(block, igniteOdds, burnOdds)`) before vanilla's table, for the block at `pos` seen from its side `face` (the one facing the fire, or the air fire may take): put
  `(ignite, burn)` in `odds` for your own blocks. Ignite odds above zero let fire stand beside the block and spread
  to the air around it (planks 5, wool 30), so a face with ignite odds of zero isn't flammable; burn odds are how fast fire destroys it (planks 20, wool 60); `(0, 0)` never
  burns; negative answers become zero, burn odds cap at 300 and spread arithmetic saturates. A waterlogged state never burns and isn't asked. It runs hundreds of times per fire tick: keep it short. `is_fire_source`, `ignited_by_lava` and `on_caught_fire` are the block's other fire questions.
  `smoke_source(dimension, state, smoke)` answers `CampfireBlock.isSmokeSource` for the block under a campfire,
  as the campfire is placed or that block changes: `Some(true)` makes the campfire a signal fire, as a hay bale does.
  Answer only for your own blocks and leave the answer empty otherwise; the level is read-only. It is asked on both sides,
  including the client's placement prediction, with a typed `LevelSide`: answer from the state and level alone.

- A server player's game mode about to change: `Hooks::player_game_mode_changed` receives a `PlayerGameModeChangedEvent` before the change (NeoForge's `PlayerChangeGameModeEvent`) for the `/gamemode` command, the F3+F4 switcher and a hardcore world's respawn into spectator, once for each change to a different mode, on a server; not for the default mode a player joins with. Its `from` and `to` are `GameMode` values, the same type used by command parsing and `PlayerRef::game_mode`, `set_game_mode` and `previous_game_mode`. Write `*event.to` to redirect the change, or return `Flow::Cancel` to keep the old mode; numeric ids are only for network and save boundaries.

- The server's difficulty changed, from one of `peaceful`, `easy`, `normal`, `hard` to another, through `difficulty_changed` (NeoForge's `DifficultyChangeEvent`). It is vanilla's `MinecraftServer.setDifficulty`, which `/difficulty`, a dedicated server's management and a test environment's difficulty all go through, and it is told only when the difficulty is another one. It only hears: the change has already happened.

- A player's fishing rod: `player_fish(player, name, state, cancel)` is heard for the cast (`state` 0), a fish biting (1), the reel-in with a catch (2), the reel-in with nothing on the line (3) and the reel-in of a bobber hooked to an entity or item (4), for a player of a server (Bukkit's `PlayerFishEvent`; not NeoForge's `ItemFishedEvent`). A mod sets `cancel` to stop a cast, make the fish get away, or leave the bobber out on a reel-in. The player's state is swapped in.

- A player took a crafting grid's result, through `item_crafted` (NeoForge's `PlayerEvent.ItemCraftedEvent`): the player's uuid, the recipe the grid matched (`minecraft:oak_planks`), the result stack it took, with its components, and the grid's cells row by row before the take used them. It only hears; the take has already happened. It fires for the crafting grid of an inventory or a crafting table, on the integrated server and on a dedicated server, guests included, and not for a stonecutter's, a smithing table's or a furnace's result.

- `native_server_event(name, uuid, message)` posts an `announce` event to a dedicated server's event stream (`GET /api/v1/events`); sunrise-server's Discord bridge relays it, in gold, when `[discord] events` lists `announce` (off by default). False elsewhere and for an empty message.

- `native_state_block(state)` names the block a state belongs to (`BlockState.getBlock()`); a `state_for_placement` part compares it with its own block. On the server, `LevelRef::state_for_placement(host, block, &context)` asks those parts with an already-resolved context, without placing the candidate.

- The helper `native_physical_window_size()`, which returns the window's width and height in physical pixels, independent of GUI scale. It returns no size on a server or when no window exists.

- A recipe ingredient of a mod's own kind asks `ingredient(context, event)` once per occurrence in the owning recipe load, including nested occurrences and slots outside `ingredients`. The first `Some(IngredientDefinition::new(items, test))` wins. The candidate list is deduplicated in first-seen order and kept with the predicate, apart from recipe JSON. No answer refuses the recipe with its file and kind; an answered empty list keeps manual matching without supplying placement candidates. A reload asks again; a remote client does not perform `ServerDataLoad`.

- A frame's two moments, as NeoForge's `RenderFrameEvent.Pre` and `.Post`: `render_frame_pre(host, event)` runs once a frame (in a world or not) after Sunrise's extras and just before the frame is rendered, and `render_frame_post` right after it was rendered; both carry the frame's time as `event.elapsed`, `world_partial_tick` (captured world interpolation, held while paused or frozen), and `partial_tick` (advancing window interpolation). Outside a world both use the window clock. Pre reports the value about to be drawn; post retains the same snapshot even if a listener changes the clock or screen. Client side; a frame with no listener costs one branch.
- Items a mod's own recipe type takes in a station slot. `recipe_property_sets(context, additions)` is asked once per load, after the recipes are resolved, with the `DataLoadContext` of the load that owns them (`origin()` `ServerDataLoad` on a server, `ClientPreload` on a client's own load), and again after `/reload`. Push one `RecipePropertyAddition::new(set, items)` per set to add to (`minecraft:furnace_input`, `blast_furnace_input`, `smoker_input`, `campfire_input`, `smithing_template`, `smithing_base`, `smithing_addition`, `brewing_input`, `brewing_reagent`); the vanilla recipes' items stay in. The furnace shift-click, the smithing and brewing slots and the set a client is sent read the one answer, so the client predicts what the server accepts. A new key in the listener's manifest-owned namespaces or in a content package it serves (`served_by`) creates a custom set, synchronized to clients alongside vanilla sets. Later listeners may extend an existing set. `Host::mod_recipe_property_contains` and the same `LoopContext` query take the set key and an `ItemKey`; a mod menu's `menu_slot_rules` reads that generation for both server acceptance and client prediction. These queries cover mod-created sets only, returning false for unknown or vanilla keys. Invalid keys, new foreign keys, unknown vanilla keys and air are logged and skipped; an item the registry does not know is ignored. The answer must be the same every time.

- The in-world HUD's two moments, as NeoForge's `RenderGuiEvent.Pre` and `.Post`: `render_gui_pre(host, event)` runs once a frame in a world, before any vanilla HUD layer, and a mod that returns `Flow::Cancel` vetoes the HUD pass, including the status slots. Chat and ordinary toasts are also vetoed; mandatory mod-error toasts still draw unless the player hides them. Unlike F1, that veto skips those slots' installed bodies and layer events. Pre is heard under F1, but not while a server dialog or the connection or loading screen covers the window. `render_gui_post(host, draw, corners)` runs after the boss overlay and before the health and contextual HUD layers. Client side; a frame with no listener costs one branch each.
- Lowering the frame rate limit: `framerate_limit(focused, minimized, in_world, limit)` is asked each frame with the limit Sunrise is about to use (Max Framerate after the inactivity throttle and Battery Mode, 260 for unlimited), whether the window has focus, whether it is minimised and whether a world is open. A mod may lower `limit`, for example to throttle a window in the background as Dynamic FPS does; Sunrise keeps the lower of its own limit and the mod's and ignores values below 1.
- The camera type F5 cycles to: `camera_type_cycle(host, event)` runs when `key.togglePerspective` cycles the window's
  camera type (`Minecraft.handleKeybinds`), with `previous` and vanilla's `next` (first person, third person behind, third
  person in front); a mod may set `next` to skip or keep a type, and the camera entity's post effect follows the answer.
  `camera_type()` reads the type and `set_camera_type(type)` sets it as F5 does, without a cycle, for a freelook mod that
  switches to third person and back; with `camera_pose` moving the drawn camera, that is a camera independent of the
  player. The game's two copies of `CameraType` are now one, `mod_api::CameraType`. Client side.

- A client tick's two moments, as NeoForge's `ClientTickEvent.Pre` and `.Post`: `client_tick_pre(host, event)` runs at the start of every client tick (`Minecraft.tick`), paused or not and on the title screen too, and `client_tick_post` at its end. They replace the single `tick` hook, which also fired on a server's own `Game`: the server's tick is `server_tick` (`ServerTickEvent`), so a mod that implemented `tick` implements `client_tick_pre` and, for a server, `server_tick`. Window only, client side; a tick with no listener costs one branch.
- How long a client tick lasts, a "timer": `tick_target_millis(host, event)` gets the milliseconds the window's
  `DeltaTracker.Timer` divides each frame's time by (`Minecraft.getTickTargetMillis`), vanilla's 50 at 20 ticks a
  second or the server's slower `/tick rate`, and may set another, once a frame before that frame's client ticks run.
  25 runs the client twice as fast, 100 half as fast, still at most ten ticks a frame; an answer that is not a finite
  number above zero keeps vanilla's. Only the window's clock changes; a server keeps its own rate. Client side; with no
  listener, one branch a frame.

- A player swapping their main and off hand items: `player_swap_hands(player, name, to_main_hand, to_off_hand, cancel)` is heard before the swap (a swap from an open inventory is not heard), for a player of a server who is not a spectator, with their state swapped in. A mod may replace the stack each hand gets, or set `cancel` to keep the items where they are; the player's client is sent both slots again.

- A player's food level changing: `food_level_changed(player, name, from, to, cancel)` is heard for a player of a server when a turn of theirs changed it (eating, hunger and saturation effects, exhaustion, starving; Bukkit's `FoodLevelChangeEvent`), before the new level reaches them. A mod changes `to` or sets `cancel` to leave the level at `from`.

- A server player teleported: `player_teleported(player, name, from, to, dimension)` is heard after `/tp` or `/teleport` has moved a player inside their level or into another one (NeoForge's `EntityTeleportEvent`), with their state swapped in. `from` and `to` are their feet before and after, `dimension` the level they are in now. It cannot stop the move; ender pearls, chorus fruit and a spectator's teleport menu are not heard yet, portals and respawns are not teleports here.

- The command tree a server sends a player: `player_commands_sent(player, name, names)` is heard when a player joins and when their permission level changes, with the root command names their client would get. For a player already on their state is swapped in. A mod removes names and that client's tree is sent without them (its chat box neither completes nor colours them); a typed command still runs, `player_command` stops it.

- A joining player the mods may refuse: `player_login(player, name, cancel, reason)` is heard once for each player a server has accepted (name, whitelist, bans and player limit checked), before their saved state is loaded and before they are placed in a level (NeoForge's `PlayerNegotiationEvent`). A mod sets `cancel` to disconnect them with `reason` ("Disconnected" when it stays unset); no join message and no `player_logged_in` follow.

- A server player's sprint: `player_sprint(player, name, sprinting)` is heard once each time a player's client starts or stops sprinting (the sprint key, a double tap forward, letting go) and the server's player has taken it, with their state swapped in. It only hears what the client asks: a sprint that ends by itself (knockback, hunger, a wall) is not reported.

- A resource pack answer: `resource_pack_status(player, name, status)` is heard when a server player's client answers a resource pack in play (`status` is `ServerboundResourcePackPacket.Action`'s number: 0 loaded, 1 declined, 2 failed download, 3 accepted, 4 downloaded, 5 invalid URL, 6 failed reload, 7 discarded), with their state swapped in. The pack offered during the configuration is answered before the player has joined and is not heard.

- A player taking an item from or putting one on an armor stand: `armor_stand_manipulate(player, name, stand_id, slot, cancel)` is heard after the stand's disabled-slot checks and before anything moves, with the stand's entity id and the slot the click reached (`mainhand`, `offhand`, `feet`, `legs`, `chest`, `head`), on a server, with the player's state swapped in. A mod sets `cancel` and neither the stand nor the player's hand changes.

- A player opening one of the game's own menus: `container_opened(player, name, kind, pos, cancel)` is heard when a chest, furnace, crafting table, villager, anvil, lectern, horse inventory... is about to open (the job of NeoForge's `PlayerContainerEvent.Open`, but before the menu opens), after the checks that refuse it, with the menu type and the block's position, on a server, with the player's state swapped in. A mod sets `cancel` and nothing opens. Menus a mod opens (`native_open_menu`) are not heard.

- A creative player setting a slot: `creative_slot(player, name, slot, item, count, cancel)` is heard when the creative screen sets an inventory slot, or throws a stack out (a slot below 0), after the packet passed vanilla's checks and before the slot changes, on a server, with the player's state swapped in. A mod sets `cancel` and the packet is ignored.

- A boat or minecart moving: `vehicle_moved(player, kind, from, to)` is heard once a tick for each boat or minecart that carries a server player (and for one a server player left, for ten seconds after it stood still), with the vehicle's type and its position before and after (`VehicleMoveEvent`), on a server, with the rider's state swapped in. `player` is the rider's UUID, none for a vehicle that carries nobody now. It only hears.

- A server player emptying a bucket: `bucket_empty(player, name, pos, fluid, cancel)` is heard right before the fluid is placed, once the player may build there. A mod sets `cancel` and nothing is poured. The player's state is swapped in, so the helpers reach them. `fill_bucket` hears the same click without the player and the fluid; dispensers and cauldrons are not heard.

- A server player placing a hanging entity: `hanging_place(player, name, kind, pos, cancel)` is heard for an item frame, a glow item frame, a painting and a leash knot (a lead tied to a fence that has none), once it fits and before it is placed or the item used. A mod sets `cancel` and nothing is placed. The player's state is swapped in, so `PlayerRef::tell` answers them.

- TNT about to be primed: `tnt_primed(player, pos, cancel)` is heard before a flint and steel, a fire charge, fire, a redstone signal, an arrow or a dispenser lights the TNT at `pos`, with the player whose click lit it (None when no player did). A mod sets `cancel` and it is not primed: the block stays, or is put back when fire or a signal had already taken it out. The clicking player's state is swapped in. TNT set off by a blast, a TNT minecart and a sulfur cube are not asked.

- A totem saving a player: `player_totem(player, name)` is heard for a server player whose hurt would have killed them and whose totem fired (`LivingEntity.checkTotemDeathProtection`, Bukkit's `EntityResurrectEvent`), just before the totem is used up. Listen only. The player's state is swapped in.

- `anvil_damaged(dimension, pos, from, to)` runs after an anvil at `pos` changes from `from` to `to`; `to` is air when it broke.

- `beacon_state_change(dimension, pos, active)` is called after the beacon at `pos` gains or loses its complete beam pyramid; `active` says whether it is active now.

- A bell is about to ring: `bell_ring(dimension, player, pos, cancel)` is asked before the bell event and sound. Set `cancel` to stop the ring and its raider reveal. `player` is the acting server player when available.

- `block_break_progress_update(player, name, pos, progress)` is called when a guest's crack animation at `pos` reaches a new stage from 0 through 9.

- Fire about to burn a block: `block_burn(dimension, pos, cancel)` is heard when fire's burnout check is about to remove the block at `pos`. Set `cancel` to keep it.

- A campfire finishes cooking `item` into `result` at `pos`, before dropping the result.

- A dispenser equips `item` on the first eligible player, mob or armor stand in front of its block at `pos`, after the item is consumed and the equip sound and game event are applied. `target` identifies the kind of entity equipped.

- A block with a vanilla experience drop is about to award its points at `pos`; `block` is its identifier. Change `experience` to adjust the amount before the orbs are awarded.

- `block_fade(dimension, pos, from, to, cancel)` runs when ice or snow melts, or coral dies. `from` and `to` are block state IDs. Set `cancel` to keep the old block.

- `block_failed_dispense(dimension, pos, item)` is called when a dispenser at `pos` fails to dispense `item`.

- `block_form(dimension, pos, from, to, cancel)` runs when precipitation is about to place ice or snow, or add a snow layer at `pos`. `from` and `to` are block state IDs. Set `cancel` to keep the old state.

- Fire about to be placed: `block_ignite(dimension, player, pos, cause, cancel)` is heard for fire spread, lava, lightning and dispenser flint-and-steel. Set `cancel` to leave the position unchanged. Direct player flint-and-steel/fire-charge placement is not covered.

- `block_lock_check(player, name, pos, locked)` runs when a guest checks the container at `pos`; `locked` starts with vanilla's result and may be changed. The guest's state is not swapped in.

- A sculk sensor, calibrated sensor or shrieker received vibration event `event` at `pos`; `block` is its identifier. Set `cancel` to suppress the block's response.

- A dispenser sheared entity `entity` of kind `kind` at the block in front of the dispenser at `pos`, after the shearing effects.

- A brewing stand finished brewing: `brewing_complete(pos)` is heard after the bottles at `pos` received their brewed contents. Server-side.

- A brewing stand consumed fuel: `brewing_fuel(pos, fuel)` is heard after the stand at `pos` consumed the item `fuel` and gained brewing fuel. Server-side.

- A brewing stand starts brewing after it consumes fuel and sets its timer.

- A system broadcast line is about to be logged and sent to every player: `broadcast_message(line, cancel)` lets a mod rewrite it or set `cancel` to drop it.

- An item was placed on a campfire and began cooking.

- A cauldron's fill level is about to change: `native_cauldron_level_changed(dimension, player, pos, from_level, to_level, reason, cancel)` is asked for rain, snow, dripstone and extinguishing. Set `cancel` to leave it as it was. `from_level` and `to_level` use 0 for empty and 1 to 3 for filled levels. `player` is the acting server player when available. Bucket, bottle and dyeing interactions are not covered yet.

- A player adds a compostable item to a composter at `pos`; the layer has been applied.

- A crafter at `pos` completed `recipe`; item consumption and output handling have finished.

- A player used an item in `minecraft:creeper_igniters` on a creeper, after it entered its ignited state.

- Lightning struck a creeper and set its charged state; the hook reports the creeper id and strike position.

- The first Ender Dragon fight is about to place its egg at `pos`: `dragon_egg_form(dimension, pos, cancel)` runs on `GameLevel`. Set `cancel` to leave the block unchanged.

- A player applied an enchantment: `enchant_item(player, name, pos, item, levels)` is heard after the player at `pos` applied it, with the finished `item` and levels charged. The guest's state is not swapped in.

- Saved entity records for a chunk finished loading into the server world: `entities_load(dimension, chunk, count)` reports the dimension, chunk coordinates and number of records read.

- Entities in a chunk left the active world and were queued for saving: `entities_unload(dimension, chunk, count)` reports the dimension, chunk coordinates and number of records.

- A zombie or raider finished breaking the door at `pos`; the mob id and entity type are reported after the door is removed.

- Two mobs completed a breeding action: `entity_breed(parent, partner, child)` reports both parents and the baby id when one was created; egg laying has no child id.

- A mob changed the block at `pos`; the hook reports the mob id and kind, and the old and new block names after the new state was applied.

- A bee was stored in a beehive or bee nest at `pos`; the hook reports the bee id and kind and the block name, after the bee was removed from the level.

- A mob entered love mode after a player interaction; the hook reports its entity id, kind and feeder UUID when known.

- A server entity finished its portal wait and is beginning to leave through the portal block at `pos`: `entity_portal(dimension, entity, pos)` reports the entity's network id, source dimension and entry block. The transfer destination follows the existing portal rules.

- A guest player's synchronized pose changed: `entity_pose_change(player, name, from, to)` reports the old and new pose names.

- A player fired an arrow from a bow or crossbow: `entity_shoot_bow(player, name, projectile, force)` reports the arrow id and shooting power after the completed shot.

- A mob became tamed after its owner and persistence were set: `entity_tame(entity, kind, owner)` reports the entity type and owner UUID when known.

- A server entity is about to be teleported by `/tp` or `/teleport`: `teleport_command(dimension, entity, from, to, cancel)` (NeoForge's `EntityTeleportEvent.TeleportCommand`) names the level it arrives in, its network id, its feet now and where the command sends it. A mod may move `to` (a value that is not a number is ignored); `cancel` leaves the entity where it is. It fires once per teleported entity, before `entity_teleport` and `player_teleported`, which hear the move after.
- A server entity completed a same-level teleport command: `entity_teleport(dimension, entity, from, to)` reports its network id and the coordinates before and after the move.

- A guest player's fall flying state changed after the server accepted or ended a glide: `entity_toggle_glide(player, name, gliding)`.

- A mob conversion completed through Sunrise’s shared `Mob.convertTo` path; the hook reports the removed entity and its replacement ids and kinds.

- An experience bottle broke, after its water splash and experience orbs were created; the hook reports the impact position and awarded experience.

- A firework rocket completed its damage, particles and explosion effects; the hook reports its entity id, position and number of burst shapes.

- `fluid_level_change(dimension, pos, fluid, from, to, cancel)` is called when flowing water or lava at `pos` is about to change level; levels run from 0 through 8. Set `cancel` to keep the old level.

- A furnace consumes `fuel` and begins burning at `pos` (`furnace_burn`).

- A server player takes `count` items of `item` from a furnace at `pos` (`furnace_extract`).

- A furnace finishes smelting `item` into `result` at `pos` (`furnace_smelt`).

- A furnace begins cooking `item` at `pos` when its cooking timer first advances (`furnace_start_smelt`).

- A hopper is about to pick up a dropped stack: `hopper_pickup_item(dimension, pos, item, count, cancel)` names its position and stack. Set `cancel` to leave the item in the world.

- `inventory_block_start(player, name, pos, cancel)` runs before a guest opens an inventory supplied by a block at `pos`. Set `cancel` to leave it closed; the guest's state is not swapped in.

- A dropped item entity was successfully added to the server level: `item_spawn(entity, item, count, pos)` reports its entity id, item type, stack count and spawn position.

- `leaves_decay(dimension, pos, cancel)` is called when a non-persistent leaf block at distance 7 is about to decay. Set `cancel` to keep it.

- A weather, Channeling-trident or `/summon` lightning bolt is about to be created: `lightning_strike(player, pos, cause, cancel)`. Set `cancel` to prevent the bolt and its effects.

- `moisture_change(dimension, pos, from, to, cancel)` runs when farmland at `pos` is about to change moisture from `from` to `to`. Set `cancel` to keep the old moisture.

- A server player put a plant into a flower pot: `player_flower_pot_manipulate(player, name, pos, plant)` is heard after the potted state is stored and the item consumed.

- A server player inserted a book into a lectern: `player_insert_lectern_book(player, name, pos, book)` is heard after the block entity and inventory changed.

- A server player changed an item frame: `player_item_frame_change(player, name, entity_id, action, item, rotation)` is heard after its contents and comparator output changed.

- An orb repaired an item through Mending: `player_item_mend(player, name, item, repair, experience)` is heard after its damage changed.

- A server player changed a lectern page: `player_lectern_page_change(player, name, pos, page)` is heard after the page and redstone pulse changed.

- A server player took a book from a lectern: `player_take_lectern_book(player, name, pos, book)` is heard after its block entity changed.

- `spawn_change(dimension, pos, angle)` is told after `/setworldspawn` moves the world's spawn point, with its level, block position and rotation.

- `world_border_bounds_changed()` is told after `/worldborder set` or `/worldborder add` changes the border size or starts its timed size change. It hears successful commands only.

- `world_border_center_changed()` is told after `/worldborder center` moves the border center. It hears successful commands only.

- A server player completed an advancement after progress was recorded; `advancement` is its identifier, such as `minecraft:story/mine_stone`.

- A server player's main-hand punch swing and attack-strength reset have happened: `player_attack_cooldown_reset(player, name)`.

- A server player about to enter a bed after its sleep checks pass: `player_bed_enter(player, name, pos, cancel)` hears the bed head position. Set `cancel` to keep the player out. The player's state is swapped in.

- A server player is waking or standing up from the bed at `pos`; `player_bed_leave` is heard before their sleeping state is cleared, so a listener still reads them as sleeping.

- A server player caught a bucketable mob: `player_bucket_entity(player, name, entity_id, entity)` runs after the mob was removed and the filled bucket was put in the player's hand or inventory. `entity` is the mob species id.

- A server player's selected beacon effects were stored: `player_change_beacon_effect(player, name, pos, primary, secondary)`. The primary and secondary values are `Option<EffectKey>`; `None` means no effect is selected.


- A server player counts as deeply asleep after 100 ticks in a bed: `player_deep_sleep(player, name)` fires once at that point.

- A server player book-and-quill packet is about to be applied. `pages` is its page count and `signing` says it has a title; set `cancel` to drop it.

- A server player threw an egg after its projectile was launched and its use applied.

- A server player used a firework rocket to boost while gliding, after the rocket was launched and consumed.


- A server player's XP orb pickup cooldown changed, after its new value is stored (`player_exp_cooldown_change(player, name, ticks)`).

- An item cooldown about to start for a server player: `player_item_cooldown(player, name, item, ticks, cancel)` is called for use cooldowns and for a shield disabled by a hit. `item` is the item id and `ticks` its duration. Set `cancel` to skip the cooldown; shield disable still stops the use and plays its configured sound. The player's state is not swapped in.

- A server player switches hotbar slot (`PlayerSwitchHotbarSlotEvent`), only when the slot differs: `player_switch_hotbar_slot_pre(player, player_entity_id, old_slot, new_slot, cancel)` is called after the slot range check and before the selected slot changes. Set `cancel` to keep the old slot; the client is sent that slot again. Otherwise `player_switch_hotbar_slot_post(player, player_entity_id, old_slot, new_slot)` follows once the slot changed and a main-hand item use stopped. The player's state is not swapped in.


- A server player selected loom pattern button `pattern`, after the menu accepted it: `player_loom_pattern_select(player, name, pattern)`.

- A server player filled an empty map: `player_map_filled(player, name, map_id)` fires after the filled map and its saved map data were created.

- A server player named a mob with a name tag, after its name and persistence changed: `player_name_entity(player, name, entity_id, entity_name)`.

- A server player opened a sign editor at `pos`, for the front or back side, after the editor state was made: `player_open_sign(player, name, pos, front)`.

- A server player's middle-click pick resolved a block or entity's pick stack and is about to put it in the hotbar: `player_pick_item(player, name, from_entity, pos, entity_id, cancel)` receives block coordinates and entity_id zero for a block, or zero coordinates and the entity network id for an entity. A pick with no result is not heard. Set `cancel` to leave the inventory and client unchanged. The player's state is swapped in.

- A server player picked up an arrow after it was removed and its pickup was applied: `player_pickup_arrow(player, name, entity_id, item)` gives the pickup item id.

- A server entity about to travel to another level, a player or not: `entity_travel_to_dimension(entity, dimension, cause, cancel)` (NeoForge's `EntityTravelToDimensionEvent`) names the target level; `cause` is `nether_portal`, `end_portal`, `end_gateway` or `teleport`. Cancel to keep the entity where it is. The player's state is not swapped in. Ordinary End exits are `end_portal`; respawn after the End credits is not travel.

- A server player requests a known recipe be placed in their crafting grid through the recipe book (`player_recipe_book_click(player, name, recipe, all)`).

- A recipe about to become known to a server player: `player_recipe_discovered(player, name, recipe, cancel)` is heard for advancement rewards, knowledge books and `/recipe give`. Set `cancel` to keep it locked; the client is sent the matching removal. The player's state is not swapped in.

- A server player launches with a riptide trident: `player_riptide(player, name)` fires after the velocity and spin are applied. The player's state is swapped in.

- A server player about to shear a sheep, mooshroom, snow golem or bogged. `off_hand` says which hand holds the shears; set `cancel` to leave the mob and tool unchanged.

- A server player's shield was disabled by a blocked melee hit: `player_shield_disable(player, name, ticks)` fires immediately before the shield cooldown is set.

- A server player selected stonecutter recipe button `recipe`, after the menu accepted it: `player_stonecutter_recipe_select(player, name, recipe)`.

- A spectator's camera returned to their own player after they stopped watching an entity: `player_stop_spectating_entity(player, name, entity_id)` gives the former target.

- A server player swapped their main hand with an equipment slot: `player_swap_with_equipment_slot(player, name, slot)` fires after both hands changed; `slot` is currently `offhand`.

- A player is about to travel through an End gateway (`player_teleport_end_gateway(player, name, gateway, cancel)`). Set `cancel` to leave them at the entrance without starting the gateway cooldown.

- A server player used a golden dandelion on an ageable mob: `player_toggle_entity_age_lock(player, name, entity_id, locked)` fires after the lock and item use changed.

- A server player completed a villager or wandering trader trade, after the merchant menu applied the trade: `player_trade(player, name, entity_id)` gives the merchant's network id.

- A server player's action removed an entity's leash: `player_unleash_entity(player, name, entity_id)` fires after the leash state is cleared.

- The server synced changed motion to a player: `player_velocity(player, name, velocity)` fires with velocity in blocks per tick.

- `portal_create(dimension, pos)` is told after a Nether portal for dimension travel or the End arrival platform has been built, with its dimension and lower corner or platform origin.

- A thrown splash or lingering potion finished applying its hit effects; the hook reports the potion item and impact position. Experience bottles are excluded.

- A crafting-grid result was prepared: `prepare_item_craft(player, name, pos, result)` is called for the inventory grid (None) or a table at `pos`; change the result before it is sent to the menu. Guest state is not swapped in.

- Enchanting offers were calculated: `prepare_item_enchant(player, name, pos, costs)` is called for a server player at `pos`; change the three costs before they are sent to the menu. Guest state is not swapped in.

- A smithing result was prepared: `prepare_smithing(player, name, pos, result)` is called for a server player at `pos`; change the result before it is sent to the menu. Guest state is not swapped in.

- A projectile was added to the server level: `projectile_launch(player, name, entity, kind)` reports the entity id and type, plus the player launcher when there is one.

- A raid wave has spawned at its position, with its raid id, wave number and raider count (riders' mounts not counted).

- A raid is triggered or reaches `victory`, `loss` or `stopped`: `raid_state_change(raid, pos, state)` reports its owning `RaidRef`, center and new state. The handle retains its level even when that level is parked; an already removed raid cannot be read.

- `sculk_bloom(dimension, pos)` is called after a sculk catalyst at `pos` takes experience and enters the blooming state.

- `sponge_absorb(dimension, pos)` is called after a sponge at `pos` has absorbed water and become wet.

- A guest's server command completion request was evaluated: `tab_complete(player, name, command, range, suggestions)` lets a mod add or filter the suggestions; `range` is the replaced byte range. The guest's state is not swapped in.

- `target_hit(dimension, pos, strength)` is called after a projectile hits the target block at `pos` and it takes the reported redstone output strength.

- The server's `/weather` command is about to change thunder: `thunder_change(dimension, thundering, cancel)`. Set `cancel` and the previous thunder state and timer remain.

- The overworld clock is about to jump after sleep or a `/time` command: `time_skip(dimension, from, to, cause, cancel)`. Set `cancel` to keep the old time.

- A player selected a trade: `trade_select(player, name, villager, index)` is heard after the trading menu updates; `villager` is the entity id when known. The player's state is not swapped in.

- `vault_change_state(dimension, pos, state)` is called after a vault at `pos` changes to `state` (`inactive`, `active`, `unlocking`, or `ejecting`).

- The server's `/weather` command is about to change its rain and thunder parameters: `weather_change(dimension, raining, thundering, cancel)`. Set `cancel` and the prior weather and timers remain.

- The dedicated server's whitelist changed state: `whitelist_state_update(enabled)` reports its final on/off state, after `/whitelist on` or `/whitelist off` from a player or console.

- The server completed a world save cycle: `world_saved(wait)` reports whether chunk writes were flushed before returning.

- The time players wake at after sleeping, as NeoForge's `SleepFinishedTimeEvent`: `sleep_finished_time(dimension, current, new_time, adjustment)` runs on the server once enough players have slept and the `advance_time` rule is on, before the clock moves. `adjustment` (a `ClockAdjustment`: `Absolute`, `Relative` or `Marker`, starting as vanilla's `wake_up_from_sleep` marker) says where the clock goes and a mod may change it; `new_time` is the vanilla morning, read only. A time before `current`, or an undefined marker, is refused. `time_skip` then hears the jump to the chosen time.

- A shift-click in a mod container's menu routed by the mod, `AbstractContainerMenu.quickMoveStack`: `menu_quick_move(kind, subject, container_slots, slot, stack, handled)` (version 6) runs on the server before the game's own routing, for a mod container on a vanilla menu type or its own menu type, lending the live resident block entity and server `Host` through `subject: BlockEntityScope<'_>`. The mod takes what it moves out of `stack` and sets `handled`; the slot keeps the rest and the client gets the menu again.


- A server player's or fox's `teleport_randomly` consume effect (chorus fruit) is about to try a spot: version 2 of `item_consumption_teleport(consumer, consumed, from, target, cancel)`, once per attempt (up to 16). `consumer` is the living actor's handle and `consumed` is the public API stack captured before the use, with its count, damage and component patch, rather than the current contents of the hand. The living handle replaces version 1's player-only UUID/name fields; players retain their acting-player context. A mod may move `target`; the game still looks down from it for ground to land on. `cancel` ends the effect with no teleport and no more attempts, not consumption. A fox's callback runs with the full server host before its living AI/movement and before the next mob ticks; listener changes to its mouth item, position or eligibility invalidate the captured decision rather than being overwritten.

- A server player's or mob's ender pearl landed: version 3 of `native_ender_pearl_teleport(owner, pearl, from, target, damage, cancel, hit_entity, hit_block, hit_face, hit_at)` runs before teleporting the live `EntityRef` owner. A mod may move `target`; cancellation spends the pearl without a teleport, sound, player damage or endermite roll. A player retains acting-player context and mutable fall damage (5). A mob preserves rotation, resets fall distance, takes no player fall damage and makes no endermite draw. A listener's move or an owner that is no longer eligible invalidates the pending move; it is refused rather than overwriting that write. `owner` replaces version 1's player-only UUID/name fields; use its handle and the host services. Version 3 adds what the pearl hit (`hit_entity`, `hit_block`, `hit_face`, `hit_at`). No compatibility fields or older alias are retained.

- Drawing an item stack on a screen: `native_item_stack(batch, stack, at, size)` draws its model, glint, damage bar, cooldown and mod decorations, but not its count. It draws nothing and returns false for an unknown item or an empty stack (air, or a count of 0).

- Hiding a player (a vanish): `player.hide(host, level)` hides that player, from the next tick, from the players below permission `level` (None shows them again): they leave those players' Tab list and world (their clients stop tracking them, as they do a spectator) and those players don't read them joining or leaving. Players of `level` or more still see them; a player made an operator sees them again. Retain a `PlayerRef` obtained while the player is available: unlike live-player writes, hide can still change that UUID's visibility after disconnect, within the handle's world epoch. It cannot target an arbitrary offline UUID, and a handle from a closed or reopened world refuses the write. The visibility setting is kept by UUID across reconnects until changed or the server stops. It requires `&mut dyn ServerHost` and an available server player list.

- A player's item used up or broken: `player_destroy_item(player, player_entity_id, original, hand)` reports the complete stack before use or durability loss, including its count, damage and components. It covers hand use, entity consumption, completed eating/drinking and durability breaks on each logical side, including the local host. `hand` is `Some(Main)` or `Some(Off)` for hands, `None` for armour or inventory ammunition. A partial consumption and creative use preserving the stack are silent. Listen only, after the consumption or break has applied and before a use remainder is installed. The callback receives the typed `HostSide`. `ItemPart::damage_item` decides wear; this notice cannot veto it (NeoForge's `PlayerDestroyItemEvent`).

- A player's damaging melee hit on a living target: `critical_hit(target_kind, vanilla_critical, critical, damage_multiplier)` may force or deny the critical hit and change its multiplier (1.5 for a vanilla crit, else 1.0); the sweep, sound and particles follow the decided result. Not fired for a dragon part or a hit that does no damage.

- Attachment reads that never insert the default: `AttachmentType::has` says whether a holder carries a value, saved or set,
  even before it decodes (NeoForge's `hasData`), and `AttachmentType::get_existing` returns `None` instead of the default
  (NeoForge's `getExistingData`). Neither adds anything to the holder's saved data.

- A player releases a bow: `arrow_loose(stack, has_ammo, charge, canceled)` may change held-tick `charge` or cancel the shot; negative charge also prevents it. Unlike `living_entity_use_item_stop`, it does not skip the whole item release; `entity_shoot_bow` reports the spawned arrow afterward with read-only force. Fires on the Game running the bow release, including LAN guests; a remote client's own release path, crossbows, and ArrowNockEvent are not covered.

- An item frame or painting about to break: `hanging_break(player, kind, pos, cancel)` is heard before a server player's hit, a blast or the loss of its wall breaks an item frame, a glow item frame or a painting, with the hitting player (None for a blast or a lost wall). A mod sets `cancel` and it stays. The hitting player's state is swapped in. A hit that only takes a frame's item out, the game's own player and `/kill` are not asked.

- A criterion of a server player's advancement was granted or revoked after its progress changed and before any completion rewards, except the `minecraft:tick` criteria granted on a network player's first tick or a reload, which are heard after that tick's rewards and packets; `advancement` is its identifier, such as `minecraft:story/mine_stone`, `criterion` the criterion name and `progress_type` is `ProgressType::Grant` or `ProgressType::Revoke`. A grant or revoke that changes nothing is not heard.

- A statistic about to be awarded to a server player: `player_stat_award(player, name, stat_type, stat, value, cancel)`. The canonical namespaced `stat_type` and `stat` strings and the `i32` amount `value` are writable. Unknown types or entries, noncanonical ids and negative amounts keep the entire original award, with a warning once. Registered custom stats are valid redirect targets. Set `cancel` to award nothing: the count and its scoreboard criteria stay as they were. Transaction-deferred awards fire only after the outermost successful commit.

- A player's data copied to their replacement: `player_clone(player, name, was_death, original)` (version 2) fires after death (`was_death` true) and the credits-showing End return (`was_death` false), after old-player cleanup (carried drop and menu close), destination level open, and copying vanilla state and serializable attachments (`copy_on_death` on death, all serializable entries on End return), and before placement (`ServerPlayer.restoreFrom`, NeoForge's `PlayerEvent.Clone`). `player` is the replacement's UUID, already carrying its destination level/dimension; `original: OriginalPlayer<'_>` directly lends the retained pre-copy player (preserving its source level/dimension) for the callback's lifetime (compile-time scoped, with no token handle or separate original-player holder variant). Its attachment methods follow normal `AttachmentHolder` semantics on the old player without touching the replacement and participate in `host.transaction`: `attachment` returns defaults without inserting them, `existing_attachment` decodes and caches saved entries once, and `has_attachment`, `with_decoded_attachment`, `set_attachment`, and `remove_attachment` act on the old holder only, rolling back with replacement writes on abort. It also exposes `with_inventory` (vanilla 0–42 slots, after death drops and menu close or intact with keep-inventory and on End return), `persistent_data`, `uuid`, `position`, `dimension`, `level`, `health`, `experience`, and `effects`. No subscriber means no retained copy (attachment copy still runs). Nothing swaps a player's state in.

- A player loaded from the world's save: `player_load_from_file(player, name, directory)` is heard once for each join (the owner of an integrated server too), after the player's `.dat` is read and applied and before they are placed in a level (NeoForge's `PlayerEvent.LoadFromFile`, `PlayerList.loadPlayerData`). `directory` is the world's player directory (`players/data`); a mod reads its own file there, named `<uuid>.<suffix>`. It cannot change or stop anything; no player's state is swapped in.

- A player saved to the world's store: `player_save_to_file(player, name, directory)` is heard once for each save of a placed player (when they leave or the world closes, when the dedicated server saves everyone, and for an integrated server's owner when the world saves them; LAN guests are not heard on integrated autosaves) after their data is written (NeoForge's `PlayerEvent.SaveToFile`, `PlayerList.save`). `directory` is the world's player directory (`players/data`); write your own file there, named `<uuid>.<suffix>`, never the player's `.dat`. The player may be leaving, so no player's state is swapped in. It cannot change or stop anything.

- `item_expire(entity, stack, extra_life)`, a dropped item reaching its lifetime on the server (NeoForge's `ItemExpireEvent`, vanilla `ItemEntity.tick`): every mod may set or add to `extra_life`, in ticks from zero; a positive total keeps the item that much longer, zero or less lets it expire. Extensions are capped at 32766 ticks.

- The title's own screens to the screen hooks. `ScreenView.id` has one id per title state (`title`, `world_select`, `world_create`, `world_create_flat`, `world_create_buffet`, `world_preset_editor`, `world_edit`, `world_backup`, `world_backup_confirm`, `world_optimize_confirm`, `world_optimize`, `low_disk_space`, `world_delete`, `world_recover`, `world_recovered`, `error`, `multiplayer`, `add_server`, `edit_server`, `direct_connect`, `server_delete`, `friends_join`, `disconnected`, `multiplayer_warning`, `onboarding`, `ban_notice`), with no `menu`, no `title` and no `slots`, and the whole window as `rect`. `screen_render_post` fires after the title draws its screen (before its toasts), and the `_pre` input hooks — mouse buttons, drag, wheel, key press and release, character typed — fire on it and can cancel it as on any screen, so a mod draws its own button or browser over Create New World and takes the clicks. A dialog, a prompt or a server's pack prompt over the title keeps them away, and typing still goes to the title's own boxes (with no box focused the key's text stays on the key, as on Options, so `screen_char_typed_pre` fires before the key acts there). `native_instance_dir()` is the instance folder, where `options.txt`, `saves/`, `resourcepacks/`, `shaderpacks/` and `sunrise-mods/` are; a world's `datapacks/` is in that world's folder.
- On the server, `anvil_craft_post(host, event)` reports a completed take after the level charge, input consumption and anvil wear. `event.inputs` lends the same pre-take stacks as Pre, even if the anvil broke. Vetoed takes do not fire Post.

- On the server, `anvil_craft_pre(host, event)` lends the left and right inputs and the result through `event.inputs` before those effects. Set `event.cancelled` or return `Flow::Cancel` to return early before charging levels, consuming inputs, damaging the anvil or firing `anvil_craft_post`, without undoing the click's item move (matching NeoForge `AnvilMenu.onTake`).

- A server player stopped digging `pos` before the block broke: `block_damage_abort(player, name, pos)` is heard after the server clears the dig and sends the block state back to the client. The player's state is not swapped in.

- A mob is about to drop `count` of `item` at `pos`: `entity_drop_item(entity, item, count, pos)`.

- A mob's leash was removed, before any lead drop is handled: `entity_unleash(entity, kind)`.

- A server explosion is about to begin at `center` with `radius` (`explosion_prime`); the hook is listen-only and the explosion proceeds as usual.

- A hopper selects a container while searching for items: `hopper_inventory_search(hopper, container, entity)` identifies the hopper and the selected block container or container entity.

- A hopper is about to move one item between containers: `inventory_move_item(source, destination, item, cancel)` runs before the source slot is changed, a composter's bone meal included; a chest boat or minecart is reported at the block it is in. Set `cancel` to keep the item in its source slot.

- A dropped item entity reached its 6000-tick lifetime and is about to be removed: `item_despawn(entity, item, count, pos)`.

- A boat or minecart joined the level at `pos`, after a player or dispenser placed it: `vehicle_create(kind, pos)` is listen-only.

- A player is about to damage a boat or minecart: `vehicle_damage(kind, damage, cancel)` may change the damage or cancel it before the vehicle's damage and wobble change; the attack still completes.

- A boat or minecart is about to be destroyed at `pos`: `vehicle_destroy(kind, pos)` is heard just before it is removed; TNT minecarts that prime instead are not heard.

- A guest's arm swing: `player_arm_swing(player, name, hand)` is called after the server handles the guest's `Punch` packet and resets attack strength. The current packet swings `mainhand`; the guest's state is swapped in.

- A server player selected the item at `index` in a bundle in menu `slot`: `player_bundle_item_select(player, name, slot, index)`.

- A server player's client finished loading the level around them after joining, respawning or changing dimension: `player_client_loaded_world(player, name)`.

- A guest's player input flags: `player_input(player, name, flags)` is called after a loaded guest's `PlayerInput` packet is applied to the hub. `flags` is the packet's bit field; the guest's state is swapped in.

- A guest jumped: `player_jump(player, name)` is called after accepted movement made the server player jump from the ground, once the guest's movement simulation finished. The guest's state is swapped in.

- A network or fake guest was kicked: `player_kicked(player, name, reason)` (version 2) is called after the hub records the rendered reason; a network guest also receives the disconnect. The guest's state is swapped in unless already removed in the same poll.

- A LAN guest launched a snowball, egg, ender pearl, potion or trident: `player_launch_projectile(player, name, item)` runs after the projectile is created.

- A server player's experience level changed: `player_level_change(player, name, from, to)` is called after experience points or levels change their level, including orb points. The player's state is swapped in when they are a guest.

- A server player released a drawn bow, by the use-item key or a mod's `release_using_item`, with `player_ready_arrow(player, name, arrow)`: the name of the selected projectile item.

- A server player's recipe book open or filter setting changed: `player_recipe_book_settings_change(player, name, book, open, filtering)`, where `book` is its typed `RecipeBookIdentity`.

- A server player released the use-item key, or a mod's `release_using_item` did, before the item finished being used: `player_stop_using_item(player, name)`.

- A guest's player input changed sneaking: `player_toggle_sneak(player, name, sneaking)` is called after the hub applies the new state, only when it changes. The guest's state is swapped in.

- A hopper begins a transfer attempt: `prepare_hopper_transfer(pos)` runs after its cooldown and enabled checks, before it pushes or pulls items.

- A vault is about to eject a loot item: `vault_loot(pos, item)` supplies the vault position and item identifier before it spawns.

- A timed world border size change finished: `world_border_bounds_change_finish()` fires after the border reaches its final size.

- `ClientHost::create_dynamic_texture(id, width, height, rgba)`, `update_dynamic_texture`, `free_dynamic_texture(id)` and `dynamic_texture_budget()`: vanilla's `DynamicTexture` registered in the `TextureManager` (`register`, `upload`, `release`), kept in the HUD atlas. A client mod names an RGBA image under a namespace it owns (`mymod:dynamic/panel`, its extra manifest namespaces, or a content package it serves via `served_by`); it takes a slot of the atlas's pool, of the smallest of three size classes that holds it (`DYNAMIC_TEXTURE_CLASSES`: 64, 256 and 1024 pixels square, 32, 8 and 2 slots), and its id names it as a GUI sprite, so `DrawContext::sprite`, `gui_nine_slice`, `tiled_sprite` and `gui_sprite` reach it as any sprite. The pool is packed only when a client mod is loaded (decided at load and on a resource reload, with no mixin), so a game without mods has no slot and no atlas growth. An update re-patches the same slot (a size of another class is `ClassChange`), a free drops the name and returns the slot, and a resource reload keeps every name and patches the new atlas from the last pixels, which the game keeps and counts in the calling mod's budget: `DYNAMIC_TEXTURE_MOD_IMAGES` (16) images and `DYNAMIC_TEXTURE_MOD_BYTES` (4 MiB), read with `dynamic_texture_budget`. Every refusal is named (`DynamicTextureError`: `NotOwned`, `BadSize`, `TooLarge` past 1024 pixels, `NoSlot`, `OverBudget`, `NameTaken`, `Unknown`, `ClassChange`, `Unavailable`) and changes nothing. Replaces `native_mod_image`, one 1024 by 1024 slot shared by every mod. Diverges from upstream: horizon-rust/horizon keeps `native_mod_image`.
- A living entity drowning (`living_drown`), with changeable drowning, damage and bubble count, and cancellation.
- `Game::native_translate(key)`: the selected language's text for a translation key, or `None` when no language layer defines it (`I18n.exists`), so a mod can fall back its own way (a recipe viewer names an item tag by `tag.item.<namespace>.<path>` and falls back to the id). NeoForge: `I18n.exists` and `I18n.get`. Client-side.
- `Game::native_enchantments()`, `Game::native_enchantment_supports(id, item)` and `Game::native_enchantment_min_cost(id, level)`: every registered enchantment as an `EnchantmentInfo { id, max_level, curse }` record in registry order, whether one can go on an item (`Enchantment.isSupportedItem`), and the power a level needs (`Enchantment.getMinCost`), or `None` for an unknown enchantment. What a recipe viewer lists for grindstone and anvil enchanting. NeoForge: the `enchantment` registry (`RegistryAccess.lookupOrThrow(Registries.ENCHANTMENT)`), `Enchantment.getMaxLevel`, `Enchantment.isSupportedItem`, `Enchantment.getMinCost`, the `minecraft:curse` tag. Client-side.
- `Game::native_fluid(batch, fluid, [x, y], size)` and `Game::native_fluid_name(fluid)`: a fluid (`minecraft:water`, `mymod:oil`) as a square of `size` GUI pixels, the GUI sprite its look names (`data/<mod>/sunrise/fluid/<name>.json`) tinted with its colour, else a flat square of the colour (vanilla water and lava have theirs), and its name as the lang gives it. What a recipe viewer draws for a fluid stack; the slot, the amount and the tooltip are the mod's. NeoForge: `IClientFluidTypeExtensions.getStillTexture` and `getTintColor`. Client-side.

- A living entity's fall (`living_fall`), with changeable distance and multiplier and cancellation.

- A living entity healing (`living_heal`), with a changeable amount and cancellation.

- A living entity jumping (`living_jump`)."##]
    fn general();
}
