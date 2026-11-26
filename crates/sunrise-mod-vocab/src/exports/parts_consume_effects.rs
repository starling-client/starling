//! Exports for custom consume-effect registrations.
pub use crate::parts::ConsumeEffectType;
mod __private {
    pub use crate::parts::{ConsumeEffectCodecs, RegisteredConsumeEffect, consume_effect_codecs};
    pub use crate::runtime::ConsumeEffectBinding;
}
