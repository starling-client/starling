use crate::{
    ItemPlace, ItemResource, ItemStack, ResourceHandler, ServerHost, Transaction, backend_of,
};

/// A bundle's item handler at a place, using the game's shared bundle weight rules.
/// Inserts move their stack to the front; only the front stack can be extracted.
/// Writes exchange one item in the caller's transaction, so aborts restore its contents.
/// A mod can return this helper from its own `ITEM_ITEMS` provider.
// no wire: the handler is reached through the ResourceHandler capability proxy.
// exhaustive: its one field is private; a mod builds it with `BundleItems::new`.
pub struct BundleItems(ItemPlace);
impl BundleItems {
    /// Handle the bundle-like item at `place`, carrying `minecraft:bundle_contents`.
    pub fn new(place: ItemPlace) -> Self {
        Self(place)
    }
}
impl BundleItems {
    fn slot(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemStack> {
        let held = self.0.with_stack(host, |stack| stack.project()).flatten()?;
        backend_of(host)?
            .item_bundle_slots(&held)?
            .into_iter()
            .nth(index)
            .flatten()
    }
    /// Rewrites one item at `.0` as `changed`: `moved` when that goes through, else 0.
    fn rewrite(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        changed: &ItemStack,
        moved: u32,
    ) -> u32 {
        match self.0.exchange(host, tx, &ItemResource::of(changed), 1) {
            1 => moved,
            _ => 0,
        }
    }
}
impl ResourceHandler<ItemResource> for BundleItems {
    fn size(&self, host: &mut dyn ServerHost) -> usize {
        self.0
            .with_stack(host, |stack| stack.project())
            .flatten()
            .and_then(|held| backend_of(host)?.item_bundle_slots(&held))
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
                Some(backend_of(host)?.item_bundle_capacity(&held, index, resource.as_stack()))
            })
            .unwrap_or(0)
    }
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        let Some(offer) = resource.to_stack(amount) else {
            return 0;
        };
        let Some((went, changed)) = self
            .0
            .with_stack(host, |stack| stack.project())
            .flatten()
            .and_then(|held| backend_of(host)?.item_bundle_insert_slot(&held, index, &offer))
        else {
            return 0;
        };
        match went {
            0 => 0,
            went => self.rewrite(host, tx, &changed, went.min(amount)),
        }
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        let Some((Some(out), changed)) = self
            .0
            .with_stack(host, |stack| stack.project())
            .flatten()
            .and_then(|held| {
                let backend = backend_of(host)?;
                let slot = backend.item_bundle_slots(&held)?.into_iter().nth(index)??;
                if !resource.matches(&slot) {
                    return None;
                }
                backend.item_bundle_extract(&held, index, amount)
            })
        else {
            return 0;
        };
        self.rewrite(host, tx, &changed, out.count().min(amount))
    }
}
