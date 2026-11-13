//! Tanks.
use crate::Chemical;

/// A tank of one chemical.
#[derive(Clone, Copy, Debug, PartialEq, Eq, mod_api::Wire)]
pub struct Tank {
    pub chemical: Chemical,
    pub amount: u32,
    pub capacity: u32,
}

impl Tank {
    /// How much more the tank takes.
    pub fn space(&self) -> u32 {
        self.capacity - self.amount
    }
}

/// A tank's own bookkeeping, never derived `Wire`: an api listener can't carry it.
pub struct Ledger {
    pub entries: Vec<u32>,
}

/// Records a transfer.
pub fn record(ledger: &mut Ledger, moved: u32) {
    ledger.entries.push(moved);
}
