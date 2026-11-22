use client::model_loader::ModelLoaderRegistration;
pub use client::model_loader::{
    ModelLoaderFn, ModelLoaderInput, ModelLoaderQuad, ModelLoaderSchema, ModelLoaderTexture,
};
mod __private {
    mod game {
        pub use crate::client::model_loader::ModelLoaderRegistration;
    }
}
