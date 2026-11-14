# libadrenotools in Starling

Vendored from [bylaws/libadrenotools](https://github.com/bylaws/libadrenotools)
at `8fae8ce254dfc1344527e05301e43f37dea2df80`, with its
`lib/linkernsbypass` submodule ([bylaws/liblinkernsbypass](https://github.com/bylaws/liblinkernsbypass))
at `aa3975893d83ef1bc84c321ec60c65fbf1287887`.

Both under the BSD-2-Clause license (`LICENSE.adrenotools`,
`lib/linkernsbypass/LICENSE`).

## What is vendored

Only what Starling builds and reads on Android (`arm64-v8a`):

- `include/adrenotools/`: the public headers (`driver.h`, `priv.h`, `bcenabler.h`).
- `src/driver.cpp`: `adrenotools_open_libvulkan`.
- `src/hook/`: `hook_impl` (`hook_impl.cpp`), the `main_hook` and
  `file_redirect_hook` entry points, and `gsl_alloc_hook.c` (kept for
  reference; not built: Starling never sets
  `ADRENOTOOLS_DRIVER_GPU_MAPPING_IMPORT`).
- `lib/linkernsbypass/`: the namespace-bypass helpers `driver.cpp` and the
  hooks link against.
- `src/bcenabler.cpp`, `src/bcenabler_patch.s`, `gen/bcenabler_patch.h`:
  kept for reference; not built (BCn patching is not enabled yet).
- `src/hook/kgsl.h`: the Linux kernel's KGSL UAPI header
  (GPL-2.0 WITH Linux-syscall-note): ioctl declarations only, used through
  syscalls as the note permits; no kernel code is compiled in.

## What is left out

Upstream's `CMakeLists.txt` files (Starling compiles these sources
directly with the NDK clang in `packaging/android/build_apk.py`, without
CMake), `build_asm.sh`, `adrenotools.pc.in`, and `tools/` (driver
packaging helpers unrelated to the game).

## Build

`packaging/android/build_apk.py` builds, for `arm64-v8a` only:

- `libadrenotools.so` (`driver.cpp` + `linkernsbypass`),
- `libhook_impl.so`, `libmain_hook.so`, `libfile_redirect_hook.so`,

with 16 KB page alignment, and packages them in the APK under
`lib/arm64-v8a/`, so Android extracts them to `nativeLibraryDir`
(`extractNativeLibs`), where `adrenotools_open_libvulkan` expects its
`hookLibDir`.
