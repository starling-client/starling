use super::*;
/// The ticket controllers registered at startup.
pub(crate) fn installed_controllers() -> &'static [ControllerInfo] {
    installed().map_or(&[], |installed| installed.controllers.as_slice())
}

/// Registrar operations for tickets.
pub trait TicketsRegistrar {
    /// Register ticket controller `name`, the way NeoForge's `RegisterTicketControllersEvent`
    /// does once at startup, and keep the returned [`TicketController`] handle for
    /// `chunk.set_mod_forced` and `level.mod_forced_chunks`. A bare name (`quarry`) uses
    /// the mod's namespace; a valid explicit `namespace:path` is used as-is. Registration
    /// and its validator are attributed to the caller, independently of the key. Malformed
    /// or duplicate IDs refuse registration. Pass `Some(Arc::new(...))` to bind a
    /// [`LoadingValidationCallback`] to this controller, or `None` to restore its saved
    /// tickets without validation. The returned handle itself authorizes active chunk-ticket
    /// operations and may be shared across callers. A world's tickets of a controller no loaded
    /// mod registers are dropped when it loads.
    fn register_ticket_controller(
        &mut self,
        name: &str,
        validator: Option<Arc<LoadingValidationCallback>>,
    ) -> TicketController;
}
