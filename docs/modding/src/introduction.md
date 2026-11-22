# Modding Sunrise

<p class="lede">Sunrise is Minecraft: Java Edition 26.3 written in Rust. This book shows you how to make mods
for it: new blocks and items from data files, and Rust code that hooks into the game.</p>

![A Sunrise mod is one .smod package. A content mod holds a manifest, assets and data. A native mod
also holds a Rust crate built for the exact game build.](images/mod-kinds.svg)

## Two kinds of mod, one package

Every Sunrise mod ships as one `.smod` file: a zip holding a `sunrise-mod.toml` manifest and any
`assets/` and `data/` folders the mod provides.

| Kind | What it holds | How it runs |
| --- | --- | --- |
| **Content mod** | Data and assets: blocks, items, recipes, loot, tags, models, textures, world generation | The game reads them at startup. No code. |
| **Native mod** | A Rust crate, its lockfile, and any content the mod provides | The code runs in the game process. It is compiled against the exact game build, using the prebuilt SDK. |

Native mods use `mod_api` (the `sunrise-mod-api` crate) for hooks, Parts and attachments.

## Where to start

<div class="cards">

[**Install and play with mods**](players/installing-mods.md)
Where mods go, how to turn them on, and what the Mods screen shows.

[**Make a content mod**](start/first-content-mod.md)
A new block with its item, model, recipe and loot, in a few files.

[**Make a native mod**](start/first-mod.md)
A Rust mod that answers a hook, built and run by `cargo sunrise-mod`.

</div>

## How this book is organised

- **Getting started** walks you from nothing to a running mod.
- **Concepts** explains how the pieces fit: hooks and the `Host`, Parts, capabilities, attachments,
  transactions, and the routes a mod has into the game's own code.
- **How-to** pages each solve one task, grouped by what you are changing.
- **Reference** lists the manifest, the package and registry formats, and links the
  [API reference](reference/api.md), which documents every hook, event and `Host` method.

The complete example mod is [sunrise-machine-sample](https://github.com/sunrise-org/sunrise-machine-sample).
A coding agent can start from [`llms.txt`](llms.txt).
