//! Configuration tasks and capabilities valid before the player joins.
use crate::callback_owner::CallbackOwner;
use crate::{Identifier, PayloadBytes, PayloadId, RegisterConfigurationTasksEvent, ServerHost};
use std::{
    any::{Any, TypeId},
    collections::HashMap,
};

// The number of concrete native task types is fixed by the installed code, not by peers.
pub(crate) type TaskOwners = HashMap<(&'static str, TypeId), CallbackOwner>;

// no wire: tasks are owned objects with server-thread callbacks and local state.
/// One task instance owned by a configuring connection. Register the instance with
/// [`RegisterConfigurationTasksEvent::register`]. The server runs it in queue order,
/// on the server thread without holding the connection's task lock, and keeps the
/// same instance while it waits. Callbacks run as the registering mod. A disabled
/// implementation is skipped. A panic in a task's type, start or tick callback
/// refuses that peer and disables the task implementation for that mod for the
/// session, without disabling other mods' tasks. Dropping a task also runs as
/// the registering mod. A destructor panic is reported and disables its
/// implementation, even if the task was rejected or skipped.
pub trait ConfigurationTask: Send + 'static {
    /// The namespaced task type, read once as the registering mod.
    fn task_type(&self) -> &Identifier;

    /// Start this task exactly once. Call [`ConfigurationTaskContext::finish`] or
    /// [`ConfigurationTaskContext::fail`], or leave it pending for a tick or a reply.
    /// A finish request publishes only after the outermost transaction commits and
    /// publishes its deferred sends.
    fn start(&mut self, host: &mut dyn ServerHost, context: &mut ConfigurationTaskContext<'_>);

    /// Poll a pending task, at most once per server tick after its start. By default
    /// the task just waits. As in [`Self::start`], use the context to send, finish or
    /// fail. A failure or panic rolls back the callback and fails admission.
    fn tick(&mut self, _host: &mut dyn ServerHost, _context: &mut ConfigurationTaskContext<'_>) {}
}

impl RegisterConfigurationTasksEvent<'_> {
    /// Append a task to this connection's FIFO queue. A failed native callback
    /// refuses this peer's configuration; a disabled implementation is skipped.
    pub fn register(&mut self, task: impl ConfigurationTask) {
        let Some((mod_id, installed)) =
            crate::content::crash::current_native_mod().zip(crate::runtime::installed())
        else {
            self.registration
                .failure
                .get_or_insert_with(|| "Configuration task registration has no owning mod".into());
            return;
        };
        let owner = installed
            .configuration_task_owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry((mod_id, (&task as &dyn Any).type_id()))
            .or_insert_with(|| CallbackOwner::for_mod(mod_id))
            .clone();
        if !owner.is_enabled() {
            dispose_task(&owner, task);
            return;
        }
        match owner.call("configuration task", "ConfigurationTask::task_type", || {
            task.task_type().clone()
        }) {
            Ok(kind) => self.registration.tasks.push(RegisteredConfigurationTask {
                kind,
                task: Some(Box::new(task)),
                owner,
            }),
            Err(reason) => {
                self.registration.failure.get_or_insert(reason);
                dispose_task(&owner, task);
            }
        }
    }
}

fn dispose_task<T>(owner: &CallbackOwner, task: T) {
    owner.dispose("configuration task", "ConfigurationTask::drop", task);
}

/// Engine staging for one registration transaction.
// no wire: owns pending server-thread tasks.
// hidden: the server stages registrations here; listeners use the scoped event.
// exhaustive: the server starts empty staging with Default; the queue and panic diagnostic fields are private.
#[doc(hidden)]
#[derive(Default)]
pub struct ConfigurationTaskRegistration {
    tasks: Vec<RegisteredConfigurationTask>,
    failure: Option<String>,
}
impl ConfigurationTaskRegistration {
    /// The first callback failure, if any.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// Checkpoint one listener's append-only staging before invoking it.
    pub fn checkpoint(&self) -> (usize, bool) {
        (self.tasks.len(), self.failure.is_some())
    }

    /// Discard only a panicking listener's tasks and diagnostic.
    pub fn rollback(&mut self, (len, had_failure): (usize, bool)) {
        let discarded = self.tasks.split_off(len);
        if !had_failure {
            self.failure = None;
        }
        for task in discarded {
            let owner = task.owner.id();
            crate::runtime::discard_registration(
                (
                    crate::HookId::RegisterConfigurationTasks,
                    "register_configuration_tasks",
                ),
                Some(owner),
                task,
            );
        }
    }

    /// Move the ordered queue to the server after registration commits.
    pub fn into_tasks(self) -> Vec<RegisteredConfigurationTask> {
        self.tasks
    }
}

/// Engine queue entry with the callback's mod scope resolved at registration.
// no wire: an owned task and its process-local mod identity.
// hidden: the server owns these queue entries; listeners register through the scoped event.
#[doc(hidden)]
#[non_exhaustive]
pub struct RegisteredConfigurationTask {
    kind: Identifier,
    task: Option<Box<dyn ConfigurationTask>>,
    /// The registering mod, for Host calls that depend on the caller's identity.
    pub owner: CallbackOwner,
}
impl RegisteredConfigurationTask {
    /// The task type captured under its owner at registration.
    pub fn task_type(&self) -> &Identifier {
        &self.kind
    }

    /// Invoke the live connection-owned instance as its registering mod.
    pub fn call<T>(
        &mut self,
        operation: &'static str,
        invoke: impl FnOnce(&mut dyn ConfigurationTask) -> T,
    ) -> Result<T, String> {
        let task = self.task.as_deref_mut().expect("live configuration task");
        self.owner
            .call("configuration task", operation, || invoke(task))
    }
}
impl Drop for RegisteredConfigurationTask {
    fn drop(&mut self) {
        dispose_task(&self.owner, self.task.take());
    }
}

/// One configuring connection, available while registering its tasks.
// no wire: a local capability for this connection, not payload data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConfigurationConnection {
    incarnation: u64,
    id: u64,
}
impl ConfigurationConnection {
    /// Whether this peer negotiated the registered configuration payload channel.
    pub fn has_channel(self, host: &dyn ServerHost, channel: &PayloadId) -> bool {
        host.configuration_has_channel(self, channel)
    }
}

