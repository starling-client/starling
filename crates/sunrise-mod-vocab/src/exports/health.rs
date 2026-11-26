pub use crate::entity::health::{
    DamageContainer, DamageContainerView, DamageWriteRefusal, HurtSource, LivingView, Reduction,
    ShieldBlock, Victim,
};

mod __private {
    pub use crate::entity::health::{
        DamageContainerBackend, DamageContainerRead, LivingParty, LivingReads,
        wrap_reduction_modifier,
    };
}
