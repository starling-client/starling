// exports: none (reached inside the crate only)
//! Load-owned raw payload registrations. No connection state lives here.
use super::*;
use crate::{
    HandlerThread, IPayloadHandler, PayloadDeclaration, PayloadDirection, PayloadId,
    PayloadProtocol, PayloadSendError, PayloadToken,
};

#[derive(Clone)]
pub(crate) struct Registered {
    pub(crate) declaration: PayloadDeclaration,
    pub(crate) owner: &'static str,
    pub(crate) owner_index: usize,
    pub(crate) server: Option<Arc<dyn IPayloadHandler>>,
    pub(crate) client: Option<Arc<dyn IPayloadHandler>>,
    pub(crate) client_thread: HandlerThread,
    /// Set once the server callback panicked: it is never invoked again, on any connection.
    pub(crate) server_panicked: Arc<std::sync::atomic::AtomicBool>,
    /// The same for the client callback.
    pub(crate) client_panicked: Arc<std::sync::atomic::AtomicBool>,
}

// no wire: frozen process-local registration storage contains callback trait objects and owning-mod identities.
/// Immutable payload registry for engine connection setup and dispatch.
/// It is absent entirely when no mod registers a payload.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct Payloads {
    pub(crate) phases: [HashMap<String, Registered>; 2],
    /// Raw play channels by exact id, each with its declaring mods in load order.
    pub(crate) raw: HashMap<String, Vec<RawRegistered>>,
    /// Typed declarations by the slot their tokens carry, in registration order.
    pub(crate) typed: Vec<TypedSlot>,
    /// The load these declarations belong to, never 0 (a refused registration's); tokens of
    /// another load are stale.
    pub(crate) load: u64,
    /// The drop counters of each `Droppable` channel, shared by every copy of this table.
    pub(crate) drops: HashMap<String, Arc<DropCounters>>,
}

// no wire: engine-internal atomics, read by mods only through `PayloadDropCounts`.
/// Why `Droppable` sends of one channel were dropped, counted by the engine's queues.
#[derive(Default)]
#[non_exhaustive]
pub struct DropCounters {
    /// Waited past the stale limit.
    pub stale: std::sync::atomic::AtomicU64,
    /// Pushed out past the queue's byte watermark.
    pub watermark: std::sync::atomic::AtomicU64,
    /// Over the byte budget.
    pub budget: std::sync::atomic::AtomicU64,
    /// Still waiting at the switch to play or the end of the connection.
    pub switch: std::sync::atomic::AtomicU64,
}
impl DropCounters {
    /// The counts so far.
    pub fn read(&self) -> crate::PayloadDropCounts {
        use std::sync::atomic::Ordering::Relaxed;
        crate::PayloadDropCounts {
            stale: self.stale.load(Relaxed),
            watermark: self.watermark.load(Relaxed),
            budget: self.budget.load(Relaxed),
            switch: self.switch.load(Relaxed),
        }
    }
}

/// What a typed token must match: its declaration as registered, before protocol splitting.
#[derive(Clone)]
pub(crate) struct TypedSlot {
    pub(crate) id: PayloadId,
    pub(crate) protocol: PayloadProtocol,
    pub(crate) direction: PayloadDirection,
    pub(crate) schema: u64,
    pub(crate) owner_index: usize,
}

#[derive(Clone)]
pub(crate) struct RawRegistered {
    pub(crate) owner: &'static str,
    pub(crate) owner_index: usize,
    pub(crate) server: Option<Arc<dyn IPayloadHandler>>,
    /// Set once the server callback panicked: it is never invoked again.
    pub(crate) panicked: Arc<std::sync::atomic::AtomicBool>,
}

// no wire: resolved process-local callback and its owning mod's live enable guard.
/// A receiving callback resolved once while a connection freezes its table.
#[derive(Clone)]
#[non_exhaustive]
pub struct PayloadReceiver {
    /// The sole registered receiver.
    handler: Arc<dyn IPayloadHandler>,
    owner: &'static str,
    /// Its chosen executor.
    pub thread: HandlerThread,
    owner_index: usize,
    /// Shared by every connection's receiver of this callback.
    panicked: Arc<std::sync::atomic::AtomicBool>,
}

