/// The structure box a piece's `bounding_box` answers, in world blocks.
pub use sunrise_core::bounding_box::BoundingBox;
pub use sunrise_core::equipment_slot::EquipmentSlot;
pub use sunrise_core::identifier::Identifier;
/// A level's key (`LevelDimension::Custom`).
pub use sunrise_core::level_key::LevelKey;
pub use sunrise_core::nbt::Compound;
pub use sunrise_core::nbt::Tag;
pub use sunrise_core::use_animation::UseAnimation;
pub use {
    sunrise_core::{
        connection_kind::ConnectionKind, dimension::Dimension as LevelDimension,
        neighbour_update::NeighbourUpdate,
    },
    sunrise_mod_runtime::attachments::{
        AttachmentCodec, AttachmentError, AttachmentForm, AttachmentPolicy, AttachmentStore,
        AttachmentType, AttachmentValue, AttachmentWrite, Holder,
    },
};
// hidden: the game's maps behind the store, not part of the mod surface
#[doc(hidden)]
pub use sunrise_mod_runtime::attachments::{AttachmentAccess, AttachmentHost, Attachments};
