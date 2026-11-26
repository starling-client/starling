pub mod tags;
pub use world::tags::{PotionKey, StructureKey, TagKey, TagMembers, Tags, WorldClockKey};
mod __private {
    pub use crate::world::tags::{tag_key, with_tag_name};
}
