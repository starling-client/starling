//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `mod_state` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait ModStateHost {
    /// Side: both.
    /// Capture a SNAPSHOT of the current dimension's attachment residency and values.
    /// Later changes require a fresh query. Removed-but-unswept entities may appear;
    /// writes through stale holders fail. Values are neither cloned nor eagerly decoded.
    /// The batch is local, not a save or sync record. In a transaction it can be used
    /// only while the capturing frame is current. An undeclared id is refused with
    /// the calling package and attachment id.
    fn attachment_query(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<AttachmentBatch, AttachmentError>;
    /// Side: both.
    /// Borrow this host for an attachment snapshot read in its transaction. Pass the
    /// result to `row.with_value` or `query.get`; it does not open a transaction.
    fn attachment_read_scope(&self) -> Result<AttachmentReadScope<'_>, AttachmentRowError>;
    /// Side: both.
    /// The server-owned holders' attachments; `TYPE.get(host, holder)` reads
    /// one. On a client it is the copy of what the server synced: `set` and
    /// `remove` change only the copy. While held during a proxy handoff, the
    /// client can already read updates or removals for entities and other
    /// players it tracks, without waiting for placement or requesting a resync.
    fn attachments(&mut self) -> &mut dyn AttachmentStore;
}
/// The server host's methods of the `mod_state` area, implemented by the game and reached through [`ServerHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ModStateServerHost {
    /// Side: server.
    /// Validate the capture's transaction, if any, and residency in the current dimension
    /// immediately before an ordinary journaled attachment write. The token comes from a
    /// captured row, not from disk or the network. Returns `Unavailable` outside the
    /// world phase. In the world phase, `OutsideTransaction` for an expired frame takes
    /// precedence over `StaleHolder` for missing or mismatched residency. This does not
    /// open a transaction.
    fn attachment_validate_write(
        &self,
        token: AttachmentWriteToken,
    ) -> Result<(), AttachmentRowError>;
    /// Side: server.
    /// Opens a transaction, nested in the open one if any (`Transaction.open`), and gives back
    /// the seal that ends it. `host.transaction` calls it; use that instead.
    fn transaction_open(&mut self) -> TransactionSeal;
    /// Side: server.
    /// Closes the transaction `seal` ends, committed or aborted, aborting first any opened
    /// inside it and left open. Refused when `seal` names no open transaction, so a mod ends
    /// only a transaction it opened. `host.transaction` calls it.
    fn transaction_close(&mut self, seal: &TransactionSeal, commit: bool);
    /// Side: server.
    /// Where the transaction system is (`Transaction.getLifecycle`): none open, one open, a
    /// nested one closing, or the outermost closing while the game tells the world of a commit.
    fn transaction_lifecycle(&self) -> TransactionLifecycle;
    /// Side: server.
    /// The innermost open transaction, to write in (`Transaction.getCurrentOpenedTransaction`);
    /// none when none is open. It names the transaction only: ending it still takes the seal its
    /// opener holds.
    fn transaction_current(&self) -> Option<Transaction>;
    /// Side: server.
    /// How deeply `tx` is nested (`TransactionContext.depth`): 0 for the outermost, none for a
    /// transaction that is not open.
    fn transaction_depth(&self, tx: &Transaction) -> Option<u32>;
    /// Side: server.
    /// Runs `task` once after `delay_ticks` server ticks, matching vanilla's
    /// `MinecraftServer.schedule(TickTask)` / `BlockableEventLoop.runAllTasks`. Due tasks drain to
    /// exhaustion after both the `server_tick` start hook and the `server_tick` end hook, so
    /// `delay_ticks == 0` runs in the current tick (including `0`-delay tasks scheduled from inside
    /// a running task). A paused server does not advance the delay. Tasks due together run in the
    /// order they were scheduled.
    ///
    /// Move owned data into the closure, not the current host or event. The task gets a fresh
    /// server host on the same thread. When scheduled inside a mod call, it runs (and drops on
    /// cancel or shutdown) under that mod's identity. A panic rolls back its journaled writes,
    /// disables that mod's delayed callbacks for the session, and names the mod in the log and
    /// in a system toast offered to `toast_add` in the integrated server owner's window.
    /// The tick and other mods' tasks carry on. If a capture's destructor panics on cancellation
    /// or world close, the same scheduling owner loses its delayed callbacks for the session and
    /// is named in the log and in a system toast that bypasses `toast_add`. Cancellation and
    /// world cleanup continue. A phase refusal rolls back the task and is logged each time
    /// without disabling it.
    ///
    /// Tasks whose mod is disabled, or whose delayed callbacks have been disabled, are discarded
    /// when due. Reaching [`MAX_PENDING_TASKS`] logs a warning naming the mod and returns
    /// [`ScheduleError::TooMany`]. Calls outside a mod call also succeed without a shared global
    /// cap. Pending tasks are dropped with the world and are not saved.
    fn schedule_task(
        &mut self,
        delay_ticks: u32,
        task: Box<dyn FnOnce(&mut dyn ServerHost)>,
    ) -> Result<TaskId, ScheduleError>;
    /// Side: server.
    /// Withdraws the waiting task identified by the opaque capability `id`: returns `true` if it
    /// was still waiting. A panic while dropping an attributed task's captures is reported against
    /// the scheduling mod, disables that mod's delayed callbacks and does not unwind through the
    /// caller.
    fn cancel_task(&mut self, id: TaskId) -> bool;
}
