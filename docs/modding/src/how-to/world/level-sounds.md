# Level sounds

Implement `play_level_sound_at_position` or `play_level_sound_at_entity` to edit
or cancel a level sound before it is broadcast (server) or played (client).
Match the hook's `HostSide` to distinguish the requests; the event's sound,
source, volume and pitch describe that side's request, and original volume and
pitch remain available after edits. Returning `Flow::Cancel` suppresses that
request. A server cancellation sends no packet, so there is no client request. With no
cancellation, the sound engine interprets the supplied volume and pitch; the level hooks do not
apply a separate volume or pitch check.
Sounds played from inside a `play_level_sound_at_position` or
`play_level_sound_at_entity` listener are not re-dispatched to level-sound
listeners; they are validated and delivered or journaled normally in call order.

Incoming sound packets, including `/playsound`, ask the client level hooks.
An entity-sound packet stays bound to its emitter. UI clicks, engine-local
sounds and `ClientHost::play_local_sound` do not ask these level hooks; the
sound engine's `play_sound` hook remains a separate, later interception point.

Client-predicted level sounds and `LocalPlayer.playSound` use those same hooks.
Canceling their sound does not cancel the player action or undo a predicted
block change. A local player's ordinary sound is an `AtEntity` request but
plays at the requested position; only a genuinely entity-bound sound
(such as a goat horn) follows its emitter after playback starts.

| Client route | Level event | Vanilla call being represented |
| --- | --- | --- |
| Placement confirmation, predicted doors/buttons/levers | AtPosition | `BlockItem.place`, block `useWithoutItem` → `Level.playSound` |
| Waxing, tool transforms, bucket use | AtPosition | `HoneycombItem.useOn`, tool `useOn`, `BucketItem` → `Level.playSound` |
| Game-event arrow hit (Players), pufferfish sting (Neutral), elder-guardian curse (Hostile) | AtPosition | `ClientPacketListener.handleGameEvent` → `Level.playSound` |
| Own steps, swimming, splash, amethyst chime, honey slide, fall | AtEntity | `Entity`/`LivingEntity` → virtual `LocalPlayer.playSound` |
| Own hurt/death, bundle menu clicks, consume effects | AtEntity | `LivingEntity`, `BundleItem`, `Consumable` → `LocalPlayer.playSound` |
| Player-bound sounds, including the local goat horn | AtEntity | `Level.playSound(except, entity, ...)` |
| Block-breaking/level-event effects, explosions, pickup packets | None | Client-level `playLocalSound` or direct sound manager playback |
| Dragon/fang/sniffer local effects, ticking ambience, UI | None | Entity-local effects, sound instances or widgets, not `playSeededSound` |

## Block hit sounds

Use a block part's `play_hit_sound` for client mining sounds. The event supplies the
block state, position and face. Play your sound through the supplied `ClientHost` and
return `Flow::Cancel` to suppress the vanilla sound for that hit.

The default plays the state's hit sound at volume `(volume + 1) / 8` and pitch
`pitch * 0.5`. Sculk growth sounds do not use this hook.

## Block break sounds

