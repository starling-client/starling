//! Engine entry points for retained and session-owned callbacks.
mod __private {
    pub use crate::runtime::retained::{RetainedCallback, RetainedFailure};
    pub use crate::runtime::{call_session_callback, session_callback_enabled};
    pub mod game {
        pub use crate::runtime::discard_registration;
    }
}
