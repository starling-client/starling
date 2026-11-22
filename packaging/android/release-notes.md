**Starling 0.9.0: a launcher before the game, Horizon's newest mod API, and the game built for your phone.**

## Install

- **Already have Starling:** tap **Update** inside the game.
- **New here, or coming from Horizon Android:** download the APK for your phone below and open it. Almost every phone takes **`starling-0.9.0-arm64-v8a.apk`**. Older 32-bit phones take `armeabi-v7a`, and x86 tablets and emulators take `x86_64` or `x86`.

## What's new in 0.9.0

### New

- **A launcher before the game,** from Horizon Android: profiles, each with its own worlds, mods and settings; a catalog of mods, resource packs and shader packs from Horizon and Modrinth; and your installed content, to turn on and off.
- **A wardrobe for skins and capes,** synced with your Microsoft account.
- **Calibrate...** tries the ways your phone can draw the world and keeps the fastest.
- **Custom GPU drivers** (64-bit ARM, experimental), from **GPU Drivers...** in the graphics settings, and **ray tracing** (experimental).
- **Display controls and safe zones,** so the game fits around a notch or rounded corners.
- **Touch controls:** a button's label can be edited, presets can be deleted, and the hub clicks only for a touch that did something.
- **Pause in the Background** (Options, Recommended): the game pauses and goes quiet when you leave it.
- **A mouse clicks the cards** on the title screen, and a small FPS counter shows over menus.
- **Phones start at 100% resolution scale** and "Mobile" far detail.
- **The loading screen shows in half a second.**
- **Diagnostics** (long-press the app icon) chooses the graphics API for the next start, even when the game can't open, and shares its ZIP.
- **Pictures of the game folder stay out of the gallery,** and **Malay** is translated further.

### Faster

- **Horizon Android's performance work:** terrain drawn as Horizon Android 0.7.8 draws it, paused menus no longer drawing the world, shadow maps of at most 4096, and menus over the world at most 60 FPS, so they don't heat the phone.
- **Games with mods open faster:** about 6 seconds less on a phone with 20 or so mods.

### Mods

- **Mods use Horizon's newest mod API.** Content mods (`.hmod`) from Horizon's catalog load. **Native mods** from Horizon's catalog install from the launcher or the in-game catalog; then **Options > Mods > Compile** downloads a library built for your exact Starling, usually in seconds (64-bit ARM).
- **Starling's old `.hnmod` mods no longer run.** Mods made for them need their Horizon versions.
- **Mod switches apply once, when you close Options,** and keep your choice.
- **A mod's recipe for a type no mod adds no longer stops the world from loading.**

### Fixed

- **The catalog lists Horizon's mods again,** and no longer downloads a mod you already have a second time.
- **The crash when the app went to the background while a world was loading** (Mali-G57 and others).
- **PowerVR:** the GE8320's crash on Vulkan, and the BXM's limit.
- **Fujitsu and other 32-bit phones start again,** and slower phones no longer crash right after loading.
- **A crash no longer switches the graphics to OpenGL ES,** and Diagnostics no longer crashes on its second report.
- **Shader packs** with compute programs no longer write outside their images on GPUs that don't guard against it.
- **World upgrades** work in the shared game folder, and a new world keeps its game mode when it restarts for its mods.
- **Sliders** no longer freeze the game or grab your finger when you scroll.
- **Touch:** the camera and chat buttons, GoyControl's sneak, the F3+F4 switcher and the control editor work as they should again.
- **The inventory's player preview** follows the virtual cursor, and vibration turned off stays off.

### Newest Horizon

Starling now sits on Horizon's newest base, about 31,000 commits ahead of 0.8.0's, most of it the new mod API the mods above are built on.

## Report a bug

Open an [issue](https://github.com/starling-client/starling/issues/new/choose) with the diagnostic ZIP: **Options > Diagnostics > Export**, or long-press the app icon > **Diagnostics** if the game can't open. On Discord, `/crash` walks you through it.

## Not sure which APK to install?

Take a screenshot of this page on your phone and send it to an AI assistant, like ChatGPT, asking it to look up your phone's model: it will tell you which APK fits your phone.

---

Starling is unofficial: not an official Minecraft product, and not approved by or associated with Mojang or Microsoft. The game is [Horizon](https://github.com/ludovic111/horizon), by Ludovic Marie; Starling, the Android port, formerly Horizon Android, is by its co-founders [@JuanLuci](https://github.com/JuanLuci) and [@Vasysik](https://github.com/Vasysik). MIT licensed.
