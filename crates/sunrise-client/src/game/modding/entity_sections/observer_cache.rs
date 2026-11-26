//! Retain the live observer's two allocations across borrowed creature passes.
use super::{EntitySections, LiveSectionObserver, Scope};
use std::{cell::RefCell, rc::Rc};

type SharedSections = Rc<RefCell<EntitySections>>;
pub(in crate::game) type SharedObserver = Rc<RefCell<LiveSectionObserver>>;

#[derive(Default)]
pub(crate) struct ObserverCache {
    sections: Option<SharedSections>,
    observer: Option<SharedObserver>,
}

impl ObserverCache {
    pub(in crate::game) fn begin(
        &mut self,
        sections: &mut EntitySections,
        scope: Option<Scope>,
    ) -> (Option<SharedSections>, Option<SharedObserver>) {
        let shared = self.sections.get_or_insert_with(Default::default);
        std::mem::swap(sections, &mut shared.borrow_mut());
        let observer = scope.map(|scope| {
            let observer = self.observer.get_or_insert_with(|| {
                Rc::new(RefCell::new(LiveSectionObserver::new(
                    Rc::clone(shared),
                    scope,
                )))
            });
            {
                let mut live = observer.borrow_mut();
                live.scope = scope;
                live.current = None;
            }
            Rc::clone(observer)
        });
        (Some(Rc::clone(shared)), observer)
    }
}

#[cfg(all(test, feature = "alloc-count", not(feature = "system-allocator")))]
mod tests {
    use super::*;

    #[test]
    fn subscribed_observer_pass_allocates_only_on_first_use() {
        use sunrise_threads::alloc_count::here;
        let mut cache = ObserverCache::default();
        let mut sections = EntitySections::default();
        let scope = Scope {
            epoch: mod_api::WorldEpoch::new(1),
            level: crate::game::Dim::Overworld.key(),
        };
        for expected in [2, 0, 0] {
            let before = here();
            let (table, observer) = cache.begin(&mut sections, Some(scope));
            drop(observer);
            std::mem::swap(&mut sections, &mut table.unwrap().borrow_mut());
            let counts = here().since(before);
            assert_eq!(counts.allocs, expected);
            assert_eq!(counts.frees, 0);
        }
    }
}
