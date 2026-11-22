//! The `validate_tickets` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `dimension`'s level has loaded the saved chunk tickets of `controller`
    /// (NeoForge's `LoadingValidationCallback.validateTickets`), before its
    /// first tick and after `level_load`, once for each open and each
    /// controller registered with a validator that has tickets in the level.
    /// At startup all built-in levels are available before validation begins.
    /// `validation` holds that controller's tickets, block owners and entity
    /// owners apart; the mod that owns the controller removes the ones that no
    /// longer hold (a machine that is gone), and they stop forcing their
    /// chunks. A callback's removals are visible to that callback immediately,
    /// but are discarded if it panics or incurs a phase refusal. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        staged_surface = "validation",
        neoforge = "LoadingValidationCallback",
        cancellable = false,
        version = 3
    )]
    fn validate_tickets(
        dimension: &str,
        controller: TicketController,
        validation: &mut dyn mod_api::TicketValidation,
    );
}
