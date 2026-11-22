# Fluid interactions

What a liquid block turns into beside another fluid or block: lava meeting water becomes
obsidian or cobblestone, a mod's honey meeting lava becomes its own stone. Vanilla's two
lava rules and a mod's are one table per source fluid, the shape of NeoForge's
`FluidInteractionRegistry`.

## Declaring one

In code, at registration:

```rust,ignore
let lava = registrar.fluid_key("minecraft:lava");
let honey = registrar.fluid_key("ruby:honey");
let slag_state = PlacedState::of(registrar.block_key("ruby:slag"));
registrar.fluid_interaction(FluidInteraction::new(
    lava,
    FluidNeighbour::Fluid(honey),
    FluidBecomes::always(slag_state),
));
```

Or as one file per interaction, `data/<mod id>/sunrise/fluid_interaction/<name>.json`, loaded
with the mod's content (as `data/<mod id>/sunrise/fluid/` is) and not reloaded with data packs:

```json
{
  "source": "minecraft:lava",
  "neighbour": { "fluid": "ruby:honey" },
  "below": { "block_tag": "ruby:cold_floor" },
  "becomes": { "source": "ruby:slag", "flowing": "ruby:slag[age=1]" }
}
```

- `source` is the fluid whose block interacts; source and flowing form are one fluid. It may be
  another mod's fluid or `minecraft:lava`, so "lava next to my fluid becomes my stone" is yours
  to say.
- `neighbour` is one of `fluid`, `fluid_tag`, `block` or `block_tag`. A fluid is read from the
  neighbour cell's fluid state, so a waterlogged block counts as water. The neighbours are above,
  north, south, west and east, never below.
- `below` is optional: the block (`block`) or block tag (`block_tag`) under the liquid.
- `becomes` gives the block state a source cell becomes (`source`) and a flowing cell becomes
  (`flowing`), written as `/setblock` writes a state. A state left out is no interaction for
  that state of the cell.

An id that names no fluid or block refuses the world at load, by name. A tag is read again on every
data pack reload, so a pack that edits `minecraft:water` changes what lava meets, as in vanilla.

## What runs

When a liquid block is placed (including by spreading into a cell) and when a neighbour of it
changes, the first match over the neighbours, then over the interactions, wins; at most one
runs. Vanilla's two rules come first, for each fluid of the `minecraft:lava` tag (lava beside a
fluid of the `minecraft:water` tag: obsidian for a source, cobblestone for flowing lava; lava
over soul soil beside blue ice: basalt), so a data pack that edits those tags changes them.
A tag neighbour matches exactly the fluids the tag lists; a `fluid` neighbour matches its source
and flowing form. After them come native mods'
interactions in load order, then content mods' files in load order and file name. A fluid outside the installed table's atomic bounds returns without a lock. Other fluids take a
read lock and index their own entries, returning immediately when that slice is empty. The lookup
allocates nothing.

The result goes through the `fluid_place_block` hook, which may veto or replace
it, then level event 1501 plays the fizz, and the liquid stops spreading. A veto writes nothing
and still fizzes and stops the spread.

## Asking the table without changing anything

`host.fluid_interaction_result(source, neighbour, below)` (server side) answers what a liquid
cell of `source` (a `FluidState`) would become beside one neighbour, with `below` under it:
`Some(block state)`, or `None` for no interaction. It reads the same table the liquid walks, with
the same tag and fluid matching and the same source or flowing result, and changes no world, asks
no `fluid_place_block` listener and plays no fizz. The neighbour is a block state; its fluid is
read through it, so a waterlogged block holds water.

A liquid checks its neighbours in vanilla's order: above, north, south, west, east, never below.
The read takes one neighbour, so to ask the same question loop over the five yourself, in that
order, and keep the first answer.

This is the question a cobblestone-generator shortcut asks (what do this lava source and the
fluid beside it make?) without running the interaction on a level that records its writes:

```rust
// Does lava here, over `below`, become something beside any of these neighbours?
fn makes_stone(host: &dyn ServerHost, lava: FluidState, around: [BlockState; 5], below: BlockState) -> Option<BlockState> {
    around.into_iter().find_map(|neighbour| host.fluid_interaction_result(lava, neighbour, below))
}
```

## Diverges from NeoForge

- NeoForge runs its registry only on `neighborChanged`; `onPlace`, which covers a fluid spreading
  into a cell, keeps vanilla's hard-coded lava check. So there a mod's interaction waits for a
  later neighbour update. Sunrise runs the table at both moments.
- NeoForge has no pure read: a mod that wants to know the result has to run the interaction against
  a level that records writes. `fluid_interaction_result` answers it from the table directly.
- NeoForge's `FluidPlaceBlockEvent` ignores cancellation; Sunrise's `fluid_place_block` can
  veto, as it already does.

Not offered: a code predicate per check (NeoForge's `HasFluidInteraction`). The table covers
fluid, block and tag neighbours and the block below; a predicate would be a `FluidPart` method.
Lava flowing down into water making stone (`LavaFluid.spreadTo`) is a different vanilla rule and
stays as it is.
