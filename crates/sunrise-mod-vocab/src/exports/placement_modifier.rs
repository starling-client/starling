pub use world::placement_modifier::{
    PlacementModifier, PlacementModifierContext, PlacementModifierDecodeContext,
    PlacementModifierRef, PlacementModifierType,
};
mod __private {
    /// The registered placement modifier types, and the handles the game's
    /// adapter builds and reads back.
    pub use crate::world::placement_modifier::{
        placement_modifier_payload, placement_modifier_ref, placement_modifier_type_declarations,
    };
}