impl PayloadReceiver {
    /// The receiving mod's id, for diagnostics.
    pub fn owner(&self) -> &'static str {
        self.owner
    }
    /// Whether the mod itself is enabled. A poisoned receiver remains a declared
    /// channel; dispatch skips its callback without turning the packet into a refusal.
    pub fn owner_enabled(&self) -> bool {
        crate::mod_index_on(self.owner_index)
    }
    /// Whether a panic already disabled this callback, independently of its owner's state.
    pub fn poisoned(&self) -> bool {
        self.panicked.load(std::sync::atomic::Ordering::Acquire)
    }
    /// Whether the mod and this callback are both still enabled.
    pub fn enabled(&self) -> bool {
        crate::callback_owner::enabled(self.owner_index, &self.panicked)
    }
    /// Invoke only this owner. The caller owns the root transaction; the shared
    /// boundary disables and reports a panicking callback, not its connection.
    pub fn handle(
        &self,
        bytes: crate::PayloadBytes,
        context: &mut dyn crate::IPayloadContext,
    ) -> Result<(), crate::callback_owner::CallbackFailure> {
        self.call("payload", || self.handler.handle(bytes, context))?
            .map_err(|crate::PayloadHandleError::Failed(error)| {
                crate::callback_owner::CallbackFailure::Refused(error)
            })
    }
    /// Scheduled work retains its original receiving owner and enable guard.
    pub fn run_work(
        &self,
        work: &dyn crate::PayloadWork,
        context: &mut dyn crate::IPayloadContext,
    ) -> Result<(), crate::callback_owner::CallbackFailure> {
        self.call("payload work", || work.run(context))?
            .map_err(crate::callback_owner::CallbackFailure::Refused)
    }
    fn call<T>(
        &self,
        operation: &'static str,
        call: impl FnOnce() -> T,
    ) -> Result<T, crate::callback_owner::CallbackFailure> {
        crate::callback_owner::call_typed(
            self.owner,
            self.owner_index,
            &self.panicked,
            "payload",
            operation,
            call,
        )
    }
}

impl Payloads {
    /// Iterate normalized single-protocol declarations for negotiation.
    pub fn declarations(&self) -> impl Iterator<Item = &PayloadDeclaration> {
        self.phases
            .iter()
            .flat_map(|phase| phase.values())
            .filter(|entry| crate::mod_index_on(entry.owner_index))
            .map(|entry| &entry.declaration)
    }
    /// Resolve the sole receiving owner. A disabled owner or absent callback has
    /// no handler; the connection must report failure, never silently skip it.
    pub fn handler(
        &self,
        id: &str,
        protocol: PayloadProtocol,
        direction: PayloadDirection,
    ) -> Option<PayloadReceiver> {
        let index = match protocol {
            PayloadProtocol::Configuration => 0,
            PayloadProtocol::Play => 1,
            PayloadProtocol::Common => return None,
        };
        let entry = self.phases[index].get(id)?;
        if !crate::mod_index_on(entry.owner_index) {
            return None;
        }
        match direction {
            PayloadDirection::ToClient => entry.client.clone().map(|handler| PayloadReceiver {
                handler,
                thread: entry.client_thread,
                owner_index: entry.owner_index,
                owner: entry.owner,
                panicked: entry.client_panicked.clone(),
            }),
            PayloadDirection::ToServer => entry.server.clone().map(|handler| PayloadReceiver {
                handler,
                thread: entry.declaration.thread,
                owner_index: entry.owner_index,
                owner: entry.owner,
                panicked: entry.server_panicked.clone(),
            }),
            PayloadDirection::Bidirectional => None,
        }
    }
    /// Whether `token` is one this load's registrar handed out: the declaration at its slot
    /// has its id, protocol, schema and a direction it sends in, and its owner is on.
    pub fn check_token(&self, token: &PayloadToken) -> Result<(), PayloadSendError> {
        let matches = token.load == self.load
            && token.direction != PayloadDirection::Bidirectional
            && usize::try_from(token.slot)
                .ok()
                .and_then(|slot| self.typed.get(slot))
                .is_some_and(|slot| {
                    slot.id == token.id
                        && slot.protocol == token.protocol
                        && slot.schema == token.schema
                        && (slot.direction == token.direction
                            || slot.direction == PayloadDirection::Bidirectional)
                        && crate::mod_index_on(slot.owner_index)
                });
        if matches {
            Ok(())
        } else {
            Err(PayloadSendError::StaleToken(token.id.clone()))
        }
    }
    /// The drop counters of `Droppable` channel `id`, which the engine's queues increment.
    pub fn drop_counters(&self, id: &str) -> Option<&Arc<DropCounters>> {
        self.drops.get(id)
    }
    /// The raw play channels an enabled mod declared, by exact id.
    pub fn raw_channels(&self) -> impl Iterator<Item = &str> {
        self.raw
            .iter()
            .filter(|(_, owners)| {
                owners
                    .iter()
                    .any(|owner| crate::mod_index_on(owner.owner_index))
            })
            .map(|(id, _)| id.as_str())
    }
    /// The enabled mods receiving raw channel `id` from clients, in load order,
    /// each on the main thread. Empty for an undeclared or send-only channel.
    pub fn raw_receivers(&self, id: &str) -> Vec<PayloadReceiver> {
        self.raw.get(id).map_or_else(Vec::new, |owners| {
            owners
                .iter()
                .filter(|owner| crate::mod_index_on(owner.owner_index))
                .filter_map(|owner| {
                    Some(PayloadReceiver {
                        handler: owner.server.clone()?,
                        owner: owner.owner,
                        thread: HandlerThread::Main,
                        owner_index: owner.owner_index,
                        panicked: owner.panicked.clone(),
                    })
                })
                .collect()
        })
    }
    /// Validate the client-only handler requirement before a client starts its
    /// connection. Dedicated servers do not require unused client callbacks.
    pub fn validate_client_handlers(&self) -> Result<(), Vec<String>> {
        let missing: Vec<_> = self
            .phases
            .iter()
            .flat_map(|phase| phase.values())
            .filter(|entry| {
                crate::mod_index_on(entry.owner_index)
                    && entry.declaration.direction != PayloadDirection::ToServer
                    && entry.client.is_none()
            })
            .map(|entry| {
                format!(
                    "{} has no client payload handler for {} in {:?}",
                    entry.owner, entry.declaration.id.0, entry.declaration.protocol
                )
            })
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }
}

