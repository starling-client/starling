//! The `mirror` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A structure template is about to place the block `state` at `pos` mirrored by `mirror`
    /// (vanilla's `BlockBehaviour.mirror(BlockState, Mirror)`, which a block overrides to flip its
    /// own properties, as `state.mirror(mirror).rotate(rotation)` places it): `mirrored` starts
    /// at the game's own answer (vanilla's table, or a mod block's template's). Set it to another
    /// state of the same block to place that instead; the `rotate` hook then turns that state. A
    /// state of another block, or one the game does not have, is refused and the game's answer
    /// kept. Asked wherever `rotate` is for a mirrored template: world generation's structures,
    /// `/place template`, the structure block, and the template reads
    /// `LevelRef::visit_structure_template` visits; asked with `StructureMirror::None` too, as
    /// vanilla's `StructureTemplate.placeInWorld` calls `mirror(NONE)`. Server-side, on the
    /// worldgen worker threads and on the server thread, with the server's worldgen `context`: no
    /// `Host`, no writes; `context.state_property` reads a property of a state and
    /// `context.state_with_property` builds states. A panicking part is contained and the game's
    /// answer kept. Diverges from upstream: new hook.
    #[notes = r#"## Mirrored block states

- **`BlockPart::mirror(context, event)`** (`MirrorEvent { state, pos, mirror }`, `event.mirrored`): vanilla's `BlockBehaviour.mirror(state, mirror)`, which a NeoForge or Fabric block overrides to flip its own properties, asked as a structure template places a block, where vanilla runs `state.mirror(mirror).rotate(rotation)`. `event.state` is the template's state, `event.pos` is where the block goes and `event.mirror` the template's mirror (`LeftRight` flips north and south, `FrontBack` east and west). `event.mirrored` starts at the game's own mirrored state; set it to another state of the same block, typically built with `context.state_with_property(event.state, "facing", "south")`, to place that instead; `context.state_property(event.state, "facing")` reads the value to flip, as vanilla's `state.getValue(FACING)`. A state of another block, or one this game does not have, is refused with a warning and the game's answer kept. The `rotate` hook turns the state this one answers.
- **Where it is asked.** Every Sunrise path that mirrors a template's blocks: world generation's structures (`StructureTemplate.placeInWorld` and the legacy pieces' `placeBlock`), `/place template` and the structure block, and `LevelRef::visit_structure_template`. The legacy pieces ask it only when they mirror, as their `placeBlock` does; a template asks it for every block, `StructureMirror::None` included. A block entity's data on mirroring is a separate step after this one.
- **What it reads.** Like every worldgen Part method, it runs on the worldgen worker threads and on the server thread, with the server's worldgen `context` (a `WorldgenHost`, no `Host` and nothing to write), so it is a pure function of its inputs: the same seed gives the same world.
- **Panics.** A panicking part is contained with a warning and the game's answer kept.
- **Cost.** With no part subscribed, one subscription check per placed block; with one, a table lookup and the state conversions per placed block."#]
    #[hook(
        side = "server",
        contract = "collect",
        out = "mirrored",
        context = "worldgen",
        scope = "block",
        key = "state",
        target = "mod_api::world::mirror::native_mirror",
        neoforge = "none (vanilla BlockBehaviour.mirror; NeoForge adds no event or extension for it)"
    )]
    fn mirror(state: BlockState, pos: [i32; 3], mirror: StructureMirror, mirrored: &mut BlockState);
}
