# Register a block with a model

Declare the block in Rust and ship its models and textures in the same mod's
`assets/` folder. Start with the [native template](../../start/first-mod.md).
This example uses the mod ID `mymod` in `sunrise-mod.toml`. If your ID is different,
replace `mymod` in both the paths and the file contents.

## Declare the block

Replace the template's `src/lib.rs` with:

```rust,ignore
use mod_api::{self, Block, BlocksEvent, Hooks, RegistrationContext};

#[derive(Default)]
struct Mod;

#[mod_api::sunrise_mod]
impl Hooks for Mod {
    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        let tile = Block::new("mymod:blue_tile", "minecraft:stone");
        // A malformed ID or a namespace the manifest doesn't own returns an error;
        // startup reports the refusal either way.
        let _ = event.register(tile);
    }
}
```

`Hooks::blocks` runs at startup before resources load. Supply the same declaration
on client and server. The stone template supplies the block's base behaviour and
state; this declaration also creates a placeable item with the same ID. No
block declaration under `data/` is needed. `Registrar::block_key` requests a key
for an existing or declared block (for example, to attach a Part); it does not
declare a block by itself.

## Give the block and item a model

**`assets/mymod/blockstates/blue_tile.json`** maps the stone template's
property-free state (the empty variant key) to your block model:

```json
{
  "variants": {
    "": { "model": "mymod:block/blue_tile" }
  }
}
```

**`assets/mymod/models/block/blue_tile.json`** uses one texture on all faces:

```json
{
  "parent": "minecraft:block/cube_all",
  "textures": { "all": "mymod:block/blue_tile" }
}
```

Save your own square PNG (for example, a solid blue 16×16 tile) as
**`assets/mymod/textures/block/blue_tile.png`**. The texture ID in the model
omits `textures/` and `.png`.

**`assets/mymod/items/blue_tile.json`** reuses the block model for the item
in the inventory and in hand:

```json
{
  "model": { "type": "minecraft:model", "model": "mymod:block/blue_tile" }
}
```

**`assets/mymod/lang/en_us.json`** gives the block item a display name:

```json
{
  "block.mymod.blue_tile": "Blue Tile"
}
```

Start the mod with `cargo sunrise-mod run --resources <prepared-folder>` and try
`/give @s mymod:blue_tile` in a world with commands enabled. Check the item in
your inventory and the block after placing it. If the block still looks like stone,
check that the blockstate is in the right namespace; without its own blockstate it
uses the template's. The example has no recipe or loot table; add those separately
if players should be able to obtain it in survival.
