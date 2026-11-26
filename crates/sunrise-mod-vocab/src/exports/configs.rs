mod configs;
pub use crate::configs::{
    ConfigEvent, ConfigListener, ConfigRegistrationError, ConfigSnapshot, ConfigSpec,
};

mod __private {
    pub use crate::configs::RegisteredConfig;
}
