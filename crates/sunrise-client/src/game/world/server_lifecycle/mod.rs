//! The server's run/stop boundary, shared by integrated and dedicated servers.

use super::Game;
use std::panic::{AssertUnwindSafe, catch_unwind};
use sunrise_client::Result;

pub(in crate::game) use sunrise_gameplay::crash_save::CrashSave;

impl Game<'_> {
    /// `MinecraftServer.runServer`'s finally: once startup begins, every exit
    /// stops and saves the server, including a failed init or tick.
    pub(super) fn run_server_lifecycle(
        &mut self,
        dedicated: bool,
        run: impl FnOnce(&mut Self, &mut bool) -> Result<()>,
    ) -> Result<()> {
        let mut startup_began = false;
        let run = catch_unwind(AssertUnwindSafe(|| run(self, &mut startup_began)));
        let crashed = run.is_err();
        let result = match run {
            Ok(result) => result,
            Err(payload) => {
                let message = super::dedicated::panic_message(payload.as_ref());
                log::error!("The server thread panicked: {message}");
                self.prepare_server_crash();
                Err(format!("the server thread panicked: {message}").into())
            }
        };
        if !startup_began {
            return result;
        }
        let run_error = result.as_ref().err().map(ToString::to_string);
        let shutdown = self.shutdown_started_server(dedicated, crashed, run_error.as_deref());
        result.and(shutdown)
    }

    /// Vanilla's nested `stopServer` finally: a listener panic still reaches
    /// cleanup, storage release and `server_stopped`, and the error is kept.
    fn shutdown_started_server(
        &mut self,
        dedicated: bool,
        crashed: bool,
        run_error: Option<&str>,
    ) -> Result<()> {
        // There will be no next server tick; do not keep logs for another world.
        sunrise_client::logging::stop_tap();
        let mut failure = None;
        self.exclude_interrupted_players(crashed, |m| log::warn!("{m}"));
        if let Err(payload) =
            catch_unwind(AssertUnwindSafe(|| self.native_server_stopping(dedicated)))
        {
            remember_shutdown(&mut failure, shutdown_panic(payload.as_ref()).into());
        }
        self.exclude_interrupted_players(crashed, |m| log::warn!("{m}"));
        match catch_unwind(AssertUnwindSafe(|| {
            if dedicated {
                self.stop_dedicated(crashed, run_error)
            } else {
                self.stop_integrated(crashed, run_error)
            }
        })) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                log::error!("Exception stopping the server: {error}");
                failure.get_or_insert(error);
            }
            Err(payload) => {
                let message = shutdown_panic(payload.as_ref());
                remember_shutdown(&mut failure, message.clone().into());
                // A config/player callback may unwind before the stop path reaches its final
                // save, including inside a new player's turn. The tail reapplies the exclusion
                // and saves before reporting the original runtime or shutdown error.
                if let Err(error) = self.finish_server_save(crashed, run_error.or(Some(&message))) {
                    remember_shutdown(&mut failure, error);
                }
            }
        }
        if let Err(payload) =
            catch_unwind(AssertUnwindSafe(|| self.native_server_stopped(dedicated)))
        {
            remember_shutdown(&mut failure, shutdown_panic(payload.as_ref()).into());
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// The crash entry's part of the decision: the interrupted players are excluded (see
    /// [`Self::exclude_interrupted_players`]); a lost hub also says players are not saved.
    fn prepare_server_crash(&mut self) {
        self.exclude_interrupted_players(true, |message| log::warn!("{message}"));
    }

    /// The one decision of a crash save, shared by the dedicated server, single player's server
    /// thread and the stop path, which applies it again after each shutdown callback panic it
    /// catches, before players are saved (#2704).
    ///
    /// A player's turn counts as interrupted when it is recorded (`interrupted_turns`: the turn
    /// helpers record the player on the way out of a panic, which also restores the outer
    /// player and clears the acting marker), when a swap that is still open displaces them, or
    /// when they are still swapped in as the acting player. Such a player is not saved: their
    /// last saved file, stats and advancements stay, for the half-done turn can hold an item
    /// taken but not yet placed. The player stays online (`placed`), so their logout still
    /// fires; only the saves read the exclusion. When that is the single-player owner, or the hub is lost
    /// (`crashed` and no hub, or an interrupted turn and no hub), `level.dat` is written without
    /// a new player file, keeping a legacy `Data.Player`.
    pub(in crate::game) fn exclude_interrupted_players(
        &mut self,
        crashed: bool,
        warn: impl Fn(&str),
    ) {
        let mut conns = self.interrupted_turns.clone();
        conns.extend(self.lan_guest_swaps.iter().copied());
        conns.extend(self.lan_acting);
        conns.sort_unstable();
        conns.dedup();
        self.crash_save.note_acting(self.lan_acting.is_some());
        let Some(host) = self.lan.as_mut() else {
            if (crashed || !conns.is_empty()) && self.crash_save.lose_hub() {
                warn(
                    "The panic interrupted the network tick: players are not saved, only the world",
                );
            }
            return;
        };
        for conn in conns {
            let owner = host.owner.as_ref().map(|o| o.uuid);
            let Some((name, is_owner)) = host.hub.guests.update_connection(conn, |guest| {
                (
                    guest.placed.then(|| guest.name.clone()),
                    Some(guest.uuid) == owner,
                )
            }) else {
                continue;
            };
            if is_owner {
                self.crash_save.keep_owner_file = true;
            }
            if let Some(name) = name
                && self.crash_save.exclude(conn)
            {
                warn(&format!(
                    "The panic interrupted a player's turn ({name}): that player keeps the last saved file"
                ));
            }
        }
    }

    /// Complete teardown even if freeing beds or the final save fails. A failed
    /// open must never overwrite a world the server did not successfully adopt.
    /// The session lock is not that marker: it is installed before the prepared
    /// level, so tag reads work, and a save in that gap would write the placeholder.
    pub(super) fn finish_server_save(
        &mut self,
        crashed: bool,
        run_error: Option<&str>,
    ) -> Result<()> {
        let before = self.save.record_save_failures();
        // IntegratedServer.teardownPublishedState must happen even if an
        // earlier callback panicked; otherwise lan_close only unpublishes a
        // still-serving server instead of disconnecting and dropping its hub.
        self.stop_serving();
        let mut result = match catch_unwind(AssertUnwindSafe(|| {
            if crashed {
                Ok(())
            } else {
                self.free_player_beds()
            }
        })) {
            Ok(result) => result,
            Err(payload) => Err(shutdown_panic(payload.as_ref()).into()),
        };
        // The cleanup can unwind inside a new player's turn too (entering a level fires a load
        // callback), before the save writes the players the exclusion does not name.
        self.exclude_interrupted_players(crashed, |m| log::warn!("{m}"));
        let saved = match catch_unwind(AssertUnwindSafe(|| {
            if self.world_adopted && self.session_lock.is_some() {
                self.save_game()
            } else {
                Ok(())
            }
        })) {
            Ok(result) => result,
            Err(payload) => Err(shutdown_panic(payload.as_ref()).into()),
        };
        result = result.and(saved);
        // The save can unwind inside a new player's turn, and `lan_close` writes every player the
        // exclusion does not name.
        self.exclude_interrupted_players(crashed, |m| log::warn!("{m}"));
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.lan_close(false))) {
            let message = shutdown_panic(payload.as_ref());
            if result.is_ok() {
                result = Err(message.into());
            }
        }
        // A player callback/save in lan_close can also unwind before taking
        // the host. Reuse its transport teardown independently of callbacks.
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.lan_disconnect(false)))
            && result.is_ok()
        {
            result = Err(shutdown_panic(payload.as_ref()).into());
        }
        match catch_unwind(AssertUnwindSafe(|| self.save.flush())) {
            Ok(saved) => result = result.and(saved),
            Err(payload) => {
                let message = shutdown_panic(payload.as_ref());
                if result.is_ok() {
                    result = Err(message.into());
                }
            }
        }
        let failed = self.save.record_save_failures() - before;
        if crashed && failed != 0 {
            log::warn!(
                "Partial save during crash shutdown: {failed} chunk records could not be saved"
            );
        }
        // Save hooks and entity references need the live epoch through the last write. Only
        // now unload the levels; an independent crash keeps its original close cause.
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
            if let Some(error) = run_error {
                if self.native_lifecycle.is_opening() {
                    self.native_lifecycle_world_failed(error);
                } else {
                    self.native_lifecycle_close_failed(error);
                }
            } else if let Err(error) = &result {
                self.native_lifecycle_close_failed(&error.to_string());
            } else {
                self.native_lifecycle_world_closed();
            }
        })) && result.is_ok()
        {
            result = Err(shutdown_panic(payload.as_ref()).into());
        }
        self.session_lock = None;
        self.interrupted_turns.clear();
        self.crash_save = Default::default();
        // The integrated client retains these resources after joining this
        // thread. Retire the world's maps here, also on failed opens that
        // published tables before adopting a world.
        if let Some(rest) = self.res.rest.get() {
            rest.behaviours.set_block_maps(None);
        }
        super::cauldron_interactions::clear();
        self.clear_world_parts();
        self.world_adopted = false;
        self.startup_ticket_restore_pending = false;
        result
    }
}

fn shutdown_panic(payload: &(dyn std::any::Any + Send)) -> String {
    format!(
        "the server shutdown panicked: {}",
        super::dedicated::panic_message(payload)
    )
}

fn remember_shutdown(
    failure: &mut Option<Box<dyn std::error::Error>>,
    error: Box<dyn std::error::Error>,
) {
    log::error!("{error}");
    failure.get_or_insert(error);
}

#[cfg(test)]
#[path = "interrupted_turn_tests.rs"]
mod interrupted_turn_tests;
#[cfg(test)]
#[path = "server_lifecycle_tests.rs"]
mod tests;
