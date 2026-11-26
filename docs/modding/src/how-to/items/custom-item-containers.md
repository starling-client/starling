# Custom bundle-like item containers

Build a bundle-like item container (such as a rune pouch or quiver) by combining a registered
data component with `BundleConfig`, `BundleItems::with_config`, and `ItemPart` hooks. Everything
runs inside the engine's transactional item and menu paths, reusing the shared exact-fraction
weight, nesting, bar, and rollback rules.

## Vanilla rules vs. Sunrise-specific extensions

The weight model follows vanilla 26.3's `BundleContents` and `BundleItem` rules:

- Exact rational occupancy (`1 / max_stack_size`, non-empty `minecraft:bees` = `1`, nested
  vanilla bundles and nested containers sharing the same component = `contents + 1/16`, evaluated
  iteratively with no artificial nesting depth cutoff).
- Vanilla's `Fraction` arithmetic: weights are commons-lang3 `Fraction`s with `int` parts, summed
  in `BundleContents.computeContentWeight`'s order, and every operation is checked. Where vanilla
  throws `ArithmeticException` (and answers `"Excessive total bundle weight"`), the helpers answer
  `Err(BundleReadError::ExcessiveWeight)`; see below.
- Front-first insertion and extraction order, fullness bar color/width (`0x7087FF` blue while
  partial, `0xFF5454` red at width `13` when full, hidden when empty), and tooltip grid layout.

The following behaviours are **Sunrise-specific extensions** beyond vanilla 26.3 (where
`BundleContents` only has capacity `1`):

- **Capacity multipliers (`capacity_multiplier > 1`)**: scale total weight capacity by a positive
  whole number (`1` = 64 normal items, `4` = 256, `8` = 512, etc.). `0` clamps to `1`.
- **Stack splitting at `max_stack`**: when inserting or merging into a container with
  `capacity_multiplier > 1`, an existing matching partial stack is topped up to the item's
  `max_stack` and any remaining inserted items are split into new front entries each `<= max_stack`.

## Storage schema, overfull, and malformed contents

- **Single storage format**: custom bundle components must store a `Tag::List` of
  `ItemStack.CODEC` compounds, matching vanilla `BundleContents.CODEC`. Each entry is read by the
  engine's one persistent-stack decoder, the one commands and data files use: a non-air `id`, which
  must also be registered in the active item catalog (registry-backed `Item.CODEC`); an optional
  numeric `count` read as Java's `Number.intValue` (so `2.7f` is `2`) and kept in `1..=99`; and an
  optional `components` compound of persistent components. A stack whose consume effects the
  decoder refuses is refused too. Unmentioned extra keys on an entry compound are accepted on
  decode and dropped on re-encode, matching vanilla `ItemStack.MAP_CODEC`. Any non-list tag shape
  (such as a `Tag::Compound` or scalar tag) **or** any element the decoder or the catalog refuses
  (e.g. `mod:removed_item`) inside the list refuses the entire payload with
  `Err(BundleReadError::UnsupportedFormat)` across all five `BundleConfig` helpers (`slots`,
  `room_for`, `try_insert`, `extract`, `metrics`) and is never overwritten by `BundleItems`,
  `CustomBundleContents::store`, or component destruction.
- **Malformed nested containers**: if a stored item (at any nesting depth, including inside a
  nested vanilla bundle) or an offered stack carries the same custom component with an unsupported
  or malformed payload, weight evaluation propagates `Err(BundleReadError::UnsupportedFormat)`
  instead of weighing the malformed container as an ordinary leaf item.
