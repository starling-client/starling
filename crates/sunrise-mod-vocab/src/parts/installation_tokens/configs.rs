//! Engine lookup of frozen config declarations.
use crate::configs::RegisteredConfig;

/// The registered configs in declaration order, shared across worlds in this installation.
pub fn registered_configs() -> &'static [RegisteredConfig] {
    crate::parts::installed().map_or(&[], |installed| installed.configs.as_slice())
}
