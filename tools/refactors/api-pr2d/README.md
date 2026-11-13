# API PR 2d: per-kind registration and capability traits

Starting point: `a790014309e9ea83930bd03d3f4b7257ad3eaef1` (API PR 2c).
After 2c merged, one authorized main merge established comparison base
`4cc97e605d4181372f411f8d00341fc66c5a7ad0`.

The scripted move splits Registrar's declarations and PartRegistrar's implementations
into kind-local traits, alongside the existing kind-local helpers. CapabilityProviders
and InstalledProviders are split the same way. Existing call signatures and method
docs stay unchanged. The generic capability presence query belongs to capabilities.rs.

The Parts folder generator discovers the per-kind traits and generates the two union
supertraits in OUT_DIR. The API owns their documentation templates. The fixture
aggregator continues to discover exported fixtures from each kind's tests module;
wasm Guest implementations follow the same discovered supertraits as native ones.
No dispatch or registration behaviour changes, and no new shared method list is added.
The root export generator exposes only the discovered area traits; it does not make
runtime-only Parts internals public. The hook-fill fixture copies the Parts folder,
so new kinds enter that fixture without edits to its method lists.

Proofs: a moved-lines check with declared wrapper/import changes, host manifest and
WIT gate against the comparison base, focused Parts and generator tests, boundary tests,
and workspace check including tests and examples.

Run `cargo build -p horizon-mod-api-gen --bin part-table-layout`, then `move.py`
on the unsplit base. It copies syn's lossless member ranges into kind-local
traits/implementations and moves union documentation to `parts/unions.rs.in`.
`check_move.py BASE` proves all 60 declaration/implementation members have
identical tokens, then compares the aggregate line multiset with only the
listed wrapper/import/heading exclusions. Documentation link repairs outside
Parts are enumerated in `move.py`; the guide describes the unchanged mod call shape.

The gate compares Registrar import methods, whole type blocks and use-list items as
name-resolved sets with exact membership checks. All type internals, including every
enum's case order, stay byte-ordered. Wasmtime 48.0.7's `typecheck_enum`
(`func/typed.rs:2287–2308`) requires matching case order, even for result errors;
case reordering would change the component link contract.

The actual move puts `declare-entity-attributes` after `provide-entity-capability`
rather than last, relocating the `entity-attribute-error` type block too; no enum
cases move. Hook ordering, hook IDs and dispatch remain byte-identical. The gate
tests refuse changed/duplicated methods, changed/duplicated cases and any enum-case
or hook reordering. No ordering attribute or list is added. New-kind coverage must
compile an added kind and exercise it through the union without changing existing
shared files.
