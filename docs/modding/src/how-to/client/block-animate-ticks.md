# Random display ticks for blocks

A block part's `animate_tick` is vanilla's `Block.animateTick` for a mod block: the client's ambient
effects that make a torch smoke, a furnace flame and redstone ore sparkle. It needs no block entity.

```rust,ignore
fn animate_tick(&self, host: &mut dyn ClientHost, event: &mut AnimateTickEvent<'_>)
```

Each client tick, `ClientLevel.animateTick` picks 667 random positions within 16 blocks of the player
and 667 within 32. When a picked position's block has a part with `animate_tick`, that part is called
once for the pick, inside it as vanilla calls a block's `animateTick`: after the block's own vanilla
effects, before its fluid's effects and the next pick. A block near the player is picked more
often than one far away, so the effect thins out with distance as vanilla's does.
A resource pack's Polytone `disable_particles` on the block stops the part's `animate_tick` too,
as it stops the block's own animation.

`AnimateTickEvent` carries:

- `level`: the client's level, to read the block's neighbours.
- `pos` and `state`: the picked block.
- `random`: the level's random (`ClientLevel.random`), a `&mut dyn RandomSource`. Java's
  `random.nextFloat()` is `next_float()`, `nextInt(bound)` is `next_int_bound(bound)`,
  `nextDouble()` is `next_double()` and `nextBoolean()` is `next_boolean()`. Drawing from it keeps a
  port's odds. It is the level's random itself: `host.add_particle`'s roll under the Decreased
  Particles option draws from it too, in call order, as `ClientLevel.addParticle` does.

Draw through the client host you are given:

- `host.add_particle(&options, at, velocity)` adds a particle as `ClientLevel.addParticle` does.
  The camera distance and the Particles option apply (see
  [Client particles](particles-and-movement-queries.md)).
- `host.play_local_sound(&SoundPlay::at(...), 0)` plays a sound at the block, as
  `ClientLevel.playLocalSound` does.

The hook runs only on a client, never on a server and never for a screen's block scene. Blocks without
the method cost nothing more: the sampler asks the per-block part table, not every part. If an effect
must show every tick, such as a running machine's smoke, use `block_entity_client_tick` instead.

## Example: AE2's quartz fixture

AE2's `QuartzFixtureBlock.animateTick` throws sparks two times in a hundred. It makes three bolts
0.3 blocks from the centre, toward the surface the fixture is mounted on, and checks AE2's particle
setting for each bolt:

```java
if (r.nextFloat() < 0.98) return;
Direction top = getOrientation(state).getSide(RelativeSide.TOP);
for (int bolts = 0; bolts < 3; bolts++) {
    if (AppEngClient.instance().shouldAddParticles(r)) // All: yes, Decreased: r.nextBoolean(), Minimal: no
        level.addParticle(ParticleTypes.LIGHTNING, 0.5 + x - 0.3 * top.x, ..., 0, 0, 0);
}
```

The port keeps the same draws in the same order:

```rust,ignore
use mod_api::{
    AnimateTickEvent, BlockPart, ClientHost, HostBlockStates, ParticleKey, ParticleOptions,
};

struct QuartzFixture {
    /// `r.particle_key("ae2:lightning")`, taken at registration.
    lightning: ParticleKey,
}

#[mod_api::sunrise_part]
impl BlockPart for QuartzFixture {
    fn animate_tick(&self, host: &mut dyn ClientHost, event: &mut AnimateTickEvent<'_>) {
        if event.random.next_float() < 0.98 {
            return;
        }
        // The fixture's `facing` is the side its light points to: the sparks sit toward its mount.
        let states = host.block_states();
        let top = match event.state.property(&states, "facing").ok().flatten() {
            Some("down") => [0.0, -1.0, 0.0],
            Some("north") => [0.0, 0.0, -1.0],
            Some("south") => [0.0, 0.0, 1.0],
            Some("west") => [-1.0, 0.0, 0.0],
            Some("east") => [1.0, 0.0, 0.0],
            _ => [0.0, 1.0, 0.0],
        };
        let p = event.pos;
        let at = [
            f64::from(p.x) + 0.5 - 0.3 * top[0],
            f64::from(p.y) + 0.5 - 0.3 * top[1],
            f64::from(p.z) + 0.5 - 0.3 * top[2],
        ];
        let particles = host.graphics_options().particles;
        for _ in 0..3 {
            // AppEngClient.shouldAddParticles(r).
            let add = match particles {
                0 => true,
                1 => event.random.next_boolean(),
                _ => false,
            };
            if add {
                host.add_particle(&ParticleOptions::simple(self.lightning), at, [0.0; 3]);
            }
        }
    }
}
```

Attach it at registration:

```rust,ignore
let key = r.block_key("ae2:quartz_fixture");
r.block_part(key, QuartzFixture { lightning });
```

A mod without its own particle can use vanilla's `minecraft:electric_spark` the same way.
