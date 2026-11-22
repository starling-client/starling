# Entities, players and commands

Pages for mods that work with players, mobs and other entities, and with the server's commands and services.

For custom status-effect behaviour, see [Ticking a registered mob effect](entities/effect-ticks.md).

`EntityRef::body_yaw(host)` reads a mob's stored body rotation on the server, in degrees without normalization. This is separate from its head rotation. It returns `None` for stale or removed entities, clients, players and other non-mobs, including dragon parts. The ender dragon returns its stored flight/body yaw. In a mob goal, the active mob and disjoint live mobs are read from the goal's view.

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

Use `EntityRef::hurt` on the server with the fully qualified id of a registered damage type, vanilla
or modded. Hurt hooks see that exact id. The direct and causing entities are separate from the type:
passing a player as the cause of `minecraft:explosion` does not turn it into `minecraft:player_explosion`.
A return value of `true` means the hurt was accepted, not necessarily that the entity lost health.
For example, a heart-bound Creaking can react to a hit without losing health.

<!-- section-pages -->
