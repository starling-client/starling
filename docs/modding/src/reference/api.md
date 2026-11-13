# The mod API (rustdoc)

The reference for every hook, event, handle and `Host` method is the rustdoc of the `horizon-mod-api` crate
(lib name `mod_api`), generated from the `///` docs of the declarations in `hooks/*.rs`:

```sh
cargo doc -p horizon-mod-api --no-deps --open
```

Each generated `Hooks` method, event and `Host` method carries its `doc`, with the side and cancel contract the
catalog records. CI builds it with `-D warnings`, so a broken intra-doc link fails. The per-hook prose that predates
the `doc` field is in [Hook notes](hook-notes.md) until it moves into the fragments.

## Plain-data source declarations

The bindings generator reads the Rust type declaration; codecs are not maintained separately for native and
wasm mods. A type with a separate public form declares `#[horizon_mod_api_macros::wasm(via = Form)]` on the
type or its re-export, with `TryFrom<&Type> for Form` and `TryFrom<Form> for Type` beside the type.

An owned tuple struct whose fields need load-owned validation declares
`#[horizon_mod_api_macros::wasm(host_constructor = from_slot)]`. The named associated constructor takes the
fields in declaration order and returns `Result<Self, E>`, where `E` implements `Display`. The macro checks
that signature, and generated incoming host lifts use the constructor and the ordinary typed refusal path.
This applies recursively inside options, lists, records and enum payloads, including hook mutation results.
Lowering is still structural numeric transport. Guest lifts of host-owned values remain structural and do not
need a guest registry table.

`BlockKey` and `EffectKey` use this rule to reject unknown incoming slots against their canonical load-owned
name tables. While `Hooks::register` is in flight, validation reads the active registrar's pending requests;
that same table moves into the installed load state afterward, without a second registry. A valid slot does not
imply that a world's resolved mapping is available: startup, another owning
thread or a refused block reload may have no mapping. Typed operations use trusted numeric indices once that
context exists; `key.id(host) -> String` remains an explicit diagnostic read through the normal generated backend,
not a per-operation name lookup. The effect registry remains fixed for the process.
