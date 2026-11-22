//! Operations reached through ConfigurationConnection and ConfigurationTaskContext.
#![allow(unused_variables)]
use crate::*;

/// Server-side configuration capabilities before a player handle exists.
#[sunrise_mod_api_macros::host_area]
pub trait ConfigurationTasksServerHost {
    /// Side: server. Whether the peer negotiated this configuration channel.
    fn configuration_has_channel(
        &self,
        connection: ConfigurationConnection,
        channel: &PayloadId,
    ) -> bool;
    /// Side: server. Queue a send on the registered configuration channel for this
    /// connection while its configuration epoch is active, regardless of current
    /// task state. A saved connection returns [`ConfigurationTaskError::Disconnected`]
    /// after it moves to Play. Prefer [`ConfigurationTaskContext::send_payload`] in
    /// start and tick callbacks.
    fn configuration_send_payload(
        &mut self,
        connection: ConfigurationConnection,
        channel: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<(), ConfigurationTaskError>;
}
