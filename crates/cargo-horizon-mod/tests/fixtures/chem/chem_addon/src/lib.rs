//! An addon patching chemlib through its mod points: an api listener that doubles the rate of
//! every transfer, and an internals listener that sees the moved amount.
use std::sync::atomic::{AtomicU32, Ordering};

/// The amount every transfer moved, as the internals return listener saw it.
pub static MOVED: AtomicU32 = AtomicU32::new(0);

/// The rate chemlib's private `flow` last ran with, as its api listener saw it.
pub static FLOW_RATE: AtomicU32 = AtomicU32::new(0);

/// At `chemlib::transfer`'s head (`access = "api"`): the record holds copies of the tanks and
/// the rate; changing the rate changes what the function runs with.
pub fn double_rate(args: &mut chemlib::__horizon_points::TransferArgs) {
    args.rate *= 2;
}

/// At the head of `chemlib::flow`, a private function (`access = "api"`): private functions are
/// points too.
pub fn saw_flow(args: &mut chemlib::__horizon_points::FlowArgs) {
    FLOW_RATE.store(args.rate, Ordering::Relaxed);
}

/// At `chemlib::transfer`'s return (`access = "internals"`): the library's own value, by reference.
pub fn saw_moved(moved: &mut u32) {
    MOVED.fetch_add(*moved, Ordering::Relaxed);
}

/// At `chemlib::tanks::record`'s head (`access = "internals"`): the ledger has no Wire form, but
/// an internals listener gets it by reference, in its own type.
pub fn mark_ledger(_: &mut chemlib::tanks::__TanksRecordRaw<'_, &mut chemlib::tanks::Ledger, u32>) {
}
