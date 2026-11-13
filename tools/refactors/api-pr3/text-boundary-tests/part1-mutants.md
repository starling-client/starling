# X2 production mutation checks

All mutations below were applied to production code, tested locally, and restored.
The unchanged focused tests passed before mutation. An abort is isolated in the
test process; recursive trees are not dropped on the small stack by the test itself.

| Production mutation | Focused test | Observed failure |
| --- | --- | --- |
| Skip API `TextForm::try_from` component-depth preflight | `too_deep_is_refused_before_cloning_opaque_metadata` | Stack overflow while cloning 4,096-container opaque metadata; exit 101 |
| Skip engine depth preflight before `component_view`/capture | Same test | Stack overflow before typed refusal; exit 101 |
| Replace generated native Text argument `?` with `unwrap()` | `generated_text_crossings_refuse_too_deep_before_boss_bar_writes` | Generated host module panics on `Err(TooDeep)`; exit 101 |
| Replace unused-style iterative disposal with ordinary `drop(style)` | `recursive_fields_survive_512_nodes_on_256_kib` | Empty-parent entity-hover chain overflows during boundary construction; exit 101 |
| Replace owned component disposal with ordinary `drop(component)` | `span_only_hover_sources_are_validated_and_materialized_iteratively` | Materialized hover JSON overflows on destruction; exit 101 |
| Replace inline-object fallback temporary disposal with ordinary `drop(text)` | `mixed_recursive_temporaries_are_released_on_256_kib` | Mixed fallback/entity-hover chain overflows inside engine construction; exit 101 |
| Use the caller's resolver for hover text | `iterative_builder_isolates_hover_resolution_and_empty_array_style` | Panic in the deliberately forbidden hover resolver; exit 101 |
| Evaluate content dependencies even when `contents` overrides the parent | `iterative_builder_preserves_lazy_content_and_callback_order` | Unexpected child callback fails the assertion; exit 101 |

The first two tests run through the actual game-local conversion trait implementation,
compiled into this isolated probe crate. The native test calls the generated host
forwarder and handle setter against a real game fixture, and also verifies that the
bar repository and existing name remain unchanged after refusal.

Reproduction commands:

```sh
cargo test --manifest-path tools/refactors/api-pr3/text-boundary-tests/Cargo.toml --lib
cargo test -p horizon-client-core component::tests --lib
HORIZON_TEST_RESOURCES=$PWD/.local/horizon-client/26.3 cargo test -p horizon-client --lib generated_text_crossings_refuse_too_deep_before_boss_bar_writes -- --ignored
```

Resource-backed tests additionally use fresh scratch home/configuration directories
as required by the repository's game-run rules. Wasm adapter tests and their mutations
are maintained alongside the generator's `wasm/text_refusal_tests.rs`.