/// The frozen process registry, shared rather than copied into every connection.
// hidden: engine registry accessor exported only through __private::game.
#[doc(hidden)]
pub fn installed_payloads() -> Option<Arc<Payloads>> {
    installed().and_then(|installed| installed.keys.payloads.clone())
}

/// The drop counts of `Droppable` channel `id`, which `payload_drop_counts` answers; `None` for
/// any other channel.
// hidden: engine read exported only through __private::game.
#[doc(hidden)]
pub fn payload_drop_counts(id: &PayloadId) -> Option<crate::PayloadDropCounts> {
    installed_payloads()?
        .drop_counters(&id.0)
        .map(|counters| counters.read())
}

/// Check `token` against the installed load: what the host runs before every typed send.
/// With no payloads installed, every token is stale.
// hidden: engine send check exported only through __private::game.
#[doc(hidden)]
pub fn check_payload_token(token: &PayloadToken) -> Result<(), PayloadSendError> {
    installed_payloads().map_or_else(
        || Err(PayloadSendError::StaleToken(token.id.clone())),
        |payloads| payloads.check_token(token),
    )
}

/// Load-only operations backing the public payload registration views.
pub trait PayloadsRegistrar {
    /// Register a declaration and its receiving callbacks. Invalid declarations
    /// join the registrar's startup refusal list, naming the owning mod.
    fn add_payload(
        &mut self,
        declaration: PayloadDeclaration,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    );
    /// Attach the client callback to this owner's already declared clientbound
    /// entries in every protocol. Duplicates and unknown ids refuse startup.
    fn add_client_payload_handler(
        &mut self,
        id: &PayloadId,
        thread: HandlerThread,
        handler: Arc<dyn IPayloadHandler>,
    );
    /// Register a typed declaration: `declaration` carries the mod's own version, to which
    /// the registrar appends `schema`. No schema means a form unavailable in typed payloads,
    /// refused like [`add_payload`](Self::add_payload). The token names the declaration's slot
    /// and this load; a refused one names load 0, so every
    /// send with it is stale.
    fn add_typed_payload(
        &mut self,
        declaration: PayloadDeclaration,
        schema: Option<u64>,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    ) -> PayloadToken;
    /// Declare a raw play channel by exact id, with an optional server receiver.
    /// Reserved namespaces, a second declaration by the same mod and an id a
    /// typed payload uses refuse startup, naming the mods involved.
    fn add_raw_payload(&mut self, id: PayloadId, server: Option<Arc<dyn IPayloadHandler>>);
}
