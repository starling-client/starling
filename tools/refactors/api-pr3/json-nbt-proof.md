# Shared JSON-to-Tag boundary

Part of #1608 and #1852. Diverges from upstream: fork-only.

The converter is `horizon_client::json_nbt::json_to_tag(&Value) -> Result<Tag, JsonNbtError>`.
It lives on the game side, so no JSON signature is added to mod_api. The API3
crafting custom-data, structure-config, Text and loot layers should all call
this converter rather than copying number/list conversion rules. This commit
provides the layer; it does not change those consumers ahead of their layers.

## Vanilla specification

Read the prepared 26.3 server decompile:

- `net.minecraft.nbt.NbtOps`: `empty`, `createBoolean`, all numeric factories,
  `createMap` and `createList`. In this version `createList` directly constructs
  ListTag, not an array collector. Byte/int/long collectors are used only by
  `mergeToList`, which JsonOps-to-NbtOps conversion does not call.
- `net.minecraft.nbt.ListTag`: mixed lists are supported directly.

Read the actual pinned DataFixerUpper 10.0.21 bytecode with `javap -c -p`:

- `com.mojang.serialization.JsonOps.convertTo`: BigDecimal.longValueExact,
  smallest signed byte/short/int/long; on nonintegral or out-of-long numbers,
  float if the double round-trips exactly through float, otherwise double.
- `DynamicOps.convertList`: recursive `convertTo`, then destination createList.
- `JsonOps.getStream` / `getMapValues`: nested JsonNull becomes Java null.
  `convertTo` throws NullPointerException on it. Top-level JsonNull instead
  becomes NbtOps.empty (EndTag). The Rust boundary reports a typed NestedNull
  refusal rather than panicking or inventing nested End values.

The parity command runs these real methods, not a reimplementation of the
oracle. DataFixerUpper is MIT (Microsoft; Mojang/DataFixerUpper LICENSE). No
third-party source was copied into the implementation; bytecode and vanilla
source establish observable rules. Game jars, decompiled source, and generated
oracle output stay local and are not committed.

## Input precision

The input is an already parsed serde_json::Value, not original JSON text.
Number typing uses that Number's decimal spelling, matching JsonOps on the
same spelling. Float-backed Values have already lost precision in serde_json;
this converter does not claim to reconstruct discarded decimal digits.
Signed and unsigned integer-backed numbers never go through f64 before the
exact-long decision. JSON arrays, including numeric and empty arrays, always
remain NBT List, never ByteArray/IntArray/LongArray.

## Proof

Run `python3 tools/refactors/api-pr3/test_json_nbt.py` with the prepared pinned
26.3 data. It runs a Java oracle, compiles the exact production Rust source in
a minimal test crate under target/ (no graphics rebuild), and compares all 43
samples structurally, including exact tag IDs and IEEE bits. It also runs four
focused unit tests. The fixture covers custom data, structure config, Text,
loot metadata, numeric boundaries, lists, nested containers and refusals.

The script temporarily applies four production mutations, requires failing
assertions (not compiler failures), restores the exact original bytes in a
finally block, and reruns the baseline. Mutants: long narrowed to int, all
fractions narrowed to float, accepting nested null, numeric list made a byte
array. All four were killed; both baseline runs passed all five tests.

Only this focused boundary harness and rustfmt of touched Rust files were run
locally. Workspace tests/check/clippy are deferred to consolidation PR CI under
the room's local-CI prohibition. No in-game trigger: this commit adds a pure
conversion primitive and does not alter timing, rendering or client/server
behaviour. No sample API is changed by this commit.

Tick-time effect: none until callers adopt the layer. Conversion allocates
owned NBT strings/containers and decimal scratch; callers should convert at
load/reload and retain Arc<Tag> for repeated metadata reads, not run it per
column, per piece or per tick.
