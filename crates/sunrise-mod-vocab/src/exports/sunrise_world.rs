/// The structure box a piece's `bounding_box` answers, in world blocks.
pub use sunrise_core::bounding_box::BoundingBox;
pub use sunrise_core::connection_kind::ConnectionKind;
pub use sunrise_core::dimension::Dimension as LevelDimension;
pub use sunrise_core::equipment_slot::EquipmentSlot;
/// An owned, validated resource name, not a process-local registry key.
/// In Wire (including typed payloads), the namespace and path are two separate UTF-8 strings.
pub use sunrise_core::identifier::Identifier;
/// A level's key (`LevelDimension::Custom`).
pub use sunrise_core::level_key::LevelKey;
pub use sunrise_core::nbt::Compound;
/// The NBT string: every `Compound` key and `Tag::String`, held as UTF-16 so it can keep Java's unpaired surrogates.
pub use sunrise_core::nbt::JavaString;
pub use sunrise_core::nbt::Tag;
pub use sunrise_core::neighbour_update::NeighbourUpdate;
pub use sunrise_core::use_animation::UseAnimation;
pub use sunrise_mod_runtime::attachments::{
    AttachmentCodec, AttachmentError, AttachmentPolicy, Holder,
};
// hidden: the game's storage backends, not part of the typed mod surface.
#[doc(hidden)]
pub use sunrise_mod_runtime::attachments::{
    AttachmentAccess, AttachmentBinding, AttachmentEncodeError, AttachmentHost, AttachmentLoan,
    AttachmentStore, AttachmentStoreKey, AttachmentStoreWrite, Attachments,
};
