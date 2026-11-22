# Wire types

`mod_api::wire::{Wire, Reader, Writer, WireError}` defines the codec used by
[typed payload tokens and bodies](../how-to/client/networking.md#typed-payloads)
and Sunrise's recipe placement and recipe-book sync. A typed body is one bounded
Wire value with no extra envelope; raw channels carry caller-supplied bytes.
Derive `mod_api::Wire` explicitly on a type that needs this encoding. The build
never adds the derive. Every field of a derived type must also implement `Wire`.

[Native recipe serializers](../crafting-recipe-kinds.md) do not use Wire for their
values. Both `CraftingRecipeKind` and `RecipeSerializer` decode and write JSON;
the loader retains each typed value with its owner for the resource generation.
Recipe content still syncs as bounded JSON, and raw recipe JSON keeps its size
limit. Recipe placement, book sync and vanilla display packets are unaffected.

`Wire` decodes owned strings (`String`,
`Cow<'static, str>`, `Box<str>`, `Arc<str>`), never `&'static str`; `#[derive(Wire)]` rejects borrowed
fields including `&'static str`, and `ItemPlace` and `MobGoalEntry` stay process-local and do not
implement `Wire`.

Hook events are native callback records, not binary messages. If you need to send data from
an event, copy the fields you intend to send into your own `Wire` payload type. The same applies to
[loader metadata](load-phases.md) such as `PackageInfo`, `DataLoadOrigin` and `LoopSide`: read it locally,
then select the facts you need for your protocol. Keep issued packet-hold and chunk-ticket controller
handles in the process that issued them; use a ticket controller's namespaced `id()` when you need to
record its name.
Likewise, keep [attachment queries and write tokens](attachments/querying.md) in process; an
attachment's own codec handles saving and syncing its value.

Local Part, capability, item-place, resource-identity, and Part-state values and
errors (`BlockSide`, `ItemPlace`, `ItemResource`, `FluidResource`, `PartStateError`,
`BlockEntityPartError`, and their sibling registration and refusal types) also stay in process;
convert to `ItemStack` or `FluidStack`, or copy the fields you need into your own
`#[derive(mod_api::Wire)]` schema when sending across a boundary.

```rust,ignore
#[derive(mod_api::Wire)]
pub struct ChemicalAmount {
    pub chemical: String,
    pub amount: u64,
}
```

Records encode fields in declaration order. Enums encode a case index followed by
its fields. Lengths and numbers are little-endian; collections are length-prefixed.
Decoding returns `WireError` for invalid data rather than accepting an unknown tag,
invalid UTF-8 or an unsupported length.

## Nesting and stacks

Ordinary Wire decoding and encoding allow **32** nested records, payload-carrying
variants and `Box`/`Arc` values (`MAX_DEPTH`). Collection iteration, `Option`
and tuples do not consume a level. The reader returns `WireError::Depth` before
entering level 33; the writer records the same error. Always check `Writer::error`
before sending its bytes. This is a transport budget for unbounded mod-defined
trees, including ingredients, loot vehicles and recipe slot displays.

Typed payloads use the same encoding with their own limits. They count lists, present
options, boxes, shared pointers, NBT containers and explicit recursion, not derived record
frames, against `PAYLOAD_MAX_DEPTH`. String and list lengths are bounded by
`PAYLOAD_MAX_STRING_BYTES` and `PAYLOAD_MAX_LIST_ENTRIES`. The reader and writer both
charge `PAYLOAD_DECODE_BUDGET` for decoded storage, separately from the 8 MiB body cap.
A failed encode is never queued.

Ordinary Wire keeps the NBT `Tag` limit of **512** containers and the `Text` limit of
**512** nested components. Both use stack-independent iterative codecs directly over
their fields and are admitted to typed payloads, where they share `PAYLOAD_MAX_DEPTH`
and `PAYLOAD_DECODE_BUDGET`.

JSON-backed `serde_json::Value`, JSON maps and `ModelPart` also refuse beyond
32 JSON containers. Their encoders preflight the in-memory tree iteratively
before serde recursion; their decoders scan the JSON's nesting outside quoted
strings before parsing. A model part's JSON includes object, children-list and
named-child-tuple containers, so one part can consume several JSON levels.

Hand-written recursive `Wire` implementations must enforce their own stack contract; the
reader cannot police code outside its codec.

Encoding borrows the input (`writer.put(&value)`); the caller still owns it.
`Text` drops iteratively, so dropping a deep component tree does not overflow the
stack. For deeply nested raw NBT `Tag` trees, pop tags from an explicit work stack:
push the children of each `Tag::List` and the values from `Compound::into_values()`
for each `Tag::Compound`, dropping leaf tags as you go.

## Checked representations

Use `#[wire(via = String)]` (or another `Wire` type) when a checked constructor owns
validation. Implement `From<&YourType> for String` and `TryFrom<String> for YourType`.
The derive writes the representation and maps a failed conversion to
`WireError::Invalid`. Its layout is the representation's layout. The conversion can
allocate outside Wire's budget, so this form is available only to ordinary Wire. For a
checked typed payload, implement `Wire` explicitly, bound any storage the codec itself
allocates, and give a different validation contract its own versioned identity.

## Layout identity

A type's layout describes the fields, enum cases and child types that Wire reads and writes.
The derive declares the layout from the Rust fields. Layouts do not include Rust type paths.
The SDK inventories layouts from source declarations. `Wire::describe` fingerprints a compiled
type for typed-payload negotiation. `payload_schema::<T>()` computes that fingerprint; it is not
written into the payload body. A different byte format or validation contract needs a
different negotiated fingerprint, even if the two formats have some bytes in common.

## Hand-written implementations

The derive handles this automatically and is preferred. A hand-written implementation must
supply `put`, `take`, `MIN` and `describe`. For a new encoding, use a versioned `WireSchema::leaf`
and combine it with the fingerprints of its parts using `WireSchema::mix`. Use
`WireSchema::named::<Self>` when describing recursive fields so the walk can recognize a
reference to an ancestor. An implementation that delegates both encoding and validation
can delegate `describe` to that type too.

A hand decoder owns tag validation and bounds for any storage it allocates itself. Decode
indirections through their `Wire` types so they count against the typed-payload limits. Use
`Reader::nested` and `Writer::nested` around explicit recursion in a handwritten codec.
Keep validation at the checked constructor boundary.
