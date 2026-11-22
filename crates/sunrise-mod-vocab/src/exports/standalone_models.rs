use client::standalone_models::StandaloneModelRegistration;
pub use client::standalone_models::{StandaloneModelBakeContext, StandaloneModelBakeError};
mod __private {
    mod game {
        pub use crate::client::standalone_models::{
            StandaloneModelBaker, StandaloneModelRegistration, standalone_model_bake_context,
        };
    }
}
