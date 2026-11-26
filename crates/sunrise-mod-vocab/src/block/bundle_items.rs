use crate::{
    Host, ItemPlace, ItemResource, ItemStack, Overanswered, ResourceHandler, ServerHost,
    TransactionContext, TransactionError, backend_of,
};
use sunrise_mod_api_macros::Wire;

/// Why a bundle query could not run on this Host.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Wire)]
pub enum BundleReadError {
    /// This Host has no active world item registry, or the stack could not be read.
    Unavailable,
    /// The stack (or a nested container carrying the same custom component) does not conform to
    /// `Tag::List` of `ItemStack.CODEC` entries naming registered items.
    UnsupportedFormat,
    /// Vanilla's `DataResult.error("Excessive total bundle weight")`: the stored weight, a
    /// weight the operation needs (room left, the weight after an insert or extract), or the
    /// capacity overflowed vanilla's `int` `Fraction` (commons-lang3 throws). A
    /// `capacity_multiplier` above `i32::MAX` always does.
    ExcessiveWeight,
}

impl std::fmt::Display for BundleReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("bundle registry unavailable on this host"),
            Self::UnsupportedFormat => {
                f.write_str("bundle component value is not a list of valid item stacks")
            }
            Self::ExcessiveWeight => f.write_str("Excessive total bundle weight"),
        }
    }
}

impl std::error::Error for BundleReadError {}

impl crate::NoBackend for BundleReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

impl From<crate::StackWriteError> for BundleReadError {
    fn from(_: crate::StackWriteError) -> Self {
        Self::Unavailable
    }
}

/// Fullness and bar metrics for a bundle-like item stack under a [`BundleConfig`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Wire)]
pub struct BundleMetrics {
    /// Stored weight divided by capacity (`0.0` when empty, `>= 1.0` when full).
    pub fullness: f32,
    /// Whether stored weight has reached or exceeded capacity.
    pub is_full: bool,
    /// Vanilla-style fullness bar `(width in 1..=13, rgb)` when non-empty, or `None` when empty.
    pub bar: Option<(i32, u32)>,
    /// Number of distinct stacks stored inside the component.
    pub stack_count: u32,
}

impl BundleMetrics {
    /// Construct fullness metrics for a bundle-like container.
    pub fn new(fullness: f32, is_full: bool, bar: Option<(i32, u32)>, stack_count: u32) -> Self {
        Self {
            fullness,
            is_full,
            bar,
            stack_count,
        }
    }
}

/// Configuration for a bundle-like item container backed by a registered data component.
///
/// Use [`BundleConfig::vanilla`] for vanilla's `minecraft:bundle_contents` (multiplier `1`, no
/// tag filter), or [`BundleConfig::custom`] with a component registered via
/// `Hooks::component_types`, an optional capacity multiplier, and an optional item tag filter.
/// The helper methods on `BundleConfig` run against the game's shared exact-fraction weight model
/// on both server and client hosts so parts (`item_stacked_on_other`, `item_destroyed`, `item_bar`,
/// `tooltip_image`, `item_model_range_property`) and [`BundleItems`] share one rule set.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Wire)]
pub struct BundleConfig {
    /// Namespaced data component id holding the stored stacks.
    pub component: String,
    /// Whole-number capacity multiplier relative to a vanilla bundle (`1` = 64 normal items).
    /// The capacity is vanilla's `int` `Fraction` `capacity_multiplier / 1`: above `i32::MAX`
    /// every helper answers [`BundleReadError::ExcessiveWeight`].
    pub capacity_multiplier: u32,
    /// Optional item tag id (without `#`, e.g. `"mymod:runes"`) restricting which items may enter.
    pub filter_tag: Option<String>,
}

impl Default for BundleConfig {
    fn default() -> Self {
        Self::vanilla()
    }
}

impl BundleConfig {
    /// Vanilla `minecraft:bundle_contents` with multiplier `1` and no tag filter.
    pub fn vanilla() -> Self {
        Self {
            component: "minecraft:bundle_contents".to_owned(),
            capacity_multiplier: 1,
            filter_tag: None,
        }
    }

