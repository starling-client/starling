# A mob riding a player

Java's `Entity.startRiding(vehicle, force)` takes any entity as the vehicle, a player included. Tinkers' Construct's
piggyback pack uses this to put a mob on its wearer. A server mod does the same with `EntityRef::ride_player`:

```rust,ignore
use mod_api::{EntityRef, RidePlayerRefusal, ServerHost};

fn pick_up(host: &mut dyn ServerHost, mob: EntityRef, player: EntityRef) {
    match mob.ride_player(host, player, false) {
        Ok(()) => {}
        // The player already carries one: Player keeps Entity.canAddPassenger's one passenger.
        Err(RidePlayerRefusal::Full) => {}
        Err(other) => eprintln!("can't pick it up: {other}"),
    }
}
```

`ride_player` is the only way onto a player. `EntityRef::start_riding(host, player, force)` keeps refusing a player
vehicle, forced or not, as vanilla's `startRiding` callers and `/ride` do.

## What the game does

- **The seat.** Each tick the mob sits on top of the player's box for its current pose (`Player`'s default passenger
  attachment), less the mob's own vehicle attachment (`Entity.positionRider`). It follows the player as the player
  moves, crouches or swims, and its own AI doesn't walk it anywhere.
- **The mount path.** It is the one every other vehicle uses. `entity_mount` hears the mount, and the dismount from a
  vehicle the mob rode before, and can cancel either one. If it does, the answer is `Canceled`.
- **`force`.** As in vanilla, it bypasses `canRide` (the mob's boarding cooldown, wardens and withers) and the
  player's one-passenger limit. It doesn't bypass the other refusals.
- **Clients.** Players who track the vehicle hear the change from the tracker (`ClientboundSetPassengersPacket`). The
  vehicle player's own client is told by the server too, since no tracker tells a client about its own entity. That is
  the packet the piggyback pack sends its wearer itself. Clients draw the mob on top of the player's box for the pose
  its server synced (standing, crouching, swimming, gliding, riptide or sleeping).
- **Getting off.** `EntityRef::stop_riding` and `eject_passengers` take the mob off as they do for any vehicle. The
  mob also gets off at every save of the level or its entities, and when the player logs out, dies or changes level.
  `entity_mount` hears each of these; one a listener cancels still takes the mob off, since no save holds a player
  vehicle and a player can't carry a mob out of its level.
- **Reads.** `vehicle`, `is_passenger`, `passengers`, `is_vehicle` and `root_vehicle` answer as for any other
  vehicle, on the server and on a client after it applies a passenger list for the vehicle,
  including the player's own client when it receives that list.

## Refusals

| `RidePlayerRefusal` | When |
|---|---|
| `NoEntity` | A handle is stale, or the player is in another level. This is also the answer with no game backend. |
| `NotAPlayer` | The vehicle isn't a player. Use `start_riding` for other vehicles. |
| `PlayerRider` | The rider is a player. `/ride` refuses this too (`cant_ride_players`). |
| `UnsupportedRider` | The rider isn't a mob. |
| `SameEntity` | The rider and the vehicle are the same entity. |
| `AlreadyRiding` | The mob already rides this player. |
| `Cycle` | The player rides the mob, directly or through other vehicles. |
| `Full` | Unforced, and the player already carries a passenger. |
| `CannotRide` | Unforced, and the mob's boarding cooldown is running, or it is a warden or a wither. |
| `Canceled` | A mod's `entity_mount` canceled the mount or the dismount before it. |

## Limits

- Only a mob can ride a player. A player can't ride a player, and items, boats and other entities can't ride one
  either.
- A player's save keeps no passengers. A mob riding a player gets off before a save, so it is saved standing where it
  is, and it doesn't ride the player after a reload.
