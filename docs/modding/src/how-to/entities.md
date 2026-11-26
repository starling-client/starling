# Entities, players and commands

Pages for mods that work with players, mobs and other entities, and with the server's commands and services.

For custom status-effect behaviour, see [Ticking a registered mob effect](entities/effect-ticks.md).

`EntityRef::body_yaw(host)` reads stored living body rotation, in degrees without normalization.
It is available for live server mobs, armor stands, the ender dragon and players with a live body;
on a client it reads only the local player's avatar. Stale handles, nonliving entities and dragon
parts have no body yaw. In a mob goal, the active mob and disjoint live mobs are read from the goal's view.

`EntityRef::set_body_yaw(server_host, degrees)` changes the current body rotation, not the previous
angle used for interpolation, the head, or the view. Like vanilla's `LivingEntity.setYBodyRot`, it
neither normalizes the angle nor sends a packet. The dragon's body turn does not steer its flight.
Vanilla's overrides still apply: silverfish and endermites also turn their view for a finite angle;
armor stands update the previous view angle and both head angles instead of assigning current body yaw.
On the client, use `PlayerRef::set_body_yaw(client_host, degrees)` on the local player. It changes the
live avatar's stored angle, not a pose already extracted for rendering. For a player riding a living
mount, pose extraction uses the mount's interpolated body and the rider's head instead. A boat assigns
its own yaw directly to its passenger when positioning them.
Both setters return `Ok(false)` for an inapplicable or stale handle and a typed refusal from a host
that cannot write living state.

Use `entity.set_persistent(host, false)` from a server hook to keep a temporary entity, such as a display,
out of future world saves. It stays live until it is removed or its chunk unloads. Passing `true` restores
the normal save rules; neither call alters an existing snapshot, and both keep the same UUID the entity
would save with. To omit a vehicle and all its passengers, set the flag on the root vehicle, not on a rider.
The flag follows the live entity across dimensions but is never saved. The setter returns `Ok(false)` for players, dragon parts and handles that do not resolve in the current level.

`EntityRef::set_on_ground(host, grounded)` is a server write to a live body's ground flag. Passing `true`
checks for a supporting block under the current footprint, without moving the entity. The first miss
keeps its prior support; another miss clears it. Passing `false` clears support immediately. Use
`entity.on_ground(host)` to read the flag and `entity.on_block(host)` to read the block position vanilla
uses for contact. If the support check cannot finish, the call returns `Unavailable`: the new flag takes
effect, but the previous supporting block stays recorded. This write does not apply landing effects or
change velocity or fall distance.

Use `entity.visit_parts(host, &mut visitor)` to visit the entities of a multipart body, not the mod API's
behaviour Parts. It visits the ender dragon's children in `EnderDragon.getSubEntities` order. The visitor
takes an `EntityRef`; return `ControlFlow::Break(())` to stop or `ControlFlow::Continue(())` to keep going.
`Ok(Continue(()))` includes entities with no parts. A stale handle returns `Err(EntityReadError::Stale)`.
If the world is unreadable, the body is parked or only packet-tracked, or the view cannot supply all
its parts, the call returns `Err(EntityReadError::Unavailable)` before visiting any.

Use `EntityRef::hurt` on the server with the fully qualified id of a registered damage type, vanilla
or modded. Hurt hooks see that exact id. The direct and causing entities are separate from the type:
passing a player as the cause of `minecraft:explosion` does not turn it into `minecraft:player_explosion`.
A return value of `true` means the hurt was accepted, not necessarily that the entity lost health.
For example, a heart-bound Creaking can react to a hit without losing health.

<!-- section-pages -->