- **Excessive weight**: weights are vanilla's `int` `Fraction`s, so contents whose weight needs a
  numerator or denominator past `i32::MAX` (for example five stacks whose `max_stack_size`s are the
  coprime 97, 89, 83, 79 and 73) are refused with `Err(BundleReadError::ExcessiveWeight)`, as
  vanilla's `ItemStack.validateComponents` refuses such a bundle with "Excessive total bundle
  weight". The same error comes back, without changing anything, when the room left, the weight
  after an insert or the weight after an extract overflows, and from every helper when
  `capacity_multiplier` is above `i32::MAX` (the capacity is the `Fraction`
  `capacity_multiplier / 1`). A refused payload is never overwritten. When the multiplier is large
  enough that `weight / capacity_multiplier` itself overflows, `metrics` reads the container as
  full, as vanilla's `BundleItem.getWeightSafe` reads an erroring weight as `Fraction.ONE`.
- **Unavailable host**: calling any of the five `BundleConfig` helpers on a `Host` without an
  active world item registry returns `Err(BundleReadError::Unavailable)`.
- **Overfull stored contents**: if a saved or command-created stack already exceeds capacity
  (e.g. from lowering `capacity_multiplier` or `/give`), `room_for` returns `Ok(0)`, `try_insert`
  inserts `0`, `metrics.is_full` is `true`, `metrics.fullness > 1.0`, and `metrics.bar` clamps to
  `Some((13, 0xFF5454))`. `extract` still removes items from slot `0`, preserving any remainder at
  the front.
- **One weight per read, nothing cached between reads**: each helper decodes the component and
  computes its weight once, and every answer it gives (fullness, full flag, bar, room) comes from
  that one weight, as vanilla memoizes `BundleContents.weight` on the component value. A second
  call decodes and weighs again, so the cost of `metrics` / `slots` grows with the number of
  stacks inside (and inside nested containers). Draw hooks such as `item_bar`, `tooltip_image`
  and `item_model_range_property` run every frame for every visible stack: ask once per hook call
  and reuse the answer rather than calling a helper per value.

## 1. Register the component and default

Register your component id in `Hooks::component_types` before content loads, and give your item
an empty default list (`[]`) in its item definition so every fresh stack carries the component:

```rust,ignore
#[sunrise_mod]
impl Hooks for MyMod {
    fn component_types(&self, _: &RegistrationContext, event: &mut ComponentTypesEvent<'_>) {
        event.register("mymod:pouch_contents".into());
    }
}
```

Define one shared `BundleConfig` for the container's component id, capacity multiplier (`1` =
64 normal items, `4` = 256, `8` = 512, etc.), and optional item tag filter:

```rust,ignore
fn pouch_config() -> BundleConfig {
    BundleConfig::custom("mymod:pouch_contents", 4).with_filter_tag("mymod:runes")
}
```

## 2. Expose `ITEM_ITEMS` for automation

Return `BundleItems::with_config(place.clone(), pouch_config())` from `r.item_capability`:

```rust,ignore
fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
    let pouch = r.item_key("mymod:rune_pouch");
    r.item_part(pouch, PouchPart);
    r.item_capability(&ITEM_ITEMS, "mymod:rune_pouch", |_, place| {
        Some(Box::new(BundleItems::with_config(place.clone(), pouch_config())))
    });
}
```

Hoppers, capabilities, and nested `ItemPlace` writes now insert at the front, merge matching
stacks up to each item's `max_stack`, respect `filter_tag` and `can_fit_inside_container_items`,
account for nested vanilla bundles and nested containers sharing the same component, and roll
back atomically if the caller's transaction aborts.

## 3. Handle menu stacking and dropped-item destruction

Implement `ItemPart::item_stacked_on_other` for inventory/container clicks (running in vanilla
`AbstractContainerMenu.tryItemClickBehaviourOverride` order inside the click transaction) and
`ItemPart::item_destroyed` to spill contents when the dropped item entity is destroyed:

