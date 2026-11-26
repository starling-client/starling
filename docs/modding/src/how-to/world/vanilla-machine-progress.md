# Read a vanilla machine's progress

A crafting CPU estimating how long a job has left, a progress line under the crosshair or a recipe
viewer's timer needs to know how far a furnace, blast furnace, smoker or brewing stand is, and how long its
next operation takes. No new read is needed: the block entity's saved data holds it, and the recipe lookup
gives each cooking recipe's time.

## The saved data

`host.block_entity(level, pos)` finds the block entity and `entity.data(host)` gives its data as
`BlockEntity.saveWithoutMetadata` writes it, the tag `/data get block` shows. It is read on the server, from
the live block entity, so it is current to the tick. `None` for an unloaded chunk, a block without a block
entity, and a block entity taken out for its own tick (only that block entity's tick can't read it).
`entity.kind(host)` tells which machine it is: `minecraft:furnace`, `minecraft:blast_furnace`,
`minecraft:smoker` or `minecraft:brewing_stand`.

A furnace, blast furnace or smoker (`AbstractFurnaceBlockEntity`) saves these ints:

| Key | Field | Meaning |
| --- | --- | --- |
| `cooking_time_spent` | `cookingTimer` | Ticks the item in the input slot has cooked. |
| `cooking_total_time` | `cookingTotalTime` | Ticks it takes at the present fuel's speed. |
| `lit_time_remaining` | `litTimeRemaining` | Ticks of burning left. |
| `lit_total_time` | `litTotalTime` | Ticks the last fuel burned for. |

and the float `speed_multiplier`, the burning fuel's speed (1 for coal; the `fast` providers a blast furnace and
a smoker read). `Items` holds slot 0 (input), 1 (fuel) and 2 (result).

A brewing stand (`BrewingStandBlockEntity`) saves `BrewTime`, the ticks left of the brew in progress (0 when
idle; it counts down), `total_brew_time` (400 at speed 1), `Fuel` (uses of blaze powder left), `total_fuel`,
the float `speed_multiplier`, and `Items`: slots 0 to 2 are the bottles, 3 the ingredient, 4 the fuel.

```rust,ignore
/// How far the furnace at `pos` is: (ticks cooked, ticks it takes), or none when it isn't cooking.
fn furnace_progress(host: &dyn Host, level: LevelRef, pos: [i32; 3]) -> Option<(u32, u32)> {
    let data = host.block_entity(level, pos)?.data(host)?;
    let int = |key: &str| match data.get(&key.into()) {
        Some(&Tag::Int(value)) => u32::try_from(value).ok(),
        _ => None,
    };
    let (done, total) = (int("cooking_time_spent")?, int("cooking_total_time")?);
    let lit = int("lit_time_remaining")? > 0;
    (lit && total > 0).then_some((done, total))
}
```

## The time an operation takes

`level.recipe_for(host, &recipe_type, &RecipeInput::Single(stack))` matches the recipe the machine would use,
with `recipe_type` the machine's recipe type: `minecraft:smelting` for a furnace, `minecraft:blasting` for a
blast furnace, `minecraft:smoking` for a smoker. Its `processing_time` is the recipe file's `cookingtime`, with no
station speed applied. The furnace divides it by the fuel's speed: `ceil(processing_time / speed_multiplier)`,
the furnace's `cooking_total_time` for that input. `Ok(None)` means the machine can't cook it. The vanilla
recipes give 200 ticks for smelting, blasting and smoking alike: in 26.3 a blast furnace or smoker is faster
through its fuel's speed, not through its recipes.

A brewing stand's brew takes `ceil(400 / speed_multiplier)` ticks for any ingredient, and brews the three
bottle slots at once. `host.recipes_of_type("minecraft:brewing")` lists the brewing recipes (`input`,
`reagent`, `output`) to tell whether an ingredient brews anything with the bottles in the stand.

## Sharing it with other mods

The game answers no capability for machine progress: NeoForge has none. A library mod that defines one (its
key, its handler trait and its providers for the vanilla machines, built on the reads above) lets machine mods
answer it for their own block entities and readers use one key for all of them, as in
[A mod's own capability](../../concepts/capabilities.md#a-mods-own-capability).
