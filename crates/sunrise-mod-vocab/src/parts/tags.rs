use crate::TagKey;

/// Name tags for use by Parts and other values constructed during registration.
pub trait TagsRegistrar {
    /// Resolves a tag name as [`Tags::key`](crate::Tags::key) does. A bare name uses the
    /// `minecraft` namespace. Item, block and fluid tags share name keys; the membership
    /// method selects the registry. The tag need not exist or have any members yet.
    ///
    /// Retain the key across data reloads. An invalid name or unavailable name slot
    /// refuses the mod at load.
    fn tag_key(&mut self, id: &str) -> TagKey;
}
