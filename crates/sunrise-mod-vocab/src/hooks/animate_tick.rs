//! The `animate_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A client's random display tick of the block `state` at `pos` (`Block.animateTick`):
    /// `ClientLevel.animateTick` picked `pos` among the 667 random positions within 16 and the
    /// 667 within 32 blocks of the player it samples each client tick. Add particles with
    /// `host.add_particle` and play sounds with `host.play_local_sound`: a torch's smoke, a
    /// furnace's flames, redstone ore's dust. `level` is the client's level, `random` its random
    /// (`ClientLevel.random`, the one vanilla lends `animateTick`), so a port keeps Java's odds:
    /// `random.next_float() < 0.04` as `random.nextFloat() < 0.04f`. Only parts on the picked
    /// state's block are called, inside the pick: after the block's own vanilla effects, before
    /// its fluid's and the next pick. A block with no part answering costs the sampler one table
    /// lookup. Never called on a server, nor for a screen's scene. A resource pack's Polytone
    /// `disable_particles` on the block stops the part's call too, as it stops the block's own.
    #[notes = r##"### Random display ticks: sparks, smoke and dust

`animate_tick` is vanilla's `Block.animateTick(BlockState, Level, BlockPos, RandomSource)` for a mod's block:
the client's ambient block effects, without a block entity. Each client tick `ClientLevel.animateTick` picks
667 random positions within 16 blocks of the player and 667 within 32, and each picked position's block draws
its effects there; a block near the player is picked more often than one far away. A part on the picked
state's block is called once for that pick, inside it as vanilla calls a block's `animateTick`: after the
block's own vanilla effects, before its fluid's effects and the next pick. It gets the client's level, the
position, the state and the level's random.

Draw from `event.random` to keep a port's odds and order (`random.nextFloat()` is `next_float()`,
`nextInt(bound)` `next_int_bound(bound)`, `nextDouble()` `next_double()`, `nextBoolean()` `next_boolean()`).
Add particles with `host.add_particle(&options, at, velocity)`, which applies the camera distance and the
Particles option as `ClientLevel.addParticle` does, and play sounds with `host.play_local_sound`
(`ClientLevel.playLocalSound`). Read the block's neighbours through `event.level`. A block that should
always show its effects, a machine's working smoke, belongs in `block_entity_client_tick` instead: a picked
position is a matter of chance.

The sampler asks the per-block table whether the picked state's block has a part answering
`animate_tick`, so a block without one costs one lookup and no allocation, and with no part answering it
nowhere the sampler asks nothing. `random` is the level's random itself: the part's draws and the game's own
during the call (`host.add_particle`'s roll under the Decreased Particles option) move the one stream in call
order. It never runs on a server, and a screen's block scene (`animate_scene`) does not call it."##]
    #[hook(
        side = "client",
        contract = "listen",
        scope = "block",
        key = "state",
        neoforge = "none (vanilla Block.animateTick from ClientLevel.animateTick; NeoForge adds no event)"
    )]
    fn animate_tick(
        level: LevelRef,
        pos: BlockPos,
        state: BlockState,
        random: &mut dyn RandomSource,
    );
}
