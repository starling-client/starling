mod __private {
    mod game {
        pub use crate::world::criteria::run_criterion_test;
    }
}

pub use world::criteria::{
    CriterionContext, CriterionTest, CriterionTestError, TriggerCriterionError,
};
