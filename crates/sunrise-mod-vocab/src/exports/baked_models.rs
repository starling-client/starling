pub use client::baked_models::{
    BakedModelKey, BakedModelKeyIter, BakedModelKeys, BakedModelView, BakedQuad,
};
mod __private {
    mod built {
        pub use crate::client::baked_models::{baked_model_keys, baked_model_view, baked_quad};
    }
    mod game {
        pub use crate::client::baked_models::BakedQuadSource;
    }
}
