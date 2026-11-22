# Installing and using mods

Mods are off until you install and turn them on. A fresh Sunrise install has no mods folder and plays exactly
like vanilla 26.3.

## Installing a mod

Install a mod from the launcher's **Mods** page or from the game's **Options > Mods > Browse Mods**, or copy its
file by hand into the game folder (the folder that holds `options.txt`):

| Mod | Goes in | What happens next |
| --- | --- | --- |
| Content mod (`.smod`, or a folder holding `sunrise-mod.toml`) | `sunrise-mods/` | Read when the game starts |
| Native mod (`.smod` with Rust source) | `sunrise-native-mods/` | The launcher builds it for that instance ([Native mods](../how-to/native-mods.md)) |

**Browse Mods** in the game lists content mods only, checked against the same registry as the launcher. Native and

Fabric and NeoForge jars do not run in Sunrise: they target Minecraft's Java runtime, which Sunrise does not have.
The data and assets inside a jar can be carried over; see
[Porting Java modpacks](https://github.com/sunrise-org/sunrise/blob/main/docs/modpacks.md).

## Turning mods on

A newly installed mod is off.

- A mod installed from the launcher's **Mods** page turns on there, or with `sunrise-launcher mods enable <project>`.
- A mod you copied in by hand turns on with a line `mod.<id>:true` in `sunrise-mods.txt`, beside `options.txt`.

A content mod that holds only assets and reloadable data (textures, models, sounds, languages, recipes, tags,
loot, advancements and functions) starts working as soon as it is installed from Browse Mods: the game reloads
its resources, then the world's data as `/reload` does, LAN guests included. A mod that adds registry entries
or changes world generation, or that carries Java jar layers, waits for a restart. On someone else's server, all
content waits: the server owns its data.

When two packages list the same extra namespace, the first admitted package owns it. The game leaves the later
package's content out and logs why. If a package lists another package's ID as an extra namespace, startup stops
with an error instead. Turn one of the conflicting mods off.

Benchmarks (`--benchmark`) never load mods.

## The Mods screen

**Options > Mods** lists Minecraft first (and Sunrise, in the official game with its Sunrise layer on), then each
installed mod with its icon, name and version. Once a package is installed, the title screen also carries a
full-width **Mods** button below Minecraft Realms, and it goes away again when the last package leaves. Search
the list and sort it A–Z or Z–A. Select a mod to read its
id, version, license, authors, credits and description:

- **Homepage** and **Issues** open the mod's links, after asking you to confirm.
- **Config** opens the settings page of a mod that registers settings.
- A mod that is installed but off says so. A mod that could not load shows the reason in red.

The screen has no on/off switch; `sunrise-mods.txt` decides which mods run.

## Making a mod

[Install the mod tool](../start/install-tool.md), then follow [Your first native mod](../start/first-mod.md) or
[Your first content mod](../start/first-content-mod.md).

## Sunrise's own mods

Sunrise's official mods are native mods, each in its own repository under
[sunrise-org](https://github.com/sunrise-org). The registry every mod is listed in is
[sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods).
