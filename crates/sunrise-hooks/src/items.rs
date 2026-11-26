//! Vanilla's own item catalogs as the engine loaded them (docs/layers.md, "Item catalog").
use crate::containers::Stack;
use std::sync::RwLock;

static CREATIVE_ORDER: RwLock<Option<engine::CreativeOrder>> = RwLock::new(None);

/// `stack`'s place in the search tab's order as built (after mods' edits and permissions), the
/// same order the built-in sorter ranks by: its own place, else its item's first; `None` if unlisted.
pub fn creative_order(stack: &Stack) -> Option<usize> {
    let order = CREATIVE_ORDER.read().unwrap_or_else(|e| e.into_inner());
    order.as_ref()?.of(&stack.item, stack.kind)
}

/// The engine's side. Only the engine calls it.
pub mod engine {
    use super::*;
    use std::collections::HashMap;

    /// Vanilla's creative order, looked up without a scan.
    #[derive(Debug, Default)]
    pub struct CreativeOrder {
        by_kind: HashMap<u64, usize>,
        by_item: HashMap<Box<str>, usize>,
    }

    impl CreativeOrder {
        /// The stacks in order as (item id, [`Stack::kind`]); repeats keep the first. Builds a
        /// table without publishing it.
        pub fn new<'a>(stacks: impl IntoIterator<Item = (&'a str, u64)>) -> Self {
            let mut order = Self::default();
            for (place, (item, kind)) in stacks.into_iter().enumerate() {
                order.by_kind.entry(kind).or_insert(place);
                order.by_item.entry(item.into()).or_insert(place);
            }
            order
        }

        /// The place of the stack of `kind`, else its `item`'s first; `None` if unlisted.
        pub fn of(&self, item: &str, kind: u64) -> Option<usize> {
            self.by_kind
                .get(&kind)
                .or_else(|| self.by_item.get(item))
                .copied()
        }
    }

    /// The stock order was built (again): [`creative_order`] answers from `order` now.
    pub fn set_creative_order(order: CreativeOrder) {
        *CREATIVE_ORDER.write().unwrap_or_else(|e| e.into_inner()) = Some(order);
    }
}

#[cfg(test)]
mod tests {
    use super::engine::CreativeOrder;

    #[test]
    fn the_same_stack_comes_first_then_the_items_first_place() {
        let order = CreativeOrder::new([
            ("minecraft:stone", 1),
            ("minecraft:potion", 20),
            ("minecraft:potion", 21),
            ("minecraft:stone", 1),
        ]);
        assert_eq!(order.of("minecraft:potion", 21), Some(2));
        assert_eq!(order.of("minecraft:stone", 1), Some(0));
        // A potion the tabs don't list takes its item's first place.
        assert_eq!(order.of("minecraft:potion", 99), Some(1));
        assert_eq!(order.of("minecraft:barrier", 7), None);
    }
}
