pub use entity::entity_tags::EntityTags;

mod __private {
    mod game {
        pub use crate::entity::entity_tags::borrow_entity_tags;
    }
}
