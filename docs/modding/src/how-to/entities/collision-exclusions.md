<!-- chapter: Collision exclusions -->
<!-- separator: 1 -->

# Collision exclusions

A portal in a wall is a hole only in the picture: the wall behind it still stops the player who walks in. A mod opens it
with collision exclusions, boxes of the player's own level (`[min_x, min_y, min_z, max_x, max_y, max_z]`) in which that
level's blocks don't stop the player, as Immersive Portals' `CollisionHelper` does:

- the part of a block's collision shape inside a box doesn't collide (`Shapes.join(shape, box, ONLY_FIRST)`), so a box
  over half a block leaves the other half solid;
- a block sharing volume with a box is no wall to suffocate in (`isInWall`), the client doesn't push its player
  out of it (`moveTowardsClosestSpace`), and it doesn't act on a player inside it (`checkInsideBlocks`: no cobweb,
  powder snow or damaging block there);
- entity colliders (boats, shulkers) and the world border still stop the player.

There are two halves, and **the mod sets both**: the engine sends neither set to the other side.

- On the server, `player.entity().set_collision_exclusions(host, &boxes)` (`EntityRef`, `&mut dyn ServerHost`). The
  server checks the player's moves through these boxes, so a move through an excluded wall isn't sent back.
- On the client, `host.set_collision_exclusions(&boxes)` (`ClientHost`), for the player the client moves itself.

A client half with boxes the server half doesn't have walks through a wall the server sends it back from; the other
way round, the client stops at a wall the server would have let it through.

Each call replaces the last set, and an empty slice clears it. A change of level, a respawn and leaving clear it too,
on each side; a mod that still wants boxes after a respawn sets them again (the client's `client_player_clone` hook is
the moment). Nothing is saved. Only players keep a set.

A call is refused with a `CollisionExclusionsError`, and the set there was stays:
`NotFinite` for a value that is NaN or infinite, `Inverted` for a box whose minimum is above its maximum on an axis,
`NotAPlayer` for another entity, and `Stale` for a stale handle or a client with no player of its own.

Use `player.collision_flags(host)` to read what the last move actually hit, on the side
that moves the player. `vertical` is set for either a floor or a ceiling; `vertical_below`
distinguishes the floor. The client reads its own player's prediction. The call returns
`Ok(None)` for a stale or untracked player or a connected server guest whose reports do not
retain collision flags. It returns `Err(LevelReadError::Unavailable)` when the host cannot
read player motion. It does not repeat the collision query.

A player with no boxes pays one empty-slice check per move. With boxes, each block shape a box reaches is joined
with it, which allocates, and the server copies the guest's set into its body every turn.

## Moving colliders

A native mod gives an entity boxes that every other entity collides with:

```rust
{{#include ../../../examples/src/entity_and_native.rs:custom_collider}}
```

