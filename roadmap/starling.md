# Starling roadmap

What is being done on Starling, the Android port of Sunrise, what comes next and what waits. There are
no dates: things are Now, Next or Later. Each item says in plain words what you will be able to do, with
the technical picture under it. The game itself is Sunrise's: its own roadmap is [ROADMAP.md](../ROADMAP.md).

## At a glance

**Being done now**

- **The first start on a phone**: sign in, and the game's files are downloaded and prepared on the
  phone, with a pond to fish in and the settings in a hand-drawn skin while it works.
- **Shader packs on a phone**: Complementary and MakeUp running smoothly on a phone, picked on the
  Shader Packs page, and Starling Visuals, Starling's own pack made for phones.
- **Servers with plugins**: The custom items, armour, models, menus and buttons of plugin servers
  (ItemsAdder, Nexo, ModelEngine, BetterModel, dialogs) look and work as on Java.
- **Settings that make sense on a phone**: Every option does what it says, and the ones a phone can't
  use are out of the way.

**Next**: a pack's own options in the game; secure chat
for servers that require it; physical controllers on Android; Open to LAN and invites on a phone;
keeping up with Sunrise.

**Later**: a store release; more of the game open to mods; Sunrise's native mods; a phone-sized mod
browser.

## Now

### The first start on a phone

You install the APK, sign in with the Microsoft account that owns the game, and play: no computer.

> Done (v0.1.0): the setup in place of the loading failure, drawn in Sunrise's own font; the sign-in
> by code; the official files fetched from Mojang for an owner (starling-account); OpenJDK 25 for
> Android fetched once, checked against its pinned hash, and run through a launcher the APK carries
> in its library folder (the only place an app may run programs from), with the exporters compiled
> on the phone from the sources the APK carries; the prepared folder becomes the game's resources
> and the runtime goes. Prepared on a POCO F6 Pro in 79 s from the official 26.3 files. While it
> waits: fishing seen from above (a bobber where the finger holds, a fish swimming to it, sounds
> made in the game) and Settings, the game's own Options screens in a hand-drawn skin
> (packaging/lowbudget: our own handwriting, every letter written in five takes and picked by where
> it stands; buttons drawn without a ruler in four takes). In v0.1.0.1: every language the game has
> (the downloaded files carry them; resources prepared on a computer before sunrise-prepare learnt
> them had English only); no file error under the card on a first start; the credits close with a
> tap or the back key; Touch Controls opens the layout editor over the setup and the title too. Left: the download with an account that owns the game, tried end to end;
> plainer words on the setup's errors; how long it takes on slower phones.

### The keyboard

You type in the chat, on signs and in books as in any app: suggestions, autocorrect, swipe, paste.

> Done (v0.1.0.1): a hidden editor (TextMirror.java) behind the on-screen keyboard instead of
> NativeActivity's key events, holding a copy of the focused field, its edits sent to the game whole
> (text and selection) and the game's own changes (Tab, the history, a line sent) sent back; no
> suggestions for commands, command blocks, names and searches; a Paste button on the chat bar;
> Enter completing the words of a command that does not parse when each has exactly one
> completion (never free text, never a word the server completes). Left: trying it on a phone with
> Gboard and other keyboards; the text fields of mods, the recipe book and the options, which
> still get the key events.

### Updates from the game

You hear about a new release on the title screen, and the game installs it: no browser.

> Done (v0.1.0.1): a release looks at GitHub's latest release on start (test builds, signed with
> another key, do not); Update downloads its APK (checked against the SHA-256 GitHub gives) into the
> app's own files and hands it to Android's installer through a provider of the app's (Android asks
> to allow installs from Starling the first time, and offers to open it once updated); Tomorrow
> asks again in a day, No at the next start. Starling Visuals comes in the APK and is put in
> `shaderpacks/` when the app carries a new one. Left: trying an update from one release to the next
> on a phone (v0.1.0 has no prompt: it is installed over by hand once; v0.1.1 is the first offered).

