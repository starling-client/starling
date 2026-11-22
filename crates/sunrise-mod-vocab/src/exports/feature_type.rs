pub use world::feature_type::{FeatureContext, FeatureError, FeatureType};
mod __private {
    /// The registered feature types, for the executable to hand to the engine's feature decoding
    /// before any world opens.
    pub use crate::world::feature_type::{decode_feature_config, feature_type_declarations};
}
