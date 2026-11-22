//! Sole-owner main execution and root-commit raw publication.
use super::Game;
use mod_api::{
    __private::game::CallbackFailure, ConfigurationTaskError, HandlerThread, IPayloadContext,
    Identifier, PayloadAccepted, PayloadBytes, PayloadContextError, PayloadDirection, PayloadId,
    PayloadProtocol, PayloadSendError, PayloadWork, PayloadWorkStatus, PlayerRef, Text,
};
use std::sync::Arc;
use sunrise_client::raw_payloads::{Body, Endpoint, Incoming, Prepared, Side, connection::Epoch};
use sunrise_protocol::payload_negotiation::Phase;

/// Fits the journal-growth allowance already charged by each prepared operation.
pub(super) struct Deferred {
    epoch: Arc<Epoch>,
    prepared: Prepared,
}
const _: () = assert!(std::mem::size_of::<Deferred>() <= 256);
impl Deferred {
    pub(super) fn publish(self) {
        let _ = self.epoch.with_active(|| self.prepared.publish());
    }
}
impl Game<'_> {
    pub(super) fn player_send_payload(
        &mut self,
        player: PlayerRef,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        let sender = self
            .online_player_record(player)
            .ok_or(PayloadSendError::StaleEpoch)?
            .sender
            .clone();
        self.queue_current_payload(Endpoint::server(sender), id, bytes)
    }
    /// `player_send_payload` to each of `players`; a refused send is skipped. The caller crossed
    /// the body once; each connection's queue owns its own copy.
    pub(super) fn players_send_payload(
        &mut self,
        players: &[PlayerRef],
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> u32 {
        players
            .iter()
            .filter(|player| {
                self.player_send_payload(**player, id, bytes.clone())
                    .is_ok()
            })
            .count() as u32
    }
    pub(super) fn send_payload_to_server(
        &mut self,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        let endpoint = self
            .remote
            .as_ref()
            .ok_or(PayloadSendError::Disconnected)?
            .session
            .payload_endpoint();
        self.queue_current_payload(endpoint, id, bytes)
    }
    pub(super) fn queue_current_payload(
        &mut self,
        endpoint: Endpoint,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        let epoch = endpoint
            .transport()
            .epoch()
            .ok_or(PayloadSendError::NotNegotiated)?;
        self.queue_payload(endpoint, epoch, id, bytes)
    }
    pub(super) fn queue_payload(
        &mut self,
        endpoint: Endpoint,
        epoch: Arc<Epoch>,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        let prepared = endpoint.prepare(&epoch, id, bytes)?;
        self.after_commit(super::transactions::After::Payload(Deferred {
            epoch,
            prepared,
        }));
        Ok(PayloadAccepted::Queued)
    }
    /// Use the receiving endpoint, not a sender resolved from a later guest incarnation.
    pub(super) fn run_payload(&mut self, endpoint: Endpoint, incoming: Incoming) {
        if !incoming.epoch.active() {
            return;
        }
        let seal = self.transaction_open();
        let mut context = MainContext {
            game: self,
            epoch: &incoming.epoch,
            endpoint: endpoint.clone(),
            receiver: &incoming.receiver,
            disconnect: None,
            failure: None,
        };
        let result = match incoming.body {
            Body::Payload(bytes) if incoming.receiver.thread == HandlerThread::Main => {
                incoming.receiver.handle(bytes, &mut context)
            }
            Body::Payload(_) => Err(CallbackFailure::Refused(
                "payload was not dispatched on its declared network executor".into(),
            )),
            Body::Work(work) => incoming.receiver.run_work(work.as_ref(), &mut context),
        };
        let reason = context.disconnect.take();
        let failure = match result {
            Err(error @ (CallbackFailure::Panicked | CallbackFailure::Poisoned)) => Some(error),
            result => context.failure.take().or_else(|| result.err()),
        };
        context.game.transaction_close(&seal, failure.is_none());
        if matches!(
            failure,
            Some(CallbackFailure::Panicked | CallbackFailure::Poisoned)
        ) {
            // The shared boundary already reported the panic; keep the epoch alive.
            return;
        }
        if let Some(CallbackFailure::Refused(error)) = failure.as_ref().filter(|_| incoming.raw) {
            // A raw channel's bytes come from any client; one declaring mod's
            // refusal rolls back its own work only, and the next mod still runs.
            log::warn!(
                "Mod {} failed on raw channel {}: {error}",
                incoming.receiver.owner(),
                incoming.id.0
            );
        } else if let Some(CallbackFailure::Refused(error)) = failure {
            // The reason goes to the log; the player is told only which mod's handler failed.
            log::warn!("payload {} failed: {error}", incoming.id.0);
            endpoint.disconnect(Text::translate(
                "disconnect.genericReason",
                vec![Text::literal(format!(
                    "a mod's payload handler failed ({})",
                    sunrise_client::raw_payloads::mod_of(&incoming.id)
                ))],
            ));
        } else if let Some(reason) = reason {
            endpoint.disconnect(reason);
        }
        // The rest of Incoming releases the turn and ingress credit after publication/rollback.
    }
}
struct MainContext<'a, 'resources> {
    game: &'a mut Game<'resources>,
    epoch: &'a Arc<Epoch>,
    endpoint: Endpoint,
    receiver: &'a mod_api::__private::game::PayloadReceiver,
    disconnect: Option<Text>,
    failure: Option<CallbackFailure>,
}
impl IPayloadContext for MainContext<'_, '_> {
    fn player(&self) -> Result<PlayerRef, PayloadContextError> {
        if !self.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        if self.epoch.phase == Phase::Configuration {
            return Err(PayloadContextError::NoPlayer);
        }
        self.endpoint
            .transport()
            .player()
            .ok_or(PayloadContextError::NoPlayer)
    }
    fn flow(&self) -> PayloadDirection {
        match self.endpoint.side() {
            Side::Client => PayloadDirection::ToClient,
            Side::Server => PayloadDirection::ToServer,
        }
    }
    fn protocol(&self) -> PayloadProtocol {
        match self.epoch.phase {
            Phase::Configuration => PayloadProtocol::Configuration,
            Phase::Play => PayloadProtocol::Play,
        }
    }
    fn server_host(&mut self) -> Result<&mut dyn mod_api::ServerHost, PayloadContextError> {
        if !self.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        if self.endpoint.side() != Side::Server {
            return Err(PayloadContextError::WrongSide);
        }
        Ok(self.game)
    }
    fn client_host(&mut self) -> Result<&mut dyn mod_api::ClientHost, PayloadContextError> {
        if !self.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        if self.endpoint.side() != Side::Client {
            return Err(PayloadContextError::WrongSide);
        }
        Ok(self.game)
    }
    fn reply(
        &mut self,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        if self.disconnect.is_some() {
            return Err(PayloadSendError::Disconnected);
        }
        self.game
            .queue_payload(self.endpoint.clone(), self.epoch.clone(), id, bytes)
    }
    fn check_payload_token(&self, token: &mod_api::PayloadToken) -> Result<(), PayloadSendError> {
        mod_api::__private::game::check_payload_token(token)
    }
    /// The main thread's group send is transactional, as every main-thread send.
    fn send_to(
        &mut self,
        players: &[PlayerRef],
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<u32, PayloadSendError> {
        if self.disconnect.is_some() {
            return Err(PayloadSendError::Disconnected);
        }
        if self.endpoint.side() != Side::Server {
            return Err(PayloadSendError::WrongDirection(id.clone()));
        }
        if self.endpoint.transport().local.droppable(&id.0).is_none() {
            return Err(PayloadSendError::NotDroppable(id.clone()));
        }
        Ok(self.game.players_send_payload(players, id, bytes))
    }
    fn finish_configuration_task(
        &mut self,
        task_type: &Identifier,
    ) -> Result<(), ConfigurationTaskError> {
        if self.endpoint.side() != Side::Server || self.epoch.phase != Phase::Configuration {
            return Err(ConfigurationTaskError::WrongContext);
        }
        if self.disconnect.is_some() || !self.epoch.active() {
            return Err(ConfigurationTaskError::Disconnected);
        }
        let session = match self.endpoint.configuration_session() {
            Some(weak) => weak.upgrade().ok_or(ConfigurationTaskError::Disconnected)?,
            None => return Err(ConfigurationTaskError::NotRunning),
        };
        if self
            .endpoint
            .transport()
            .epoch()
            .is_none_or(|epoch| !Arc::ptr_eq(&epoch, self.epoch))
        {
            return Err(ConfigurationTaskError::Disconnected);
        }
        session.finish_type(task_type)
    }
    fn disconnect(&mut self, reason: Text) {
        self.disconnect = Some(reason);
    }
    fn enqueue_work(
        &mut self,
        work: Arc<dyn PayloadWork>,
    ) -> Result<PayloadWorkStatus, PayloadContextError> {
        if !self.epoch.active() || self.disconnect.is_some() {
            return Err(PayloadContextError::Disconnected);
        }
        let receiver = self.receiver.clone();
        if let Err(error) = receiver.run_work(work.as_ref(), self) {
            // Preserve the first panic even if the caller catches or overwrites its
            // error by attempting more work on the now-disabled callback.
            if !matches!(
                self.failure,
                Some(CallbackFailure::Panicked | CallbackFailure::Poisoned)
            ) {
                self.failure = Some(error);
            }
            return Err(PayloadContextError::Disconnected);
        }
        Ok(PayloadWorkStatus::Complete)
    }
}
#[cfg(test)]
#[path = "payload_transport/config_tests.rs"]
mod config_tests;
#[cfg(test)]
#[path = "payload_transport/disconnect_tests.rs"]
mod disconnect_tests;
#[cfg(test)]
#[path = "payload_transport/main_panic_tests.rs"]
mod main_panic_tests;
#[cfg(test)]
#[path = "payload_transport/raw_fault_tests.rs"]
mod raw_fault_tests;
#[cfg(test)]
#[path = "payload_transport/sender_tests.rs"]
mod sender_tests;
#[cfg(test)]
#[path = "payload_transport/task_tests.rs"]
mod task_tests;
#[cfg(test)]
#[path = "payload_transport/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "payload_transport/typed_tests.rs"]
mod typed_tests;
