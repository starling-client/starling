//! `BundleContents.Mutable`: what a bundle takes in and gives back, by
//! weight. The menus (`menu`, `lan::menu`) run `BundleItem`'s click
//! overrides on it.
//!
//! Weights are vanilla's commons-lang3 `Fraction`
//! ([`sunrise_world::bundle_weight::Fraction`]): `int` parts, every operation checked, and an
//! overflow is [`ExcessiveWeight`], the `ArithmeticException` that
//! `BundleContents.computeContentWeight` turns into
//! `DataResult.error("Excessive total bundle weight")`.
use crate::item_data::{BundleSelected, StackData};
use crate::items::{ItemStack, Items};
use crate::properties::ItemBar;
use std::collections::HashSet;
use std::sync::OnceLock;
use sunrise_core::nbt::Tag;
use sunrise_world::bundle_weight::{ExcessiveWeight, Fraction};

/// `DataComponents.BUNDLE_CONTENTS`.
const VANILLA_COMPONENT: &str = "minecraft:bundle_contents";

/// `BundleContents.BUNDLE_IN_BUNDLE_WEIGHT`: `Fraction.getFraction(1, 16)`, what a nested
/// bundle weighs on top of its contents.
fn bundle_in_bundle_weight() -> Result<Fraction, ExcessiveWeight> {
    Fraction::new(1, 16)
}

/// `Mutable.getStackedWeight(Fraction, int)`: `weight.multiplyBy(Fraction.getFraction(count, 1))`.
/// A count past `i32::MAX` has no `int`, so it overflows like the product would.
fn stacked(weight: Fraction, count: u32) -> Result<Fraction, ExcessiveWeight> {
    let count = i32::try_from(count).map_err(|_| ExcessiveWeight)?;
    weight.checked_multiply(Fraction::new(count, 1)?)
}

/// `Mutable.getMaxAmountToAdd`: `capacity.subtract(weight).divideBy(unit).intValue()`, at least
/// 0. Vanilla's capacity is `Fraction.ONE`.
fn room(capacity: Fraction, weight: Fraction, unit: Fraction) -> Result<u32, ExcessiveWeight> {
    let left = capacity.checked_subtract(weight)?.checked_divide_by(unit)?;
    Ok(left.int_value().max(0) as u32)
}

/// `Mth.mulAndTruncate(Fraction, int)`: `getNumerator() * factor / getDenominator()` in `int`
/// arithmetic. The product is Java's `imul`, which wraps rather than throws, so this wraps too
/// (`wrapping_mul`) instead of panicking or saturating; the denominator is positive.
fn mul_and_truncate(fraction: Fraction, factor: i32) -> i32 {
    fraction.numerator().wrapping_mul(factor) / fraction.denominator()
}

/// `BundleItem.isBarVisible`, `getBarWidth` and `getBarColor` for a shown weight fraction:
/// visible when above `Fraction.ZERO`; `min(1 + Mth.mulAndTruncate(weight, 12), 13)` wide,
/// blue until full (`>= 1`), then red.
fn bar_from_weight(shown: Fraction) -> Option<(i32, u32)> {
    if shown.compare_to(Fraction::ZERO).is_le() {
        return None;
    }
    let width = 1i32.wrapping_add(mul_and_truncate(shown, 12)).min(13);
    let color = if shown.compare_to(Fraction::ONE).is_ge() {
        0xFF5454
    } else {
        0x7087FF
    };
    Some((width, color))
}

/// The sounds of `BundleItem` (`Entity.playSound` at the player): insert
/// and remove one at volume 0.8 and pitch `0.8 + random * 0.4`, a failed
/// insert at 1, 1; the drop of its contents is played at the player's block
/// like remove one.
pub const INSERT: &str = "minecraft:item.bundle.insert";
pub const INSERT_FAIL: &str = "minecraft:item.bundle.insert_fail";
pub const REMOVE_ONE: &str = "minecraft:item.bundle.remove_one";
pub const DROP_CONTENTS: &str = "minecraft:item.bundle.drop_contents";

pub(crate) fn max_stack(items: &Items, stack: &ItemStack) -> u32 {
    let info = items.get(&stack.item);
    if !stack.carries_component(info, "minecraft:max_stack_size") {
        return 1;
    }
    stack
        .data()
        .and_then(|data| data.other("minecraft:max_stack_size"))
        .and_then(crate::properties::component_int)
        .unwrap_or_else(|| info.map_or(1, |item| item.max_stack as i32))
        .max(1) as u32
}

/// The exact weight of a contents list, resolved once (`BundleContents` computes `weight` in its
/// constructor and keeps it in the component). A [`StackData`] holds it for its own list and drops
/// it when it is handed out for editing (`ItemStack::data_mut`) or cloned, so the bar and
/// the menus read a weight that matches the list and cycles in the component graph are found
/// once, where the list is resolved.
#[derive(Default)]
pub(crate) struct WeightCache(OnceLock<Result<Fraction, ExcessiveWeight>>);

impl Clone for WeightCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for WeightCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WeightCache")
    }
}

/// The component data that holds the stack's effective `minecraft:bundle_contents`: its own
/// patch (including an explicitly empty list), `None` when removed with
/// `!minecraft:bundle_contents`, otherwise the item prototype's default.
fn effective<'a>(items: &'a Items, stack: &'a ItemStack) -> Option<&'a StackData> {
    if let Some(data) = stack.data() {
        if data.sets_component(VANILLA_COMPONENT) {
            return Some(data);
        }
        if data.other("!minecraft:bundle_contents").is_some() {
            return None;
        }
    }
    let prototype = items.get(&stack.item)?.component_prototype.data()?;
    prototype
        .sets_component(VANILLA_COMPONENT)
        .then_some(prototype)
}

/// The stack's effective `minecraft:bundle_contents` (see [`effective`]).
pub fn contents_of<'a>(items: &'a Items, stack: &'a ItemStack) -> Option<&'a [ItemStack]> {
    effective(items, stack).map(|data| &data.bundle_contents[..])
}

/// The resolved weight of `data`'s vanilla contents: `Err(ExcessiveWeight)` for a cycle or an
/// overflow.
fn resolved_weight(items: &Items, data: &StackData) -> Result<Fraction, ExcessiveWeight> {
    *data.bundle_weight.0.get_or_init(|| {
        contents_weight(items, &data.bundle_contents, None).map_err(|_| ExcessiveWeight)
    })
}

/// Identifies a bundle-like item component and its weight capacity multiplier.
///
/// Vanilla bundles use [`BundleSpec::VANILLA`] (`"minecraft:bundle_contents"`, multiplier `1`).
/// Custom containers name a registered data component and a positive whole-number multiplier
/// relative to a vanilla bundle (`1` = 64 normal items, `4` = 256, etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BundleSpec<'a> {
    /// Namespaced data component id storing the container's items.
    pub component: &'a str,
    /// Whole-number capacity multiplier relative to a vanilla bundle (clamped >= 1). The capacity
    /// is the `Fraction` `capacity_multiplier / 1`, so a multiplier above `i32::MAX` has no
    /// capacity and every read refuses it with [`CustomBundleError::Weight`].
    pub capacity_multiplier: u32,
}

impl<'a> BundleSpec<'a> {
    /// Vanilla `minecraft:bundle_contents` with multiplier `1`.
    pub const VANILLA: Self = Self {
        component: VANILLA_COMPONENT,
        capacity_multiplier: 1,
    };

    /// A bundle-like component with capacity `capacity_multiplier.max(1)`.
    pub const fn custom(component: &'a str, capacity_multiplier: u32) -> Self {
        Self {
            component,
            capacity_multiplier: if capacity_multiplier == 0 {
                1
            } else {
                capacity_multiplier
            },
        }
    }

    /// Whether this spec targets vanilla's `minecraft:bundle_contents` storage.
    pub fn is_vanilla(&self) -> bool {
        self.component == VANILLA_COMPONENT
    }

    fn multiplier(&self) -> u32 {
        self.capacity_multiplier.max(1)
    }

    /// The capacity as a `Fraction`: `Fraction.ONE` for a vanilla bundle,
    /// `getFraction(multiplier, 1)` for a custom container.
    fn capacity(&self) -> Result<Fraction, ExcessiveWeight> {
        let multiplier = i32::try_from(self.multiplier()).map_err(|_| ExcessiveWeight)?;
        Fraction::new(multiplier, 1)
    }

    /// The component a nested container must carry to count as one inside this container:
    /// vanilla bundles always do (`BundleContents.getWeight`), and a custom container also
    /// counts others carrying its own component.
    fn nested_component(&self) -> Option<&'a str> {
        (!self.is_vanilla()).then_some(self.component)
    }
}

/// Why a bundle-like component on an item stack could not be read or changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomBundleError {
    /// The stack (or a nested container carrying the same custom component) does not conform to
    /// `Tag::List` of `ItemStack.CODEC` entries, as
    /// [`crate::item_nbt::item_from_persistent_nbt`] reads them, naming registered items.
    UnsupportedFormat,
    /// `DataResult.error("Excessive total bundle weight")`: a weight, or the capacity, overflowed
    /// vanilla's `int` `Fraction`. The same error as [`sunrise_world::bundle_weight`]'s vanilla
    /// contents validation (`ContentsError::Weight`).
    Weight(ExcessiveWeight),
}

impl From<ExcessiveWeight> for CustomBundleError {
    fn from(error: ExcessiveWeight) -> Self {
        Self::Weight(error)
    }
}

impl std::fmt::Display for CustomBundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormat => {
                f.write_str("custom bundle component value is not a list of valid item stacks")
            }
            Self::Weight(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for CustomBundleError {}

/// Decode a custom bundle-like component tag as `BundleContents.CODEC` reads one: a `Tag::List`
/// of `ItemStackTemplate.CODEC` entries.
///
/// Every entry goes through the one persistent-stack decoder,
/// [`crate::item_nbt::item_from_persistent_nbt`] (a numeric `count` read through
/// `Number.intValue` and kept in 1..=99, persistent components only, no refused consume
/// effects), plus the item registry check of `Item.CODEC`, which that decoder leaves to its
/// callers. Extra entry keys are ignored there and dropped on re-encode, as
/// `ItemStackTemplate.MAP_CODEC` does. Any non-list tag or any entry the decoder or the registry
/// refuses makes the whole payload [`CustomBundleError::UnsupportedFormat`] rather than silently
/// dropping entries on rewrite.
fn try_decode_custom_tag(items: &Items, tag: &Tag) -> Result<Vec<ItemStack>, CustomBundleError> {
    let Tag::List(entries) = tag else {
        return Err(CustomBundleError::UnsupportedFormat);
    };
    entries
        .iter()
        .map(|entry| {
            crate::item_nbt::item_from_persistent_nbt_with(
                entry,
                |id| !id.starts_with("minecraft:"),
                |id| items.get(id).is_some(),
            )
            .map_err(|_| CustomBundleError::UnsupportedFormat)
        })
        .collect()
}

/// The stack's decoded custom component contents when it carries `component`.
///
/// Returns `Ok(None)` when `stack` does not carry `component`, `Ok(Some(items))` when the
/// component decodes (see [`try_decode_custom_tag`]), and
/// `Err(CustomBundleError::UnsupportedFormat)` otherwise.
fn try_custom_contents_of(
    items: &Items,
    stack: &ItemStack,
    component: &str,
) -> Result<Option<Vec<ItemStack>>, CustomBundleError> {
    let info = items.get(&stack.item);
    if !stack.carries_component(info, component) {
        return Ok(None);
    }
    let tag = stack
        .data()
        .and_then(|d| d.other(component))
        .or_else(|| info.and_then(|i| i.custom_components.get(component)));
    match tag {
        Some(tag) => try_decode_custom_tag(items, tag).map(Some),
        None => Ok(Some(Vec::new())),
    }
}

/// The stacks a container stack holds: its `minecraft:bundle_contents` (vanilla's
/// `BundleContents.getWeight` looks for that first), borrowed, or the decoded contents of the
/// custom component being weighed.
enum Held<'a> {
    Borrowed(&'a [ItemStack]),
    Owned(Vec<ItemStack>),
}

impl Held<'_> {
    fn as_slice(&self) -> &[ItemStack] {
        match self {
            Self::Borrowed(stacks) => stacks,
            Self::Owned(stacks) => stacks,
        }
    }
}

