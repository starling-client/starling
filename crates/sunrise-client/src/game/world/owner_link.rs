//! The owner link: the closed set of requests the window sends its integrated server thread.
use super::{
    Dim, Game,
    integrated::{
        ServerOptions,
        capture::{Live, Target},
    },
    lan::LanSettings,
    lan_screen::Snapshot,
    memory::{ServerMemoryRequest, ServerSide},
    mods::ModsReload,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, SyncSender},
    },
};
use sunrise_core::geometry::Vec3;
use sunrise_perf_format::v1::StopReason;

/// Where the server puts the answer to a request the window waits for.
pub(super) type Reply<T> = SyncSender<T>;

/// Work the window hands its server thread between two ticks (`MinecraftServer.execute`).
pub(super) enum OwnerRequest {
    /// `IntegratedServer.publishServer`.
    Publish(LanSettings, Reply<Result<u16, String>>),
    /// `IntegratedServer.unpublishServer`: whether it was published.
    Unpublish(Reply<bool>),
    /// `sunrise-ctl lan status`.
    LanStatus(Reply<serde_json::Value>),
    /// New command access and game mode forcing settings of the open world.
    LanApplySettings {
        guest_commands: bool,
        force: bool,
        done: Reply<()>,
    },
    /// The world options screen's state.
    ServerOptions(Reply<ServerOptions>),
    /// The world options screen's changes, from `before` to `wanted`.
    ApplyWorldOptions {
        before: Snapshot,
        wanted: Snapshot,
        done: Reply<()>,
    },
    /// Far terrain for a client in `dim`.
    LodSource(
        Dim,
        Reply<Option<Arc<dyn sunrise_client::streaming::LodSource>>>,
    ),
    /// The pregen commands are on or off.
    SetPregen(bool),
    /// A live content mod switch: the data packs reload from the new root.
    ReloadMods(Box<ModsReload>),
    /// The server thread's side of a capture starts, for `target`; the reply, when one waits, gets its path.
    StartServerCapture(Target, Arc<Live>, Option<Reply<Result<PathBuf, String>>>),
    /// The server thread's capture stops; the saved path goes to the reply when one waits.
    StopServerCapture(StopReason, Option<Sender<String>>),
    /// `/profiler mark`: a named moment in the server thread's capture.
    MarkServerCapture(String),
    /// What agent control asks of the world.
    Control(ControlRequest),
    /// Tests reach into the server thread's `Game` (`Integrated::call`, `Integrated::post`).
    #[cfg(test)]
    InTest(Box<dyn FnOnce(&mut Game<'_>) + Send>),
}

/// The agent-control asks, answered on the server thread in the owner's level.
pub(super) enum ControlRequest {
    /// `simulation blocks`: the server's own blocks in a box.
    Blocks {
        lo: [i32; 3],
        hi: [i32; 3],
        reply: Reply<Value>,
    },
    /// `simulation`: the server's clock and projectiles.
    Simulation(Reply<Value>),
    /// `server-hold`: stop or resume the server's tick.
    HoldServer { held: bool, done: Reply<()> },
    /// A console command run as the owner: the chat lines it produced and its outcome.
    ConsoleCommand {
        command: String,
        reply: Reply<(Vec<String>, Option<(bool, i32)>)>,
    },
    /// `time`: the server's day time.
    SetDayTime { time: i64, done: Reply<()> },
    /// `summon`: the new mob's id.
    Summon {
        kind: String,
        at: [f64; 3],
        baby: bool,
        reply: Reply<Result<u32, String>>,
    },
    /// `mobs`: the mobs near `center`.
    MobsAt {
        radius: f64,
        center: Vec3,
        reply: Reply<Value>,
    },
    /// `hurtlog`.
    HurtLog(Reply<Value>),
    /// `save` and `save-all`.
    Save {
        all: bool,
        reply: Reply<Result<(), String>>,
    },
    /// `setblock`, `fill`, `getblock` and `container`, with their words.
    BlockCommand {
        words: Vec<String>,
        reply: Reply<Result<Option<Value>, String>>,
    },
    /// The explicit memory report: the server's own parts.
    ServerSide(Reply<ServerSide>),
    /// A profiling sample, delivered directly to the worker while it is wanted.
    MemorySample(ServerMemoryRequest),
    /// `lan op NAME LEVEL`: whether a guest of that name was found.
    SetOp {
        name: String,
        level: u8,
        reply: Reply<bool>,
    },
    /// The save folder of the server's world.
    WorldRoot(Reply<PathBuf>),
    /// The recording's copy: save, then hold the server thread until `copied` is dropped.
    SaveThenWait {
        saved: Sender<Result<(), String>>,
        copied: Receiver<()>,
    },
}

impl Game<'_> {
    /// Answers one request of the window, on the server thread.
    pub(super) fn serve_owner_request(&mut self, request: OwnerRequest) {
        match request {
            OwnerRequest::Publish(settings, reply) => {
                let _ = reply.send(self.publish(settings));
            }
            OwnerRequest::Unpublish(reply) => {
                let _ = reply.send(self.unpublish());
            }
            OwnerRequest::LanStatus(reply) => {
                let _ = reply.send(self.lan_status());
            }
            OwnerRequest::LanApplySettings {
                guest_commands,
                force,
                done,
            } => {
                self.lan_apply_settings(guest_commands, force);
                let _ = done.send(());
            }
            OwnerRequest::ServerOptions(reply) => {
                let _ = reply.send(self.server_options());
            }
            OwnerRequest::ApplyWorldOptions {
                before,
                wanted,
                done,
            } => {
                self.apply_world_options(&before, &wanted);
                let _ = done.send(());
            }
            OwnerRequest::LodSource(dim, reply) => {
                let _ = reply.send(self.lod_source_in(dim));
            }
            OwnerRequest::SetPregen(enabled) => self.set_pregen_commands(enabled),
            OwnerRequest::ReloadMods(reload) => self.apply_mods_reload(*reload),
            OwnerRequest::StartServerCapture(target, live, reply) => {
                let started = self.start_server_capture(target, live);
                match reply {
                    Some(reply) => {
                        let _ = reply.send(started);
                    }
                    None => {
                        if let Err(e) = started {
                            log::warn!("Sunrise: the server thread is not captured: {e}");
                        }
                    }
                }
            }
            OwnerRequest::StopServerCapture(reason, reply) => {
                self.stop_server_capture(reason, reply)
            }
            OwnerRequest::MarkServerCapture(name) => self.mark_server_capture(&name),
            OwnerRequest::Control(request) => self.serve_control_request(request),
            #[cfg(test)]
            OwnerRequest::InTest(task) => task(self),
        }
    }

    /// Answers one agent-control ask, on the server thread.
    fn serve_control_request(&mut self, request: ControlRequest) {
        match request {
            ControlRequest::Blocks { lo, hi, reply } => {
                let _ = reply.send(self.in_owner_level(|g| g.blocks_json(lo, hi)));
            }
            ControlRequest::Simulation(reply) => {
                let _ = reply.send(self.in_owner_level(|g| {
                    json!({ "game_time": g.world.game_time, "projectiles": g.projectiles_status() })
                }));
            }
            ControlRequest::HoldServer { held, done } => {
                self.agent.hold_server = held;
                let _ = done.send(());
            }
            ControlRequest::ConsoleCommand { command, reply } => {
                let _ = reply.send(self.owner_console_command(&command));
            }
            ControlRequest::SetDayTime { time, done } => {
                self.world.day_time = time;
                let _ = done.send(());
            }
            ControlRequest::Summon {
                kind,
                at,
                baby,
                reply,
            } => {
                let id = self.in_owner_level(|g| {
                    g.summon_command_mob(&kind, Some(at), baby)
                        .map_err(|e| e.to_string())
                });
                let _ = reply.send(id);
            }
            ControlRequest::MobsAt {
                radius,
                center,
                reply,
            } => {
                let _ = reply.send(self.in_owner_level(|g| g.control_mobs_at(radius, center)));
            }
            ControlRequest::HurtLog(reply) => {
                let _ = reply.send(self.control_hurt_log());
            }
            ControlRequest::Save { all, reply } => {
                let _ = reply.send(self.control_save(all).map_err(|e| e.to_string()));
            }
            ControlRequest::BlockCommand { words, reply } => {
                let words: Vec<&str> = words.iter().map(String::as_str).collect();
                let out =
                    self.in_owner_level(|g| g.block_command(&words).map_err(|e| e.to_string()));
                let _ = reply.send(out);
            }
            ControlRequest::ServerSide(reply) => {
                let _ = reply.send(self.server_side());
            }
            ControlRequest::MemorySample(request) => request.collect(|| self.server_side()),
            ControlRequest::SetOp { name, level, reply } => {
                let _ = reply.send(self.lan_set_op(&name, level));
            }
            ControlRequest::WorldRoot(reply) => {
                let _ = reply.send(self.world_root.clone());
            }
            ControlRequest::SaveThenWait { saved, copied } => {
                let _ = saved.send(self.save_game().map_err(|e| e.to_string()));
                let _ = copied.recv();
            }
        }
    }

    /// The command tree shows the pregen commands only while they are on.
    fn set_pregen_commands(&mut self, enabled: bool) {
        use sunrise_client::command::vanilla;
        if self.ui.dispatcher.0.find("pregen").is_some() != enabled {
            vanilla::set_sunrise_commands(enabled);
            self.ui.dispatcher =
                super::chat::SessionDispatcher(Arc::new(vanilla::session_dispatcher(
                    &self.level.rule_registry,
                    &self.level.enabled_features,
                )));
            self.restore_native_commands();
            self.send_native_command_report(true);
        }
    }
}
