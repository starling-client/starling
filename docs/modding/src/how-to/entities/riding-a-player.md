# Mounting and riding entities

## A mob riding a player

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

### What the game does

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
- **Reads.** `vehicle`, `is_passenger`, `visit_passengers`, `is_vehicle` and `root_vehicle` answer as for any other
  vehicle, on the server and on a client after it applies a passenger list for the vehicle,
  including the player's own client when it receives that list.

### Refusals

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

### Limits

- Only a mob can ride a player. A player can't ride a player, and items, boats and other entities can't ride one
  either.
- A player's save keeps no passengers. A mob riding a player gets off before a save, so it is saved standing where it
  is, and it doesn't ride the player after a reload.

## Mounting other vehicles and refusal causes

`EntityRef::start_riding(host, vehicle, force)` seats a player, mob or declared general entity on
a mob, boat, minecart, declared general vehicle or generic seat, and answers `Ok(())`. Where it
refuses, it names the cause as a `StartRidingRefusal`:

| Refusal | Cause |
| --- | --- |
| `NoEntity` | A handle is stale or held out, or names an entity that was removed, is dying, or is in another level. Also the answer with no game backend. |
| `PlayerVehicle` | The vehicle is a player, forced or not. |
| `UnsupportedRider` | The rider is not a player, mob or declared general entity. |
| `UnsupportedVehicle` | The vehicle cannot carry passengers, for example a marker or a general type declared `NoSave`. |
| `SameEntity` | The rider is the vehicle. |
| `AlreadyRiding` | The rider already rides this vehicle. |
| `Cycle` | The vehicle rides the rider, directly or through other vehicles. |
| `CannotRide` | Unforced, the rider may not ride now: a sneaking player, a mob or general rider in its boarding cooldown, a warden or wither. |
| `Full` | The vehicle has no room. `force` passes a mob's or a seat's count, not a boat's or a minecart's storage limit. |
| `Canceled` | A mod's `entity_mount` hook canceled the mount or the dismount from the old vehicle. |

```rust,ignore
match rider.start_riding(host, vehicle, false) {
    Ok(()) => {}
    Err(StartRidingRefusal::Full) => { /* try another seat */ }
    Err(other) => { /* `other` displays its cause */ }
}
```

The enum is `#[non_exhaustive]`: keep a wildcard arm.

### Why `start_riding` refuses a player vehicle

Vanilla's `Entity.startRiding` refuses a vehicle whose entity type is not saved with the level
before it reads `force`, and the player type is one: a player's save keeps no passengers, so a
passenger on a player would have nowhere to be written. `start_riding` keeps that refusal, forced
or not, and names it `PlayerVehicle`.

### When `entity_mount` fires

The initial ride and capacity checks run before `entity_mount`, so `CannotRide` and `Full` at
that stage never reach the hook. Once those initial eligibility checks pass, `entity_mount`
fires. If a listener vetoes the mount or changes the rider's current vehicle, `start_riding`
returns `Canceled` immediately. After an allowed callback, eligibility is checked again—since a
listener can fill, remove or remount the vehicle—and checked once more after dismounting any
previous vehicle. If any recheck fails, the refusal names the cause; the requested mount does
not complete, while the listener's own changes remain.

Some removal effects run while the entity still resolves. A nested `EntityRef::remove` or
`EntityRef::discard` for that same entity returns `false` while those effects run.

When a vehicle is destroyed or discarded, its own dismount is offered first, then each direct
passenger's in passenger-list order. The removed entity no longer resolves during these calls. A vetoed mob rider
of a removed boat, cart or mob keeps its edge and retries after the next eligible, uncanceled
`entity_tick_pre`, before it ticks. A rider refused level residency does not retry on its own.
