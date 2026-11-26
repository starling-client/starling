# Client particles and movement queries

## Client particles

`ClientHost::add_particle` submits the API's `ParticleOptions`, a world position and
velocity to the ordinary client particle queue. The client must have a loaded player
and world. Non-finite numeric inputs are refused. A successful submission does not
promise a visible particle: provider availability, sprite sets, distance and the
player's particle settings apply. This method is not available on `ServerHost`.

Particle options carry a registered key, not an engine particle enum or a provider name.
Request the key with `Registrar::particle_key` during registration and retain it. The game resolves
provider identities once when content loads. `ParticleOptions::simple(key)` handles both vanilla
and asset-only sprite types; those types require an `assets/<namespace>/particles/<path>.json`
sprite set. All option constructors take the resolved key, including `dust`, `item`, `trail`,
`vibration`, `shriek`, `sculk_charge` and `dust_transition`. Use the corresponding vanilla key
for these parameter families. Sending an option performs no string parsing or registry scan.

For stack-derived breaking or eating particles, call
`ClientHost::item_particle_options(&stack)` and pass the returned options to
`add_particle`. This read resolves the ground model with seed 0, preserving all
layer particle sprites and resource-pack and stack-component overrides. It returns
`None` for invalid stacks or unavailable resources/provider. It is available in
Setup, Reloading and World. Resolve again after resource reloads.
It allocates a sprite list only when called; it adds no per-tick work.

### Changing particles as they are added

