# Android architectures

The same ABI selection supports `x86` and `x86_64`, adapted from the Android
fork's four-ABI build. Their native launchers, C++ libraries and updater APK
selection match the selected ABI. CI builds each ABI separately.
Android x86_64 uses the pinned x86_64 Java 25 runtime. The runtime publisher
does not provide Android x86 Java 25: 32-bit x86 uses imported prepared `.starling`
resources, as the Android fork's prepared-resource approach did. On-device
preparation reports this limitation instead of downloading an ARM runtime.
The x86 architectures have not been tested on a physical device.

ARM64, ARMv7, x86 and x86_64 share the same build/sign/release matrix. Each is
published as a separate APK; release publication waits for all four builds.
A failed build of any ABI deliberately blocks that release batch, including
x86, whose resource setup uses import rather than on-device preparation.
ARMv7 builds use `armeabi-v7a` and require Android
8 or newer. CPU ABI support alone does not guarantee that a device's graphics
driver can run the engine. Older PowerVR/Mali/Adreno hardware needs separate
on-device validation; a successful run on a modern tablet is not evidence for it.

## Build

Install the Android SDK/NDK pinned in `packaging/android/build_apk.py`, a JDK,
`cargo-ndk` and Rust's `armv7-linux-androideabi` target. Then:

```sh
python packaging/android/build_apk.py --abi armeabi-v7a --dev
```

`--abi arm64-v8a` preserves the default build. Each APK contains only its selected
ABI. The ARMv7 Java launchers and C++ runtime are 32-bit, and resource preparation
selects the pinned Android ARM32 OpenJDK 25 archive with SHA-256 verification.
Every architecture follows the same Preview/debug/release signing rules and
profile selection. Tagged releases are published once, with all four APKs and
one shared Starling Visuals archive, only in the existing release repository.
PR and fork builds never publish releases. ARM64 is only the local command's
default ABI. Manual native-mod APK builds also receive the selected ABI and
variant, with separate build caches for each architecture; the separate native-mod
prebuilt-library service retains its existing scope.

## Graphics and address space

The renderer still selects available backends and checks their capabilities.
Legacy GLES driver workarounds are adapted from the Android fork: conditional
sprite selection, unrolled RGSS/overlay/emissive lookups, packed terrain varyings
and conditional HDR texture selection. Starling's tiled-face RGSS behavior, HDR
scene varying and all 128 emissive entries are preserved. These transformations
apply only to GLES; the Vulkan WGSL remains byte-for-byte unchanged.
The Android fork's fallback for adapters without `BASE_VERTEX` rebases the
vertex-buffer binding and draws with a zero base vertex. It applies to both
opaque and translucent terrain, retaining Starling's bounded arena bindings.
Vulkan is optional in the additional-ABI manifests so a GLES-only device can attempt to
start; this does not guarantee support for every GLES driver. Actual API,
texture-array and shader limits remain authoritative.

Existing terrain texture-array splitting follows the adapter's layer limit,
with at most eight arrays. A pack that needs more layers than those arrays hold
is rejected rather than silently losing textures.

The shared GUI/entity atlas follows the GPU's 2D texture limit. Android uses
Sunrise Android's skyline packing for small atlases so gaps below shorter
images remain usable. Its fitting policy preserves GUI, fonts and dynamic
slots, reducing eligible non-UI sprites only when packing still fails.
Android OpenGL panoramas use the fork's 256-pixel box-filtered faces and an
explicit cube vertex buffer. Vulkan retains full-resolution panorama faces.

Block-sprite budgets retain Starling's configured values and its 256 MiB default,
as in the Android fork; there is no extra 32-bit 64/128 MiB override. These
budgets do not bound total game memory.
Normal GPU arena growth on Android ARM32 uses steps of at most 8 MiB, following
the Android fork's policy while retaining Starling's newer paged arenas. A single
larger allocation request can still require a larger step.

The updater selects the APK matching the process ABI. A release with only an
ARM64 APK is not offered as an ARMv7 update. Native-mod libraries must also match
the process ABI; existing ARM64 prebuilt native mods are not ARMv7 builds.
The build service and prebuilt selection are offered only in ARM64 processes.
Other architectures explain this limitation on imported native mods and do not
submit builds or download incompatible libraries.

On 32-bit Android, preparation runs its lanes sequentially and caps each Java
heap at 512 MiB, including the persistent exporters and data generator.
Devices below 1.5 GiB of RAM use 384 MiB, also the fallback when total RAM cannot
be read. The selected limit is logged; it leaves more room for Android and the
installer's renderer.
Smaller requested heaps stay smaller. This bounds JVM heap reservations, not
total Java/Rust/native memory; very low-memory devices can still run out of RAM.

APKs carry Sunrise's original compiled exporter bundle, checked against its
source/pin key and class hashes. A host JDK 25 compiles and link-checks it while
building the APK; pinned Minecraft jars/libraries stay in the ignored build
cache and are never packaged. `SUNRISE_MINECRAFT_DIR` can supply those inputs;
otherwise the build fetches and verifies them. No Minecraft assets are needed
for building the bundle. ARM32's Java runtime has no `jdk.compiler` module, so
preparation executes the shipped classes and does not invoke javac on the phone.
A missing or damaged bundle fails before downloading Java, with a reinstall
message. Updating the kit retires old sources/classes and retains its work cache.
ARM32 uses the checksum-pinned EclipseJRE Java 25.0.4 runtime. The previously
tested AngelAuraMC ARM32 25.0.5 runtime produced incorrect floating-point results
and stalled or failed while loading registries and generating reports; disabling
JIT did not fix it. The replacement passes those stages with the normal batch
JIT policy. Command registries and data-generator command/registry reports match
the x86_64 output byte-for-byte; creative inventory matches after normalizing
the iteration order within item tag sets.

## Relation to diagnostics

Android preparation-lock fixes and renderer-independent diagnostic recovery from
PR #272 are included from the target branch. Never commit Minecraft files,
generated resource tables or prepared resources.