/// What `stack` holds when it is a container counted by the container being weighed: a vanilla
/// bundle, or a stack carrying `nested` (the custom component being weighed). A malformed
/// nested custom payload is [`CustomBundleError::UnsupportedFormat`].
fn held<'a>(
    items: &'a Items,
    stack: &'a ItemStack,
    nested: Option<&str>,
) -> Result<Option<Held<'a>>, CustomBundleError> {
    if let Some(inside) = contents_of(items, stack) {
        return Ok(Some(Held::Borrowed(inside)));
    }
    match nested {
        Some(component) if component != VANILLA_COMPONENT => {
            Ok(try_custom_contents_of(items, stack, component)?.map(Held::Owned))
        }
        _ => Ok(None),
    }
}

/// `BundleContents.getWeight` for a stack holding nothing: `BEEHIVE_WEIGHT` (`Fraction.ONE`)
/// with bees, else `Fraction.getFraction(1, getMaxStackSize())`.
fn leaf_weight(items: &Items, stack: &ItemStack) -> Result<Fraction, ExcessiveWeight> {
    let bees = stack
        .data()
        .and_then(|d| d.other("minecraft:bees"))
        .is_some_and(|b| matches!(b, Tag::List(l) if !l.is_empty()));
    if bees {
        return Ok(Fraction::ONE);
    }
    let max = i32::try_from(max_stack(items, stack)).map_err(|_| ExcessiveWeight)?;
    Fraction::new(1, max)
}

/// One container being weighed: its stacks, the next to weigh, its total so far, how many
/// of it the container around it holds, and the prototype/slice key on the active path (for
/// cycle detection across item prototypes).
struct Frame<'a> {
    stacks: Held<'a>,
    next: usize,
    total: Fraction,
    count: u32,
    active_key: Option<*const ItemStack>,
}

/// `BundleContents.computeContentWeight`, in vanilla's order: from `Fraction.ZERO`, each stack's
/// `getWeight` is multiplied by its count (`getFraction(count, 1)`) and added to the total, and
/// a nested container's `getWeight` is its own contents' weight plus `BUNDLE_IN_BUNDLE_WEIGHT`.
/// Each overflow surfaces where commons-lang3 throws, as [`CustomBundleError::Weight`].
///
/// An explicit stack of [`Frame`]s replaces vanilla's recursion so nesting depth cannot exhaust
/// the call stack, and tracks active borrowed slices so a cycle across item prototypes is
/// refused as [`CustomBundleError::Weight`] without masking sibling stacks that inherit the same
/// prototype. It keeps vanilla's grouping (a nested container is weighed whole, then multiplied
/// by its count), so the overflow points are vanilla's. `nested` is
/// [`BundleSpec::nested_component`].
fn contents_weight<'a>(
    items: &'a Items,
    inside: &'a [ItemStack],
    nested: Option<&str>,
) -> Result<Fraction, CustomBundleError> {
    // The containers around `current`, outermost first.
    let root_key = (!inside.is_empty()).then_some(inside.as_ptr());
    let mut active: Option<HashSet<*const ItemStack>> = None;
    let mut around: Vec<Frame<'a>> = Vec::new();
    let mut current = Frame {
        stacks: Held::Borrowed(inside),
        next: 0,
        total: Fraction::ZERO,
        count: 1,
        active_key: None,
    };
    loop {
        let index = current.next;
        if index == current.stacks.as_slice().len() {
            if let Some(key) = current.active_key
                && let Some(set) = active.as_mut()
            {
                set.remove(&key);
            }
            let Some(mut parent) = around.pop() else {
                return Ok(current.total);
            };
            let unit = current.total.checked_add(bundle_in_bundle_weight()?)?;
            parent.total = parent.total.checked_add(stacked(unit, current.count)?)?;
            parent.next += 1;
            current = parent;
            continue;
        }
        // A borrowed level hands its nested vanilla contents on as borrowed; stacks decoded
        // from a custom component are owned by their frame, so what they hold is copied.
        let (count, child) = match &current.stacks {
            Held::Borrowed(stacks) => {
                let stacks: &'a [ItemStack] = stacks;
                let stack = &stacks[index];
                let child = held(items, stack, nested)?.map(|h| {
                    let key = match &h {
                        Held::Borrowed(slice) if !slice.is_empty() => Some(slice.as_ptr()),
                        _ => None,
                    };
                    (h, key)
                });
                (stack.count, child)
            }
            Held::Owned(stacks) => {
                let stack = &stacks[index];
                let child = held(items, stack, nested)?.map(|held| match held {
                    Held::Borrowed(inside) => {
                        let key = (!inside.is_empty()).then_some(inside.as_ptr());
                        (Held::Owned(inside.to_vec()), key)
                    }
                    Held::Owned(inside) => (Held::Owned(inside), None),
                });
                (stack.count, child)
            }
        };
        match child {
            Some((stacks, active_key)) => {
                if let Some(key) = active_key
                    && (root_key == Some(key)
                        || !active.get_or_insert_with(HashSet::new).insert(key))
                {
                    return Err(CustomBundleError::Weight(ExcessiveWeight));
                }
                let child = Frame {
                    stacks,
                    next: 0,
                    total: Fraction::ZERO,
                    count,
                    active_key,
                };
                around.push(std::mem::replace(&mut current, child));
            }
            None => {
                let unit = leaf_weight(items, &current.stacks.as_slice()[index])?;
                current.total = current.total.checked_add(stacked(unit, count)?)?;
                current.next += 1;
            }
        }
    }
}

/// `BundleContents.getWeight(stack)`: a container weighs its contents plus
/// `BUNDLE_IN_BUNDLE_WEIGHT`, anything else [`leaf_weight`].
fn item_weight(
    items: &Items,
    stack: &ItemStack,
    nested: Option<&str>,
) -> Result<Fraction, CustomBundleError> {
    match held(items, stack, nested)? {
        Some(inside) => Ok(contents_weight(items, inside.as_slice(), nested)?
            .checked_add(bundle_in_bundle_weight()?)?),
        None => Ok(leaf_weight(items, stack)?),
    }
}

/// `BundleItem.isBarVisible`, `getBarWidth` and `getBarColor`, over the
/// effective component. Missing or empty contents show no bar.
pub(crate) fn stack_bar(items: &Items, stack: &ItemStack) -> Option<ItemBar> {
    // BundleItem.getWeightSafe draws a full bar when the weight is invalid.
    let weight = resolved_weight(items, effective(items, stack)?).unwrap_or(Fraction::ONE);
    let (width, color) = bar_from_weight(weight)?;
    Some(ItemBar { width, color })
}

/// `BundleItem.onDestroyed` (`ItemStack.onDestroyed` for a bundle): the stack's effective contents
/// (its own, or its item's default) come back as their own item copies, which
/// `ItemUtils.onContainerDestroyed` drops where the entity lay, and the stack is left with an
/// effective empty component and no selected index.
pub fn destroyed(items: &Items, stack: &mut ItemStack) -> Vec<ItemStack> {
    let spilled = match contents_of(items, stack) {
        Some(inside) if !inside.is_empty() => inside.to_vec(),
        _ => return Vec::new(),
    };
    // `set(BUNDLE_CONTENTS, BundleContents.EMPTY)`.
    write_contents(items, stack, Vec::new(), -1);
    spilled
}

/// Writes `inside` and the selected index as the bundle's component, keeping an explicitly empty
/// list when the item's prototype would otherwise show its default contents again.
fn write_contents(items: &Items, bundle: &mut ItemStack, inside: Vec<ItemStack>, selected: i32) {
    let explicit_empty = inside.is_empty()
        && !items
            .get(&bundle.item)
            .and_then(|info| info.component_prototype.data())
            .is_some_and(|proto| {
                proto.sets_component(VANILLA_COMPONENT) && proto.bundle_contents.is_empty()
            });
    let data = bundle.data_mut();
    data.set_other("!minecraft:bundle_contents", None);
    data.set_other(
        VANILLA_COMPONENT,
        explicit_empty.then(|| Tag::List(Vec::new())),
    );
    data.bundle_contents = inside;
    data.bundle_selected = BundleSelected(selected);
    bundle.tidy();
}

/// A command's or an automation's edit of the bundle component: `inside` replaces the effective
/// contents, with nothing selected, and an empty list stays empty instead of showing the item's
/// default contents.
pub fn set_contents(items: &Items, bundle: &mut ItemStack, inside: Vec<ItemStack>) {
    write_contents(items, bundle, inside, -1);
}

/// Generalized `BundleItem.onDestroyed` for any bundle-like component id: empties `component`
/// on `stack` and returns the stored stacks for dropping. Delegates `"minecraft:bundle_contents"`
/// to [`destroyed`]. Leaves stacks without `component`, or whose payload cannot be read
/// ([`CustomBundleContents::try_of`] fails), untouched and returns an empty `Vec`.
pub fn destroyed_component(
    items: &Items,
    stack: &mut ItemStack,
    component: &str,
) -> Vec<ItemStack> {
    if component == VANILLA_COMPONENT {
        return destroyed(items, stack);
    }
    let Ok(Some(mut contents)) =
        CustomBundleContents::try_of(items, stack, BundleSpec::custom(component, 1))
    else {
        return Vec::new();
    };
    if contents.items.is_empty() {
        return Vec::new();
    }
    let spilled = std::mem::take(&mut contents.items);
    contents.weight = Fraction::ZERO;
    contents.selected = -1;
    contents.store(items, stack);
    spilled
}

/// Generalized `BundleContents.Mutable` parameterized by component id and capacity multiplier.
///
/// Vanilla [`Contents`] is a thin wrapper over `CustomBundleContents` with
/// [`BundleSpec::VANILLA`], sharing the exact fraction weight, front-insert, merge, removal,
/// bar, and tooltip geometry logic. Capacity multipliers above `1` and splitting merged/inserted
/// stacks at each item's `max_stack` are Sunrise-specific extensions beyond vanilla 26.3's
/// capacity-`1` `BundleContents`.
///
/// The weight is computed once, when the contents are read, and kept exact by every change, as
/// vanilla memoizes `BundleContents.weight` in its constructor (`Suppliers.memoize`) and
/// `Mutable` carries its `weight` field.
pub struct CustomBundleContents {
    component: String,
    capacity_multiplier: u32,
    /// [`BundleSpec::capacity`]: `Fraction.ONE` for a vanilla bundle.
    capacity: Fraction,
    items: Vec<ItemStack>,
    weight: Fraction,
    /// `selectedItem`: an index into `items`, -1 for none.
    selected: i32,
}

impl CustomBundleContents {
    /// Contents of `spec` holding `items` that weigh `weight`.
    fn with_weight(
        spec: BundleSpec<'_>,
        items: Vec<ItemStack>,
        weight: Fraction,
        selected: i32,
    ) -> Result<Self, CustomBundleError> {
        Ok(Self {
            component: spec.component.to_owned(),
            capacity_multiplier: spec.multiplier(),
            capacity: spec.capacity()?,
            items,
            weight,
            selected,
        })
    }

    fn nested_component(&self) -> Option<&str> {
        (self.component != VANILLA_COMPONENT).then_some(self.component.as_str())
    }

    /// Build contents from an explicit item list under `spec`, validating any nested containers
    /// sharing `spec.component`. [`CustomBundleError::Weight`] when their weight, or the
    /// capacity, overflows vanilla's `Fraction`.
    pub fn try_from_items(
        items: &Items,
        spec: BundleSpec<'_>,
        inside: Vec<ItemStack>,
    ) -> Result<Self, CustomBundleError> {
        let weight = contents_weight(items, &inside, spec.nested_component())?;
        Self::with_weight(spec, inside, weight, -1)
    }