The boxes are in the entity's own frame, placed in the world by `ColliderPose` (`rotation` about
`pivot`: `entity_position + pivot + rotation * (local - pivot)`). `rotation` is a quaternion
`[x, y, z, w]` normalized when set, and `pivot` is `[x, y, z]` in blocks from the entity's
position. Other entities collide with the boxes as solid ground and walls: the entity's box is
swept against each turned box (a separating-axis test with the time of impact), it steps up onto a
top within its step height, and standing on a tilted top follows the surface. The collider is solid
to every mover except spectators, entities with no physics and the entities in its own vehicle stack
(the root vehicle and every passenger, at any depth, the collider's entity among them). Collider
boxes are solid but do not supply block contact effects such as step sounds, sprint particles,
friction or bounce.

The game checks a collider when it is set and refuses by name (`ColliderError`), keeping the old one:
every box and pivot coordinate must be finite, the rotation must be a finite, non-zero quaternion
(normalized when set; zero, NaN or infinite quaternions are refused with `NonFinite`), and the
collider's wire encoding must fit within one clientbound custom payload
(`ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE`, 1,048,576 bytes, refused with `TooLarge`). Boxes
are kept as `f32`; read back, they are the rounded values both sides collide with. A box whose
minimum is above its maximum is read with the corners swapped, and a box with no volume (zero
thickness on an axis) collides with nothing.

A collider is saved with the entity. The server sends it only to Sunrise clients that track the entity:
the whole collider when they start to (in messages of at most 64 KiB), then only what changed, by box
index, and the pose alone for `set_collider_pose`. Clients interpolate between two poses by moving
the pivot linearly and turning by the shortest arc about it. **A vanilla client does not see a collider**:
it sees the entity and walks through it. A mod that needs players to stand on one requires the Sunrise
client at join.

### Carrying

A collider that moves carries what stands on it. Once a tick, after the level's entities ticked
(so whatever order they ticked in), every body standing on a box of a collider that moved or turned
this tick moves by the displacement of its contact point, the point of the box under its feet: the
translation and the rotation of the collider's change, as an ordinary collided `Entity.move` with the
cause `MoveCause::Carried`. An `entity_move` listener sees it and may cancel or replace it like any other
move. It stops at walls and steps up like any move, and yaw is never turned. A body's support is the box
under its feet with the highest top (a tie goes to the lower entity id). A collider standing on another
collider is carried first, and the bodies on it take its whole displacement. A body that rides something is not carried itself; it
follows its vehicle. The server carries the entities it simulates, and each Sunrise client its own player.

After carrying, a collider that moved or turned shoves bodies in the simultaneous swept path of its
boxes (conservative advancement over linear pivot motion and shortest-arc rotation together). This
also uses `MoveCause::Carried`. Changing a box's bounds does not sweep that box between the old and
new shape.

The velocity response follows Create's `ContraptionCollider.collideEntities`. Once a shove actually moves
a body, the component of its velocity pointing into the shove is removed; velocity along the shove and
tangential to it stay. An upward shove never lowers vertical velocity below the collider's own vertical
speed, so a body already rising faster keeps it. Riders already carried by the collider are not shoved
again. Vanilla pistons and shulkers keep their ordinary velocity behaviour. The server shoves non-player
entities and never a player; the client predicts the shove for its own player, and ordinary server
movement checks still apply.

Neither carrying nor shoving pushes through a wall, and the collider is never stopped by a body in
its way. A squeezed body stays where the collision leaves it. A living body still takes `in_wall`
damage while a box covers its eyes, as in a block. The shove itself does not deal impact damage.

### Players on a moving collider

A client moves its player with the collider as it last heard of it, so the position it reports is in a
frame that is some ticks old: the round trip, plus how stale its copy was. The server therefore keeps
each collider's recent transforms (about a second of them: about 2.8 KiB of frame data per collider that
moves, plus the container and index around it, and one entry for one that stands still) and judges a report against the collider as it was at some age
around the one it expects, the player's round-trip time plus two ticks, give or take four. A report
fits an age when the player stands on the top of a box (within 0.05 blocks) or is above one, is over
it, and is not inside any box deeper than that. Of the ages that fit, the one nearest the expected age
wins. The move from the last accepted position, measured in the collider's frame (so the speed test sees how far the
player walked on the collider, not how far the collider carried it), then goes through the
ordinary move check of a walking player, with the colliders placed where they stood at that age, so
walls, blocks beside the track and the collider itself stop a carried player as they stop any other,
and a player already touching one box of a collider may not walk into another box it did not touch.
The position the server keeps for an accepted report is the same place on the collider under its
transform now, not the position in the older frame the client reported in. A player on a collider's box
is held up by it, so standing on a collider is never counted as floating. A
Sunrise client also says how old its copy of the collider was before each report
(`sunrise:carry_age`); the server only starts its search there, and the report is still checked
against the same window.

A report that fits no age in the window is not carried: if the player was on the collider, the ordinary
rules judge the move, and if they refuse it the player is sent back to where the server last had it
on the collider, placed under the collider's transform now. Such a correction is counted per player and
never counts toward the floating-player kick. A teleport (a portal, a command) and a collider
that is gone let go of the player.

What it holds for: Create's speeds, 1.4 blocks a tick on straight track (2.0 powered), and 0.7 and 1.0
on a curve of 4 degrees a tick, at round trips from 50 to 200 ms and a tick of jitter either way, never
refuse an honest player, and the daily tests run curves up to 2.0 blocks a tick the same way. **The
limit is the age:** the window is four ticks either side of the age the server expects, and on a curve a
report held to a wrong age moves the far end of a long collider sideways, so a round-trip estimate that
is off by more than the window, or a client that is more than four ticks stale, is refused. A collider
that goes faster or turns harder than 2.0 blocks and 4 degrees a tick is not covered by any test.
Past a second of round trip the window stops growing: a player whose copy is
older than it reaches is judged as a walker in the frame the cap allows, not carried. A vanilla client, which sends no age, gets the
window around the expected age.
