<!-- chapter: Moving a player -->
<!-- separator: 1 -->

# Moving a player

`player.entity().move_by(host, delta, cause)` (`EntityRef::move_by`, the call that moves any other entity, on a
`&mut dyn ServerHost`) moves a player by `delta` through the level as `Entity.move` would: for non-spectators, the
move resolves against block shapes after applying the player's collision exclusions and against other entities'
colliders, taking a step up where allowed; spectators bypass collision. It returns the displacement made. As for every entity, a
`delta` beyond 64 blocks on an axis is refused. The `entity_move` hook hears it once, for
the player and with `cause`, and a listener may replace or cancel it as for any other entity (a cancelled move is
`Some([0.0; 3])` and the client is told nothing); `cause` changes only what the hook hears, not the physics.

The server's copy moves at once, and the player's client is sent a relative teleport: it adds the displacement to where
it is when the packet arrives and keeps its motion and look, so what it walked in the meantime is kept. Until it
confirms, which a client does at the start of its next frame, the server drops the position of the player's own reports
(their rotation is kept, and nothing is counted against the player).

So `move_by` is for a one-off move, a launch or a pull, and not for every tick: a development build logs a warning when
one player is moved on 20 ticks in a row. For motion on every tick, use `player.entity().push(host, delta)` from the
server half of the mod, or move the player from the client half of the mod.

A player on a moving collider stays on it when the target is still over it, and lets go otherwise. The call is refused
(`None`, nothing moves) for a value that is not finite, a target outside the world, a player that is riding something,
a sleeper, a player without a connection, or a move the collision query cannot answer.

## Velocity and fall distance on the server

Use `player.entity().set_velocity(host, [x, y, z])` to replace a player's velocity, in blocks
per tick, or `player.entity().push(host, delta)` to add to it. The server updates the player's
body and sends the result to the player's own client, which uses it on its next movement tick.
Both return `false` without changing motion for a stale or unavailable body (including a player
whose turn is suspended) or non-finite input. These calls also work on simulated non-players.

`entity.fall_distance(host)` reads the server's accumulated fall distance in blocks, or `None`
for a stale or unavailable body. For a player the server tracks it from accepted moves and resets
it on landing. `entity.reset_fall_distance(host)` clears the server's value without moving the player.

## Checking what blocks a move

On either side, `entity.piston_push_reaction(host)` reads vanilla's piston reaction, including
`PushReaction::IgnoreEntity` for marker armor stands. `entity.is_hanging(host)` reads the class:
paintings and frames qualify; leash knots do not. Both take `&dyn Host` and return `Ok(None)` for
a stale handle or `Err(LevelReadError::Unavailable)` if the host does not support that read.
A general type can supply a reaction to `declare_general_entity`; pass `None` to use
`PushReaction::PushPull`.

## The client half

The client moves its own player. Write a prediction (a conveyor moving a player, boots that hover) from
`player_tick_pre` on `HostSide::Client`, when `host.local_player() == Some(event.player)`, before the player's own
movement:

- `host.move_local_player_by([x, y, z])` applies a displacement in blocks as `Entity.move(MoverType.SELF, delta)`.
  It steps up and collides as the player's own move does, including moving colliders and collision exclusions.
  It updates the ground flag and fall state, and has the usual effects on velocity. It returns `Ok(Some(delta))`
  with the displacement actually made. It returns `Ok(None)` without moving if there is no local player, the
  player is dead, sleeping or riding, or a component is non-finite or outside ±64 blocks. An unavailable world
  phase or a move the engine cannot finish returns `Err(LevelWriteError::Unavailable)`. Other hooks with a
  writable client world host, such as `client_tick_pre` and `input_key`, can also call it. The client does not
  fire the server's `entity_move` hook. A crossing notifies `entity_entering_section` at the position write,
  before the move updates the ground flag, even when another move returns the player to its starting section in
  the same tick.
- `host.set_local_player_velocity([x, y, z])` replaces the local player's velocity, in blocks per tick (`false`, and
  nothing changes, for a value that is not finite or with no player of its own).
- `host.set_local_player_on_ground(flag)` sets its on-ground flag, which this tick's jump decision reads and the move
  then sets again from the collision (`false` with no player of its own).
- `player.reset_fall_distance(host)` clears the local player's accumulated fall distance without moving it.
  It returns `Ok(false)` for a stale or non-local handle or when there is no local player, and
  `Err(LevelWriteError::Unavailable)` if the hook has no writable client world, including while
  `entity_entering_section` borrows it.
  It does not reset the server's copy.

None of these calls sends a packet. The normal position report carries the movement and ground flag, not the
fall-distance reset. The server checks the report as usual, correcting a move its speed or collision check refuses.
A server velocity write reaches that player's client by a motion packet, which replaces the
client's velocity outright.

On either side, `player.supporting_block(host)` returns the player's recorded support block,
which may be beside the column under their feet. `player.on_block(host)` reads vanilla's
`getOnPos()`: the support when present, otherwise the block at the feet with vanilla's small
downward offset. The client reads its own player's last move; other players return `Ok(None)`
there.

At `player_tick_pre` the player's input fields still hold the previous tick's applied values. To read or edit this
tick's input, use `movement_input_update`, which lends the sampled input before auto-jump and sprint decisions:

```rust,ignore
fn movement_input_update(
    &self,
    _host: &dyn mod_api::ClientHost,
    event: &mut mod_api::MovementInputUpdateEvent<'_>,
) {
    event.input.move_vector[1] *= 1.3; // Scale forward movement without changing the keys.
}
```

The vector is `[left, forward]`. `event.input.key_presses` holds `forward`, `backward`, `left`, `right`, `jump`,
`sneak` and `sprint` separately. Editing a direction key does not recalculate the vector; scaling the vector does
not change those keys. The server receives the final flags, including a jump press added by auto-jump, not the
vector. Sprint is a key press, not a sprint-state switch: vanilla eligibility, latched sprint and double-tap still
apply. A listener that only reads the input does not change sprinting.