    /// Read the container's contents from `stack` under `spec`.
    ///
    /// Returns `Ok(None)` when `stack` does not carry `spec.component`,
    /// `Err(CustomBundleError::UnsupportedFormat)` when a custom component (on `stack` or any
    /// nested container carrying `spec.component`) does not decode, and
    /// `Err(CustomBundleError::Weight)` when the contents' weight or the capacity overflows
    /// vanilla's `Fraction` (`ItemStack.validateComponents` refuses such a vanilla bundle).
    pub fn try_of(
        items: &Items,
        stack: &ItemStack,
        spec: BundleSpec<'_>,
    ) -> Result<Option<Self>, CustomBundleError> {
        if spec.is_vanilla() {
            let Some(data) = effective(items, stack) else {
                return Ok(None);
            };
            let weight = resolved_weight(items, data)?;
            let selected = stack.data().map_or(-1, |d| d.bundle_selected.0);
            return Self::with_weight(spec, data.bundle_contents.clone(), weight, selected)
                .map(Some);
        }
        let Some(inside) = try_custom_contents_of(items, stack, spec.component)? else {
            return Ok(None);
        };
        let weight = contents_weight(items, &inside, spec.nested_component())?;
        Self::with_weight(spec, inside, weight, -1).map(Some)
    }

    /// Read the container's contents from `stack` under `spec`; `None` when `stack` does not
    /// carry `spec.component` or [`Self::try_of`] refuses it.
    pub fn of(items: &Items, stack: &ItemStack, spec: BundleSpec<'_>) -> Option<Self> {
        Self::try_of(items, stack, spec).ok().flatten()
    }

    /// Remaining capacity for `offered` in `bundle` under `spec`, distinguishing missing
    /// components (`Ok(None)`) from payloads that cannot be read (either in `bundle` or nested
    /// inside `offered`) and weights that overflow, without copying the contents.
    pub fn try_room_in_stack(
        items: &Items,
        bundle: &ItemStack,
        spec: BundleSpec<'_>,
        offered: &ItemStack,
    ) -> Result<Option<u32>, CustomBundleError> {
        let nested = spec.nested_component();
        let weight = if spec.is_vanilla() {
            let Some(data) = effective(items, bundle) else {
                return Ok(None);
            };
            resolved_weight(items, data)?
        } else {
            let Some(inside) = try_custom_contents_of(items, bundle, spec.component)? else {
                return Ok(None);
            };
            contents_weight(items, &inside, nested)?
        };
        let unit = item_weight(items, offered, nested)?;
        Ok(Some(room(spec.capacity()?, weight, unit)?))
    }

    /// Remaining capacity for `offered` in `bundle` under `spec`; `None` without the component
    /// or when [`Self::try_room_in_stack`] refuses.
    pub fn room_in_stack(
        items: &Items,
        bundle: &ItemStack,
        spec: BundleSpec<'_>,
        offered: &ItemStack,
    ) -> Option<u32> {
        Self::try_room_in_stack(items, bundle, spec, offered)
            .ok()
            .flatten()
    }

    /// The component id backing these contents.
    pub fn component(&self) -> &str {
        &self.component
    }

    /// The capacity multiplier relative to a vanilla bundle (`>= 1`).
    pub fn capacity_multiplier(&self) -> u32 {
        self.capacity_multiplier
    }

    /// Fallible `getMaxAmountToAdd` for `stack`'s kind of item: an error if `stack` carries (or
    /// contains) a malformed container of the same custom component, or a weight overflows.
    pub fn try_room_for(&self, items: &Items, stack: &ItemStack) -> Result<u32, CustomBundleError> {
        let unit = item_weight(items, stack, self.nested_component())?;
        Ok(room(self.capacity, self.weight, unit)?)
    }

    /// `getMaxAmountToAdd` for `stack`'s kind of item; `0` when [`Self::try_room_for`] fails.
    pub fn room_for(&self, items: &Items, stack: &ItemStack) -> u32 {
        self.try_room_for(items, stack).unwrap_or(0)
    }

    /// Fallible `tryInsert`: what fits moves to the front, merged with its kind up to each stored
    /// stack's `max_stack` (splitting any overflow into new front entries when
    /// `capacity_multiplier > 1`). Returns an error without modifying `self` or `stack` if
    /// `stack` carries (or contains) a malformed container of the same custom component, or the
    /// weight overflows (where vanilla's `getMaxAmountToAdd` or `weight.add` would throw).
    pub fn try_insert_checked(
        &mut self,
        items: &Items,
        stack: &mut ItemStack,
        fits: bool,
    ) -> Result<u32, CustomBundleError> {
        if stack.count == 0 {
            return Ok(0);
        }
        if !fits {
            if self.nested_component().is_some() {
                let _ = item_weight(items, stack, self.nested_component())?;
            }
            return Ok(0);
        }
        let unit = item_weight(items, stack, self.nested_component())?;
        let amount = stack.count.min(room(self.capacity, self.weight, unit)?);
        if amount == 0 {
            return Ok(0);
        }
        self.weight = self.weight.checked_add(stacked(unit, amount)?)?;
        // ItemStack.isStackable: a damaged stack is damageable.
        let max = max_stack(items, stack);
        let stackable = max > 1 && stack.damage == 0;
        let mut remaining = amount;
        if stackable
            && let Some(i) = self
                .items
                .iter()
                .position(|s| s.count < max && s.same_item_same_components(stack))
        {
            let mut merged = self.items.remove(i);
            let add = remaining.min(max - merged.count);
            merged.count += add;
            remaining -= add;
            self.items.insert(0, merged);
        }
        while remaining > 0 {
            let chunk = remaining.min(max);
            let mut part = stack.clone();
            part.count = chunk;
            remaining -= chunk;
            self.items.insert(0, part);
        }
        stack.count -= amount;
        Ok(amount)
    }

    /// `tryInsert`: what fits moves to the front, merged with its kind up to each stored stack's
    /// `max_stack` (splitting any overflow into new front entries when `capacity_multiplier > 1`).
    /// `fits` is the caller's container-fit and filter check; an empty stack, or one that
    /// [`Self::try_insert_checked`] refuses, never goes in (vanilla returns 0 for an erroring
    /// `getWeight`).
    pub fn try_insert(&mut self, items: &Items, stack: &mut ItemStack, fits: bool) -> u32 {
        self.try_insert_checked(items, stack, fits).unwrap_or(0)
    }

    /// `removeOne`: the selected stack, else the first; nothing is selected after. `None`, with
    /// nothing removed, when the stack's weight or the new total overflows (vanilla's
    /// `getStackedWeight(..).getOrThrow()` and `weight.subtract` would throw).
    pub fn remove_one(&mut self, items: &Items) -> Option<ItemStack> {
        if self.items.is_empty() {
            return None;
        }
        let at =
            usize::try_from(self.selected).map_or(0, |i| if i < self.items.len() { i } else { 0 });
        let taken = &self.items[at];
        let unit = item_weight(items, taken, self.nested_component()).ok()?;
        let weight = self
            .weight
            .checked_subtract(stacked(unit, taken.count).ok()?)
            .ok()?;
        let stack = self.items.remove(at);
        self.weight = weight;
        self.selected = -1;
        Some(stack)
    }

    /// Fallible front extraction (slot 0), ignoring UI selection and keeping any remainder at the
    /// front even for overfull contents. An error leaves the contents unchanged.
    pub fn try_extract_front(
        &mut self,
        items: &Items,
        max: u32,
    ) -> Result<Option<ItemStack>, CustomBundleError> {
        if max == 0 || self.items.is_empty() {
            return Ok(None);
        }
        let unit = item_weight(items, &self.items[0], self.nested_component())?;
        let take = self.items[0].count.min(max);
        let weight = self.weight.checked_subtract(stacked(unit, take)?)?;
        let out = if take < self.items[0].count {
            let mut taken = self.items[0].clone();
            taken.count = take;
            self.items[0].count -= take;
            taken
        } else {
            self.items.remove(0)
        };
        self.weight = weight;
        self.selected = -1;
        Ok(Some(out))
    }

    /// Extract up to `max` items from the front stack (slot 0), ignoring UI selection and
    /// keeping any remainder at the front even for overfull contents.
    pub fn extract_front(&mut self, items: &Items, max: u32) -> Option<ItemStack> {
        self.try_extract_front(items, max).ok().flatten()
    }

    /// `getSelectedItemIndex`.
    pub fn selected(&self) -> i32 {
        self.selected
    }

    /// The stacks inside, in display order (the first is the next out).
    pub fn items(&self) -> &[ItemStack] {
        &self.items
    }

    /// `getNumberOfItemsToShow`: a full grid shows 11 and a `+N`, so the last
    /// row keeps its fourth cell for the count.
    pub fn shown_count(&self) -> usize {
        let count = self.items.len();
        let available: usize = if count > 12 { 11 } else { 12 };
        let on_last_row = count % 4;
        let empty = if on_last_row == 0 { 0 } else { 4 - on_last_row };
        count.min(available.saturating_sub(empty))
    }

    /// The items the tooltip shows, and how many sit beyond them.
    pub fn shown(&self) -> (&[ItemStack], u32) {
        let n = self.shown_count();
        let hidden = self.items[n..].iter().map(|s| s.count).sum();
        (&self.items[..n], hidden)
    }

    /// The weight the bar and tooltip read, `BundleItem.getWeightSafe` over this container's
    /// capacity: a vanilla bundle's is its weight (the capacity is `Fraction.ONE`), a larger
    /// capacity divides it (`Fraction.divideBy`). A division that overflows reads as
    /// `Fraction.ONE`, full, as `getWeightSafe` reads an erroring weight.
    fn shown_weight(&self) -> Fraction {
        if self.capacity == Fraction::ONE {
            return self.weight;
        }
        self.weight
            .checked_divide_by(self.capacity)
            .unwrap_or(Fraction::ONE)
    }

    /// `BundleItem.getFullnessDisplay`: the shown weight's `floatValue` (stored weight over
    /// capacity), for bars and tooltips.
    pub fn weight_fraction(&self) -> f32 {
        self.shown_weight().float_value()
    }

    /// Whether stored weight has reached or exceeded capacity: the shown weight's
    /// `compareTo(Fraction.ONE) >= 0`, as `getBarColor` and the tooltip ask.
    pub fn is_full(&self) -> bool {
        self.shown_weight().compare_to(Fraction::ONE).is_ge()
    }

    /// `BundleItem.isBarVisible`, `getBarWidth` and `getBarColor`: the fullness bar
    /// (width, RGB), blue until the container is full, then red. Visible when the shown weight
    /// is above `Fraction.ZERO`; `min(1 + Mth.mulAndTruncate(weight, 12), 13)` wide.
    pub fn bar(&self) -> Option<(i32, u32)> {
        bar_from_weight(self.shown_weight())
    }

    /// `ClientBundleTooltip.getHeight`: height of this container's tooltip component in GUI
    /// pixels. `description_lines` is the empty-container description line count in the active
    /// font (ignored when non-empty); `line_height` is `Font.lineHeight` (9 in vanilla).
    pub fn tooltip_height(&self, description_lines: usize, line_height: i32) -> i32 {
        if self.items.is_empty() {
            (description_lines as i32)
                .wrapping_mul(line_height)
                .wrapping_add(13 + 8)
        } else {
            let rows = self.items.len().min(12).div_ceil(4) as i32;
            rows.wrapping_mul(24).wrapping_add(13 + 8)
        }
    }

