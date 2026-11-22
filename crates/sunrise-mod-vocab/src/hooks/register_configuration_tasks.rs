//! Tasks registered before admitting a connecting player.
pub trait Hooks {
    /// Append tasks for this connecting profile, on the server thread after the
    /// built-in configuration steps. Runs for every peer, including vanilla.
    /// The profile has no player handle yet. Use `event.connection.has_channel`
    /// to check what the peer negotiated and `event.register(task)` to give this
    /// connection an owned task. Tasks start in registration order, including tasks of the same
    /// type, and can finish, wait or fail this peer.
    /// A panic while reading a task's type refuses admission and disables that task implementation
    /// for its mod. If a registration listener itself panics, only its staged tasks, diagnostic
    /// and journaled writes are discarded. Other listeners still configure the same peer, and
    /// the panicking registration listener is skipped for later peers.
    /// Delayed tasks complete via `IPayloadContext::finish_configuration_task`
    /// with their task type from either a server main-thread or network-thread
    /// configuration payload handler.
    /// There is no task timeout: answering keep-alives can hold configuration
    /// indefinitely. A silent or disconnected connection still ends normally.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 3,
        neoforge = "RegisterConfigurationTasksEvent"
    )]
    fn register_configuration_tasks(
        connection: mod_api::ConfigurationConnection,
        player: u128,
        name: &str,
        registration: &mut mod_api::__private::ConfigurationTaskRegistration,
    );
}
