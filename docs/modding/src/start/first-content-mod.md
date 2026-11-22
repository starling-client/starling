# Your first content mod

A content mod adds blocks, items, recipes, loot, models and textures with data files alone. In this page you
make one block, see it in the game, and pack it as a `.smod`.

You need Rust 1.95 or newer (`cargo`). To play the mod you also need a Sunrise game and resources prepared from your
own Minecraft: Java Edition 26.3 (step 4).

## 1. Install the mod tool

Follow [Install the mod tool](install-tool.md). It adds the `cargo sunrise-mod` command.

## 2. Make the project

```sh
cargo sunrise-mod new glowing-blocks
cd glowing-blocks
```

The tool turns the folder name into the mod id (`glowing_blocks`) and writes a complete mod with one block:

```text
glowing-blocks/
├── sunrise-mod.toml                                   # the manifest: id, name, version, api, capabilities
├── assets/glowing_blocks/
│   ├── blockstates/glowing_amethyst.json
│   ├── models/block/glowing_amethyst.json
│   ├── items/glowing_amethyst.json                    # the item model
│   └── lang/en_us.json                                # the block's name
└── data/
    ├── glowing_blocks/sunrise/block/glowing_amethyst.json   # the block itself
    ├── glowing_blocks/loot_table/blocks/glowing_amethyst.json
    ├── glowing_blocks/recipe/glowing_amethyst.json
    └── minecraft/tags/block/mineable/pickaxe.json
```

The block declaration starts from a vanilla block and changes what differs:

```json
{
  "template": "minecraft:amethyst_block",
  "properties": { "hardness": 2.0, "light": 7 }
}
```

The manifest declares the capability `registries`: a mod needs it to add blocks, items, mob effects or entity
types to a world.

## 3. Build it

```sh
cargo sunrise-mod build
```

This reads the content with the game's own package rules, checks that the files of your mod agree with each other
(see [`check`](../how-to/content-mods.md#create-and-package-your-mod)) and writes `target/sunrise-mod/glowing_blocks.smod`,
the file you share.

## 4. Play it

<!-- quickstart-test: skip, needs the reader's game and prepared resources -->
```sh
cargo sunrise-mod run --game <path to sunrise-client> -- --resources <resources folder> \
  --game-dir target/sunrise-mod/game --home target/sunrise-mod/run/home
```

`run` builds the mod and starts that game with it always on. Everything after `--` goes to the game.

- `sunrise-client` is the game, from the download for your computer in the
  [Nightly release](https://github.com/sunrise-org/sunrise/releases/tag/nightly), or one you built yourself.
- `--resources` names [resources prepared from your own Minecraft](first-mod.md#prepared-resources). Write the
  folder's full path: only `--game-dir`, `--home` and the mod's own path are read from the folder you start in.
- `--game-dir` and `--home` keep the game's saves, `options.txt` and logs under the project's `target/`. Without
  them the saves and settings land in the project folder itself and the logs in your own Sunrise data folder.
- If your mod uses [`[[jar]]` originals](../how-to/content-mods.md#java-jars-as-external-assets), `run` sets the
  development home itself; you can omit `--home`. Use `--jars <dir>` to choose a different source folder.
- Quote a path that has spaces.

Open a world and craft the block from an amethyst block and glowstone dust. The game reads content once, at
start-up: after you edit a file, run the command again.

To play it in a game folder you already use, copy `glowing_blocks.smod` into that folder's `sunrise-mods/` (the game
folder is the one that holds `options.txt`; create `sunrise-mods/` if it is missing) and turn it on with the line
`mod.glowing_blocks:true` in `sunrise-mods.txt` there ([Installing and using mods](../players/installing-mods.md)).

## 5. Check it before you publish

```sh
cargo sunrise-mod check target/sunrise-mod/glowing_blocks.smod
```

`check` runs what the mod registry runs on a submission. Follow [Publishing your mod](../how-to/publish.md) to
submit the `.smod` without binaries; content projects do not generate the native release workflow.

## Next

- [Content mods](../how-to/content-mods.md): every kind of content file, from blocks to world generation.
- [The manifest and the package](../reference/manifest.md): every manifest field and package rule.
- [Publishing your mod](../how-to/publish.md): signing and the registry.