/// The active task during one server-thread start or tick callback.
// no wire: borrows the callback's outcome.
#[non_exhaustive]
pub struct ConfigurationTaskContext<'a> {
    connection: ConfigurationConnection,
    outcome: &'a mut ConfigurationTaskOutcome,
}
impl ConfigurationTaskContext<'_> {
    /// Whether this task's peer negotiated the registered configuration channel.
    pub fn has_channel(&self, host: &dyn ServerHost, channel: &PayloadId) -> bool {
        self.connection.has_channel(host, channel)
    }
    /// Queue a payload in the current transaction on this connection's registered
    /// configuration channel. Another handler completing the task does not prevent
    /// sends on a connection that is still configuring. Returns
    /// [`ConfigurationTaskError::Disconnected`] if it has moved to Play.
    pub fn send_payload(
        &self,
        host: &mut dyn ServerHost,
        channel: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<(), ConfigurationTaskError> {
        host.configuration_send_payload(self.connection, channel, bytes)
    }
    /// Request completion when this callback finishes and publishes its deferred sends.
    /// Repeated requests coalesce; failure, panic or disconnect prevents it.
    pub fn finish(&mut self) {
        if matches!(self.outcome, ConfigurationTaskOutcome::Pending) {
            *self.outcome = ConfigurationTaskOutcome::Finish;
        }
    }
    /// Fail configuration for this peer and discard the callback's changes and any finish
    /// request. The first explicit failure supplies the diagnostic.
    pub fn fail(&mut self, reason: &str) {
        if !matches!(self.outcome, ConfigurationTaskOutcome::Fail(_)) {
            *self.outcome = ConfigurationTaskOutcome::Fail(reason.into());
        }
    }
}

/// A task operation was refused.
// no wire: local task and transport diagnostics, not payload data.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigurationTaskError {
    /// The connection or its configuration epoch has ended.
    Disconnected,
    /// The running task on this connection has a different type.
    WrongType,
    /// A completion was requested but no configuration task is currently running
    /// on this connection (or this handler already requested completion).
    NotRunning,
    /// Completion requires a server configuration payload handler.
    WrongContext,
    /// The registered payload transport refused the send.
    Payload(crate::PayloadSendError),
}
impl crate::NoBackend for ConfigurationTaskError {
    fn no_backend() -> Self {
        Self::Disconnected
    }
}

// hidden: the engine reads the callback outcome when closing its transaction.
// no wire: the engine's local decision for one callback.
// exhaustive: matched only by the engine.
#[doc(hidden)]
pub enum ConfigurationTaskOutcome {
    Pending,
    Finish,
    Fail(String),
}

// hidden: the engine makes capabilities after allocating a connection identity.
#[doc(hidden)]
pub fn configuration_connection(incarnation: u64, id: u64) -> ConfigurationConnection {
    ConfigurationConnection { incarnation, id }
}
// hidden: the engine resolves and validates a capability before every operation.
#[doc(hidden)]
pub fn configuration_connection_parts(connection: ConfigurationConnection) -> (u64, u64) {
    (connection.incarnation, connection.id)
}
// hidden: the engine lends the callback its outcome slot.
#[doc(hidden)]
pub fn configuration_task_context(
    connection: ConfigurationConnection,
    outcome: &mut ConfigurationTaskOutcome,
) -> ConfigurationTaskContext<'_> {
    ConfigurationTaskContext {
        connection,
        outcome,
    }
}