    /// A registered custom bundle-like component with capacity `capacity_multiplier.max(1)`.
    pub fn custom(component: impl Into<String>, capacity_multiplier: u32) -> Self {
        Self {
            component: component.into(),
            capacity_multiplier: capacity_multiplier.max(1),
            filter_tag: None,
        }
    }

    /// Restrict insertion to items matching item tag `tag` (a leading `#`, if present, is stripped).
    pub fn with_filter_tag(mut self, tag: impl Into<String>) -> Self {
        let tag = tag.into();
        let normalized = tag.strip_prefix('#').unwrap_or(&tag).to_owned();
        self.filter_tag = (!normalized.is_empty()).then_some(normalized);
        self
    }

    /// Read the stored stacks of `stack` in front-first order.
    ///
    /// Returns `Ok(None)` if `stack` does not carry this config's component,
    /// `Err(BundleReadError::Unavailable)` when the host has no active world item registry,
    /// `Err(BundleReadError::UnsupportedFormat)` when the component (or a nested container
    /// carrying the same custom component) is not a `Tag::List` of `ItemStack.CODEC` entries, or
    /// `Err(BundleReadError::ExcessiveWeight)` when its weight overflows vanilla's `Fraction`.
    pub fn slots(
        &self,
        host: &dyn Host,
        stack: &ItemStack,
    ) -> Result<Option<Vec<ItemStack>>, BundleReadError> {
        let slots = backend_of(host)
            .ok_or(BundleReadError::Unavailable)?
            .bundle_slots(stack, self)?;
        Ok(slots.map(|slots| slots.into_iter().flatten().collect()))
    }

    /// Remaining capacity in `stack` for `item` under this config's multiplier, nested occupancy,
    /// container-fit rules, and optional `filter_tag`.
    ///
    /// Returns `Ok(0)` when `item` is refused or `stack` does not carry the component,
    /// `Err(BundleReadError::Unavailable)` when the host has no active world item registry,
    /// `Err(BundleReadError::UnsupportedFormat)` when `stack`, a nested container inside it, or
    /// `item` carries a malformed custom component payload, or
    /// `Err(BundleReadError::ExcessiveWeight)` when a weight or the room left overflows vanilla's
    /// `Fraction`.
    pub fn room_for(
        &self,
        host: &dyn Host,
        stack: &ItemStack,
        item: &ItemStack,
    ) -> Result<u32, BundleReadError> {
        backend_of(host)
            .ok_or(BundleReadError::Unavailable)?
            .bundle_capacity(stack, 0, item, self)
    }

    /// Insert as much of `item` as fits into one item of `stack` at the front, returning the
    /// moved count and the updated single container stack.
    ///
    /// Returns `Ok(None)` if `stack` does not carry this config's component,
    /// `Err(BundleReadError::Unavailable)` when the host has no active world item registry, or
    /// `Err(BundleReadError::UnsupportedFormat)` or `Err(BundleReadError::ExcessiveWeight)`
    /// without modifying the stack when `stack`, a nested container inside it, or `item` carries
    /// a malformed custom component payload, or a weight overflows vanilla's `Fraction`.
    pub fn try_insert(
        &self,
        host: &dyn Host,
        stack: &ItemStack,
        item: &ItemStack,
    ) -> Result<Option<(u32, ItemStack)>, BundleReadError> {
        backend_of(host)
            .ok_or(BundleReadError::Unavailable)?
            .bundle_insert_slot(stack, 0, item, self)
    }

