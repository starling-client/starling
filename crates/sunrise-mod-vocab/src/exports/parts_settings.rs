//! Exports of `crate::parts` items defined in `parts/settings.rs`.
pub use crate::parts::ConfigScreenFactory;
mod __private {
    pub use crate::parts::{
        create_config_screen, has_config_screen, setting, setting_category, setting_entry,
        setting_key, setting_key_by_name, settings,
    };
}
