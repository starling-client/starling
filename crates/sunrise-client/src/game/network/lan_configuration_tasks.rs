//! Dispatch each configuring connection on the server's ordinary tick.
use super::{Game, lan::LanHost};
use mod_api::{
    __private::{
        ConfigurationTaskOutcome, ConfigurationTaskRegistration, configuration_task_context,
    },
    ConfigurationConnection, ConfigurationTaskError, PayloadBytes, PayloadId,
};
use std::{io, sync::Arc};
use sunrise_client::lan::{Lan, NetConfig, configuration_tasks::Session, data::SyncData};

pub(super) fn start(sync: Arc<SyncData>, net: NetConfig) -> io::Result<Lan> {
    Lan::start_with_configuration_tasks(
        sync,
        net,
        mod_api::__private::listened(mod_api::HookId::RegisterConfigurationTasks),
    )
}
impl Game<'_> {
    fn configuration_callback(
        &mut self,
        session: &Arc<Session>,
        callback: impl FnOnce(&mut Self) -> Result<Option<usize>, String>,
    ) -> bool {
        let seal = self.transaction_open();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(self)))
            .unwrap_or_else(|_| Err("Configuration task callback panicked".into()));
        let ordinal = match result {
            Ok(ordinal) => ordinal,
            Err(reason) => {
                self.transaction_close(&seal, false);
                session.fail(reason);
                return false;
            }
        };
        let commit = !session.closed() && !matches!(session.result(), Some(Err(_)));
        self.transaction_close(&seal, commit);
        if commit && let Some(ordinal) = ordinal {
            session.finish_current(ordinal);
        }
        commit
    }
    pub(super) fn take_lan_for_packet_tick(&mut self) -> Option<LanHost> {
        if self.level_pass.first {
            let requests: Vec<_> = self
                .lan
                .as_ref()?
                .hub
                .lan
                .configuration_requests()
                .collect();
            for request in requests {
                if request.session.closed() {
                    continue;
                }
                self.lan
                    .as_mut()?
                    .hub
                    .lan
                    .configuration_tasks
                    .insert(&request);
                let mut registration = ConfigurationTaskRegistration::default();
                if self.configuration_callback(&request.session, |game| {
                    game.native_register_configuration_tasks(
                        request.connection,
                        request.player,
                        &request.name,
                        &mut registration,
                    );
                    registration
                        .failure()
                        .map_or(Ok(None), |error| Err(error.to_owned()))
                }) {
                    self.lan
                        .as_mut()?
                        .hub
                        .lan
                        .configuration_tasks
                        .register(request.connection, registration.into_tasks());
                }
            }
            let mut after = None;
            while let Some((connection, session)) = self
                .lan
                .as_ref()?
                .hub
                .lan
                .configuration_tasks
                .next_session(after)
            {
                after = Some(connection);
                if session.closed() {
                    let retired = self
                        .lan
                        .as_mut()?
                        .hub
                        .lan
                        .configuration_tasks
                        .remove(connection);
                    drop(retired);
                    continue;
                }
                loop {
                    let (completed, turn) = session.turn();
                    if let Some(ordinal) = completed {
                        let retired = self
                            .lan
                            .as_mut()?
                            .hub
                            .lan
                            .configuration_tasks
                            .take_task(connection, ordinal);
                        drop(retired);
                    }
                    let Some(turn) = turn else { break };
                    let Some(mut task) = self
                        .lan
                        .as_mut()?
                        .hub
                        .lan
                        .configuration_tasks
                        .take_task(connection, turn.ordinal)
                    else {
                        break;
                    };
                    // No map borrow or session lock survives extraction. The callback
                    // can use the Host (including this session) and its transaction.
                    let commit = self.configuration_callback(&session, |game| {
                        let call = if turn.start {
                            "ConfigurationTask::start"
                        } else {
                            "ConfigurationTask::tick"
                        };
                        if !task.owner.is_enabled() {
                            return Ok(Some(turn.ordinal));
                        }
                        let mut outcome = ConfigurationTaskOutcome::Pending;
                        let mut context = configuration_task_context(connection, &mut outcome);
                        task.call(call, |task| {
                            if turn.start {
                                task.start(game, &mut context);
                            } else {
                                task.tick(game, &mut context);
                            }
                        })?;
                        match outcome {
                            ConfigurationTaskOutcome::Pending => Ok(None),
                            ConfigurationTaskOutcome::Finish => Ok(Some(turn.ordinal)),
                            ConfigurationTaskOutcome::Fail(reason) => Err(reason),
                        }
                    });
                    let mut task = Some(task);
                    if commit {
                        self.lan.as_mut()?.hub.lan.configuration_tasks.restore_task(
                            connection,
                            &session,
                            turn.ordinal,
                            &mut task,
                        );
                    }
                    drop(task);
                    // Only a committed start can immediately open the next turn.
                    if !turn.start || !commit || !session.finished() {
                        break;
                    }
                }
            }
        }
        self.lan.take()
    }
    pub(in crate::game) fn configuration_session(
        &self,
        connection: ConfigurationConnection,
    ) -> Option<&Arc<Session>> {
        self.lan
            .as_ref()?
            .hub
            .lan
            .configuration_tasks
            .get(connection)
    }
    pub(in crate::game) fn configuration_has_channel(
        &self,
        connection: ConfigurationConnection,
        channel: &PayloadId,
    ) -> bool {
        self.configuration_session(connection)
            .is_some_and(|session| session.has_channel(channel))
    }
    pub(in crate::game) fn configuration_send_payload(
        &mut self,
        connection: ConfigurationConnection,
        channel: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<(), ConfigurationTaskError> {
        let (endpoint, epoch) = self
            .configuration_session(connection)
            .ok_or(ConfigurationTaskError::Disconnected)?
            .payload_target()?;
        self.queue_payload(endpoint, epoch, channel, bytes)
            .map(|_| ())
            .map_err(ConfigurationTaskError::Payload)
    }
}