    /// Extract up to `max` items from the front of one item of `stack`, returning the removed
    /// stack (if any) and the updated single container stack. Pass `u32::MAX` to take the entire
    /// front stack.
    ///
    /// Returns `Ok(None)` if `stack` does not carry this config's component,
    /// `Err(BundleReadError::Unavailable)` when the host has no active world item registry, or
    /// `Err(BundleReadError::UnsupportedFormat)` or `Err(BundleReadError::ExcessiveWeight)`
    /// without modifying the stack when the component (or a nested container carrying the same
    /// custom component) carries a malformed payload, or a weight overflows vanilla's `Fraction`.
    pub fn extract(
        &self,
        host: &dyn Host,
        stack: &ItemStack,
        max: u32,
    ) -> Result<Option<(Option<ItemStack>, ItemStack)>, BundleReadError> {
        backend_of(host)
            .ok_or(BundleReadError::Unavailable)?
            .bundle_extract(stack, 0, max, self)
    }

    /// Exact fullness, full flag, vanilla-style bar `(width, rgb)`, and stored stack count for
    /// `stack` under this config; `Ok(None)` when `stack` does not carry the component, or
    /// `Err(BundleReadError::UnsupportedFormat)` / `Err(BundleReadError::ExcessiveWeight)` when
    /// the component (or a nested container carrying the same custom component) carries a
    /// malformed payload or its weight overflows vanilla's `Fraction`.
    ///
    /// The four values come from one weight, computed as the contents are read, as vanilla
    /// memoizes `BundleContents.weight` on the component value; the bar follows
    /// `BundleItem.getBarWidth` / `getBarColor` over `weight / capacity_multiplier`. Each call
    /// decodes the component and weighs it again (nothing is cached between calls), so a part
    /// that draws many times a frame should ask once per frame and reuse the answer.
    pub fn metrics(
        &self,
        host: &dyn Host,
        stack: &ItemStack,
    ) -> Result<Option<BundleMetrics>, BundleReadError> {
        backend_of(host)
            .ok_or(BundleReadError::Unavailable)?
            .bundle_metrics(stack, self)
    }
}

/// A bundle's item handler at a place, using the game's shared bundle weight rules.
/// Inserts move their stack to the front; only the front stack can be extracted.
/// Writes exchange one item in the caller's transaction, so aborts restore its contents.
/// A mod can return this helper from its own `ITEM_ITEMS` provider for either vanilla
/// `minecraft:bundle_contents` ([`BundleItems::new`]) or a registered custom component
/// ([`BundleItems::with_config`]).
// no wire: the handler is reached through the ResourceHandler capability proxy.
// exhaustive: its fields are private; a mod builds it with `BundleItems::new` or `with_config`.
pub struct BundleItems(ItemPlace, BundleConfig);
impl BundleItems {
    /// Handle the bundle-like item at `place`, carrying `minecraft:bundle_contents`.
    pub fn new(place: ItemPlace) -> Self {
        Self::with_config(place, BundleConfig::vanilla())
    }

    /// Handle the bundle-like item at `place` using `config`'s component id, capacity multiplier,
    /// and optional item tag filter.
    pub fn with_config(place: ItemPlace, config: BundleConfig) -> Self {
        Self(place, config)
    }