```rust,ignore
#[sunrise_part]
impl ItemPart for PouchPart {
    fn item_stacked_on_other(
        &self,
        host: HostSide<'_>,
        event: &mut ItemStackedOnOtherEvent<'_>,
    ) -> Flow {
        let config = pouch_config();
        let host_ref: &dyn Host = match &host {
            HostSide::Server(h) => &**h,
            HostSide::Client(h) => &**h,
        };
        match event.event.direction {
            StackedOnOtherDirection::CarriedOnOther => {
                let Some(carried) = event.event.carried.as_ref() else {
                    return Flow::Continue;
                };
                if event.event.primary {
                    let Some(clicked) = event.event.clicked.as_ref() else {
                        return Flow::Continue;
                    };
                    let Ok(Some((moved, updated))) = config.try_insert(host_ref, carried, clicked)
                    else {
                        return Flow::Continue;
                    };
                    if moved > 0 {
                        event.result.carried = StackResult::Set(Some(updated));
                        event.result.clicked =
                            ClickedStackResult::Set(clicked.clone().shrink(moved));
                        event.result.handled = true;
                        return Flow::Cancel;
                    }
                } else if event.event.clicked.is_none()
                    && let Ok(Some((Some(out), updated))) =
                        config.extract(host_ref, carried, u32::MAX)
                {
                    event.result.carried = StackResult::Set(Some(updated));
                    event.result.clicked = ClickedStackResult::Set(Some(out));
                    event.result.handled = true;
                    return Flow::Cancel;
                }
            }
            StackedOnOtherDirection::OtherStackedOnMe => {
                let Some(clicked) = event.event.clicked.as_ref() else {
                    return Flow::Continue;
                };
                if event.event.primary
                    && let Some(carried) = event.event.carried.as_ref()
                    && let Ok(Some((moved, updated))) =
                        config.try_insert(host_ref, clicked, carried)
                    && moved > 0
                {
                    event.result.clicked = ClickedStackResult::Set(Some(updated));
                    event.result.carried = StackResult::Set(carried.clone().shrink(moved));
                    event.result.handled = true;
                    return Flow::Cancel;
                }
            }
            StackedOnOtherDirection::Event => {}
        }
        Flow::Continue
    }

    fn item_destroyed(&self, host: &mut dyn ServerHost, event: &mut ItemDestroyedEvent<'_>) {
        if let Ok(Some(inside)) = pouch_config().slots(host, event.stack) {
            event.drops.extend(inside);
        }
    }
}
```

## 4. Add fullness bar, custom tooltip, right-click use, and model properties

Reuse the existing Item Part and model hooks over the same `BundleConfig`:

- **Fullness bar (`ItemPart::item_bar`)**: read `pouch_config().metrics(host, event.stack)` and
  set `*event.bar = metrics.bar` (blue until full, red at width 13 when full, hidden when empty).
- **Tooltip image (`ItemPart::tooltip_image` and a client tooltip component)**: in
  `tooltip_image`, read `config.slots(host, event.stack)` and `config.metrics(host, event.stack)`
  and return `Some(TooltipComponent::custom(data))`, where `data` is your own
  `TooltipComponentData` type carrying the stacks to show, `metrics.fullness` and
  `metrics.is_full` (vanilla's `BundleItem.getTooltipImage` hands a `BundleTooltip` its contents
  the same way). Register a `ClientTooltipComponent` for that type in
  `Hooks::register_client_tooltip_component_factories`; it measures itself (`width`, `height`)
  and draws the stored stacks and the progress bar in `draw_image` from the data alone. See
  [Custom tooltip components](../client/client-effects.md#custom-tooltip-components-and-render-hooks).
- **Right-click use (`ItemPart::use_item`)**: extract from the held pouch with `config.extract`
  and rewrite the held stack through `ItemPlace::PlayerHand` inside a transaction.
- **Nesting refusal (`ItemPart::can_fit_inside_container_items`)**: set `*event.fits = false` if
  your pouch should not fit inside shulker boxes or other bundles.
- **Filled model variant (`register_range_select_item_model_property` / `item_model_range_property`)**:
  answer `metrics.fullness` so a `minecraft:range_dispatch` model switches between empty, partial,
  and full textures.
