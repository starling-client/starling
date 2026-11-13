# Text boundary traversal probe

This small crate compiles the production game Text conversion module directly, without linking Game.
It declares only the two internal conversion trait interfaces. The tests and conversion behavior are
included from `game/modding/api_values/text_boundary.rs`, not copied into this crate.

```sh
cargo test --manifest-path tools/refactors/api-pr3/text-boundary-tests/Cargo.toml --lib
```

The 256 KiB tests use the real conversion traits, including final engine span construction. They cover
512-node round trips and typed `TextFormError::TooDeep` refusals at 513 through children, translation
arguments, selector and NBT separators, atlas and player fallbacks, hover text and hover entity names.
An empty parent with a deep entity-hover name covers disposal of unused styles. Source-less spans
are captured directly, preserving opaque native Tags without a JSON materialization step. A separate
test puts 4,096-container opaque metadata beside an over-depth component to prove component refusal
precedes metadata cloning. The typed-payload tests additionally cover late capture cleanup,
representation refusals across eight payload locations, and source-less mixed Tags at 512 and 513.
Comparisons use flat `TextForm` values; recursive trees are deliberately forgotten on the small stack.

The core component tests additionally cover callback order and lazy content selection, hover resolver
isolation, empty first-array style inheritance, structured hover translations and mixed fallback or
unused-argument temporaries containing deep hover trees. Temporary engine text and JSON disposal use
explicit work lists; neither the API nor engine Text type has a custom Drop or Clone implementation.

Native generated host calls and wasm adapter refusal have separate tests in the game and generator
crates. The generator stages all mutable event-field conversions before applying any field. Text-only
host methods return `Result<_, TextFormError>`; existing mixed error channels retain a typed Text case.
A wasm Text form refusal remains typed through guest lowering and host lifting; unrelated malformed
transport values remain transport refusals.

## Typed nodes and opaque data

`TextForm` contains shallow `TextNodeContent`, `TextNodeStyle` and `TextNodeHover` records, not JSON
strings. All component recursion is represented by parent/relation rows. A Text hover marker requires
one hover-text child; singleton separators, fallbacks and entity names reject duplicates. The public
Text API has no JSON carriers and no 100-container gap. Metadata, dialog/custom payloads and hover
item component data use `Tag`; profiles use `TextProfile` and ordered signed properties with opaque
unknown fields.

Native capture first plans the entire borrowed component topology. Native emission validates the API
tree without cloning it, then creates borrowed JSON-representation proofs for every opaque payload
before constructing any JSON. Only the engine-side codec uses JSON, through the shared core converter.
`NonFinite` and `InvalidString` remain typed refusals, never null or replacement-character clamps.
Dynamic contents, styles, metadata, object fallback and the untrusted file-action policy have focused
fidelity tests. Event metadata is filtered by the actual action, not by fields belonging to other kinds.

### Existing component-model Tag wire limit

Native Tags retain the semantic 512-container bound. The existing wasm Tag byte codec can refuse a
mixed-type list near that bound: network NBT inserts synthetic compounds which consume additional
encoded nesting. A 512-list payload ending in `[Byte(1), Short(128)]` demonstrates this distinction.
This is a known wasm-only wire limit, not a reason to tighten native Text. No new carrier or separate
encoded-wire budget is installed here; the replacement derived `Wire` path in #1909 handles it without
synthetic wrappers. The Text Via boundary maps such carrier failures to `TextFormError::Malformed`, while preserving
existing typed Text refusals. Generated-runtime tests exercise second-stage carrier lowering/lifting,
unsent calls and unchanged event writeback; they do not alter or retune the Tag codec.

## Ownership versus crossing safety

Recursive `Clone`, `Eq` and `Drop` of a tree the mod itself built are ordinary Rust ownership and are
not part of the boundary guarantee. A guest overflowing its own stack on a 512-deep tree it received
traps within its dependency group. Neither statement excuses recursive boundary traversal.

## Host stack measurements

`stack_need.py` bisects abort-isolated debug processes running the real host builder, conversion,
recursive clone/drop and return conversion with prepared vanilla language data. It includes ordinary
ownership operations, unlike the 256 KiB boundary-only tests. Repeated 4 KiB-resolution measurements
for the iterative implementation passed at these stack sizes:

| Shape | Largest failing bytes | Smallest passing bytes |
| --- | ---: | ---: |
| Extra | 1,396,736 | 1,400,832 |
| Translation argument | 1,716,224 | 1,720,320 |
| Hover text | 1,105,920 | 1,110,016 |
| Selector separator | 561,152 | 565,248 |
| Atlas fallback | 1,044,480 | 1,048,576 |
| Entity hover name | 1,105,920 | 1,110,016 |
| NBT separator | 561,152 | 565,248 |
| Player fallback | 1,044,480 | 1,048,576 |

These are observed debug-build thresholds, not portable stack guarantees or an entire tick-frame
margin proof. No thread-stack constant is installed here; X3 owns that follow-up. See [X3.md](X3.md)
for the earlier measurement procedure and [thread-inventory.md](thread-inventory.md) for the builders.
