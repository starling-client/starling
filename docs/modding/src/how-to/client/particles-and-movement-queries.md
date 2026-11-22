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
server. Register the provider in `Hooks::register_particle_providers` on the client. Its sprite
list is `assets/<namespace>/particles/<path>.json`. Missing providers or sprite sets draw nothing.
The last successful registration for a type wins;
[a failed registration callback](../../concepts/load-phases.md#registration-and-setup) leaves the earlier provider in place.

For example, a rising particle with a signed `lift` option can use two forms of the same value:
SNBT fields for structured data and `/particle example:riser{lift:5}`, and a derived `Wire` value
for play packets. The `type` field is written by the structured dispatcher, not by the map codec.

```rust,ignore
use mod_api::{Compound, ModParticle, ParticleBatch, ParticleLevelView, ParticleMapCodec,
    ParticleProvider, ParticleStreamCodec, ParticleType, RandomSource, Registrar, Tag, Wire};
use std::sync::Arc;

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

struct Riser;
impl ParticleProvider<Rise> for Riser {
    fn create_particle(&self, options: &Rise, _level: &dyn ParticleLevelView,
        _at: [f64; 3], aux: [f64; 3], _random: &mut dyn RandomSource,
        particle: &mut ModParticle) -> Option<()> {
        particle.velocity = [aux[0], f64::from(options.lift) * 0.01, aux[2]];
        Some(())
    }

    fn tick(&self, particles: &mut ParticleBatch, _level: &dyn ParticleLevelView) {
        for particle in particles {
            for axis in 0..3 {
                particle.position[axis] += particle.velocity[axis];
            }
        }
    }
}

// In Hooks::register_particle_providers, using the retained `riser` type:
event.providers.register_sprite_set(riser, Arc::new(Riser));
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

The game calls `ParticleProvider::tick(&self, particles: &mut ParticleBatch, level: &dyn ParticleLevelView)`
once per type per tick, after it has aged the particles, with every live particle of the type; it is not called
when none is alive, and a registered provider with no particle alive is not visited. Batch iteration
(`for particle in particles`) preserves length and order. Set `removed` to expire a particle. If a callback
returns a different length, the engine expires the batch's particles.

Each `ModParticle` carries the particle's pose and look: `position`, `previous`, `velocity`, `size`, `rgba`, `frame`,
`state` (four numbers kept from tick to tick), a `facing` and a `roll`. `facing` is `Camera` (vanilla's
`LOOKAT_XYZ`), `AroundY` (`LOOKAT_Y`, which uses the camera's rotation about Y alone) or `Fixed([x, y, z, w])`, a
world-fixed unit quaternion that turns the quad's default plane (corners in XY, facing +Z); a quad lying flat facing up is
`[-sin(pi/4), 0, 0, cos(pi/4)]`. `roll` turns the quad about its own normal in radians, interpolated from the previous
tick's value, and nothing is drawn rotated while it is zero. A number that is not finite, a position past the world's limit,
a quaternion of no length and a roll beyond half of `f32::MAX` expire the particle.

`ParticleLevelView` reads the loaded level around the particles, read-only: `block_state(pos)` and
`brightness(layer, pos)`. A missing chunk reads as air. Light follows the render world's stored light; a dimension
without skylight always reads zero on the sky layer. Currently, an absent chunk or a position above the build height
reads full sky where present and no block light; below a loaded column both layers read zero. The client light storage
for padding sections and unloaded borders is tracked in [#2761](https://github.com/sunrise-org/sunrise/issues/2761).

Dispatch reuses its batch storage once it has enough capacity.

### Extracting your own quads

`ParticleProvider::extract` runs for each live particle whose current center is in the world
camera's culling frustum, even when optional particle culling is off. The culling lens uses the wider
of the configured and effective FOV, not the bobbed or narrowed render projection. Live GUI scenes
use the ordinary quad and do not call `extract`. This is a draw call, separate from
`tick`; it borrows the current and previous particle state, camera and frame light. By default
it calls `output.add_default()`, which uses the same sprite, pose and blend as an ordinary quad.
An override can call that method, append custom quads, or leave the output empty to draw nothing.
The usual stock-quad geometry checks apply only to `add_default`.

For example, a provider's `extract` can draw two overlapping opaque shapes at different angles:

```rust,ignore
use mod_api::{ModParticle, ParticleBlend, ParticleExtractView, ParticleQuad, QuadParticleRenderState};

fn extract(&self, particle: &ModParticle, view: &ParticleExtractView,
    output: &mut dyn QuadParticleRenderState) {
    let t = f64::from(view.partial_tick);
    let center: [f32; 3] = std::array::from_fn(|i| {
        let world = particle.previous[i] + t * (particle.position[i] - particle.previous[i]);
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
        quad.half_size = particle.size;
        quad.frame = particle.frame;
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
blend. Both are depth tested and culled from the back. The group still controls admission of the particle,
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
and world, for non-finite numbers and for more than 256 explosions.

## Bot movement and block collisions

`PlayerRef::simulate_move` predicts one server player's movement tick from
`PlayerInput`, using the game's movement implementation without replacing the
live player's body. It returns `SimulatedMove` with position, velocity, ground,
horizontal-collision and sprinting results. It refuses stale or inaccessible players,
dead players, spectators, riders and invalid numeric input. The prediction includes the
local player's ordinary wall-escape nudge, which checks solid collider boxes when a collision
scene is in scope. A supplied velocity is input to the prediction, not a write to the player.

`LevelRef::collision_boxes` reads block collision boxes within a finite, ordered
axis-aligned box. It excludes entity and border collisions. A refused level or a
query covering more than 1,000,000 cells returns `None`; a valid query with no
collisions returns `Some` containing an empty vector. Keep queries bounded rather
than scanning a whole level each tick.

`LevelRef::block_shape(host, state, pos, kind)` reads an explicit `BlockState`'s
outline or collision boxes with an empty context at a position in that level.
The supplied state can differ from the world's block, without changing it;
level-dependent shape inputs come from the actual level. To query the
stored block, first read `level.block_state(host, pos)` and pass that state.
Returns block-local boxes for the current client level and for open server levels,
including parked levels, or `None` for an unknown state, stale handle or unavailable level.
