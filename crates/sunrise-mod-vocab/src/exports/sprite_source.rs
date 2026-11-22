mod sprite_source;
use sprite_source::SpriteSourceRegistration;
pub use sprite_source::{ResourceReader, SpriteSourceFn, SpriteSourceInput, SpriteSourceOutput};
mod __private {
    mod game {
        pub use crate::sprite_source::SpriteSourceRegistration;
    }
}
