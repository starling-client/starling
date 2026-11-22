mod __private {
    // hidden: the game publishes its entity types' attribute suppliers once its catalog is built
    pub use crate::entity::entity_type_defaults::{
        entity_type_default_attribute, publish_entity_type_defaults,
    };
}
