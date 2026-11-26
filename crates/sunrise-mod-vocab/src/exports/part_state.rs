pub use block::part_state::{
    BlockEntityPartState, BlockEntityStateError, PartStateDeclaration, PartStateError,
    PartStateHolder, PartStateRefusal, edit_part_state, read_state,
};
pub use entity::part_state::{EntityPartState, EntityPartStateError, EntityResidency};
mod __private {
    pub use crate::block::part_state::part_state_matches_registration;
    pub use crate::entity::part_state::{
        EntityPartStateDeclaration, entity_part_state_declarations,
    };
}
