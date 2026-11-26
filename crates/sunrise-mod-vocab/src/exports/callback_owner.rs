mod callback_owner;
mod __private {
    mod game {
        pub use crate::callback_owner::{
            CallbackOwner, CallbackValue, contain_unwind, error_chain, panic_payload_text,
            record_contained_panic, retained_callback_active,
        };
    }
}
