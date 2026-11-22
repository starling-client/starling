//! The `register_particle_providers` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Binds a logical client's sprite-set providers to already registered [`ParticleType`]s
    /// (NeoForge's `RegisterParticleProvidersEvent`). Each provider creates particles from typed
    /// options, ticks its live particles in one batch, and extracts quads for visible particles. Later registration for the same type
    /// replaces the previous provider. Registration runs once before resources load; each reload
    /// rebinds the sprite sets and discards live provider particles.
    /// If this listener panics or a phase call is refused, none of its registrations take effect.
    #[notes = r#"## Particle providers

Register a common `ParticleType<T>` in `Hooks::register`, supplying its map and stream codecs.
`Hooks::register_particle_providers` binds it on the logical client with
`event.providers.register_sprite_set(ty, Arc::new(provider))`. The matching sprite list is
`assets/<namespace>/particles/<path>.json`. Spawn using `ty.options(value)`; a unit type uses
`ParticleMapCodec::unit()`, `ParticleStreamCodec::wire()` and `ty.options(())`.

The spawn call passes the typed value, client level, position, auxiliary inputs and provider RNG
once to `ParticleProvider::create_particle`. The sprite set and `sunrise:behaviour` initialize the
particle; the factory can then edit its starting state or decline by returning `None`.
The engine owns sprite handling and the camera-distance, Particles-setting and population limits.
Before each `tick` the game advances `age` (1 on the first tick) and sets
`previous`, and it expires the particle once `age` passes `lifetime`, or after a tick that sets `removed` or
leaves a number that is not finite. The particle draws on the blended layer, so `rgba[3]` may fade it, unless a `register_particle_groups`
group lists its type: it then draws in that group, on the group's blend.
`position`, `frame` (a sprite of the set, clamped), `size`, `rgba`, `lifetime`, `removed`, `facing` and `roll` are the tick's to
write. `facing` is `Camera` (vanilla's `LOOKAT_XYZ`), `AroundY` (`LOOKAT_Y`: the camera's `y` and `w` alone) or
`Fixed(quaternion)`, a world-fixed orientation such as a quad lying flat; `roll` turns the quad about its normal and is
interpolated from the previous tick's (no turn is drawn while it is zero), and a roll beyond half of `f32::MAX` expires the
particle. `state` is four numbers that stay with the particle, zero at spawn; it is the only per-particle data,
since every particle of a type shares the one provider. `tick` is one call per type per tick with every live
particle of the type, and is not called when none is alive. Dispatch reuses its batch storage once it has
enough capacity. A tick has no `Host`, since the engine ticks particles below gameplay, but it reads the
level through `ParticleLevelView`: `block_state(pos)` and `brightness(layer, pos)`, both read-only. Light follows
what the render level stores and the sky layer always reads zero in a dimension without skylight.
A code-defined type without its provider or sprite set draws nothing. Asset-only particles
keep declarative motion. Reload drops the previous sprite set's live provider particles.

Each visible particle also calls `ParticleProvider::extract` with the current frame. The default
submits its normal quad. An override may append independently placed opaque or translucent
quads to its own sprite atlas or emit nothing."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        version = 3,
        contract = "listen",
        staged_surface = "providers",
        context = "registration",
        neoforge = "RegisterParticleProvidersEvent"
    )]
    fn register_particle_providers(providers: &mut dyn ParticleProviders);
}
