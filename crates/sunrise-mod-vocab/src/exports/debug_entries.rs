pub use client::debug_entries::{DebugEntry, DebugEntryLines, DebugGroup, DebugRenderer};
mod __private {
    pub use crate::client::debug_entries::DebugEntryStates;
    mod game {
        pub use crate::client::debug_entries::debug_renderer_callback_type;
    }
}