    /// `ClientBundleTooltip` geometry from these already-decoded contents in O(1) without
    /// re-decoding NBT or re-walking nested weights.
    pub fn tooltip_layout(
        &self,
        x: f32,
        y: f32,
        box_width: f32,
        description_lines: usize,
    ) -> TooltipLayout {
        let offset = ((box_width - 96.0) / 2.0).floor();
        if self.items.is_empty() {
            // The description takes the top; the bar follows it. Its lines are
            // the caller's font's (`getEmptyBundleDescriptionTextHeight`).
            let description_height = 9.0 * description_lines as f32;
            return TooltipLayout {
                width: 96.0,
                height: self.tooltip_height(description_lines, 9) as f32,
                slots: Vec::new(),
                selected: None,
                overflow: None,
                bar: [x + offset, y + description_height + 4.0],
                fill: 0.0,
                full: false,
                label: Some("item.minecraft.bundle.empty"),
                empty_description: Some([x + offset, y]),
            };
        }
        let (shown, hidden) = self.shown();
        // `gridSizeY` counts `slotCount()` (at most twelve), not the slots the
        // overflow count leaves room for.
        let rows = self.items.len().min(12).div_ceil(4).max(1);
        let grid_height = rows as f32 * 24.0;
        let x_start = x + offset + 96.0;
        let y_start = y + grid_height;
        let mut slots = Vec::with_capacity(shown.len());
        let mut selected = None;
        let mut overflow = None;
        // Rows fill bottom-up and columns right-to-left, as vanilla draws them.
        let mut slot = 0;
        for row in 1..=rows {
            for column in 1..=4 {
                let at = [x_start - column as f32 * 24.0, y_start - row as f32 * 24.0];
                if hidden > 0 && row == 1 && column == 1 {
                    overflow = Some((at, hidden));
                } else if slot < shown.len() {
                    // `extractSlot`: the cell of slot number n holds item `size - n`.
                    let index = shown.len() - 1 - slot;
                    if usize::try_from(self.selected) == Ok(index) {
                        selected = Some(slots.len());
                    }
                    slots.push((at, shown[index].clone()));
                    slot += 1;
                }
            }
        }
        let weight = self.weight_fraction();
        TooltipLayout {
            width: 96.0,
            height: self.tooltip_height(description_lines, 9) as f32,
            slots,
            selected,
            overflow,
            bar: [x + offset, y + grid_height + 4.0],
            // `Mth.mulAndTruncate(weight, 94)`, kept within the bar.
            fill: (weight * 94.0).trunc().clamp(0.0, 94.0),
            full: self.is_full(),
            label: if self.is_full() {
                Some("item.minecraft.bundle.full")
            } else {
                None
            },
            empty_description: None,
        }
    }

    /// `toImmutable`, written back into the container's component as a `Tag::List` of
    /// `ItemStack.CODEC` entries. Leaves any existing custom payload that [`Self::try_of`]
    /// refuses (non-list, malformed list entry, malformed nested container, or excessive weight)
    /// untouched.
    pub fn store(self, items: &Items, bundle: &mut ItemStack) {
        if self.component == VANILLA_COMPONENT {
            write_contents(items, bundle, self.items, self.selected);
            return;
        }
        if Self::try_of(
            items,
            bundle,
            BundleSpec::custom(&self.component, self.capacity_multiplier),
        )
        .is_err()
        {
            return;
        }
        let info = items.get(&bundle.item);
        let tag = Tag::List(
            self.items
                .iter()
                .map(|s| crate::item_nbt::item_to_nbt(s, items))
                .collect(),
        );
        let default_matches = info
            .and_then(|i| i.custom_components.get(&self.component))
            .is_some_and(|default| default == &tag);
        if default_matches {
            bundle.data_mut().set_other(&self.component, None);
        } else {
            bundle.data_mut().set_other(&self.component, Some(tag));
        }
        bundle.tidy();
    }
}

/// `BundleContents.Mutable`: vanilla `minecraft:bundle_contents` with capacity multiplier `1`.
pub struct Contents(CustomBundleContents);

impl Contents {
    /// Width of a bundle tooltip component in GUI pixels (`ClientBundleTooltip.GRID_WIDTH`).
    pub const TOOLTIP_WIDTH: i32 = 96;

    /// Whether `stack` carries `minecraft:bundle_contents`, without cloning its items or
    /// recomputing their weight.
    pub fn is_bundle(items: &Items, stack: &ItemStack) -> bool {
        contents_of(items, stack).is_some()
    }

    /// `ClientBundleTooltip.getHeight`: height of this bundle tooltip component in GUI pixels.
    /// `description_lines` is the empty-bundle description line count in the active font (ignored
    /// when non-empty); `line_height` is `Font.lineHeight` (9 in vanilla).
    pub fn tooltip_height(&self, description_lines: usize, line_height: i32) -> i32 {
        self.0.tooltip_height(description_lines, line_height)
    }

    /// Lay out this prepared bundle's tooltip (`ClientBundleTooltip`) at `(x, y)` centred inside
    /// `box_width`, reusing these retained items and weight without re-reading the stack.
    pub fn tooltip_layout(
        &self,
        x: f32,
        y: f32,
        box_width: f32,
        description_lines: usize,
    ) -> TooltipLayout {
        self.0.tooltip_layout(x, y, box_width, description_lines)
    }

    /// A component collection may belong to any item, including an item with
    /// no previous bundle_contents component. Reuse the same weight model. `None` when the
    /// stacks' weight overflows vanilla's `Fraction` (`Excessive total bundle weight`).
    pub fn from_items(items: &Items, inside: Vec<ItemStack>) -> Option<Self> {
        CustomBundleContents::try_from_items(items, BundleSpec::VANILLA, inside)
            .ok()
            .map(Self)
    }
    /// The bundle's contents; None when the stack is no bundle, or its weight overflows vanilla's
    /// `Fraction` (`ItemStack.validateComponents` refuses such a stack in vanilla; reading it as
    /// unreadable keeps its contents, where `asMutable`'s empty fallback would drop them).
    pub fn of(items: &Items, stack: &ItemStack) -> Option<Self> {
        CustomBundleContents::of(items, stack, BundleSpec::VANILLA).map(Self)
    }

    /// Remaining capacity for `offered` in a stack's bundle component, without cloning contents.
    pub fn room_in_stack(items: &Items, bundle: &ItemStack, offered: &ItemStack) -> Option<u32> {
        CustomBundleContents::room_in_stack(items, bundle, BundleSpec::VANILLA, offered)
    }

    /// `getMaxAmountToAdd` for `stack`'s kind of item, or 0 for invalid weight.
    pub fn room_for(&self, items: &Items, stack: &ItemStack) -> u32 {
        self.0.room_for(items, stack)
    }

    /// `tryInsert`: what fits moves to the front, merged with its kind. `fits` is the stack's
    /// `canFitInsideContainerItems` answer, which `canItemBeInBundle` asks and the caller
    /// gives, since the parts of the stack's item may change it; an empty stack never goes in.
    pub fn try_insert(&mut self, items: &Items, stack: &mut ItemStack, fits: bool) -> u32 {
        self.0.try_insert(items, stack, fits)
    }

    /// `removeOne`: the selected stack, else the first; nothing is selected after.
    pub fn remove_one(&mut self, items: &Items) -> Option<ItemStack> {
        self.0.remove_one(items)
    }

    /// `getSelectedItemIndex`.
    pub fn selected(&self) -> i32 {
        self.0.selected()
    }

    /// The stacks inside, in display order (the first is the next out).
    pub fn items(&self) -> &[ItemStack] {
        self.0.items()
    }

    /// `getNumberOfItemsToShow`: a full grid shows 11 and a `+N`, so the last
    /// row keeps its fourth cell for the count.
    pub fn shown_count(&self) -> usize {
        self.0.shown_count()
    }

    /// The items the tooltip shows, and how many sit beyond them.
    pub fn shown(&self) -> (&[ItemStack], u32) {
        self.0.shown()
    }

    /// The `Fraction` weight as the float the progress bar reads.
    pub fn weight_fraction(&self) -> f32 {
        self.0.weight_fraction()
    }

    /// `Fraction.ONE`: a full bundle draws the full bar and says so.
    pub fn is_full(&self) -> bool {
        self.0.is_full()
    }

    /// `BundleItem.isBarVisible`, `getBarWidth` and `getBarColor`: the fullness bar
    /// (width, RGB), blue until the bundle is full, then red.
    pub fn bar(&self) -> Option<(i32, u32)> {
        self.0.bar()
    }

    /// `toImmutable`, written back into the bundle's component, preserving an
    /// explicitly empty patch when the item's prototype does not default to empty.
    pub fn store(self, items: &Items, bundle: &mut ItemStack) {
        self.0.store(items, bundle);
    }
}

