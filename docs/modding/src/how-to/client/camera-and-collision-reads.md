# Camera and collision reads

`ClientHost::is_paused()` reads the applied client pause state, not whether a pause
screen is open. A multiplayer game continues running while its pause screen is open.
The pause-change pre hook sees the old state; the post hook sees the new state.

`ClientHost::gui_ticks()` reads the HUD's signed 32-bit tick counter (`Hud.getGuiTicks`).
It advances once per unpaused client tick, including on the title screen, and wraps like
Java's `int`. Pausing holds it; a server tick freeze does not. This is not world game time
or the frame's animation clock.

`LevelRef::tick_rate(host)` reads the displayed client level's configured ticks per second
(`ClientLevel.tickRateManager().tickrate()`), not measured TPS or the frame interpolation rate.
Freezing the level does not change the value. Both `render_frame_pre` and `render_frame_post` can
read it. On the server, for a preview level or stale handle, and without a displayed level, it
returns `Ok(None)`; a host that cannot read the level returns a typed refusal.

Two reads give the facts the HUD picks its contextual bar from (the jump bar, the locator bar or the
experience bar): `ClientHost::jumpable_vehicle()` is the vehicle the local player controls that can jump now
(`LocalPlayer.jumpableVehicle`: the player is its first passenger, on a saddled horse, donkey, mule, nautilus or
camel, and the camel is not sitting; never a pig or a strider, and not for a second passenger),
and `ClientHost::has_waypoints()` is whether the server tracks any waypoint for this client
(`ClientWaypointManager.hasWaypoints`). The HUD reads the same state. The bar it chose is not exposed, as
vanilla's `Hud.nextContextualInfoState` is private.

`ClientHost::gui_scale()` reads the GUI scale in effect (`Window.getGuiScale`), at least 1,
or `None` without a window (a server, a headless run). Vanilla's `Window.calculateScale` resolves
it from the GUI Scale option and the window size: the option is a maximum, where 0 means automatic,
so a large window still resolves below it, and with the Unicode font forced an odd result is raised
by one. `ClientHost::gui_options().gui_scale` is only that option. Divide `ClientHost::window_size()`
by this scale, rounding up, for the GUI-scaled width and height (`Window.getGuiScaledWidth/Height`).

`ClientHost::window_size()` reads the window's width and height in framebuffer pixels
(`Window.getWidth`, `Window.getHeight`), whatever the GUI scale, or `None` without a window. Read it
at any time, not only in `window_resize`: that hook reports this same size, so the two always agree.
In a draw callback,
`draw.size()` on the `DrawContext` returns the GUI-scaled width and height of the drawing area.

