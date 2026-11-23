# The catalog on Starling

The **Catalog** button finds and installs mods, resource packs and shader packs from inside the game
(`game/mod_catalog.rs`). The Mods, Resource Packs and Shader Packs pages of Options each have one, and each opens on its
own shelf. The menus before the game has loaded don't have it. The searching, the compatibility checks and the checked
downloads are the desktop launcher's mod browser: the `starling-catalog` crate compiles `launcher/src/browser.rs` as it
is, without its window, and answers what the browser asks of the rest of the launcher.

One list you drag with a finger holds the results (`starling_catalog::list`). The next page comes by itself near the
end of the list and adds to it; a new search or shelf starts it over. A page that fails keeps the rows and offers Try
again.

| Shelf | Where from | Goes to | Then |
|---|---|---|---|
| Mods | Sunrise's registry of reviewed mods | `sunrise-native-mods/` or `sunrise-mods/` | A native mod gets its Compile button ([starling-native-mods.md](starling-native-mods.md)); a content mod loads at the next start |
| Resource packs | Modrinth | `resourcepacks/` | The Resource Packs page lists it at once; the player turns it on |
| Shader packs | Modrinth's Iris and OptiFine packs; a search also finds the registry's | `shaderpacks/` | The player chooses it |

- **Native mods** are only offered on ARM64: the build service makes no library for the other architectures.
- **Java mods aren't offered.** Modrinth's mods are Java: on the desktop their content is ported, and this catalog
  leaves that out.
- **Resource packs for other versions** stay hidden while the toggle says **Other game versions: Off**. With it on,
  each says what it was made for.
- **Modrinth's shader packs** aren't in the desktop browser, because the desktop's own format is `.sshader`.
  `starling-catalog` searches them itself, the most downloaded first, downloads the newest ZIP (one made for this
  version of the game, if any), checks its SHA-512 and that it holds a `shaders/` folder. Whether it runs well is up to
  the converter and the phone.
- **What's installed** is listed in the browser's own file, `mod-browser.json`, in the game folder.

## Testing without a phone

`cargo test -p starling-catalog` runs what needs no network, the list's pages and the finger on it among them. The
browser's own tests stay in the launcher's workspace. `cargo test -p sunrise-client-render --lib options_ui` covers the
pages' Catalog buttons and the Resource Packs page's list after an install.
