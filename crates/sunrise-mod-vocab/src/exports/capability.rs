pub(crate) use item::capability::BlockCapabilityDefault;
// hidden: generated host forwarders pass the resolved engine lookup, not a mod-facing value
#[doc(hidden)]
pub use item::capability::BlockCapabilitySite;
pub use item::capability::{BlockCapabilityOffer, BlockSide, Capability, CapabilityKey, ITEMS};
mod __private {
    pub use crate::item::capability::item_capability_present;
}
