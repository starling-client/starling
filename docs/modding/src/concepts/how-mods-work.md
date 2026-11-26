# How a mod works

**One folder while you work, one `.smod` file when you share it.** The `.smod` is a zip archive of the
files Sunrise needs, not your build cache. Both kinds of mod use the same package format:

- **Content mods** use data and assets to add things like blocks, items and recipes. Nothing compiles.
- **Native mods** use Rust code to add behaviour, such as greeting players when they join. The mod tool
  compiles the code against the pinned prebuilt game, without rebuilding the game itself. The `.smod`
  carries the Rust source, not the compiled library. A native mod can include data and assets too.

## What goes in the folder

```text
my_mod/
├── sunrise-mod.toml    # the manifest: the mod's ID, name and version
├── icon.png            # optional picture for the mod
├── data/my_mod/        # declarations, recipes, loot tables and tags
├── assets/my_mod/      # models, textures and names players see
├── Cargo.toml          # native mods only
└── src/lib.rs          # native mods only
```

Native mods also have a `[native]` section in `sunrise-mod.toml` that pins the game build. The manifest
is the source of the mod's ID and version, even when the mod has a `Cargo.toml`.

`cargo sunrise-mod build` writes `target/sunrise-mod/my_mod.smod`. Files keep their paths inside the archive:
`sunrise-mod.toml` is at the top level, not inside another `my_mod/` folder.

## How the game finds your block

A **registry** is the game's list of things of one kind: blocks, items, and so on. Each entry is known by
an ID of the form `namespace:name`. Vanilla stone is `minecraft:stone`; your Glowing Amethyst could be
`my_mod:glowing_amethyst`. Your mod's ID, set in `sunrise-mod.toml`, is its default namespace. Keep that
ID when you publish a new version so the game can still find your mod and its content.

For a block, the file path `data/my_mod/sunrise/block/glowing_amethyst.json` gives the game both parts of
its ID. Inside the file you can start from a vanilla block and say what changes, such as the light it gives
off. Your assets, recipes and loot tables refer to the new block by ID. Those files use the familiar
resource pack and data pack formats; declarations under `data/my_mod/sunrise/` are Sunrise's own format.

The **mod registry** is different: it lists published `.smod` packages, not the blocks or items in a world.

## When mods load

The game starts with vanilla's content and adds mod content at startup. Rebuild the package and restart
when you edit your project files. Native hooks run when the game reaches the moments they listen for:
registration hooks run during setup, while a join hook runs when a player joins.

If your mod needs another mod first, declare it in `[dependencies]` in `sunrise-mod.toml`. By default a
dependency's content loads before yours, and among listeners of the same priority its listeners run before
yours (listener priority comes first; dependency order breaks ties). Under the default rules, a missing applicable required
mod, a version that doesn't satisfy the declared version requirement, or a cycle in the order stops loading.
An optional dependency can be absent; when present and applicable, it must match the declared version
requirement. See [Dependencies between mods](dependencies.md) for instance overrides and ordering options.

## Next

- [Your first content mod](../start/first-content-mod.md): make a block that glows.
- [Your first native mod](../start/first-mod.md): greet players with a Rust hook.
- [The manifest and the package](../reference/manifest.md): see the full layout and rules.
