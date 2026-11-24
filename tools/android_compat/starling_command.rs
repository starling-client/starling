//! Starling's port to Sunrise's API: Horizon's `run_mod_command` hook, which got every command of
//! the mod in one event, is a per-usage executor in Sunrise. The old event is rebuilt here around
//! the executor's arguments, so the mod's command code runs as it was written.
use mod_api::{CommandInvocation, CommandSource, Flow, ServerHost};

/// Horizon's `RunModCommandEvent`: the parsed command and who ran it.
pub(crate) struct RunModCommandEvent<'a> {
    pub command: &'a CommandInvocation,
    pub source: &'a CommandSource,
    /// Whether an earlier mod took the command: in Sunrise each usage has its own executor, so never.
    pub cancelled: bool,
}

/// The executor of every usage the mod declares: the old hook's body (`run`), whose `Cancel` was a
/// command it ran and `Continue` one it did not.
pub(crate) fn execute(
    host: &mut dyn ServerHost,
    source: &CommandSource,
    command: &CommandInvocation,
    run: fn(&mut dyn ServerHost, &mut RunModCommandEvent<'_>) -> Flow,
) -> Option<i32> {
    let mut event = RunModCommandEvent {
        command,
        source,
        cancelled: false,
    };
    match run(host, &mut event) {
        Flow::Continue => None,
        _ => Some(1),
    }
}
