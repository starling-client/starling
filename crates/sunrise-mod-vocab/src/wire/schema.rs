//! The structural fingerprint of the bytes a Wire codec writes.
use std::any::TypeId;

/// Walks the layouts of a Wire value. Only named records occupy the ancestor stack, so a
/// recursive field points back to the same depth independently of Rust names or type aliases.
/// Compute this at registration or table build time, not for every value.
// exhaustive: its only state, the ancestor stack, is private.
// no wire: a live layout walk, not a value.
#[derive(Default)]
pub struct WireSchema {
    ancestors: Vec<TypeId>,
    ordinary_only: bool,
}

impl WireSchema {
    // hidden: generated Wire codecs mark conversions that cannot use the payload budget.
    #[doc(hidden)]
    pub fn ordinary_only(&mut self) {
        self.ordinary_only = true;
    }

    pub(crate) fn admits_payload(&self) -> bool {
        !self.ordinary_only
    }
    /// A format with no child fields. Use a distinct versioned name for a different encoding.
    pub const fn leaf(name: &str) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let bytes = name.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            hash = (hash ^ bytes[i] as u64).wrapping_mul(0x0000_0100_0000_01b3);
            i += 1;
        }
        hash
    }

    /// Append a count, tag or child fingerprint to a format.
    pub const fn mix(mut hash: u64, child: u64) -> u64 {
        let bytes = child.to_le_bytes();
        let mut i = 0;
        while i < bytes.len() {
            hash = (hash ^ bytes[i] as u64).wrapping_mul(0x0000_0100_0000_01b3);
            i += 1;
        }
        hash
    }

    /// Describe a record, emitting a backreference rather than descending into an ancestor.
    pub fn named<T: 'static>(&mut self, describe: impl FnOnce(&mut Self) -> u64) -> u64 {
        let id = TypeId::of::<T>();
        if let Some(level) = self.ancestors.iter().rposition(|entered| *entered == id) {
            return Self::mix(
                Self::leaf("recursive"),
                (self.ancestors.len() - level) as u64,
            );
        }
        self.ancestors.push(id);
        let hash = describe(self);
        self.ancestors.pop();
        hash
    }
}
