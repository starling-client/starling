# Item-resource metadata and damage snapshots

Part of API boundary tracking issue #1608. The game keeps its property readers and damage algorithm.
The API owns the data forms and forwards property reads through Metadata access on ItemsBackend.

## Focused scalar regression

From the repository root:

```sh
rustc --edition 2024 --test tools/refactors/api-pr3/resource-damage/probe.rs -o "$CARGO_TARGET_DIR/resource-damage-probe"
"$CARGO_TARGET_DIR/resource-damage-probe"
```

The probe imports the actual API health types, engine damage sequence and game conversion module.
Only the two conversion trait declarations are supplied locally; no conversion or damage algorithm
is copied. It checks independent armor and innate-resistance amounts, blocking, shield damage,
side effects, cooldown writes and restoration of the hidden pre-hook input when damage becomes NaN.

A production mutant replacing the armor snapshot read with `0.0` was compiled and run: the probe
failed with `left: 0.0, right: 4.0`. Restoring the source made the same probe pass.
This is a conversion proof, not a real-game or dispatch benchmark.

## Integration regressions

`native_damage_tests::snapshot_crosses_the_real_damage_hook_with_reductions` fires the real
subscribed Pre hook with an armor reduction and checks both the listener's observed reduction
and the applied damage. `unsubscribed_damage_does_not_convert` checks the production conversion
counter at the unsubscribed Pre boundary. Both require prepared test resources.

The existing `item_resource_property_tests::effective_resource_properties_match_vanilla` corpus
compares defaults, overrides and removals against recorded outcomes and the gameplay readers.
`item_resource_changed_defaults_and_consumption` additionally covers changed defaults and repair tags.

## Generated contract comparison

Generated with the unchanged generator at `origin/api-lead-api3` and this item's source head.
The WIT diff contains only three new backend imports:

- `item-resource-damageable(stack: item-stack) -> bool`
- `item-resource-foil(stack: item-stack) -> bool`
- `item-resource-repairable(stack: item-stack, material: item-key) -> bool`

The damage snapshot, reduction cases, item rarity cases and bar record retain their wire shape.
The host union gains the corresponding three backend signatures; no existing signature is removed.
The private reduction array is crate-visible to the generated codec, not public to mods.

## Performance scope

All three combat bridges guard before copying the scalar snapshot. Conversion uses fixed arrays
and scalar assignments, no heap storage. Explicit item metadata queries still pay the existing
stack boundary conversion; repair checks construct a plain engine material stack by cloning the
registry's retained item-name Arc, not allocating another name. No measured whole-hook timing
or allocation claim is made here.