`LevelRef::brightness(host, layer, pos)` (`BlockAndLightGetter.getBrightness`) reads the stored light
(0 to 15) for the selected sky or block layer. On a client it reads what the server sent for the level
named by the handle, whether shown or [open for viewing](#viewed-levels). It returns `None` before
login, for a closed view or stale handle, or for a position outside build height or in an unloaded
chunk. It never loads chunks. `LevelRef::block_face_sturdy(host, pos, face, support)` uses the current
level's local shapes for a loaded block inside build height. On a client, `LevelRef::can_see_sky(host, pos)`
only reads the shown level's published sky light, even outside build height or in an unloaded column.
See [Level hosts](../../concepts/hooks-and-the-host.md#level-hosts) for the boundary rules.

`ClientHost::playing()` is true while the player is in the world with nothing over it: no screen, title, sign
editor or book, so in-game keys and camera effects apply. A HUD or screen hook can read held state, the current
binding, or its Controls label using a [live vanilla mapping](input.md#reading-live-vanilla-mappings) retained
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
holds none) and `ClientHost::set_clipboard_text(text)` writes it (`setClipboard`; empty text is ignored and
leaves the clipboard unchanged). Both work from any client callback (a tick, a frame, a payload handler, an
input event, a screen hook), as vanilla's `KeyboardHandler` does. Without a native window (a server or a
headless host) the read is `None` and the write is `false`. Both take `&self`, so a screen hook can call them.

## Camera snapshots and perspective

The `ClientHost::camera()` helper returns the most recently extracted world frame's camera, or none before one exists. Its position, interpolated eye height, partial tick, angles (`yaw`, `pitch`, and `roll`), final FOV, near/far planes and dimension accompany column-major view and projection matrices. The view is camera-relative and includes roll, bob and spin; projection uses reversed Z. Earlier callbacks in a frame can see the previous frame.

Resolve a level with `host.level(dimension)` and use
`level.visit_block_collision(host, pos, &mut visit)`. The visitor receives block-local
`[min_x, min_y, min_z, max_x, max_y, max_z]` boxes with an empty collision context.
Return `ControlFlow::Break(())` to stop at a box, or `ControlFlow::Continue(())` to keep
going. The same value is returned in `Ok`; unloaded positions and shape query failures
visit no boxes. The current and viewed client levels are read separately; an invalid handle
or unavailable phase returns a `LevelReadError`.

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
`calculate_player_turn` changes mouse sensitivity and cinematic smoothing (`MouseHandler.turnPlayer`);
`player_turn` scales or adjusts the relative `Entity.turn` delta across mouse, controller and keyboard look;
`set_local_rotation` sets the view directly.

### Camera angles, camera position, and view transforms

Three separate client hooks match vanilla's and NeoForge's distinct camera stages:

- `compute_camera_angles(partial, yaw, pitch, roll)` matches NeoForge's
  `ViewportEvent.ComputeCameraAngles` inside `Camera.alignWithEntity`. It starts from the camera
  entity's interpolated `(view_y_rot(partial), view_x_rot(partial), 0.0)` and runs **before**
  `Camera.setRotation`, third-person front mirroring (`yaw + 180`, `-pitch`, `-roll`), detached
  back-off along the rotated look vector, and sleeping bed rotation. Edit the event's `yaw`, `pitch`,
  and `roll` fields directly in degrees, without filtering. Like NeoForge, the hook skips the interpolating minecart rider
  branch; it also skips the studio's custom camera, which does not align to an entity.
- `camera_position(partial, position)` sees the final world-space position (`Camera.setPosition`),
  including the interpolating rider's placement or the studio's selected camera. Read the current
  coordinates with `event.position.coordinates()` and pin X with `event.position.set([Some(x), None, None])`.
  A non-finite position edit returns `false` without changing any axis. The overridden position and
  effective angles drive the view and directional audio, without changing the entity's eye and look or
  the player's position and aim.
- `render_view_transform(partial, transform)` runs at vanilla's `GameRenderer.bobHurt` / `bobView`
  stage, after hurt tilt and view bobbing, with a `ViewTransform` that starts at identity each frame. Compose local translations, rotations, or scales onto it in
  dependency order for presentation-only effects such as camera shake, recoil, or lean without moving
  the camera, player or audio listener. It runs even while a screen has input focus if the world bob
  stage still runs. If the transform or any affected frame matrix cannot be inverted finitely, the
  operation returns `false` without changing the transform. Read `ClientHost::gui_options().screen_effect_scale` (`Options.screenEffectScale`)
  when your effect should respect the player's Distortion Effects slider. Sunrise applies no artificial
  angle or offset caps.

`ClientHost::set_camera_type(type)` is a queued client-only effect and takes `&self`, so
shared callbacks such as `compute_camera_angles` or `camera_position` can request a perspective.
`camera_type()` reports the latest request immediately; several requests coalesce to the last one.
Requests before or during `compute_camera_angles` apply before that frame's detached camera
placement, FOV and spectator post effects. Requests during `camera_position` refresh the entity
alignment using the already-computed angles, carrying only explicitly pinned axes (even when their
values match the original placement).
They do not re-position a selected custom camera. Requests from later render callbacks apply at the
next frame's extraction. Setting a type never emits
`camera_type_cycle`; F5 alone cycles, and its final event choice wins over requests made inside the
cycle callback.

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
arriving and leaving (a column that merely leaves the view stays loaded, out of view, and is told only
once its data is freed); `client_block_changed(dimension, pos, old, new, predicted)` tells each block of a
held column that changes state, after the write, so `host.level(dimension)` already reads `new` there:

```rust
{{#include ../../../examples/src/client_misc.rs:client_block_changed}}
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
`block_property`, `clip`, [`visit_block_collisions`](particles-and-movement-queries.md#bot-movement-and-block-collisions),
`has_collisions`, `visit_block_collision`, `get_enchant_power_bonus`, and the fluid reads built on `block_state` (`fluid_shape`, `fluid_height`,
`fluid_height_for_camera`, `fluid_can_be_replaced_with`). So a mod can aim a ray or collide a box
through a portal. `brightness` also reads the viewed level's own sky or block light, as sent by the
server, not the light of the shown level. These reads never load a column. Direct block reads return `None` for a column the client doesn't hold; `clip` stops at the first
cell of a column the client doesn't hold with `ClipOutcome::Unloaded { at }`, reads out-of-height
cells as air, and returns `Err(ClipError::StaleLevel)` for a stale handle; `visit_block_collisions`
and `has_collisions` skip missing columns and keep scanning the loaded ones.

`host.level` returns `None` while the server has the level closed, and reads from a retained handle
return `None`. If the server reopens the level within the same client level, the old handle reads
again. A login, a reconfiguration or a respawn into another level replaces the client level: the old
handle stays stale, and the mod calls `host.level` again once the server reopens the level. A respawn
in the same level drops the viewed levels too, but keeps the client level, so the old handle reads
again after the reopen. Writes, server-only reads and the other level reads refuse viewed levels.
In particular,
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

`ClientHost::view_hit()` and `raycast(distance)` cast their own ray along the view
across loaded chunks only: far terrain is never hit. `raycast` stops at `distance`
(returning `None` for nonpositive or non-finite values), while `view_hit` traverses the
full loaded-chunk extent. `view_hit` clips each candidate using that block's geometry,
so loading an unrelated chunk does not move an inside-block hit. Both return `None` on a miss.

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
for a tracked target they use the client's live body. Block, item and text displays
have coincident box corners at the tick position returned by `position(host)`. That
position follows the tick's movement interpolation, not the packet destination or
the draw-frame pose. A display intersects a box query only strictly inside all faces;
its render culling box does not count, and the ordinary crosshair pick still ignores it.
End crystals expose their 2 x 2 box; dragon parts expose their own box. Stale handles
and entities the client cannot read return `None`.

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
- `host.client_session().client_tab_list()` borrows the live Tab overlay view.
  `Ok(None)` means no logged-in client world; `Err(ClientSessionReadError::Unavailable)`
  means the phase or backend cannot supply it. A logged-in world can have an empty list.
  `list.visit_entries(&mut visit)` lends one row at a time in overlay order
  (`PlayerTabOverlay.getPlayerInfos`), only up to the 80 listed players it draws.
  Use `row.name()` for the borrowed profile name, and `row.uuid()`, `row.game_mode()` and
  `row.latency()` for values. Return `ControlFlow::Break(())` to stop or `Continue(())` to
  keep visiting. For display text, call `row.copy_display()`: the name the server sent or,
  otherwise, the profile name with team decoration; spectators are italic. Use
  `list.copy_header()` and `list.copy_footer()` for the optional header and footer.
  A missing or cleared header or footer is `Ok(None)`; a text conversion error is returned
  only by the copy you request, not by the view or visit. No plaintext is substituted.
- `ClientHost::client_session().client_waypoints()` is the locator bar's tracked waypoints:
  `ClientWaypoints { entries }`. Each `ClientWaypointEntry` has the waypoint's `id` (`Uuid` for
  a player's waypoint, `Name` for a named one), its `target` (`Block`, `Chunk`, `Azimuth` or
  `Empty`), the icon `style` (`waypoint_style` asset id) and the icon `color` as `0xRRGGBB`
  (`None` when the server sent none — the bar then colours the dot from the waypoint's id).
  With a camera entity the entries follow `ClientWaypointManager.forEachWaypoint`: farthest
  first by the vanilla squared distance, ties in no promised order; without a camera entity,
  in no promised order. It is what this client has been sent and
  changes as the server tracks and untracks waypoints; `None` outside a world.
- `ClientHost::client_session().client_player_skins()` is the listed players' skins as the Tab
  overlay draws them: `ClientPlayerSkins { entries }`. Each `ClientPlayerSkin` has the player's
  `uuid`, the skin texture path `skin` (`skins/<hash>` for a downloaded skin, or a default like
  `entity/player/wide/steve`), and `slim` for the Alex model. Draw the face with a `GuiDraw::Blit`
  of uv [8, 8], src [8, 8] over tex [64, 64], then the hat at uv [40, 8] — the same blit the tab
  list uses. Entries come in no order; join on `uuid`. It is what this client has been sent and
  changes as skins download; `None` outside a world.
- `ClientHost::client_session().client_titles()` is the title, subtitle and action bar on screen
  now: `ClientTitles { title, subtitle, action_bar }`. Each is `None` once its timer has run out,
  and the subtitle shows only with a title. It is `Ok(None)` outside a world; a text conversion
  error is returned for the failing read, never plain words.
- `ClientHost::client_session().client_boss_bars()` is every boss bar the client holds, in
  server order: `Vec<ClientBossBarEntry>` with each bar's `name` and `progress`
  (0 to 1, as drawn). The HUD draws only those that fit within `guiHeight / 3`.
  It is `Ok(None)` outside a world; a text conversion error is returned
  for the failing read, never plain words.
- `ClientHost::local_item_use()` reports ordinary item use: `ItemUse { hand, ticks_used, ticks_remaining }`
  (`isUsingItem`, `getUsedItemHand`, `getTicksUsingItem`, `getUseItemRemainingTicks`).
  `ticks_used + ticks_remaining` is the use duration. It is `None` when no item is in use or no world
  is open. Track intercepted main-hand input through [`ClientItemUseBehavior` callbacks](../../concepts/parts/item-and-tag-parts.md#intercept-main-hand-client-input)
  instead; interception does not make the player use or block with the item.
- `ClientHost::client_session().client_fishing_hook()` is the local player's fishing hook:
  `ClientFishingHook { position, biting }`. `position` is the bobber's position as the client
  tracks it; `biting` is true while a fish is biting. It is `None` outside a world or when
  no hook is out.
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

`ClientHost::jump_riding_scale()` is how far the local player has charged the jump of its jumpable vehicle (`LocalPlayer.getJumpRidingScale`), 0.0 to 1.0; the jump bar draws the same value.