/// `BundleItem.toggleSelectedItem`: select `index`, or nothing when it is the
/// selected one already or outside the effective contents. The selection is kept beside the
/// component, so inherited contents need no override of their own.
pub fn toggle_selected(items: &Items, bundle: &mut ItemStack, index: i32) {
    let Some(count) = contents_of(items, bundle).map(<[ItemStack]>::len) else {
        return;
    };
    let current = bundle.data().map_or(-1, |d| d.bundle_selected.0);
    let inside = (0..count as i32).contains(&index);
    bundle.data_mut().bundle_selected = BundleSelected(if current != index && inside {
        index
    } else {
        -1
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(numerator: i32, denominator: i32) -> Fraction {
        Fraction::new(numerator, denominator).unwrap()
    }

    #[test]
    fn weights_add_up_exactly() {
        let w = stacked(f(1, 64), 32)
            .unwrap()
            .checked_add(stacked(f(1, 16), 4).unwrap())
            .unwrap();
        assert_eq!(w, f(3, 4));
        assert_eq!(room(Fraction::ONE, w, f(1, 64)), Ok(16));
        assert_eq!(room(Fraction::ONE, Fraction::ONE, Fraction::ONE), Ok(0));
        // An over-full bundle (made by a command) has no room.
        assert_eq!(room(Fraction::ONE, f(5, 4), f(1, 64)), Ok(0));
    }

    /// Every weight helper surfaces commons-lang3's overflows as `ExcessiveWeight` instead of
    /// panicking or saturating.
    #[test]
    fn weight_helpers_overflow_into_excessive_weight() {
        // `getFraction(count, 1)` has no int count past i32::MAX; the product overflows
        // `mulAndCheck` well before that.
        assert_eq!(stacked(f(1, 1), u32::MAX), Err(ExcessiveWeight));
        assert_eq!(stacked(f(i32::MAX, 1), 2), Err(ExcessiveWeight));
        // `getMaxAmountToAdd`: `subtract` past i32::MIN, then `divideBy` past i32::MAX.
        assert_eq!(
            room(f(i32::MIN + 1, 1), f(2, 1), f(1, 64)),
            Err(ExcessiveWeight)
        );
        assert_eq!(
            room(f(i32::MAX, 1), Fraction::ZERO, f(1, 64)),
            Err(ExcessiveWeight)
        );
        // A multiplier past i32::MAX has no capacity.
        assert_eq!(
            BundleSpec::custom("runes:pouch_contents", u32::MAX).capacity(),
            Err(ExcessiveWeight)
        );
        assert_eq!(
            BundleSpec::custom("runes:pouch_contents", i32::MAX as u32).capacity(),
            Ok(f(i32::MAX, 1))
        );
    }

    /// `Mth.mulAndTruncate` multiplies with `imul`, which wraps; vanilla's bar width wraps with it.
    #[test]
    fn mul_and_truncate_wraps_like_imul() {
        assert_eq!(mul_and_truncate(f(9, 16), 12), 6);
        assert_eq!(mul_and_truncate(f(-7, 2), 1), -3);
        // 2^29 * 12 wraps to -2^31 in `int`; truncating division by 2^31 - 1 gives -1, where
        // the exact product would give 3.
        assert_eq!(mul_and_truncate(f(1 << 29, i32::MAX), 12), -1);
    }
}

// ------------------------------------------------------------- the tooltip

/// `ClientBundleTooltip`'s geometry, relative to the tooltip box's top-left:
/// the 96 pixel column of a 4 wide slot grid and the progress bar under it.
#[derive(Clone, Debug, PartialEq)]
pub struct TooltipLayout {
    /// The component's own width (96) and height, the text's excluded.
    pub width: f32,
    pub height: f32,
    /// Each slot's top-left and the stack in it, in drawing order.
    pub slots: Vec<([f32; 2], ItemStack)>,
    /// The position in `slots` of the selected item, which is drawn highlighted.
    pub selected: Option<usize>,
    /// The `+N` of the items past the grid: its top-left and the count.
    pub overflow: Option<([f32; 2], u32)>,
    /// The progress bar's top-left, its fill's width out of 94, and whether
    /// the weight fills it (the full sprite).
    pub bar: [f32; 2],
    pub fill: f32,
    pub full: bool,
    /// `item.minecraft.bundle.empty` / `.full`, centred over the bar.
    pub label: Option<&'static str>,
    /// An empty bundle's description, wrapped at `width` from here.
    pub empty_description: Option<[f32; 2]>,
}

/// The tooltip of a bundle-like container (`ClientBundleTooltip`), or None when
/// the stack does not carry `spec.component`. `x`/`y` are the box's top-left,
/// `box_width` the tooltip's width, which the component is centred in.
pub fn tooltip_layout_for(
    items: &Items,
    stack: &ItemStack,
    spec: BundleSpec<'_>,
    x: f32,
    y: f32,
    box_width: f32,
    description_lines: usize,
) -> Option<TooltipLayout> {
    CustomBundleContents::of(items, stack, spec)
        .map(|contents| contents.tooltip_layout(x, y, box_width, description_lines))
}

/// The tooltip of a bundle's contents (`ClientBundleTooltip`), or None when
/// the stack is not a bundle. `x`/`y` are the box's top-left, `box_width` the
/// tooltip's width, which the component is centred in.
pub fn tooltip_layout(
    items: &Items,
    stack: &ItemStack,
    x: f32,
    y: f32,
    box_width: f32,
    description_lines: usize,
) -> Option<TooltipLayout> {
    tooltip_layout_for(
        items,
        stack,
        BundleSpec::VANILLA,
        x,
        y,
        box_width,
        description_lines,
    )
}

#[cfg(test)]
mod tooltip_tests {
    use super::*;

    /// The prepared resources, when they are on this machine: the tooltip
    /// asks the item registry whether a stack is a bundle.
    fn items() -> Option<Items> {
        crate::items::prepared_items()
    }

    fn bundle_with(counts: &[(&str, u32)]) -> ItemStack {
        let mut bundle = ItemStack::new("minecraft:bundle", 1);
        bundle.data_mut().bundle_contents = counts
            .iter()
            .map(|(item, n)| ItemStack::new(item, *n))
            .collect();
        bundle
    }

    #[test]
    fn a_grid_shows_twelve_stacks_and_counts_the_rest() {
        let Some(items) = items() else { return };
        let twelve: Vec<(&str, u32)> = (0..12).map(|_| ("minecraft:stone", 1)).collect();
        let layout = tooltip_layout(&items, &bundle_with(&twelve), 0.0, 0.0, 96.0, 2).unwrap();
        assert_eq!(layout.slots.len(), 12);
        assert_eq!(layout.overflow, None);
        assert_eq!(layout.height, 3.0 * 24.0 + 13.0 + 8.0);
        // Rows fill bottom-up and columns right-to-left: the twelfth item is in
        // the bottom-right cell, the first in the top-left.
        assert_eq!(layout.slots[0].0, [96.0 - 24.0, 3.0 * 24.0 - 24.0]);
        assert_eq!(&*layout.slots[0].1.item, "minecraft:stone");
        assert_eq!(layout.slots[11].0, [0.0, 0.0]);
        // A wider tooltip centres the 96 pixel column in it.
        let centred = tooltip_layout(&items, &bundle_with(&twelve), 0.0, 0.0, 120.0, 2).unwrap();
        assert_eq!(centred.slots[0].0[0], 12.0 + 96.0 - 24.0);
        // Thirteen stacks: `getNumberOfItemsToShow` leaves the last row room
        // for the count, so 8 slots show and `+6` (four stones and two dirt)
        // takes the first cell.
        let mut counts = twelve;
        counts.push(("minecraft:dirt", 2));
        let layout = tooltip_layout(&items, &bundle_with(&counts), 0.0, 0.0, 96.0, 2).unwrap();
        assert_eq!(layout.slots.len(), 8);
        assert_eq!(layout.overflow, Some(([96.0 - 24.0, 48.0], 6)));
    }

    #[test]
    fn the_bar_fills_with_the_weight_and_an_empty_bundle_explains_itself() {
        let Some(items) = items() else { return };
        let layout = tooltip_layout(&items, &bundle_with(&[]), 4.0, 8.0, 96.0, 2).unwrap();
        assert!(layout.slots.is_empty());
        assert_eq!(layout.fill, 0.0);
        assert_eq!(layout.label, Some("item.minecraft.bundle.empty"));
        assert_eq!(layout.empty_description, Some([4.0, 8.0]));
        assert_eq!(layout.bar, [4.0, 8.0 + 18.0 + 4.0]);
        // A bundle holding one 64 stack is full: the whole bar, the full label.
        let layout = tooltip_layout(
            &items,
            &bundle_with(&[("minecraft:stone", 64)]),
            0.0,
            0.0,
            96.0,
            2,
        )
        .unwrap();
        assert!(layout.full);
        assert_eq!(layout.fill, 94.0);
        assert_eq!(layout.label, Some("item.minecraft.bundle.full"));
        // Something that is not a bundle has no component.
        let dirt = ItemStack::new("minecraft:dirt", 1);
        assert!(tooltip_layout(&items, &dirt, 0.0, 0.0, 96.0, 2).is_none());
    }
    /// `getEmptyBundleBackgroundHeight`: the description's wrapped height (9 a
    /// line) plus the bar's 13 and the tooltip's 8, not a fixed two lines.
    #[test]
    fn an_empty_bundles_height_follows_its_description_lines() {
        let Some(items) = items() else { return };
        let layout = tooltip_layout(&items, &bundle_with(&[]), 4.0, 8.0, 96.0, 3).unwrap();
        assert_eq!(layout.height, 3.0 * 9.0 + 13.0 + 8.0);
        assert_eq!(layout.bar, [4.0, 8.0 + 3.0 * 9.0 + 4.0]);
        assert_eq!(layout.empty_description, Some([4.0, 8.0]));
    }
}

#[cfg(test)]
mod custom_bundle_tests {
    use super::*;
    use std::fs;
    use sunrise_world::loot::PackOverlay;

    fn test_catalog(name: &str) -> Items {
        let root = std::env::temp_dir().join(format!(
            "sunrise-custom-bundle-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        let json = r#"{
            "minecraft:stone": {"minecraft:max_stack_size": 64},
            "minecraft:ender_pearl": {"minecraft:max_stack_size": 16},
            "minecraft:diamond_sword": {"minecraft:max_stack_size": 1},
            "minecraft:bundle": {"minecraft:max_stack_size": 1, "minecraft:bundle_contents": []},
            "runes:pouch": {"minecraft:max_stack_size": 1, "runes:pouch_contents": []},
            "runes:compound_pouch": {"minecraft:max_stack_size": 1, "runes:compound_contents": {"items": [], "tier": 2}},
            "runes:entries_pouch": {"minecraft:max_stack_size": 1, "runes:entries_contents": {"entries": [], "tier": 2}}
        }"#;
        fs::write(root.join("items.json"), json).unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let catalog = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        fs::remove_dir_all(&root).unwrap();
        catalog
    }

    #[test]
    fn custom_component_multipliers_split_stacks_and_scale_bar() {
        let items = test_catalog("multipliers");
        let spec4 = BundleSpec::custom("runes:pouch_contents", 4);
        assert_eq!(
            BundleSpec::custom("runes:pouch_contents", 0).multiplier(),
            1
        );

        let mut pouch = ItemStack::new("runes:pouch", 1);
        let mut contents = CustomBundleContents::of(&items, &pouch, spec4).unwrap();
        assert_eq!(contents.capacity_multiplier(), 4);
        assert_eq!(contents.bar(), None);
        assert!(!contents.is_full());
        assert_eq!(
            contents.room_for(&items, &ItemStack::new("minecraft:stone", 1)),
            256
        );
        assert_eq!(
            contents.room_for(&items, &ItemStack::new("minecraft:ender_pearl", 1)),
            64
        );

        // Insert 16 pearls (weight 1), then 10 pearls, then 10 more pearls:
        // merges 6 into the partial 10-stack (capped at max_stack 16), splits remaining 4 to front.
        let mut p1 = ItemStack::new("minecraft:ender_pearl", 16);
        assert_eq!(contents.try_insert(&items, &mut p1, true), 16);
        assert_eq!(p1.count, 0);
        let mut p2 = ItemStack::new("minecraft:ender_pearl", 10);
        assert_eq!(contents.try_insert(&items, &mut p2, true), 10);
        let mut p3 = ItemStack::new("minecraft:ender_pearl", 10);
        assert_eq!(contents.try_insert(&items, &mut p3, true), 10);
        assert_eq!(
            contents
                .items()
                .iter()
                .map(|s| (&*s.item, s.count))
                .collect::<Vec<_>>(),
            vec![
                ("minecraft:ender_pearl", 4),
                ("minecraft:ender_pearl", 16),
                ("minecraft:ender_pearl", 16),
            ]
        );
        // 36/16 = 9/4 weight out of 4 => 9/16 fullness (exact in f32), partial blue bar.
        assert_eq!(contents.weight_fraction(), 9.0 / 16.0);
        assert!(!contents.is_full());
        assert_eq!(contents.bar(), Some((1 + (9 * 12) / 16, 0x7087FF)));

        // Fill the remaining 7/4 weight = 112 stones: splits into [48, 64] at front.
        let mut stones = ItemStack::new("minecraft:stone", 150);
        assert_eq!(contents.try_insert(&items, &mut stones, true), 112);
        assert_eq!(stones.count, 38);
        assert!(contents.is_full());
        assert_eq!(contents.bar(), Some((13, 0xFF5454)));
        assert_eq!(contents.items()[0].count, 48);
        assert_eq!(contents.items()[1].count, 64);
        assert!(
            contents
                .items()
                .iter()
                .all(|s| s.count <= max_stack(&items, s))
        );

        // Persist and reload under multipliers 4, 8, and 12.
        contents.store(&items, &mut pouch);
        assert_eq!(
            CustomBundleContents::room_in_stack(
                &items,
                &pouch,
                spec4,
                &ItemStack::new("minecraft:stone", 1)
            ),
            Some(0)
        );
        assert_eq!(
            CustomBundleContents::room_in_stack(
                &items,
                &pouch,
                BundleSpec::custom("runes:pouch_contents", 8),
                &ItemStack::new("minecraft:stone", 1)
            ),
            Some(256)
        );
        assert_eq!(
            CustomBundleContents::room_in_stack(
                &items,
                &pouch,
                BundleSpec::custom("runes:pouch_contents", 12),
                &ItemStack::new("minecraft:stone", 1)
            ),
            Some(512)
        );
    }

    #[test]
    fn custom_component_nested_occupancy_bees_and_removal() {
        let items = test_catalog("nested");
        let spec = BundleSpec::custom("runes:pouch_contents", 4);

        // Nested vanilla bundle with 16 stones weighs 1/4 + 1/16 = 5/16.
        let mut vanilla_inner = ItemStack::new("minecraft:bundle", 1);
        vanilla_inner.data_mut().bundle_contents = vec![ItemStack::new("minecraft:stone", 16)];

        // Nested custom pouch with 8 pearls weighs 1/2 + 1/16 = 9/16.
        let mut custom_inner = ItemStack::new("runes:pouch", 1);
        let mut inner_c = CustomBundleContents::of(&items, &custom_inner, spec).unwrap();
        let mut pearls = ItemStack::new("minecraft:ender_pearl", 8);
        assert_eq!(inner_c.try_insert(&items, &mut pearls, true), 8);
        inner_c.store(&items, &mut custom_inner);

        // Beehive with non-empty bees weighs 1.
        let mut beehive = ItemStack::new("minecraft:stone", 1);
        beehive.data_mut().set_other(
            "minecraft:bees",
            Some(Tag::List(vec![Tag::compound([
                (
                    "entity_data",
                    Tag::compound([("id", Tag::String("minecraft:bee".into()))]),
                ),
                ("ticks_in_hive", Tag::Int(0)),
                ("min_ticks_in_hive", Tag::Int(600)),
            ])])),
        );

        let mut outer = ItemStack::new("runes:pouch", 1);
        let mut outer_c = CustomBundleContents::of(&items, &outer, spec).unwrap();
        assert_eq!(outer_c.try_insert(&items, &mut vanilla_inner, true), 1);
        assert_eq!(outer_c.try_insert(&items, &mut custom_inner, true), 1);
        assert_eq!(outer_c.try_insert(&items, &mut beehive, true), 1);
        // Total weight = 5/16 + 9/16 + 1 = 30/16 = 15/8. Remaining = 4 - 15/8 = 17/8 = 136 stones.
        assert_eq!(
            outer_c.room_for(&items, &ItemStack::new("minecraft:stone", 1)),
            136
        );
        outer_c.store(&items, &mut outer);
        assert_eq!(
            CustomBundleContents::room_in_stack(
                &items,
                &outer,
                spec,
                &ItemStack::new("minecraft:stone", 1)
            ),
            Some(136)
        );

        // Component removal (`!runes:pouch_contents`) hides the container.
        let mut removed = ItemStack::new("runes:pouch", 1);
        removed.data_mut().set_other(
            "!runes:pouch_contents",
            Some(Tag::Compound(Default::default())),
        );
        assert!(CustomBundleContents::of(&items, &removed, spec).is_none());
    }

    #[test]
    fn custom_component_extract_destroyed_and_tooltip_layout() {
        let items = test_catalog("extract_destroy_tooltip");
        let list_spec = BundleSpec::custom("runes:pouch_contents", 4);

        // Emptying a default-component pouch restores the default stack cleanly.
        let mut pouch = ItemStack::new("runes:pouch", 1);
        let mut contents = CustomBundleContents::of(&items, &pouch, list_spec).unwrap();
        let mut stones = ItemStack::new("minecraft:stone", 128);
        assert_eq!(contents.try_insert(&items, &mut stones, true), 128);
        contents.store(&items, &mut pouch);

        // Tooltip layout scales bar fill by capacity multiplier: 128/256 = 0.5 -> 47px, not full.
        let layout = tooltip_layout_for(&items, &pouch, list_spec, 0.0, 0.0, 96.0, 2).unwrap();
        assert_eq!(layout.slots.len(), 2);
        assert!(!layout.full);
        assert_eq!(layout.fill, 47.0);
        assert_eq!(layout.label, None);

        // Extract partial from front preserves remainder at index 0.
        let mut contents = CustomBundleContents::of(&items, &pouch, list_spec).unwrap();
        let taken = contents.extract_front(&items, 10).unwrap();
        assert_eq!(taken.count, 10);
        assert_eq!(contents.items()[0].count, 54);
        contents.store(&items, &mut pouch);

        // Destroyed spills remaining stacks and clears the override back to the default stack.
        let spilled = destroyed_component(&items, &mut pouch, "runes:pouch_contents");
        assert_eq!(
            spilled.iter().map(|s| s.count).collect::<Vec<_>>(),
            vec![54, 64]
        );
        assert!(pouch.same_item_same_components(&ItemStack::new("runes:pouch", 1)));
    }

    #[test]
    fn custom_component_deep_nesting_past_32_computes_exact_weight() {
        let items = test_catalog("deep_nesting");
        let spec4 = BundleSpec::custom("runes:pouch_contents", 4);

        // Build 40 nested custom pouches (empty innermost pouch wrapped 39 times).
        // Exact weight is 40 * (1/16) = 5/2 = 2.5 (not truncated at depth 32, which would have
        // returned 32/16 + 1 = 3.0 by falling back to max_stack = 1 at level 32).
        let mut nested = ItemStack::new("runes:pouch", 1);
        for _ in 0..39 {
            let mut outer = ItemStack::new("runes:pouch", 1);
            let mut contents = CustomBundleContents::of(&items, &outer, spec4).unwrap();
            let mut offer = nested;
            assert_eq!(contents.try_insert(&items, &mut offer, true), 1);
            contents.store(&items, &mut outer);
            nested = outer;
        }

        // Refused by capacity-2 pouch (2 < 5/2).
        let spec2 = BundleSpec::custom("runes:pouch_contents", 2);
        let mut cap2 = ItemStack::new("runes:pouch", 1);
        let mut cap2_contents = CustomBundleContents::of(&items, &cap2, spec2).unwrap();
        assert_eq!(cap2_contents.room_for(&items, &nested), 0);
        let mut refused = nested.clone();
        assert_eq!(cap2_contents.try_insert(&items, &mut refused, true), 0);
        assert_eq!(refused.count, 1);
        cap2_contents.store(&items, &mut cap2);

        // Accepted into capacity-3 pouch with exact remaining weight 3 - 5/2 = 1/2 = 32 stones.
        let spec3 = BundleSpec::custom("runes:pouch_contents", 3);
        let mut cap3 = ItemStack::new("runes:pouch", 1);
        let mut cap3_contents = CustomBundleContents::of(&items, &cap3, spec3).unwrap();
        assert_eq!(cap3_contents.room_for(&items, &nested), 1);
        let mut offer = nested;
        assert_eq!(cap3_contents.try_insert(&items, &mut offer, true), 1);
        assert_eq!(
            cap3_contents.room_for(&items, &ItemStack::new("minecraft:stone", 1)),
            32
        );
        cap3_contents.store(&items, &mut cap3);
        assert_eq!(
            CustomBundleContents::try_room_in_stack(
                &items,
                &cap3,
                spec3,
                &ItemStack::new("minecraft:stone", 1),
            ),
            Ok(Some(32))
        );
    }

    #[test]
    fn custom_component_refuses_non_list_formats_without_overwriting() {
        let items = test_catalog("unsupported_formats");

        // 1. Compound without "items" (e.g. {"entries": [], "tier": 2}) in prototype default.
        let entries_spec = BundleSpec::custom("runes:entries_contents", 2);
        let mut entries_pouch = ItemStack::new("runes:entries_pouch", 1);
        let original_entries = entries_pouch.clone();
        assert_eq!(
            CustomBundleContents::try_of(&items, &entries_pouch, entries_spec).err(),
            Some(CustomBundleError::UnsupportedFormat)
        );
        assert_eq!(
            CustomBundleContents::try_room_in_stack(
                &items,
                &entries_pouch,
                entries_spec,
                &ItemStack::new("minecraft:stone", 1),
            ),
            Err(CustomBundleError::UnsupportedFormat)
        );
        assert!(CustomBundleContents::of(&items, &entries_pouch, entries_spec).is_none());
        assert!(
            tooltip_layout_for(&items, &entries_pouch, entries_spec, 0.0, 0.0, 96.0, 2).is_none()
        );
        assert!(
            destroyed_component(&items, &mut entries_pouch, "runes:entries_contents").is_empty()
        );
        assert!(entries_pouch.same_item_same_components(&original_entries));
        // Even direct `store` refuses to overwrite a non-list custom component.
        CustomBundleContents::try_from_items(
            &items,
            entries_spec,
            vec![ItemStack::new("minecraft:stone", 1)],
        )
        .unwrap()
        .store(&items, &mut entries_pouch);
        assert!(entries_pouch.same_item_same_components(&original_entries));

        // 2. Compound WITH "items" ({"items": [], "tier": 2}) is also refused (single List schema).
        let compound_spec = BundleSpec::custom("runes:compound_contents", 2);
        let mut compound_pouch = ItemStack::new("runes:compound_pouch", 1);
        let original_compound = compound_pouch.clone();
        assert_eq!(
            CustomBundleContents::try_of(&items, &compound_pouch, compound_spec).err(),
            Some(CustomBundleError::UnsupportedFormat)
        );
        assert!(
            destroyed_component(&items, &mut compound_pouch, "runes:compound_contents").is_empty()
        );
        assert!(compound_pouch.same_item_same_components(&original_compound));

        // 3. Stack override holding a compound or scalar tag on an otherwise list-backed item.
        let list_spec = BundleSpec::custom("runes:pouch_contents", 2);
        let bad_tag = Tag::compound([("entries", Tag::List(Vec::new())), ("tier", Tag::Int(2))]);
        let mut overridden = ItemStack::new("runes:pouch", 1);
        overridden
            .data_mut()
            .set_other("runes:pouch_contents", Some(bad_tag.clone()));
        assert_eq!(
            CustomBundleContents::try_of(&items, &overridden, list_spec).err(),
            Some(CustomBundleError::UnsupportedFormat)
        );
        assert!(destroyed_component(&items, &mut overridden, "runes:pouch_contents").is_empty());
        assert_eq!(
            overridden
                .data()
                .and_then(|d| d.other("runes:pouch_contents")),
            Some(&bad_tag)
        );

        let mut scalar = ItemStack::new("runes:pouch", 1);
        scalar
            .data_mut()
            .set_other("runes:pouch_contents", Some(Tag::Int(7)));
        assert_eq!(
            CustomBundleContents::try_of(&items, &scalar, list_spec).err(),
            Some(CustomBundleError::UnsupportedFormat)
        );
    }

    #[test]
    fn custom_component_overfull_contents_refuse_insert_clamp_bar_and_allow_extract() {
        let items = test_catalog("overfull");
        let spec4 = BundleSpec::custom("runes:pouch_contents", 4);
        let spec2 = BundleSpec::custom("runes:pouch_contents", 2);

        // Fill a pouch to weight 3 (192 stones) under multiplier 4, then inspect under multiplier 2.
        let mut pouch = ItemStack::new("runes:pouch", 1);
        let mut contents = CustomBundleContents::of(&items, &pouch, spec4).unwrap();
        let mut stones = ItemStack::new("minecraft:stone", 192);
        assert_eq!(contents.try_insert(&items, &mut stones, true), 192);
        contents.store(&items, &mut pouch);

        let mut overfull = CustomBundleContents::try_of(&items, &pouch, spec2)
            .unwrap()
            .unwrap();
        assert!(overfull.is_full());
        assert_eq!(overfull.weight_fraction(), 1.5);
        assert_eq!(overfull.bar(), Some((13, 0xFF5454)));
        assert_eq!(
            overfull.room_for(&items, &ItemStack::new("minecraft:stone", 1)),
            0
        );
        let mut extra = ItemStack::new("minecraft:stone", 8);
        assert_eq!(overfull.try_insert(&items, &mut extra, true), 0);
        assert_eq!(extra.count, 8);

        let layout = tooltip_layout_for(&items, &pouch, spec2, 0.0, 0.0, 96.0, 2).unwrap();
        assert!(layout.full);
        assert_eq!(layout.fill, 94.0);
        assert_eq!(layout.label, Some("item.minecraft.bundle.full"));

        // Front extraction still works on overfull contents and preserves the remainder at index 0.
        let taken = overfull.extract_front(&items, 10).unwrap();
        assert_eq!(taken.count, 10);
        assert_eq!(overfull.items()[0].count, 54);
        assert!(overfull.is_full());
    }

    #[test]
    fn custom_component_malformed_list_entries_refuse_whole_payload_without_overwriting() {
        let items = test_catalog("malformed_entries");
        let spec = BundleSpec::custom("runes:pouch_contents", 2);
        let valid_stone = Tag::compound([
            ("id", Tag::String("minecraft:stone".into())),
            ("count", Tag::Int(12)),
        ]);

        // Every single invalid ItemStack.CODEC entry shape must refuse the ENTIRE list (even when
        // paired with a valid sibling stack) and leave the stack untouched across all operations.
        let malformed_cases: Vec<(&str, Tag)> = vec![
            ("non_compound_int", Tag::Int(42)),
            (
                "air_stack",
                Tag::compound([
                    ("id", Tag::String("minecraft:air".into())),
                    ("count", Tag::Int(1)),
                ]),
            ),
            (
                "bare_air_stack",
                Tag::compound([("id", Tag::String("air".into())), ("count", Tag::Int(1))]),
            ),
            (
                "invalid_identifier",
                Tag::compound([
                    ("id", Tag::String("NOT A VALID ID".into())),
                    ("count", Tag::Int(5)),
                ]),
            ),
            ("missing_id", Tag::compound([("count", Tag::Int(3))])),
            (
                "count_zero",
                Tag::compound([
                    ("id", Tag::String("minecraft:stone".into())),
                    ("count", Tag::Int(0)),
                ]),
            ),
            (
                "count_above_99",
                Tag::compound([
                    ("id", Tag::String("minecraft:stone".into())),
                    ("count", Tag::Int(200)),
                ]),
            ),
            (
                "count_wrong_type_string",
                Tag::compound([
                    ("id", Tag::String("minecraft:stone".into())),
                    ("count", Tag::String("12".into())),
                ]),
            ),
            (
                "components_non_compound",
                Tag::compound([
                    ("id", Tag::String("minecraft:stone".into())),
                    ("count", Tag::Int(1)),
                    ("components", Tag::String("not-a-compound".into())),
                ]),
            ),
            (
                "transient_component",
                Tag::compound([
                    ("id", Tag::String("minecraft:stone".into())),
                    ("count", Tag::Int(1)),
                    (
                        "components",
                        Tag::compound([(
                            "minecraft:creative_slot_lock",
                            Tag::Compound(Default::default()),
                        )]),
                    ),
                ]),
            ),
            (
                "unknown_registry_id",
                Tag::compound([
                    ("id", Tag::String("mod:removed_item".into())),
                    ("count", Tag::Int(1)),
                ]),
            ),
            (
                "unknown_minecraft_id",
                Tag::compound([
                    ("id", Tag::String("minecraft:nonexistent_item".into())),
                    ("count", Tag::Int(2)),
                ]),
            ),
            ("refused_consume_effects", refused_consume_effects_entry()),
        ];

        for (case, bad_entry) in malformed_cases {
            let payload = Tag::List(vec![bad_entry, valid_stone.clone()]);
            let mut pouch = ItemStack::new("runes:pouch", 1);
            pouch
                .data_mut()
                .set_other("runes:pouch_contents", Some(payload.clone()));
            let original = pouch.clone();

            assert_eq!(
                CustomBundleContents::try_of(&items, &pouch, spec).err(),
                Some(CustomBundleError::UnsupportedFormat),
                "{case}: try_of must refuse whole payload"
            );
            assert_eq!(
                CustomBundleContents::try_room_in_stack(
                    &items,
                    &pouch,
                    spec,
                    &ItemStack::new("minecraft:stone", 1),
                ),
                Err(CustomBundleError::UnsupportedFormat),
                "{case}: try_room_in_stack must refuse whole payload"
            );
            assert!(
                CustomBundleContents::of(&items, &pouch, spec).is_none(),
                "{case}: of must return None"
            );
            assert!(
                tooltip_layout_for(&items, &pouch, spec, 0.0, 0.0, 96.0, 2).is_none(),
                "{case}: tooltip_layout_for must return None"
            );
            assert!(
                destroyed_component(&items, &mut pouch, "runes:pouch_contents").is_empty(),
                "{case}: destroyed_component must not spill or clear malformed payload"
            );
            assert!(
                pouch.same_item_same_components(&original),
                "{case}: stack must stay untouched after destroyed_component"
            );

            // Even direct `store` refuses to overwrite a list containing a malformed entry.
            CustomBundleContents::try_from_items(
                &items,
                spec,
                vec![ItemStack::new("minecraft:stone", 1)],
            )
            .unwrap()
            .store(&items, &mut pouch);
            assert_eq!(
                pouch.data().and_then(|d| d.other("runes:pouch_contents")),
                Some(&payload),
                "{case}: store must not overwrite malformed list payload"
            );
        }

        // Valid `ItemStack.CODEC` entries (including omitted `count`, which defaults to 1) decode.
        let mut valid_pouch = ItemStack::new("runes:pouch", 1);
        valid_pouch.data_mut().set_other(
            "runes:pouch_contents",
            Some(Tag::List(vec![
                Tag::compound([("id", Tag::String("minecraft:stone".into()))]),
                valid_stone,
            ])),
        );
        let decoded = CustomBundleContents::try_of(&items, &valid_pouch, spec)
            .unwrap()
            .unwrap();
        assert_eq!(
            decoded
                .items()
                .iter()
                .map(|s| (&*s.item, s.count))
                .collect::<Vec<_>>(),
            vec![("minecraft:stone", 1), ("minecraft:stone", 12)]
        );
        assert_eq!(
            decoded
                .try_room_for(&items, &ItemStack::new("minecraft:stone", 1))
                .unwrap(),
            115
        );
    }

    #[test]
    fn custom_component_extra_entry_keys_match_vanilla_decode_and_reencode() {
        let items = test_catalog("extra_keys");
        let spec = BundleSpec::custom("runes:pouch_contents", 2);

        // Vanilla `ItemStack.MAP_CODEC` (`RecordCodecBuilder`) — and Sunrise's vanilla bundle path
        // (`item_data::template_from` / `template_tag`, `item_nbt::item_from_persistent_nbt` /
        // `item_to_nbt`) — accept unmentioned keys on an entry compound and drop them on re-encode.
        // An unmentioned legacy `"Count"` key is ignored as an extra key, so omitted `"count"`
        // defaults to `1`.
        let mut pouch = ItemStack::new("runes:pouch", 1);
        pouch.data_mut().set_other(
            "runes:pouch_contents",
            Some(Tag::List(vec![
                Tag::compound([
                    ("id", Tag::String("minecraft:stone".into())),
                    ("count", Tag::Int(3)),
                    ("tier", Tag::Int(2)),
                    ("custom_meta", Tag::String("ignored".into())),
                ]),
                Tag::compound([("id", Tag::String("stone".into())), ("Count", Tag::Byte(4))]),
            ])),
        );

        let contents = CustomBundleContents::try_of(&items, &pouch, spec)
            .unwrap()
            .unwrap();
        assert_eq!(
            contents
                .items()
                .iter()
                .map(|s| (&*s.item, s.count))
                .collect::<Vec<_>>(),
            vec![("minecraft:stone", 3), ("minecraft:stone", 1)]
        );
        // Weight = 4/64 out of capacity 2 = 128 stones -> 124 remaining.
        assert_eq!(
            contents
                .try_room_for(&items, &ItemStack::new("minecraft:stone", 1))
                .unwrap(),
            124
        );

        // Re-encoding via `store` drops the unmentioned extra keys, matching vanilla re-encode.
        contents.store(&items, &mut pouch);
        let stored = pouch
            .data()
            .and_then(|d| d.other("runes:pouch_contents"))
            .expect("stored list");
        let Tag::List(entries) = stored else {
            panic!("expected Tag::List after store");
        };
        assert_eq!(entries.len(), 2);
        assert!(entries[0].get("tier").is_none());
        assert!(entries[0].get("custom_meta").is_none());
        assert!(entries[1].get("Count").is_none());
        assert_eq!(entries[0].get("count"), Some(&Tag::Int(3)));
        assert_eq!(entries[1].get("count"), Some(&Tag::Int(1)));
    }

    #[test]
    fn custom_component_nested_malformed_payload_refuses_outer_weight_and_insert() {
        let items = test_catalog("nested_malformed");
        let spec = BundleSpec::custom("runes:pouch_contents", 2);

        // Build two malformed inner custom pouches: one non-list compound, one list with a bad element.
        let mut bad_compound_inner = ItemStack::new("runes:pouch", 1);
        bad_compound_inner.data_mut().set_other(
            "runes:pouch_contents",
            Some(Tag::compound([("tier", Tag::Int(2))])),
        );
        let mut bad_list_inner = ItemStack::new("runes:pouch", 1);
        bad_list_inner.data_mut().set_other(
            "runes:pouch_contents",
            Some(Tag::List(vec![
                Tag::Int(42),
                Tag::compound([
                    ("id", Tag::String("minecraft:stone".into())),
                    ("count", Tag::Int(1)),
                ]),
            ])),
        );

        for (label, malformed_inner) in [
            ("compound_inner", bad_compound_inner.clone()),
            ("bad_list_inner", bad_list_inner.clone()),
        ] {
            // 1. Direct nesting: valid outer pouch list whose entry is a malformed inner pouch.
            let mut outer = ItemStack::new("runes:pouch", 1);
            outer.data_mut().set_other(
                "runes:pouch_contents",
                Some(Tag::List(vec![crate::item_nbt::item_to_nbt(
                    &malformed_inner,
                    &items,
                )])),
            );
            let outer_original = outer.clone();

            assert_eq!(
                CustomBundleContents::try_of(&items, &outer, spec).err(),
                Some(CustomBundleError::UnsupportedFormat),
                "{label}: outer try_of must propagate nested error"
            );
            assert_eq!(
                CustomBundleContents::try_room_in_stack(
                    &items,
                    &outer,
                    spec,
                    &ItemStack::new("minecraft:stone", 1),
                ),
                Err(CustomBundleError::UnsupportedFormat),
                "{label}: outer try_room_in_stack must propagate nested error"
            );
            assert!(tooltip_layout_for(&items, &outer, spec, 0.0, 0.0, 96.0, 2).is_none());
            assert!(destroyed_component(&items, &mut outer, "runes:pouch_contents").is_empty());
            assert!(outer.same_item_same_components(&outer_original));
            CustomBundleContents::try_from_items(&items, spec, Vec::new())
                .unwrap()
                .store(&items, &mut outer);
            assert!(outer.same_item_same_components(&outer_original));

            // 2. Indirect nesting: malformed custom pouch inside a vanilla bundle inside outer pouch.
            let mut vanilla_wrapper = ItemStack::new("minecraft:bundle", 1);
            vanilla_wrapper.data_mut().bundle_contents = vec![malformed_inner.clone()];
            let mut outer_via_vanilla = ItemStack::new("runes:pouch", 1);
            outer_via_vanilla.data_mut().set_other(
                "runes:pouch_contents",
                Some(Tag::List(vec![crate::item_nbt::item_to_nbt(
                    &vanilla_wrapper,
                    &items,
                )])),
            );
            assert_eq!(
                CustomBundleContents::try_of(&items, &outer_via_vanilla, spec).err(),
                Some(CustomBundleError::UnsupportedFormat),
                "{label}: malformed custom pouch nested inside vanilla bundle must refuse outer"
            );

            // 3. Offered stack: valid empty outer pouch offered a malformed inner pouch (directly
            //    or wrapped inside a vanilla bundle) must refuse weight, room, and insert.
            let valid_outer = ItemStack::new("runes:pouch", 1);
            assert_eq!(
                CustomBundleContents::try_room_in_stack(
                    &items,
                    &valid_outer,
                    spec,
                    &malformed_inner
                ),
                Err(CustomBundleError::UnsupportedFormat),
                "{label}: offering malformed custom pouch must fail try_room_in_stack"
            );
            assert_eq!(
                CustomBundleContents::try_room_in_stack(
                    &items,
                    &valid_outer,
                    spec,
                    &vanilla_wrapper
                ),
                Err(CustomBundleError::UnsupportedFormat),
                "{label}: offering vanilla bundle holding malformed pouch must fail try_room_in_stack"
            );
            let mut contents = CustomBundleContents::try_of(&items, &valid_outer, spec)
                .unwrap()
                .unwrap();
            assert_eq!(
                contents.try_room_for(&items, &malformed_inner),
                Err(CustomBundleError::UnsupportedFormat)
            );
            assert_eq!(contents.room_for(&items, &malformed_inner), 0);
            let mut offered = malformed_inner.clone();
            assert_eq!(
                contents.try_insert_checked(&items, &mut offered, true),
                Err(CustomBundleError::UnsupportedFormat)
            );
            assert_eq!(
                contents.try_insert_checked(&items, &mut offered, false),
                Err(CustomBundleError::UnsupportedFormat)
            );
            assert_eq!(offered.count, 1);
            assert!(contents.items().is_empty());
        }
    }

    /// A 40-deep pouch holding 16 stones at the bottom: its contents weigh
    /// `1/4 + 40 * 1/16 = 11/4`.
    fn deep_pouch(items: &Items) -> ItemStack {
        let spec4 = BundleSpec::custom("runes:pouch_contents", 4);
        let mut current = ItemStack::new("runes:pouch", 1);
        let mut contents = CustomBundleContents::of(items, &current, spec4).unwrap();
        let mut stones = ItemStack::new("minecraft:stone", 16);
        assert_eq!(contents.try_insert(items, &mut stones, true), 16);
        contents.store(items, &mut current);
        for _ in 0..40 {
            let mut outer = ItemStack::new("runes:pouch", 1);
            let mut outer_contents = CustomBundleContents::of(items, &outer, spec4).unwrap();
            let mut inner = current;
            assert_eq!(outer_contents.try_insert(items, &mut inner, true), 1);
            outer_contents.store(items, &mut outer);
            current = outer;
        }
        current
    }

    /// The bar, fullness and full flag come from the weight computed once when the contents are
    /// read (vanilla memoizes `BundleContents.weight` on the value); every read reads the stack
    /// against the catalog it is given.
    #[test]
    fn custom_component_metrics_come_from_the_decoded_contents() {
        let items = test_catalog("metrics");
        let spec4 = BundleSpec::custom("runes:pouch_contents", 4);
        let spec2 = BundleSpec::custom("runes:pouch_contents", 2);
        let current = deep_pouch(&items);

        // 11/4 of 4 is 11/16: a 1 + 132/16 = 9 pixel blue bar. Of 2 it is 11/8: full and red.
        let m4 = CustomBundleContents::try_of(&items, &current, spec4)
            .unwrap()
            .unwrap();
        assert_eq!(m4.weight_fraction(), 0.6875);
        assert!(!m4.is_full());
        assert_eq!(m4.bar(), Some((9, 0x7087FF)));
        assert_eq!(m4.items().len(), 1);
        let m2 = CustomBundleContents::try_of(&items, &current, spec2)
            .unwrap()
            .unwrap();
        assert_eq!(m2.weight_fraction(), 1.375);
        assert!(m2.is_full());
        assert_eq!(m2.bar(), Some((13, 0xFF5454)));
        // Room is the same weight against the capacity: 4 - 11/4 = 5/4 = 80 stones.
        assert_eq!(
            CustomBundleContents::try_room_in_stack(
                &items,
                &current,
                spec4,
                &ItemStack::new("minecraft:stone", 1)
            ),
            Ok(Some(80))
        );

        // An empty pouch shows no bar.
        let empty = CustomBundleContents::try_of(&items, &ItemStack::new("runes:pouch", 1), spec4)
            .unwrap()
            .unwrap();
        assert_eq!(empty.weight_fraction(), 0.0);
        assert!(!empty.is_full());
        assert_eq!(empty.bar(), None);

        // A catalog whose stone stacks to 16 weighs the 16 stones at 1: 1 + 40/16 = 7/2.
        let mut items_v2 = items.clone();
        items_v2.items.get_mut("minecraft:stone").unwrap().max_stack = 16;
        let heavier = CustomBundleContents::try_of(&items_v2, &current, spec4)
            .unwrap()
            .unwrap();
        assert_eq!(heavier.weight_fraction(), 0.875);

        // A catalog without the stored item refuses the payload, as registry-backed `Item.CODEC`.
        let mut items_v3 = items.clone();
        items_v3.items.remove("minecraft:stone");
        assert_eq!(
            CustomBundleContents::try_of(&items_v3, &current, spec4).err(),
            Some(CustomBundleError::UnsupportedFormat)
        );
    }

    /// Vanilla's `DataResult.error("Excessive total bundle weight")`, at any success type.
    fn excessive<T>() -> Result<T, CustomBundleError> {
        Err(CustomBundleError::Weight(ExcessiveWeight))
    }

    /// Stones stacking to `max` (a `minecraft:max_stack_size` patch), as an entry and as a stack.
    fn stone_entry(max: i32) -> Tag {
        Tag::compound([
            ("id", Tag::String("minecraft:stone".into())),
            ("count", Tag::Int(1)),
            (
                "components",
                Tag::compound([("minecraft:max_stack_size", Tag::Int(max))]),
            ),
        ])
    }
    fn stone_stack(max: i32) -> ItemStack {
        crate::item_nbt::item_from_persistent_nbt(&stone_entry(max)).unwrap()
    }

    /// Five stones stacking to the coprime 97, 89, 83, 79 and 73: in vanilla's order the fifth
    /// `add` needs the denominator 56606581 * 73, past `i32::MAX`, so `mulPosAndCheck` throws and
    /// `computeContentWeight` answers "Excessive total bundle weight". Four of them still fit.
    const COPRIME_MAXES: [i32; 5] = [97, 89, 83, 79, 73];

    #[test]
    fn excessive_contents_weight_refuses_like_compute_content_weight() {
        let items = test_catalog("excessive");
        let spec = BundleSpec::custom("runes:pouch_contents", 2);
        let entries = |n: usize| {
            Tag::List(
                COPRIME_MAXES[..n]
                    .iter()
                    .copied()
                    .map(stone_entry)
                    .collect(),
            )
        };

        // Four kinds weigh 2618148/56606581 (`1/97 + 1/89 + 1/83 + 1/79`), still an int fraction.
        let mut four = ItemStack::new("runes:pouch", 1);
        four.data_mut()
            .set_other("runes:pouch_contents", Some(entries(4)));
        let contents = CustomBundleContents::try_of(&items, &four, spec)
            .unwrap()
            .unwrap();
        assert_eq!(contents.items().len(), 4);

        // The fifth overflows: every read refuses with the weight error and nothing is changed.
        let mut five = ItemStack::new("runes:pouch", 1);
        five.data_mut()
            .set_other("runes:pouch_contents", Some(entries(5)));
        let original = five.clone();
        assert_eq!(
            CustomBundleContents::try_of(&items, &five, spec).map(|c| c.is_some()),
            excessive()
        );
        assert_eq!(
            CustomBundleContents::try_room_in_stack(
                &items,
                &five,
                spec,
                &ItemStack::new("minecraft:stone", 1)
            ),
            excessive()
        );
        assert!(tooltip_layout_for(&items, &five, spec, 0.0, 0.0, 96.0, 2).is_none());
        assert!(destroyed_component(&items, &mut five, "runes:pouch_contents").is_empty());
        CustomBundleContents::try_from_items(&items, spec, Vec::new())
            .unwrap()
            .store(&items, &mut five);
        assert!(five.same_item_same_components(&original));
        let stacks: Vec<ItemStack> = COPRIME_MAXES.iter().copied().map(stone_stack).collect();
        assert_eq!(
            CustomBundleContents::try_from_items(&items, spec, stacks.clone()).map(|_| ()),
            Err(CustomBundleError::Weight(ExcessiveWeight))
        );

        // The same five stacks in a vanilla bundle: vanilla's own weight refuses them, and
        // offering that bundle to a valid pouch refuses the insert without changing anything.
        let mut bundle = ItemStack::new("minecraft:bundle", 1);
        bundle.data_mut().bundle_contents = stacks;
        assert!(Contents::of(&items, &bundle).is_none());
        assert_eq!(
            CustomBundleContents::try_of(&items, &bundle, BundleSpec::VANILLA).map(|c| c.is_some()),
            excessive()
        );
        let mut pouch = CustomBundleContents::try_of(&items, &four, spec)
            .unwrap()
            .unwrap();
        assert_eq!(pouch.try_room_for(&items, &bundle), excessive());
        let mut offered = bundle.clone();
        assert_eq!(
            pouch.try_insert_checked(&items, &mut offered, true),
            excessive()
        );
        assert_eq!(pouch.try_insert(&items, &mut offered, true), 0);
        assert_eq!(offered.count, 1);
        assert_eq!(pouch.items().len(), 4);

        // Room for a stone stacking to 41 in the four-kind pouch: `divideBy(1/41)` multiplies
        // 2 - 2618148/56606581 = 110595014/56606581 by 41 past `i32::MAX` (`mulAndCheck`).
        let mut forty_one = stone_stack(41);
        assert_eq!(pouch.try_room_for(&items, &forty_one), excessive());
        assert_eq!(
            pouch.try_insert_checked(&items, &mut forty_one, true),
            excessive()
        );
        assert_eq!(forty_one.count, 1);
        assert_eq!(pouch.items().len(), 4);
    }

    /// A mod's capacity multiplier is the `Fraction` `multiplier / 1`: past `i32::MAX` it has no
    /// capacity, and a large one overflows the room and shown-weight divisions.
    #[test]
    fn a_mod_multiplier_that_overflows_is_an_excessive_weight() {
        let items = test_catalog("multiplier_overflow");
        let empty = ItemStack::new("runes:pouch", 1);
        let stone = ItemStack::new("minecraft:stone", 1);

        // u32::MAX: every read refuses, before any weight is computed.
        let huge = BundleSpec::custom("runes:pouch_contents", u32::MAX);
        assert_eq!(
            CustomBundleContents::try_of(&items, &empty, huge).map(|c| c.is_some()),
            excessive()
        );
        assert_eq!(
            CustomBundleContents::try_room_in_stack(&items, &empty, huge, &stone),
            excessive()
        );
        assert_eq!(
            CustomBundleContents::try_from_items(&items, huge, Vec::new()).map(|_| ()),
            excessive()
        );

        // i32::MAX reads, but room for a stone is `i32::MAX * 64` (`mulAndCheck`), so nothing
        // goes in and nothing changes; the empty pouch still shows no bar.
        let max = BundleSpec::custom("runes:pouch_contents", i32::MAX as u32);
        let mut contents = CustomBundleContents::try_of(&items, &empty, max)
            .unwrap()
            .unwrap();
        assert_eq!(contents.try_room_for(&items, &stone), excessive());
        assert_eq!(contents.room_for(&items, &stone), 0);
        let mut offered = ItemStack::new("minecraft:stone", 5);
        assert_eq!(
            contents.try_insert_checked(&items, &mut offered, true),
            excessive()
        );
        assert_eq!(offered.count, 5);
        assert!(contents.items().is_empty());
        assert_eq!(contents.weight_fraction(), 0.0);
        assert_eq!(contents.bar(), None);

        // 2^30 holds a stone made directly, but 1/64 divided by 2^30 needs the denominator
        // 2^36 (`mulPosAndCheck`): that shown weight reads as `Fraction.ONE`, full, as
        // `BundleItem.getWeightSafe` reads an erroring weight.
        let large = BundleSpec::custom("runes:pouch_contents", 1 << 30);
        let one_stone =
            CustomBundleContents::try_from_items(&items, large, vec![stone.clone()]).unwrap();
        assert_eq!(one_stone.weight_fraction(), 1.0);
        assert!(one_stone.is_full());
        assert_eq!(one_stone.bar(), Some((13, 0xFF5454)));
    }

    /// An entry the one persistent decoder refuses for its consume effects: an unregistered mod
    /// effect whose payload holds a non-finite float stays in the saved patch but may not run.
    fn stone_with_effect_strength(strength: f32) -> Tag {
        Tag::compound([
            ("id", Tag::String("minecraft:stone".into())),
            ("count", Tag::Int(1)),
            (
                "components",
                Tag::compound([(
                    "minecraft:consumable",
                    Tag::compound([(
                        "on_consume_effects",
                        Tag::List(vec![Tag::compound([
                            ("type", Tag::String("runes:chill".into())),
                            ("strength", Tag::Float(strength.to_bits())),
                        ])]),
                    )]),
                )]),
            ),
        ])
    }
    fn refused_consume_effects_entry() -> Tag {
        stone_with_effect_strength(f32::NAN)
    }

    /// Entries decode through `item_nbt::item_from_persistent_nbt`: a floating-point `count`
    /// is read through `NbtOps.getNumberValue` and `Number.intValue`, and a stack with refused
    /// consume effects is refused, so the whole payload is.
    #[test]
    fn custom_component_entries_use_the_persistent_stack_decoder() {
        let items = test_catalog("persistent_decoder");
        let spec = BundleSpec::custom("runes:pouch_contents", 2);
        let read = |payload: Tag| {
            let mut pouch = ItemStack::new("runes:pouch", 1);
            pouch
                .data_mut()
                .set_other("runes:pouch_contents", Some(payload));
            CustomBundleContents::try_of(&items, &pouch, spec)
                .map(|c| c.map(|c| c.items().iter().map(|s| s.count).collect::<Vec<_>>()))
        };

        // 2.7f and 3.9d truncate to 2 and 3, as `Codec.INT` reads a boxed float or double.
        assert_eq!(
            read(Tag::List(vec![
                Tag::compound([
                    ("id", Tag::String("minecraft:stone".into())),
                    ("count", Tag::Float(2.7f32.to_bits())),
                ]),
                Tag::compound([
                    ("id", Tag::String("minecraft:stone".into())),
                    ("count", Tag::Double(3.9f64.to_bits())),
                ]),
            ])),
            Ok(Some(vec![2, 3]))
        );

        // The finite effect is admitted; the non-finite one is refused by the decoder itself,
        // and with it the payload.
        assert_eq!(
            read(Tag::List(vec![stone_with_effect_strength(0.5)])),
            Ok(Some(vec![1]))
        );
        let refused = refused_consume_effects_entry();
        // The patch itself is valid; the stack is refused for its effect alone.
        assert_eq!(
            sunrise_world::item_defaults::validate_component_map_tag(
                refused.get("components").unwrap()
            ),
            Ok(())
        );
        assert!(
            crate::item_nbt::item_from_nbt(&refused)
                .unwrap()
                .has_refused_consume_effects()
        );
        assert!(crate::item_nbt::item_from_persistent_nbt(&refused).is_err());
        assert_eq!(
            read(Tag::List(vec![refused])),
            Err(CustomBundleError::UnsupportedFormat)
        );
    }
}
