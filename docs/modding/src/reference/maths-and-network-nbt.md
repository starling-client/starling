# Maths and network NBT

## Portable maths

`mod_api::strict_math` provides platform-independent StrictMath operations:
`sin`, `cos`, `tan`, `exp`, `pow`, `log`, `log10`, `cbrt`, `hypot`, `atan`,
`asin`, `acos` and `atan2`. These are pure functions; NaN payloads are unspecified. The five `sin`/`cos`/`tan`/
`exp`/`pow` additions use fdlibm's ordered IEEE double algorithms, including
full large-argument trigonometric reduction, signed zeros, subnormals and
overflow/underflow. They use no FMA or platform/libm calls and allocate no heap
memory. These operations do not call the Host.
The AArch64 FMA variants are deliberately not part of this StrictMath set.

The committed literal reference table and deterministic seeded generator
(`tools/generate_strict_math_vectors.py`) use only public JDK `StrictMath`
calls, not engine outputs. Tests compare exact bits, except that NaNs are compared by classification. Cross-platform claims require running these vectors on each platform, not just compiling.

`mod_api::java_math` exposes the engine’s vanilla maths functions for native mods,
including `round` and `round_f32`. These are the paths used for vanilla `Math`
call sites; rounding can differ by machine, and universal HotSpot bit equality
is not promised.

## The math profile

`host.math_profile()` is a `Host` read, constant for the process lifetime. Its non-exhaustive `MathProfile` describes the
engine's selection for `sin`, `cos`, `tan`, `exp`, `log`, `log10` and `pow`, without
returning machine/vendor/OS identification:

- `HotSpotX86_64`: HotSpot's x86-64 stubs for `sin`, `cos` and `tan` (a
  translation of Intel's libm) and for `exp`; correctly rounded `log` and
  `log10`; platform libm for `pow`. This is the selection on every x86-64
  target, Windows included.
- `Aarch64Fma`: fdlibm with fused multiply-adds for `sin` and `cos`, fdlibm for
  `log` and `log10`, and the Rust `libm` crate for `tan`, `exp` and `pow`.
  Those last three are not advertised as StrictMath. This branch wins on all
  AArch64 targets, including Windows.
- `CorrectlyRounded`: correctly rounded `sin`, `cos`, `log` and `log10`;
  HotSpot's x86-64 `exp` algorithm; platform libm for `tan` and `pow`. This is
  the selection on Windows targets that are neither x86-64 nor AArch64.
- `LibmRounding`: platform libm for `sin`, `cos`, `tan` and `pow`; HotSpot's
  x86-64 `exp` algorithm; correctly rounded `log` and `log10`. This is the
  selection on every remaining target.
- `StrictMath`: fdlibm throughout. No target selects it.

Every profile retains the engine's Java `pow` special cases. Reading the profile
does not change `strict_math` or promise universal HotSpot equality. It allocates
nothing.

## Network NBT

`Tag::to_network_bytes()` writes the engine's nameless network root, preserving
NaN bits and modified UTF-8 (including unpaired UTF-16 surrogates).
Reading follows the engine codec: negative floating zero normalizes to positive
zero, and mixed-list compound wrappers are unwrapped.
`Tag::from_network_bytes(&bytes, None)` returns `(tag, consumed_bytes)`;
bytes following the root remain unread. `Tag::End` encodes as `[0]` and represents
vanilla's null marker. Errors have the exported `mod_api::NbtError` type.

The default read quota is `mod_api::NETWORK_NBT_QUOTA` (2,097,152 bytes), matching
vanilla 26.3 `ByteBufCodecs.TAG` and `NbtAccounter.defaultQuota`. It counts heap
charges, not wire bytes: primitive tag overhead, array contents, UTF-16 strings
and compound keys, list references, and distinct compound entries. Pass
`Some(larger_quota)` for trusted data. Both methods allow 512 container levels;
the writer refuses strings over 65,535 modified UTF-8 bytes. Vanilla applies
heap quotas on reads, so writing does not impose a heap quota.

The implementation follows vanilla 26.3 `NbtIo.readAnyTag`/`writeAnyTag`,
`FriendlyByteBuf.readNbt`/`writeNbt`, `NbtAccounter`, and each `TagType.load`.
Codec calls take time proportional to the encoded data.
