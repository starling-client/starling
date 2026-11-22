# Randomness

A mod draws random numbers through one trait, `RandomSource`, with vanilla's draw names:
`next_int`, `next_int_bound`, `next_long`, `next_boolean`, `next_float`, `next_double` and
`next_gaussian`, plus two range helpers, `next_int_between_inclusive(min, max)` and
`next_int_range(origin, bound)`.

## Streams the game lends you

Wherever the game has a random stream a mod may draw from, you get a `&mut dyn RandomSource`:

| Where | Stream |
|---|---|
| `GenerationContext::random`, `PieceContext::random` | The structure start's or the piece's random |
| `GroundLevel::random` | The tree decorator's random |
| The tree-grow hook's random | The tree's own random |
| `GoalContext::random` | The mob's own random |
| `host.with_random(\|rng\| ...)` on a restricted level host | The hook's live level stream |
| `level.with_random(host, \|rng\| ...)` on a `LevelRef` | The named server level's live stream, even when parked |

A draw moves the game's own stream, in vanilla's order, so your draws shift vanilla's later ones
exactly as a Java mod's would. You need no import to call these methods on a `dyn RandomSource`.
For example, `goal_context.random().next_int_bound(20)` draws from the mob's own stream.

## Borrow the live level stream

In a hook with a `LevelRandomHost`, `HitHost` or `LevelWriteHost`, draw from that hook's level:

```rust,ignore
let level_roll = host.with_random(|rng| rng.next_int_bound(bound));
```

With a `LevelRef` and a mutable `ServerHost`, draw from the named level:

```rust,ignore
let named_roll = level.with_random(host, |rng| rng.next_int_bound(bound));
```

Both return the closure's value in `Some`. If the host cannot lend an authoritative stream,
they return `None` without calling the closure. A `LevelRef` also refuses a stale handle, a
level that isn't resident or a phase without world access. A parked level is still resident:
drawing from it neither loads a level nor switches which one the server is processing.
The closure borrows the stream only while it runs. It cannot keep, replace or reseed it.

Draws advance the level's live stream directly in vanilla's order and are not rolled back if
the closure panics and unwinds, or if an enclosing transaction aborts, matching vanilla's
`Level.getRandom()`.

## Your own seeded stream

`LegacyRandomSource` is vanilla's `LegacyRandomSource` (the generator Java's `Random` is):
`LegacyRandomSource::new(seed)`, `set_seed(seed)`, and `RandomSource` for the draws (import the
trait to call them on the concrete type). Two sources with the same seed give the same values.
If your mod chooses the seed, it must own the generator: this stream is independent of the
level's random. Its Wire form preserves the owned generator's state and cached Gaussian.

## Never panics

Mod input never panics a draw. Where vanilla throws, `RandomSource` answers instead:

| Call | Answer |
|---|---|
| `next_int_bound(bound)` with `bound < 1` | 0, with no draw |
| `next_int_between_inclusive(min, max)` with `max < min` | `min`, with no draw |
| `next_int_range(origin, bound)` with `bound <= origin` | `origin`, with no draw |

Forked or positional sources (`forkPositional`) are not offered.