`client_particle_add` (vanilla's `ClientLevel.doAddParticle`) sees every particle the client is
asked to add, whoever asked: the server, a block's or entity's ambient tick, an explosion or
another mod's `add_particle`. The listener may move it (`event.pos`), change its velocity
(`event.velocity`) or return `Flow::Cancel` to drop it; the options are read-only, so a mod that
wants a different particle cancels this one and adds its own. The 32-block camera check and the
Particles setting then apply at the position the listener left, and a position or velocity left
non-finite drops the particle. The listener gets no host: it runs inside the particle engine, so
keep what it needs (a wind, a limit) in the mod's own state and update it from `client_tick`.

```rust,ignore
fn client_particle_add(&self, event: &mut ClientParticleAddEvent<'_>) -> Flow {
    let [wx, wz] = *self.wind.lock().unwrap();
    event.velocity[0] += wx;
    event.velocity[2] += wz;
    Flow::Continue
}
```

### Tinting an item crumb

The item crumbs (`item` and its `item_*` variants) multiply their red, green and blue by
`ParticleOptions::color` when they spawn and ignore its alpha; the default white changes
nothing, and `item_particle_options` results and server `item_stack` sends stay untinted.
A mod sets it with `with_color`, and a block-atlas sprite such as a fluid's still sprite is a
valid layer, so a fluid crumb is the sprite tinted by the fluid:

```rust,ignore
let crumb = ParticleOptions::item(item_key, vec!["minecraft:block/water_still".into()])
    .with_color([0.8 * r, 0.8 * g, 0.8 * b, 1.0]);
host.add_particle(&crumb, at, velocity);
```

An item crumb's colour must be finite with every component in `0.0..=1.0`; `add_particle`
returns `false` otherwise and nothing is clamped. The crumb keeps its small random spread of
velocity and the level's light.

### Particle providers

A code-defined particle has a common `ParticleType<T>` and a client sprite-set provider.
Register the type in `Hooks::register`; neither the client atlas nor its JSON is needed on a
server. Register its factory in `Hooks::register_particle_providers` on the client. It binds on every
resource load, after the sprite list is resolved. Its sprite
list is `assets/<namespace>/particles/<path>.json`. Missing providers or sprite sets draw nothing.
The last successful registration for a type wins;
[a failed registration callback](../../concepts/load-phases.md#registration-and-setup) leaves the earlier provider in place.

For example, a rising particle with a signed `lift` option can use two forms of the same value:
SNBT fields for structured data and `/particle example:riser{lift:5}`, and a derived `Wire` value
for play packets. The `type` field is written by the structured dispatcher, not by the map codec.

```rust,ignore
use mod_api::{Compound, CustomParticle, ParticleLevelView, ParticleMapCodec,
    ParticleProvider, ParticleSpawn, ParticleStreamCodec, ParticleTickView, ParticleType,
    Registrar, Tag, Wire};

#[derive(PartialEq, Wire)]
struct Rise { lift: i32 }

// In Hooks::register, retain the returned type in your mod's state.
fn register_riser(registrar: &mut dyn Registrar) -> ParticleType<Rise> {
    registrar.register_particle_type(
        "example:riser", false,
        ParticleMapCodec::new(
            |fields, _registries| match fields.get(&"lift".into()) {
                Some(Tag::Int(lift)) => Ok(Rise { lift: *lift }),
                _ => Err("lift must be an integer".into()),
            },
            |value: &Rise, _registries| {
                let mut fields = Compound::new();
                fields.insert("lift".into(), Tag::Int(value.lift));
                Ok(fields)
            },
        ),
        ParticleStreamCodec::wire(),
    )
}

struct RiserProvider;
struct Riser;

impl ParticleProvider<Rise> for RiserProvider {
    fn create_particle(&self, spawn: &mut ParticleSpawn<'_, Rise>,
        particle: &mut dyn ParticleTickView) -> Option<Box<dyn CustomParticle>> {
        let [x, _, z] = spawn.velocity;
        particle.set_velocity([x, f64::from(spawn.options.lift) * 0.01, z]);
        Some(Box::new(Riser))
    }
}

impl CustomParticle for Riser {
    fn tick(&mut self, particle: &mut dyn ParticleTickView, _level: &dyn ParticleLevelView) {
        if particle.advance_age() {
            let velocity = particle.velocity();
            particle.move_by(velocity);
        }
    }
}

// In Hooks::register_particle_providers, using the retained `riser` type:
event.providers.register_sprite_set(riser, |_sprites| RiserProvider);
// From a client hook:
host.add_particle(&riser.options(Rise { lift: 5 }), at, [0.0; 3]);
```

For a provider that takes no options, use `ParticleMapCodec::unit()` and
`ParticleStreamCodec::wire()` and call `ty.options(())`. Codecs may borrow their
`ParticleRegistryView` to resolve item, fluid or particle names; encode stable names, not key
slots. A codec error refuses the input rather than substituting default options.

`create_particle` runs once for each spawn that reaches the provider, before the first tick.
It can edit the sprite particle initialized by its JSON, or return `None` to decline that spawn.
The position and three auxiliary values are exactly what the spawn supplied; the provider can
interpret the auxiliary values as something other than velocity. The RNG is the particle
engine's provider RNG, not the one used to thin spawns.

Each `CustomParticle` owns its own state; put temporary counters or progression in its fields.
Its `tick` receives a live `ParticleTickView`, not a copy. The default runs `tick_base`, which
records the old pose, advances age, checks lifetime and applies vanilla motion. To replace the
motion, use `advance_age()` and `move_by(delta)`, or control explicit removal with `remove()`.
Invalid setter inputs leave that field unchanged. The level and particle views cannot be retained.

`ParticleFacing` can be `Camera`, `AroundY` or `Fixed` with a world-space XYZW quaternion.
A nonzero roll is interpolated from the previous roll. Omitting a frame's geometry does not
alter the pose or end the particle's lifetime. Resource reload drops native instances from the
old sprite generation, then binds the factories again.

`ParticleLevelView` reads the loaded level around the particles, read-only: `block_state(pos)` and
`brightness(layer, pos)`. A missing chunk reads as air. Light follows the render world's stored light; a dimension
without skylight always reads zero on the sky layer. An absent chunk or a position above the build height
reads full sky where present and no block light; below a loaded column both layers read zero.

### Extracting your own quads

`CustomParticle::extract` runs for each live particle whose current center is in the world
camera's culling frustum, even when optional particle culling is off. The culling lens uses the wider
of the configured and effective FOV, not the bobbed or narrowed render projection. Live GUI scenes
use the ordinary quad and do not call `extract`. This is a draw call, separate from
`tick`; it borrows the current and previous particle state, camera and frame light. By default
it calls `output.add_default()`, which uses the same sprite, pose and blend as an ordinary quad.
An override can call that method, append custom quads, or leave the output empty to draw nothing.
The usual stock-quad geometry checks apply only to `add_default`.

For example, a provider's `extract` can draw two overlapping opaque shapes at different angles:

```rust,ignore
use mod_api::{ParticleView, ParticleBlend, ParticleExtractView, ParticleQuad, QuadParticleRenderState};

fn extract(&self, particle: &dyn ParticleView, view: &ParticleExtractView,
    output: &mut dyn QuadParticleRenderState) {
    let t = f64::from(view.partial_tick);
    let center: [f32; 3] = std::array::from_fn(|i| {
        let world = particle.previous_position()[i] + t * (particle.position()[i] - particle.previous_position()[i]);
        (world - view.camera[i]) as f32
    });
    for (offset, rotation) in [
        (-0.125, [0.0, 0.0, 0.0, 1.0]),
        ( 0.125, [0.0, 1.0, 0.0, 1.0]),
    ] {
        let mut quad = ParticleQuad::default();
        quad.layer = ParticleBlend::Opaque;
        quad.center = [center[0] + offset, center[1], center[2]];
        quad.rotation = rotation;
        quad.half_size = particle.size();
        quad.frame = particle.sprite_index() as u32;
        quad.color = 0xffcceeff;
        quad.light_coords = view.light_coords;
        output.add(quad);
    }
}
```

Centers are camera-relative and rotations are XYZW quaternions; the renderer does not normalize
rotations. Each `frame` is an existing index into the bind-time sprite list. UVs are local to that
sprite: `[0.0, 0.0, 1.0, 1.0]` selects it whole. `color` is `0xAARRGGBB`; for packed light use the
view's value or vanilla's `LightCoordsUtil.pack`/`smoothPack` layout. The explicit layer wins over
the group's blend. Opaque quads replace covered pixels after the alpha cut; translucent quads
blend; additive quads draw after the translucent layer with depth writes off and additive blending.
All three are depth tested and culled from the back. The group still controls admission of the particle,
not the number of quads it emits. The sink ignores an unresolved frame or non-finite geometry/UV
without dropping other quads.

### Particle render groups

In `Hooks::register_particle_groups`, call `event.register(id, factory)` with an
`Identifier` having a nonempty namespace and path (any namespace, including `minecraft`,
is allowed) and a factory returning `ParticleGroup { blend, particles }`. The hook runs
once at client startup; each accepted factory runs once per particle engine construction.
Duplicate IDs fail client startup without invoking any factory (matching NeoForge
`RegisterParticleGroupsEvent`). Reloading resources re-associates sprite sets with the
groups already built; it does not call factories again. Provider registration is separate:
a group chooses the draw layer and quota for the listed particle types, not how those
particles move.

## Server item particles

On a `ServerHost`, build `ParticleOptions::item_stack(item_particle_key, stack)` from the
load-resolved `minecraft:item` key and an API `ItemStack`. Pass it to
`level.send_particles(host, &options, state, at, spread, speed, count)` on a `LevelRef`.
An invalid stack (unknown item, invalid components or count) returns `false` without
queueing particles. `true` means queued, not drawn. In a transaction, accepted particles
follow the [deferred effect rules](../../concepts/transactions.md#deferred-level-entity-and-player-effects).

The packet carries the stack, not sprite names. Each client resolves its ground model
with seed 0, as in `ClientHost::item_particle_options`, so the client's resources and
the stack's components choose the particle sprites. The explicit sprite list in
`ParticleOptions::item` is for local rendering, not server sends.

Block, block-marker, falling-dust, dust-pillar and block-crumble requests need a valid
block state argument. Server sends refuse trail and vibration requests.

To send the same effect around several positions, pass a slice to
`level.send_particles_batch(host, &options, state, &positions, spread, speed, count)`. The whole
request is checked before anything is queued, and each position becomes its own send, in slice
order.

## Block hit and destroy debris

Use the [`BlockPart` hit/destroy callbacks](../../concepts/parts/block-parts.md#hit-and-destroy-particles)
to replace or omit a block's ordinary crumbs. The raw client level helpers queue a hit crumb
or an individual terrain particle without dispatching the callback again. They are separate from
`host.add_particle`, which follows the ordinary camera and Particles setting checks.

## Client fireworks

`ClientHost::create_fireworks` draws a firework's explosions from a `FireworksRequest`: the
position, the rocket's velocity carried by the sparks, a list of `FireworkExplosion`s (shape,
colours, fade colours, trail, twinkle) and `audible`. It uses the starter a bursting rocket
uses, so the explosions follow each other every other tick as a rocket's do. With `audible`
false the sparks and flash draw but the blast and twinkle sounds don't play, as for a
silent rocket. Nothing reaches the server or other players: a mod's server half tells each
client half to draw it. The request is refused on a server, without a loaded client player
and world, or for non-finite numbers.

## Bot movement and block collisions

`PlayerRef::simulate_move` predicts one server player's movement tick from
`PlayerInput`, using the game's movement implementation without replacing the
live player's body. It returns `SimulatedMove` with position, velocity, ground,
horizontal-collision and sprinting results. It refuses stale or inaccessible players,
dead players, spectators, riders and invalid numeric input. A supplied velocity is
input to the prediction, not a write to the player.

`LevelRef::visit_block_collisions` lends each intersecting, unclipped world-space `VoxelShape`
for a finite, ordered axis-aligned box. It uses vanilla's `BlockCollisions` walk, skips missing
chunks without loading them, and excludes entities and the world border. Use `shape.boxes()`
to walk its occupied boxes without allocating a list, or clone the shape to retain it.
Return `ControlFlow::Break(())` to stop the walk. `None` means invalid bounds, unavailable
level access or a shape error, not a completed empty visit. `LevelRef::has_collisions`
uses the same walk and stops at the first hit.

`LevelRef::visit_block_shape(host, state, pos, kind, &mut visit)` visits an explicit
`BlockState`'s block-local outline or collision boxes with an empty context. The supplied
state can differ from the world's block without changing it; level-dependent inputs come
from the actual level. First read `level.block_state(host, pos)` to query the stored block,
or use `visit_block_collision` to visit only its collision boxes. The supplied-state read
works in the current client level and in open server levels, including parked ones, and
returns `None` for an unknown state, stale handle or unavailable level.