A block part's `play_break_sound` is asked before the client plays a block's break sound
(level event 2001, sent for a broken block and played by the client's own predicted break). The
event supplies the block state and position. Play your sound through the supplied `ClientHost`
and return `Flow::Cancel` to suppress the vanilla sound; the destroy particles still follow
(`add_destroy_effects` decides those). Air is never asked.

The default plays the state's break sound at the block's centre, at volume `(volume + 1) / 2`
and pitch `pitch * 0.8`.

## Playing a level sound from a server mod

`LevelRef::play_sound(host, sound, at, source, volume, pitch, except)` is `ServerLevel.playSound(except, ...)`:
every player in range hears it, except the player in `except`. Pass the player whose own client already played the
sound for its action; pass `None` to play it to everyone. The game finds that player's connection itself. A fake
player has no client, so it leaves no one out; a stale player handle queues nothing and the call returns `false`.
Registered sound events are sent by ID; other valid namespaced event keys are sent by name, regardless of
namespace. Queuing or delivering a packet does not mean the client has the resources to play that event.

Inside an open transaction, `LevelRef::play_sound`, `EntityRef::play_sound`, `EntityRef::play_bound_sound`,
`PlayerRef::play_sound`, and `PlayerRef::stop_sound` validate arguments at the call and defer delivery according to
[the owning level's transaction](../../concepts/transactions.md#deferred-level-entity-and-player-effects).
For transactional server requests, `play_level_sound_at_position` and `play_level_sound_at_entity` run
synchronously at the call; only the resolved sound is deferred. Commit replays no mod callbacks and delivers
packets in journal order. For directed sounds, `true` means accepted, not that a client received or played it.
A deferred directed request is discarded on abort or if the player no longer resolves at commit.

Outside a transaction, a queued level sound draws its seed at the call even if a later hook cancels it. Inside
one, it draws a seed only when the resolved sound is published. A directed play request draws from the request
level's random when published. The `HitHost::play_sound_at` and `play_sound_from` calls below always queue at the call.

When a block callback lends a `LevelWriteHost`, use `host.play_sound(sound, at, SoundSource::Blocks, volume, pitch)`
to queue a positional sound for nearby players. A transaction's level writer publishes its queued sounds only on commit.

### Sound seeds

A level sound's seed picks which of its variants the clients play. Vanilla's `Level.playSound` draws it from the
level's `soundSeedGenerator` and then calls `playSeededSound` with it; a mod can take those two steps apart:

- `LevelRef::next_sound_seed(host)` draws the next seed of the stream `play_sound` takes its seeds from, without
  playing anything.
- `LevelRef::play_seeded_sound(host, sound, at, source, volume, pitch, except, seed)` is
  `ServerLevel.playSeededSound(except, x, y, z, ...)`, and `EntityRef::play_seeded_bound_sound(host, sound, source,
  volume, pitch, except, seed)` its entity-bound overload. They send `seed` as given and draw nothing.

Draw first and play second to model a `playSound` call whose sound may still be refused after the draw: the stream
moves as vanilla's does whether the sound plays or not. Inside a transaction the draw happens at the call and stays
drawn if the transaction aborts, while a held `play_sound` draws only when it is published at commit.

## Engine playback and live parameters

`play_sound` also observes transient playback updates. `event.starting` is true for a
new sound or loop replay and false for a live playback update. `event.instance` identifies the same engine-local
instance across those calls. A ticking sound applies volume, pitch and effects edits;
updates from a category volume change apply volume and pitch. `None` stops playback.
Finite base volume and pitch edits go through the same final calculations as local sound
requests, even when negative. Non-finite numeric edits or invalid effects refuse the
refresh. Updates do not replay the sound, resolve a different source, or modify the
instance's stored settings.

## Sounds of viewed levels

A client mod that shows another level, as Immersive Portals shows a portal's far side, hears it
through `viewed_level_sound`, a client-only Sunrise hook with no NeoForge counterpart. The server
sends a level's sounds to a client that views it, from `sunrise:levels` revision 3: only to one
whose chunks of that level hold the sound's chunk, and whose area there has its centre chunk within
the sound's broadcast distance plus 16 blocks. The engine plays none of them by itself. The hook
gets the viewed level's handle, the sound, its source, its position in that level, its volume,
pitch and seed, and `play_at`, which starts as `None`:

```rust,ignore
fn viewed_level_sound(&self, _: &mut dyn ClientHost, e: &mut ViewedLevelSoundEvent) {
    // Through the nearest portal: where it opens in the player's level, and quieter with distance.
    if let Some((opening, distance)) = self.portal_towards(e.level, e.position) {
        *e.play_at = Some(opening);
        e.volume = (e.volume - distance as f32 / 16.0).max(0.0);
    }
}
```

Left `None`, the sound is not played. Set, it plays at that position in the player's level as a
sound packet of that level plays (no distance delay), with the volume and pitch the listeners left
and the server's seed. A non-finite `play_at` is not a placement. The sound engine handles volume and pitch
as it does for the player's own level; to suppress playback, leave or set `play_at` to `None`. Sounds of the player's own
level never ask this hook. A sound bound to an entity of a viewed level comes with that entity's
position there, is sent only while the client tracks the entity there, and plays at `play_at`
without following it. With no listener a viewed sound costs one check.

## From a damage listener

A damage hook's host is a `HitHost`, which plays level sounds without any block write. Draw the pitch
from the attacker's own random, as vanilla's hit sounds do, and play the sound at a position or bound to an
entity:

```rust,ignore
fn living_incoming_damage(&self, hit: &mut dyn HitHost, e: &mut LivingIncomingDamageEvent<'_>) -> Flow {
    let Some(attacker) = self.attacker_ref(e.source.causing) else { return Flow::Continue };
    if let Some(roll) = attacker.with_random(hit, |rng| rng.next_float()) {
        hit.play_sound_at("mymod:hit.crunch", e.victim.position, SoundSource::Players, 1.0, 0.8 + roll * 0.4);
    }
    Flow::Continue
}
```

`play_sound_at` is `Level.playSound(null, x, y, z, ...)` and `play_sound_from(entity, ...)` is
`Level.playSound(null, entity, ...)`: every player in range hears it, the entity's own player included
(unlike `EntityRef::play_sound`). The sounds queue at the call, with their seeds drawn then from the level's
sound-seed stream, so a hit a listener cancels still plays them. `with_random(host, |random| ...)` borrows
the entity's real random stream: a callback can make several draws on that loan (`next_float`,
`next_double`, `next_bool`, `next_int_bound(bound)`, which returns `0` without drawing for a bound
below 1), and they advance the stream in listener order. A hit lends the entity the hit's source names as its causing entity, where the game has it at hand: the attacking player or mob of a melee hit (player → mob, mob → mob and mob → player) or of Thorns, the shooter or owner of a projectile, a firework rocket, evoker fangs, a wither skull, a thrown trident or a wind charge, the igniter or owner of an explosion, and a pufferfish stinging a mob inside its own tick. A guest that is not in place is lent from its hub record, for its reads only (it lends no random and plays no sound); the integrated owner displaced by another player's turn is not lent. Other damage lends none: damage with no causing entity (environment, fall, fire, drowning) and the sites that do not lend theirs (a thrown potion's or an effect cloud's owner, among others). When the victim caused its own hit (a player's own TNT hurting that player), the source names the victim and `causing()` answers `None`: the listener reads it through `victim()`. The attacking mob's own random is the one its AI draws from. Any other entity, a stale handle and a client answer `None` without entering the callback (`false` for a sound). Where a hit lends no sound queue (a client's prediction, piston restitution) `play_sound_at` and `play_sound_from` return false.
Hooks: `living_incoming_damage`, `living_damage_pre`, `living_damage_post`, `living_shield_block`,
`living_use_totem` and `living_death_protection`.
