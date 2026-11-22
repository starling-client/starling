use crate::ClientHost;
use std::sync::Arc;

type Task = Box<dyn FnOnce(&mut dyn ClientHost) + Send>;
type Submit = dyn Fn(Task) + Send + Sync;

/// A handle to the client's task drain, obtained when `host.client_executor()` returns `Some`.
/// Clone it into workers; it never borrows a host or a connection.
// no wire: submits closures to the live client in this process.
// exhaustive: its field is private, so mods cannot build or destructure it.
#[derive(Clone)]
pub struct ClientExecutor(Arc<Submit>);

impl ClientExecutor {
    /// Game-side constructor for the calling mod's executor.
    // hidden: only the game can supply the calling mod’s task queue.
    #[doc(hidden)]
    pub fn new(submit: impl Fn(Task) + Send + Sync + 'static) -> Self {
        Self(Arc::new(submit))
    }

    /// Queues `task` for the executor phase after the packet processor; it never runs
    /// inline, even on the client thread. Tasks get a live host in submission order.
    /// The packet phase also handles packets and queued payload work published during
    /// packet handling. If any packet work is deferred, executor tasks wait too.
    /// Move owned data into the task, not the caller's host or payload context.
    /// Quitting drops tasks that have not run. The executor adds no transaction: effects
    /// already applied to the live client, such as audio placement, are not undone on panic.
    /// A panic is logged and shown in the shared error toast, naming the mod. It disables
    /// this mod's executor for the rest of the client session: pending tasks for this mod
    /// are skipped and retained executors discard new tasks. Other mods' tasks still run.
    /// A phase refusal is reported without disabling the executor.
    pub fn execute(&self, task: Box<dyn FnOnce(&mut dyn ClientHost) + Send>) {
        (self.0)(task);
    }
}