### What players reported

The touch controls players know from Bedrock, a world that keeps when the app is closed, and no
anticheat flags for digging up while flying.

> Done (v0.1.1): Bedrock's own touch buttons, fetched on the phone from Mojang's bedrock-samples
> (bedrock_touch.rs, a pinned revision, each file checked by its git blob SHA-1; the project carries
> names and hashes only), the ones Bedrock has none for drawn on its plate with Starling's glyphs,
> Starling's own look where they are missing; the layout editor before a world too; Sneak as Hold or
> Toggle, and in flight going down only while held (a choice for touch: Java's toggled sneak keeps
> sinking); the autosave's save as soon as the app loses focus, without holding Android's UI thread,
> level.dat_old read when level.dat is missing, the last run's log kept as previous.log; a dig sent
> before the move into its space with the stick in Free Direction, and a refused break putting the
> player back (BlockStatePredictionHandler). Left: trying the dig fix on a GrimAC server with
> latency; placing blocks while steering, which still waits a tick.
>
> Done (v0.1.2): Options > Skin Customization with the name the worlds use (vanilla's offline-name
> rules, kept in sunrise-player.txt; a signed-in account's name on servers, which cannot be told
> apart before joining) and a skin from `skins/` or the file picker, classic or slim, worn in the
> atlas's own-skin slot the way a Mojang skin is (first and third person, the inventory, the player
> list), a skin PNG opened with Starling put on at once; the player on the title screen beside the
> menu, a press opening Skin Customization; Ely.by's skins (elyby.rs: skinsystem.ely.by's
> `/textures/{name}` once a name a session, on by default, Ely.by Skins in Skin Customization):
> offline, the skin the name worlds open with has there when no file is chosen, and on an
> offline-mode server the players it sends no textures for; the Starling folder offered in the setup
> before the sign-in; with no frame cap, the screen's fastest refresh rate asked for
> (ANativeWindow_setFrameRate), where a phone's own policy held games at 60 Hz; the window's
> surface dropped when the app goes to the background, as before v0.1.1 (Sunrise's
> `release_surface` now keeps it for a replacement renderer, and the way back drew to the
> destroyed window: a black screen after the sign-in or the file picker, and most likely the
> crash a player had there on an Adreno 830). Left: a skin others see on servers (it is the
> account's, or a plugin's such as SkinsRestorer).

### Shader packs on a phone

You play with Complementary or MakeUp on your phone at a frame rate you can play at, and choose the pack
and its quality in the game.

> Sunrise's shader-pack translator (crates/sunrise-shaders) runs Iris and OptiFine packs as they are,
> GLSL translated to WGSL at load time; nothing is hard-coded per pack. On an Adreno 740 at half
> resolution Complementary's LOW profile draws in about 16 to 21 ms of GPU time (45 to 55 fps) with the
> experimental depth pass for solid terrain (`SUNRISE_SHADER_PACK_PREPASS`), which lets the tiler drop
> hidden grass and leaves before the pack's fragment stage runs. Done: the Shader Packs page (Video
> Settings: the pack and its profile, saved in sunrise-graphics.txt), option values and profiles,
> constant texture matrices, a depth pass that keeps no more than the pack's own alpha cut, resolution
> down to 25%, outputs kept finite (Adreno made NaNs that turned Complementary black for good after a
> pond was flooded), `cameraPositionInt`/`Fract`, terrain's face normals and `mc_Entity` off a Mac
> (they read up and -1 everywhere but Metal: side faces were shaded as tops, which striped and
> flickered their shadows, and the pack could not tell water or leaves), programs translated and
> built beside the game (the world draws without the pack for the few seconds it takes, instead of
> stopping), view bobbing in the model-view matrix as Iris has it (reflections and the water's dark
> band swung with each step), the shadow view turned about the player rather than the world's origin
> (far from spawn its shadows shook), casters between the light and the shadow camera kept, at its
> near plane (the sun lit caves when it was low). Starling Visuals, Starling's own pack
> (packaging/shaderpacks/starling-visuals): lit per vertex, shadows with a soft edge, water with
> waves and depth, haze, sun rays, bloom and light from what you hold; 70 to 80 fps with shadows at
> full resolution on an Adreno 740. Left: the pack's own options screen
> (`SUNRISE_SHADER_OPTIONS` for now) and light defaults on phones; a cache of the built programs;
> `cloudHeight` and the other uniforms still read as zero; the depth pass on by default once it is
> checked on more packs.

