//! Initial client configuration identity declaration (see `hooks/mod.rs`).

pub trait Hooks {
    /// Choose what the client reports to the server at initial configuration
    /// (`ClientHandshakePacketListenerImpl.handleLoginFinished`,
    /// `ClientBrandRetriever.getClientModName`). Runs on the network thread once
    /// per join that reaches configuration. A transfer joins a new destination;
    /// play-to-configuration transitions and the integrated server do not fire it.
    ///
    /// [`ClientIdentityHost::server_address`] gives the original target, not the
    /// resolved host or port. This Host has no game state or packet sending.
    /// Return `None` to abstain. The first `Some` (by listener priority, then
    /// dependency order) replaces the whole default identity, including any extra
    /// payloads. With no answer the layer's identity is used, or `vanilla` with no
    /// extras if the layer has none.
    ///
    /// The brand goes first, then the returned payloads in order, then client
    /// information. The vanilla packet codec's limits apply. Reserved or
    /// transport-owned channels and encoding errors fail the join.
    #[hook(
        dispatch = "flat",
        side = "client",
        contract = "replace",
        out = "identity",
        context = "client_identity",
        target = "sunrise_client_lib::net::native_client_identity",
        neoforge = "none (NeoForge patches ClientBrandRetriever.getClientModName to use BrandingControl.getClientBranding, without a per-address event)"
    )]
    fn client_identity(identity: &mut Option<ClientIdentity>);
}
