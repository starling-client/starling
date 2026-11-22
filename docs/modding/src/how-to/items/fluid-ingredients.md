# Fluid ingredients

Use `FluidIngredient::from_json(json, context)` to parse fluid inputs for your recipe type. Pass the
`DataLoadContext` supplied to your data-load hook. For direct tag reads,
[`DataLoadContext::fluid_tags()`](../../concepts/load-phases.md#load-contexts) borrows the recipe
pass's frozen table without reading files. `None` means the load has no recipe overlay;
`Some(&[])` means the overlay is available but has no fluid tags. In a populated table, each
tag's members are in source order. Use the borrowed table inside the callback; copy it only if
it must outlive the callback.

| JSON input | Matches |
| --- | --- |
| `"minecraft:water"` | The named fluid. |
| `"#my_mod:coolants"` | Fluids in the load context's tag. |
| A list of ingredients | Any listed ingredient. |
| `neoforge:compound` | Any child. |
| `neoforge:intersection` | Every child. |
| `neoforge:difference` | The base, excluding the subtracted ingredient. |

Call `test(&FluidStack)` to match a tank's contents. Components are not checked. Use `fluids()` to list accepted fluid keys in source order, including tag order, for a recipe book.

Parsing returns `Result<FluidIngredient, FluidIngredientError>`. Invalid shapes, custom kinds and a list holding an entry that isn't a fluid name or `#tag` return `FluidIngredientError::Invalid`; unknown fluid names return `FluidIngredientError::UnknownFluid`. `Hooks::ingredient` binds predicates only for custom **item** ingredients. A custom kind in fluid JSON is refused, not passed to that hook.

## Required amounts

`SizedFluidIngredient::from_json` parses an ingredient and a positive amount in millibuckets:

```json
{"ingredient": "minecraft:water", "amount": 500}
```

- Omit `amount` to require one bucket (1000 millibuckets).
- Invalid input, including zero or a negative amount, returns `None`.
- `test(&FluidStack)` requires both a matching fluid and at least the requested amount.
