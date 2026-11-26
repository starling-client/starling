# Your first content mod

Make **Glowing Amethyst**, a block you can craft, place to light a room, and mine with a pickaxe. You'll start
from a working example, see what each file does, and pack your mod into a single `.smod` file to share.
There's no Rust code to write: a content mod describes its blocks, items and recipes in data files.

You need Rust 1.95 or newer (`cargo`) and a text editor. You can make and pack the mod before you have the
Sunrise game and [prepared resources](first-mod.md#prepared-resources), which you only need to play it.

## 1. Install the mod tool

Follow [Install the mod tool](install-tool.md) to get the `cargo sunrise-mod` command. Keep your terminal
open as you go through the guide.

## 2. Make the project

In your terminal, start in the folder where you keep your projects:

```sh
cargo sunrise-mod new glowing-blocks
cd glowing-blocks
```

Open `glowing-blocks` in your editor. The tool gave you a complete mod, not a blank folder. Here's where
its block, assets and crafting recipe live:

```text
glowing-blocks/
├── sunrise-mod.toml
├── assets/
│   └── glowing_blocks/
│       ├── blockstates/glowing_amethyst.json
│       ├── models/block/glowing_amethyst.json
│       ├── items/glowing_amethyst.json
│       └── lang/en_us.json
└── data/
    ├── glowing_blocks/
    │   ├── sunrise/block/glowing_amethyst.json
    │   ├── loot_table/blocks/glowing_amethyst.json
    │   └── recipe/glowing_amethyst.json
    └── minecraft/tags/block/mineable/pickaxe.json
```

### Give the mod an identity

Open `sunrise-mod.toml`. The tool turned the folder name `glowing-blocks` into the mod ID `glowing_blocks`.
That ID is the namespace for the files in `assets/glowing_blocks/` and `data/glowing_blocks/`, so your new block
will be `glowing_blocks:glowing_amethyst`, distinct from vanilla's `minecraft:amethyst_block`.
[How a mod works](../concepts/how-mods-work.md) explains why those names matter.

`name` is what players see in the Mods screen and `version` identifies this release. Try filling in `description`
and `authors`, for example `authors = ["Your name"]`. Keep `side = "both"`: when you play on a server,
the join handshake checks registered content separately from the mod ID and version, so a block mod needs the
same mod ID, version and registered content on both the server and the joining client.

### Define the block

Open `data/glowing_blocks/sunrise/block/glowing_amethyst.json`. The path gives the block its ID, and the
file says what to borrow from vanilla and what to change:

```json
{
  "template": "minecraft:amethyst_block",
  "properties": { "hardness": 2.0, "light": 7 }
}
```

`template` gives the block amethyst's shape and sounds. `hardness` helps determine how long it takes to
mine, and `light` sets the light it gives off. Set `light` to `15` if you want to see it shine more brightly
in step 4. The declaration also creates a matching item so you can hold the block.

### Give it a look and a name

Inside `assets/glowing_blocks/`, follow the block from its state to the picture you see:

1. `blockstates/glowing_amethyst.json` maps the block's default state (`""`) to `glowing_blocks:block/glowing_amethyst`.
2. `models/block/glowing_amethyst.json` defines that model. It uses vanilla's `minecraft:block/cube_all`
   with `minecraft:block/amethyst_block` as the texture on every face. You don't need to draw anything yet.
3. `items/glowing_amethyst.json` points the item at the same model, so it looks like the block in your inventory.
4. `lang/en_us.json` gives `block.glowing_blocks.glowing_amethyst` its English name, “Glowing Amethyst”.
   Edit the displayed name if you like, without changing the key.

### Make it craftable and mineable

- `data/glowing_blocks/recipe/glowing_amethyst.json` is a shapeless recipe: an amethyst block and a glowstone
  dust can go anywhere in the crafting grid. The result names your block's item by ID.
- `data/glowing_blocks/loot_table/blocks/glowing_amethyst.json` gives the block a drop. It rolls once for the
  item `glowing_blocks:glowing_amethyst`.
- `data/minecraft/tags/block/mineable/pickaxe.json` adds your block's ID to vanilla's pickaxe tag so a pickaxe
  mines it quickly. That's why this file goes under `minecraft`, not `glowing_blocks`.

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

Open a test world and craft the block from an amethyst block and glowstone dust. To skip the crafting for
now, enable commands and enter `/give @s glowing_blocks:glowing_amethyst` in chat. Place it in a dark spot
and see it glow. Then switch to Survival and break it with a pickaxe: it drops itself.

Want your block to look different from ordinary amethyst? Save your own PNG at
`assets/glowing_blocks/textures/block/glowing_amethyst.png`, then set the model's `textures.all` to
`glowing_blocks:block/glowing_amethyst`. After editing a file, quit the game and run the same command again
so the game sees the rebuilt package.

To play it in a game folder you already use, copy `glowing_blocks.smod` into that folder's `sunrise-mods/` (the game
folder is the one that holds `options.txt`; create `sunrise-mods/` if it is missing) and turn it on with the line
`mod.glowing_blocks:true` in `sunrise-mods.txt` there ([Installing and using mods](../players/installing-mods.md)).

## 5. Check the package

```sh
cargo sunrise-mod check
```

Before tagging a release, `cargo sunrise-mod check` in the project folder also reports any missing page
information. The content template includes the shared release workflow caller. Follow
[Releasing](../how-to/publish.md#releasing) to register and publish your mod.

`check` validates the project. To check a downloaded `.smod` after release, pass its path and keep its
[release attestation](../reference/sdk/05-commands.md#check-a-released-package) beside it.

## Next

- [Content mods](../how-to/content-mods.md): every kind of content file, from blocks to world generation.
- [The manifest and the package](../reference/manifest.md): every manifest field and package rule.
- [Publishing your mod](../how-to/publish.md): registering your repository and releasing to the registry.