### Servers with plugins

On a server with plugins you see its custom items, armour and models, click its chat buttons, open its
menus and dialogs, and stay on it without the anticheat pulling you back.

> Done: dialogs; server resource packs reloaded on a worker (no "app not responding" on join);
> ItemsAdder's custom armour (its leather-atlas shader trick emulated, custom_armor.rs) and armour from
> any namespace a stack's own `equippable` names (Nexo, Oraxen, ItemsAdder's modern mode); chat click
> events (`run_command`, `open_url` with vanilla's confirmation, `show_dialog`, `custom`); interaction
> entities that can be hit and used; display entities that ride their mob or the player (ModelEngine and
> BetterModel models, cosmetics, nameplates) and interpolate their transformation and moves as vanilla
> does; item cooldowns, forced looks and custom chat completions;
> the player's real language, chat visibility and main hand sent to the server; Free Direction, D-pad
> and Analog movement chosen per server, tested against GrimAC; text `shadow_color` and a glyph region
> that starts again when full (HUD and menu plugins); block displays drawn from their block state, and
> displays' brightness. Left, most important first: secure chat (servers that enforce signed chat),
> glowing outlines, the components
> still dropped from items (custom tooltips, food, gliders), `object` text (sprites and heads in chat),
> server-synced biomes, mob variants, paintings and trims, and the packets still ignored (camera, server
> links, message deletion).

### Settings that make sense on a phone

Every option you see works, and those a phone can't use don't clutter the menus.

> Done: Controller Off no longer turns the touch controls off; Entity Shadows, Chat Visibility and High
> Contrast do what they say; options Sunrise stores but does not draw are greyed out; the controller
> page on a phone keeps what the touch controls use; Discord is gone on a phone. Left: a touch button
> for Zoom; the Keystrokes overlay following the touch stick; one graphics preset instead of vanilla's
> and Sunrise's side by side.

## Next

- **Keeping up with Sunrise**: v0.1.1 merged Sunrise's main of 2026-10-28 (2118 commits: the client
  split into crates, server mods, HDR, Sunrise's own shader packs); to merge it again often, so that
  its fixes come over and mods made for a Sunrise revision fit.
- **A pack's own options in the game**: the settings screen a shader pack describes, beside its profile.
- **Secure chat**: signed chat and its acknowledgements, for servers that refuse unsigned messages.
- **Physical controllers on Android**: a gamepad backend for the Android build (the desktop one is off
  there).
- **Open to LAN and invites on a phone**: built in, not tried on Android yet.

## Later

- **A store release**, once the first releases have been played for a while.
- **More of the game open to mods**, guided by porting popular Java mods (see
  [mod-porting](../docs/mod-porting.md)).
- **Sunrise's native mods** ([horizon-mixins](https://github.com/horizon-rust/horizon-mixins)):
  mixins rewrite the game's source and the game is built again with them, each mod for one Sunrise
  revision. On a phone that needs Starling at that revision and a way to build it there.
- **A mod browser made for a phone.**
- **Discord Rich Presence on a phone**: Discord's Social SDK (1.10 and later) shows the game through the
  Discord app on Android without signing in, but it needs a Starling application on Discord's developer
  portal and the SDK downloaded from there; until then the setting stays hidden on Android.
