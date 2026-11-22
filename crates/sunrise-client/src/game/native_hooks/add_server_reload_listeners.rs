use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// Register server data reload listeners after packs are selected. The
    /// host gives registry access at registration, as
    /// `AddServerReloadListenersEvent.getRegistryAccess` does.
    pub(in crate::game) fn native_add_server_reload_listeners(
        &self,
        listeners: &mut mod_api::ServerListenerRegistry,
    ) {
        Self::fire_add_server_reload_listeners(self, |listen| listeners.register_listener(listen));
    }
}
