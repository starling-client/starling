# Camera and collision reads

`ClientHost::is_paused()` reads the applied client pause state, not whether a pause
screen is open. A multiplayer game continues running while its pause screen is open.
The pause-change pre hook sees the old state; the post hook sees the new state.

`ClientHost::gui_ticks()` reads the HUD's signed 32-bit tick counter (`Hud.getGuiTicks`).
It advances once per unpaused client tick, including on the title screen, and wraps like
Java's `int`. Pausing holds it; a server tick freeze does not. This is not world game time
or the frame's animation clock.

`ClientHost::gui_scale()` reads the GUI scale in effect (`Window.getGuiScale`), at least 1,
or `None` without a window (a server, a headless run). Vanilla's `Window.calculateScale` resolves
it from the GUI Scale option and the window size: the option is a maximum, where 0 means automatic,
so a large window still resolves below it, and with the Unicode font forced an odd result is raised
by one. `ClientHost::gui_options().gui_scale` is only that option. The framebuffer size stays
`native_physical_window_size()`; divide it by this scale, rounding up, for the GUI-scaled width and
height (`Window.getGuiScaledWidth/Height`).

`LevelRef::brightness(host, layer, pos)` (`getBrightness`, the stored light 0 to 15) reads on a client too: the
light of the level the client shows, as the server sent it for a chunk the client holds. It is `None` for a
chunk the client does not hold and before login. `LevelRef::block_face_sturdy(host, pos, face, support)` uses
that level's local shapes for a loaded block inside build height. `LevelRef::can_see_sky(host, pos)` reads
its published sky light, even outside build height or in an unloaded column. See
[Level hosts](../../concepts/hooks-and-the-host.md#level-hosts) for the boundary rules.

`ClientHost::playing()` is true while the player is in the world with nothing over it: no screen, title, sign
editor or book, so keys and zoom apply. A HUD or screen hook can read held state, the current binding,
or its Controls label using a [live vanilla mapping](input.md#reading-live-vanilla-mappings) retained
from `client_setup`.

`ClientHost::select_hotbar_slot(slot)` chooses the local player's hotbar slot (0 to 8) as a number key
does (`Inventory.setSelectedSlot`), and sends nothing itself. The game sends the carried-item packet
exactly when it does for a key press: at the next tick's start at the latest, and before an attack, a
use, an interact, a release, a drop and a continued block break (not before a block break start), so a
selection made earlier in the same tick goes out before the action. One packet goes per change. A slot
above 8 is `OutOfRange { slot }`, a client with no world or player is `NoWorld`, and a spectator is
`Refused` (its number keys open the spectator menu); each leaves the selection unchanged. Read the slot
back with `player_selected_slot` on the local player.

`ClientHost::clipboard_text()` reads the system clipboard (`KeyboardHandler.getClipboard`, empty text when it
holds none) and `ClientHost::set_clipboard_text(text)` writes it (`setClipboard`; empty text copies nothing). Both
work only while the game handles a key, mouse or typed-text event, from the window or from the control socket
(`sunrise-ctl key`, `mouse`, `type` and its clicks): a key, mouse or screen hook that fires inside one, such as
Ctrl+V into a mod's own field. At any other moment (a tick, a frame, a packet) the read is `None` and the write
is `false` with nothing copied, so no mod reaches the clipboard without the player doing something. Both are
`None` and `false` on a server, and take `&self`, so a screen hook can call them. Vanilla's `KeyboardHandler`
has no such rule: Sunrise adds it.

## Camera snapshots and perspective

The `ClientHost::camera()` helper returns the most recently extracted world frame's camera, or none before one exists. Its position, interpolated eye height, partial tick, angles, final FOV, near/far planes and dimension accompany column-major view and projection matrices. The view is camera-relative and includes bob and spin; projection uses reversed Z. Earlier callbacks in a frame can see the previous frame. `block_collision(pos)` returns block-local `[min_x, min_y, min_z, max_x, max_y, max_z]` boxes with an empty collision context; unloaded blocks and collision errors give no boxes.

`ClientHost::project(at)` places a world point on this frame's screen, as vanilla's
`GameRenderer.projectPointToScreen`: normalised device x and y (-1 to 1, x right, y up) and a
reversed depth z. A HUD or screen hook turns it into GUI pixels with the `DrawContext`'s size
`[w, h]`: `((x + 1) / 2 * w, (1 - y) / 2 * h)`. It is `None` for a point behind the camera or a
non-finite one (vanilla returns a mirrored point there), outside a world, before extraction and between render frames. It is supported in GUI, screen
and world drawing callbacks after camera extraction, not setup or tick callbacks.

`ClientHost::set_local_rotation(yaw, pitch)` turns the local player's view, as vanilla's `setYRot`
and `setXRot` store it: yaw as given (not wrapped; the server wraps it), pitch `% 360` then clamped to
-90..90. The previous-frame angles move by the same amount, so the view doesn't jump. The call sends no
packet. The game's own player sync at the end of the client tick sends the look in its one move packet,
of the kind vanilla picks, and only while the camera is the player (a rider sends the rotation-only
packet). Several calls in one tick send the last values once, and a call after the sync goes out the
next tick. NaN or infinity returns `NotFinite` and changes nothing; outside a world the call returns
`NoWorld`. The server sees an ordinary client look, at tick granularity, as from a mouse.
`calculate_player_turn` changes how far the mouse turns the view; `set_local_rotation` sets it directly.

`ClientHost::set_camera_type(type)` is a queued client-only effect and takes `&self`, so
shared callbacks such as `camera_pose` can request a perspective. `camera_type()` reports
the latest request immediately; several requests coalesce to the last one. Requests before
or during `camera_pose` apply before that frame's final camera, FOV and spectator post effects.
The pose hook runs only once: its explicit position or angle overrides survive a refreshed
vanilla camera. Requests from later render callbacks apply at the next frame's extraction.
Setting a type never emits `camera_type_cycle`; F5 alone cycles, and its final event choice
wins over requests made inside the cycle callback.

## The fluid the camera is in

The `camera_submersion` hook answers which fluid the camera is in, as vanilla's
`Camera.getFluidInCamera` does, given as the fog type at the camera: `Atmospheric`, `Water`, `Lava` or
`PowderSnow`. It comes in as the game's answer, after the `state_at_viewpoint` parts of the blocks it reads,
with the camera's position. Set it to `Atmospheric` to clear underwater, lava and powder snow submersion,
or to a fluid to force it. A failed listener leaves the last successful value in place for the next
listener. A panic disables that mod's `camera_submersion` listener for the session and names the mod
in the log and client error toast.
A free camera that passes through water, for one, keeps the air's fog:

```rust,ignore
fn camera_submersion(&self, _host: &dyn ClientHost, event: &mut CameraSubmersionEvent<'_>) {
    if self.detached() {
        *event.fog_type = FogType::Atmospheric;
    }
}
```

Camera fluid readers include the field of view in a fluid, the frame's fog, the sky pass and the
sky's dark disc, water and conduit vision, and a shader pack's `isEyeInWater`. The first-person water
overlay also lets this hook override whether it appears, but it asks at the player's eye and starts
with the player's tick sample: `Water` if the eye was in water, `Atmospheric` otherwise. After the
local player is removed on death tick 20, the overlay keeps using that last sample until respawn.
The overlay uses `Water` to show and any other answer to hide. `state_at_viewpoint` affects camera
geometry reads, not the overlay's tick sample.

The hook fires once per reader, several times a frame. The position and the input can differ between
reads, even at the same point. The position can't be changed. Server and gameplay fluid checks (an
entity in water, drowning, swimming, the sculk vibrations' water test) don't read it.

## Block changes in the client level

A client map or minimap keeps its own picture of the terrain and only has to redraw what changed.
`client_chunk_load(dimension, chunk)` and `client_chunk_unload(dimension, chunk)` tell whole columns
arriving and leaving; `client_block_changed(dimension, pos, old, new, predicted)` tells each block of a
held column that changes state, after the write, so `host.level(dimension)` already reads `new` there:

```rust
fn client_block_changed(&self, _host: &dyn ClientHost, event: &mut ClientBlockChangedEvent<'_>) {
    // The chunk to redraw: [x, z], in chunks.
    self.stale(event.dimension, [event.pos.x >> 4, event.pos.z >> 4]);
}
```

Every route a client writes a block by tells it: the server's block update, each block of a batched
section update, the client's own predictions (a break, a placement, a use; `predicted` is true) and,
when the server acknowledges them, each block that takes the server's state back (`predicted` false). A
server update for a block still under a pending prediction is held, as in vanilla, and told when the
acknowledgement applies it. A write that keeps the state is not told, so nothing has to rescan the
loaded terrain. It needs no mod on the server: it runs on a client of any server, a LAN guest included.
All three hooks also fire in singleplayer, where the window is a client of its own in-process server.
The server-side world still reports its edits through `block_changed`. A level a server opens for
viewing (below) is not told. A section update tells each of its blocks, so keep the listener cheap:
mark the chunk and redraw it later.
sunrise-org/sunrise-minimap is such a mod: it redraws a marked chunk into its map's dynamic texture and draws
the map in `render_gui_post`.

## Viewed levels

A server can open other levels for a client to view besides the one the player is in (`sunrise:levels`,
as Immersive Portals' `ClientWorldLoader` keeps a `ClientLevel` per dimension). On a client,
`host.level("minecraft:the_nether")` also returns a handle for each such level while the server keeps it
open. Its block reads read the viewed level's own world, never the player's: `block_state`, `block`,
`block_property`, `clip`, `collision_boxes`, `has_collisions`, and the fluid reads built on
`block_state` (`fluid_shape`, `fluid_height`, `fluid_height_for_camera`, `fluid_can_be_replaced_with`).
So a mod can aim a ray or collide a box through a portal. They never write and never load a column: a
column the client doesn't hold is `None`, for a collision box as soon as its walk reaches one. `clip`
keeps its traversal limit and `collision_boxes` its cell cap.

The handle is `None` while the server has the level closed. If the server closes the level and opens it
again within the same client level, the old handle reads again. A login, a reconfiguration or a respawn
into another level replaces the client level: the old handle stays stale for good, and the mod calls
`host.level` again once the server reopens the level. A respawn in the same level drops the viewed
levels too, but keeps the client level, so the old handle reads again after the reopen. Writes,
server-only reads and the other level reads refuse viewed levels. In particular,
`visit_entities_in` and `ray_trace_nearest_entity` return `Err(LevelReadError::Unavailable)`
for a current handle to an open viewed level; they do not query the displayed level instead.

## Crosshair reads

`ClientHost::crosshair_target()` returns the ordinary interaction-range target as
`CrosshairTarget::Block { pos, state, hit }` or
`CrosshairTarget::Entity { entity, point }`. The block hit contains its world-space
location, face and inside flag; the entity point is also in world coordinates.
It reads the last committed `client_pick` decision, and returns `None`
on a miss or before the first pick. A tick refreshes it before key handling;
a frame refreshes it before extraction. Consumption does not recheck liveness. It never
casts a new ray. Without listeners the decision is the vanilla block/entity pick.
It can be read with a screen open. `entity` is an `EntityRef` carrying the current
client level's epoch and network entity ID, including dragon parts. It becomes stale
when the client level is replaced or the client disconnects; a same-level respawn
keeps its epoch. Unlike `view_hit` and `raycast`, this does not return a block
behind a targeted entity.

`ClientHost::view_hit()` and `raycast(distance)` cast their own ray along the view, up
to 1024 blocks, and hit blocks in loaded chunks only: far terrain is never hit. Both
return `None` on a miss.

To query the displayed level independently of the committed crosshair pick, use
[`visit_entities_in`](../world/spawning.md#visiting-entities-in-a-box) for a box, or
`ray_trace_nearest_entity` for one entity hit. The latter clips only entity boxes,
not blocks, and uses the segment and exclusions you supply, not vanilla's pick policy.
For example, filter the ray using a shared read from the same client host:

```rust,ignore
let hit = level.ray_trace_nearest_entity(host, from, to, margin, exclude, &mut |entity| {
    entity.kind(host) == Some(wanted_kind)
})?;
```

`Ok(None)` means no accepted candidate was hit, not that the read was refused. The
predicate runs on each non-excluded candidate `EntityRef` in spatial traversal order
before its box is read and clipped (`ProjectileUtil.getEntityHitResult`); on an exact
squared-distance tie the first visited hit wins. See
[entity rays](../world/spawning.md#finding-the-nearest-entity-hit) for the geometry
and when to keep the ordered all-hit query.

A client `EntityRef` supports `bounding_box(host)`, `eye_height(host)` and
`eye_position(host)`. For the local player these use the current predicted body and pose;
for a tracked target they use the client's displayed body. End crystals expose their 2 x 2 box;
dragon parts expose their own box. Stale handles and entities the
client cannot read return `None`.

The local `PlayerRef` also supports `within_entity_reach(host, target, rule)`: it reads the
predicted eyes, the target's displayed bounding box, the synchronized interaction-range
attribute and the main hand's attack-range component. `EntityReach::Interact` and `Container`
use strict limits; `Attack` includes the outer boundary. Only the local player can make this
client query, and the result sends nothing and authorizes no action on the server.

## Local player reads

These read the local player as the client sees it. They take no mod-controlled numbers and send nothing.

- `ClientHost::client_session().client_latency()` is the player's latency in milliseconds as the
  tab list draws it (`PlayerInfo.getLatency`): the server's smoothed keep-alive average, refreshed by
  the server's own broadcast. It reads 0 before the server's first measurement, as vanilla does, and
  `None` outside a world.
- `ClientHost::client_session().client_tab_list()` is the player list as the Tab overlay draws it:
  `ClientTabList { entries, header, footer }`. Each `ClientTabListEntry` has the profile `name`,
  `uuid`, `game_mode`, `latency` and the `display` text as drawn (`getNameForDisplay`: the display
  name the server set, else the name with its team's prefix, suffix and colour; italic for a
  spectator). Entries come in the overlay's order (`getPlayerInfos`: higher list order first,
  then, among equal list orders, spectators last, team name and name) and stop at the 80 it draws. Header and footer are `None` when empty. It is what
  this client has been sent and changes as the server updates the list; `None` outside a world.
  There is no NeoForge call for this: it is Sunrise's own read.
- `ClientHost::local_item_use()` reports ordinary item use: `ItemUse { hand, ticks_used, ticks_remaining }`
  (`isUsingItem`, `getUsedItemHand`, `getTicksUsingItem`, `getUseItemRemainingTicks`).
  `ticks_used + ticks_remaining` is the use duration. It is `None` when no item is in use or no world
  is open. Track intercepted main-hand input through [`ClientItemUseBehavior` callbacks](../../concepts/parts/item-and-tag-parts.md#intercept-main-hand-client-input)
  instead; interception does not make the player use or block with the item.
- `ClientHost::creative_search_items()` returns the built creative search tab's display order,
  for ranking stacks in inventory sorters. It requests the catalog with the local player's permission
  and Operator Items Tab option. A cache miss can fire the creative build hook, so this read needs a
  mutable Host. It returns an empty list during setup and reload, and excludes client-only
  presentation additions.
  For membership reads on either side, see [creative tab contents](../content-mods.md).
- `ClientHost::item_uses_block_light(&stack, context)` is `ItemStackRenderState.usesBlockLight` of the
  stack's model resolved for an `ItemDisplayContext` (no owner or level): true for a model lit like a
  block from the side (`gui_light: side`), false for a flat item lit from the front. It is what
  `BakedModel.isGui3d` became in 26.3, so use it to draw a stack as a flat sprite or as a 3D model.
  It is false for an empty stack or one with no model.
- Reach comes from the interaction-range attributes with the server's and equipment's
  modifiers. Menu slot numbers are `Slot.index`, the position in the menu's slot list
  that `menu_slot` and `menu_slot_count` take; `menu_slot_backing` gives the index in the backing container.

## Combat targeting

`client_pick` runs at both vanilla pick call sites: every client tick at
partial tick 1.0 after the game mode tick and before key handling, even while
paused; and every frame at its game-time partial tick before render-state
extraction. Both write the same stored result. NeoForge 26.3 has no counterpart.
The event supplies the vanilla `original`, the independent `block_candidate` and
`entity_candidate`, and the pick's `origin: [f64; 3]` in world-space XYZ. This is
the camera entity's eye for that pick: current for a tick, interpolated for a frame.
Use it with a candidate's hit point to measure the ray distance without casting
another ray. All listeners receive the same inputs. Only `result`, initialized to
`original`, carries edits to later listeners. Choose the block candidate to target
through an entity, choose a target from the typed ray APIs, or clear the result for
a miss. A reentrant `crosshair_target()` read sees `original`.

The game accepts an unchanged original without revalidating its geometry. A
replacement is validated at writeback: finite point, current level, live entity or
matching block state, and the vanilla reach envelope, including forward movement
for attack-range items. Invalid output becomes a miss, never a new ray. Attack,
use and pick-item read their tick's stored pick; both hands of use share it. The
highlight and crosshair read the frame's pick, which can differ from the tick's.
Consumption does not recheck liveness: a creative break followed by use in the
same tick still uses that tick's original block hit. Listener writes and publication
run in one transaction. Buckets, bottles, boats, brushes and other item-specific
rays remain independent and vanilla-exact.

```rust,ignore
fn client_pick(&self, _host: &mut dyn ClientHost, event: &mut ClientPickEvent<'_>) {
    *event.result = event.block_candidate;
}
```

Reach remains attribute-backed: use the server player's living handle and
`set_attribute_base(host, "minecraft:block_interaction_range", value)` or
`"minecraft:entity_interaction_range"`. The client's cached reach follows the server's
attribute updates. Replacing the client pick does not grant extra server reach.

`client_attack_target` receives a mutable `targets` list before an ordinary melee click
(spear jabs use their existing path). It starts with the crosshair entity. Replace it
for redirection, append for multiple hits, or clear it to suppress an entity attack.
`Flow::Cancel` cancels the batch. There is no batch cap; duplicates are sent once
in first-occurrence order. The selected entry keeps its committed tick-pick point
and client occlusion policy. Additional targets are checked
against live handles, attackability, reach, held-item range and ordinary block occlusion. Clearing an entity hit
does not mine the block behind it; an empty list on a block or miss keeps the ordinary
path. Each accepted hit uses normal attack packets and prediction. Server validation
and cooldowns apply.
