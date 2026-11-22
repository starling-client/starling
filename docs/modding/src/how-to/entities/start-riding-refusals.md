# Why a mount was refused

`EntityRef::start_riding(host, vehicle, force)` seats a player, mob or declared general entity on
a mob, boat, minecart, declared general vehicle or generic seat, and answers `Ok(())`. Where it
refuses, it names the cause as a
`StartRidingRefusal`:

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

## Why nothing rides a player

Vanilla's `Entity.startRiding` refuses a vehicle whose entity type is not saved with the level
before it reads `force`, and the player type is one: a player's save keeps no passengers, so a
passenger on a player would have nowhere to be written. `start_riding` keeps that refusal, forced
or not, and names it `PlayerVehicle`.

## When `entity_mount` fires

`entity_mount` fires only after every other check passed: a refusal at
those checks reaches no hook. (This is this engine's order; moving the hook earlier is tracked in #1557.) The checks run again after the hook, since a hook can fill, remove
or remount the vehicle, and then name what the hook changed. Such a refusal means the requested
mount did not complete; what the hook moved or removed stays as it left it.

Some removal effects run while the entity still resolves. A nested `EntityRef::remove` or
`EntityRef::discard` for that same entity returns `false` while those effects run.

When a vehicle is destroyed or discarded, its own dismount is offered first, then each direct
passenger's in passenger-list order. The removed entity no longer resolves during these calls. A vetoed mob rider
of a removed boat, cart or mob keeps its edge and retries after the next eligible, uncanceled
`entity_tick_pre`, before it ticks. A rider refused level residency does not retry on its own.