    /// The component, multiplier, and filter configuration backing this handler.
    pub fn config(&self) -> &BundleConfig {
        &self.1
    }
}
impl BundleItems {
    fn slot(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemStack> {
        let held = self.0.with_stack(host, |stack| stack.project()).flatten()?;
        backend_of(host)?
            .bundle_slots(&held, &self.1)
            .ok()??
            .into_iter()
            .nth(index)
            .flatten()
    }
    /// Rewrites one item at `.0` as `changed`: `moved` when that goes through, else 0.
    fn rewrite(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        changed: &ItemStack,
        moved: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        Ok(
            match self.0.exchange(host, tx, &ItemResource::of(changed), 1)? {
                1 => moved,
                _ => 0,
            },
        )
    }
}
impl ResourceHandler<ItemResource> for BundleItems {
    fn size(&self, host: &mut dyn ServerHost) -> usize {
        self.0
            .with_stack(host, |stack| stack.project())
            .flatten()
            .and_then(|held| {
                backend_of(host)?
                    .bundle_slots(&held, &self.1)
                    .ok()
                    .flatten()
            })
            .map_or(0, |slots| slots.len().max(1))
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
        Some(ItemResource::of(&self.slot(host, index)?))
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.slot(host, index).map_or(0, |stack| stack.count())
    }
    fn capacity(
        &self,
        host: &mut dyn ServerHost,
        index: usize,
        resource: Option<&ItemResource>,
    ) -> u32 {
        let Some(resource) = resource else {
            return if index < self.size(host) { u32::MAX } else { 0 };
        };
        self.0
            .with_stack(host, |stack| stack.project())
            .flatten()
            .and_then(|held| {
                backend_of(host)?
                    .bundle_capacity(&held, index, resource.as_stack(), &self.1)
                    .ok()
            })
            .unwrap_or(0)
    }
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        let Some(offer) = resource.to_stack(amount) else {
            return Ok(0);
        };
        let Some((went, changed)) = self
            .0
            .with_stack(host, |stack| stack.project())
            .flatten()
            .and_then(|held| {
                backend_of(host)?
                    .bundle_insert_slot(&held, index, &offer, &self.1)
                    .ok()
                    .flatten()
            })
        else {
            return Ok(0);
        };
        match went {
            0 => Ok(0),
            went => self.rewrite(host, tx, &changed, went.min(amount)),
        }
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        let Some((Some(out), changed)) = self
            .0
            .with_stack(host, |stack| stack.project())
            .flatten()
            .and_then(|held| {
                let backend = backend_of(host)?;
                let slot = backend
                    .bundle_slots(&held, &self.1)
                    .ok()??
                    .into_iter()
                    .nth(index)??;
                if !resource.matches(&slot) {
                    return None;
                }
                backend.bundle_extract(&held, index, amount, &self.1).ok()?
            })
        else {
            return Ok(0);
        };
        self.rewrite(host, tx, &changed, out.count().min(amount))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Reader, Writer};

    struct BareHost;
    impl Host for BareHost {}
    crate::impl_host_areas!(BareHost);

    #[test]
    fn bundle_config_and_metrics_round_trip_on_wire() {
        let config =
            BundleConfig::custom("runes:pouch_contents", 0).with_filter_tag("#runes:runes");
        assert_eq!(config.component, "runes:pouch_contents");
        assert_eq!(config.capacity_multiplier, 1);
        assert_eq!(config.filter_tag.as_deref(), Some("runes:runes"));

        let mut writer = Writer::new();
        writer.put(&config);
        let decoded = Reader::decode::<BundleConfig>(writer.bytes()).unwrap();
        assert_eq!(decoded, config);

        let metrics = BundleMetrics::new(0.5, false, Some((7, 0x7087FF)), 3);
        let mut writer = Writer::new();
        writer.put(&metrics);
        let decoded = Reader::decode::<BundleMetrics>(writer.bytes()).unwrap();
        assert_eq!(decoded, metrics);

        for err in [
            BundleReadError::Unavailable,
            BundleReadError::UnsupportedFormat,
            BundleReadError::ExcessiveWeight,
        ] {
            let mut writer = Writer::new();
            writer.put(&err);
            let decoded = Reader::decode::<BundleReadError>(writer.bytes()).unwrap();
            assert_eq!(decoded, err);
        }
    }

    #[test]
    fn bundle_config_helpers_return_unavailable_without_backend() {
        let host = BareHost;
        let config = BundleConfig::custom("runes:pouch_contents", 4);
        let pouch = ItemStack::new(crate::ItemKey(0), 1).unwrap();
        let item = ItemStack::new(crate::ItemKey(1), 1).unwrap();

        assert_eq!(
            config.slots(&host, &pouch),
            Err(BundleReadError::Unavailable)
        );
        assert_eq!(
            config.room_for(&host, &pouch, &item),
            Err(BundleReadError::Unavailable)
        );
        assert_eq!(
            config.try_insert(&host, &pouch, &item),
            Err(BundleReadError::Unavailable)
        );
        assert_eq!(
            config.extract(&host, &pouch, 1),
            Err(BundleReadError::Unavailable)
        );
        assert_eq!(
            config.metrics(&host, &pouch),
            Err(BundleReadError::Unavailable)
        );
    }
}
