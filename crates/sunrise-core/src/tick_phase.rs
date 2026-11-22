//! Whether this thread is doing a tick phase's work; in sunrise-core so dylibs share one mark without the allocator crate.

thread_local! {
    static IN_TICK_PHASE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether this thread runs a tick phase's work now (`sunrise_client::parallel::phase`): tick-owned state refuses it by name.
pub fn in_tick_phase() -> bool {
    IN_TICK_PHASE.get()
}

/// Runs `f` as a tick phase's work ([`in_tick_phase`]), restoring the previous mark even when `f` panics.
pub fn as_tick_phase<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            IN_TICK_PHASE.set(self.0);
        }
    }
    let _restore = Restore(IN_TICK_PHASE.replace(true));
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_holds_inside_and_is_restored_after_a_panic() {
        assert!(!in_tick_phase());
        assert!(as_tick_phase(in_tick_phase));
        assert!(as_tick_phase(
            || as_tick_phase(in_tick_phase) && in_tick_phase()
        ));
        assert!(std::panic::catch_unwind(|| as_tick_phase(|| panic!("in a phase"))).is_err());
        assert!(!in_tick_phase());
    }
}
