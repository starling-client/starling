//! Command execution (the `execute` side of each vanilla command class):
//! entity selectors resolved against the game, vanilla feedback and errors.
mod admin;
mod clone_payload;
pub(super) mod compute;
pub(in crate::game) mod data;
mod difficulty;
mod display_names;
pub(super) use data::place_summoned;
mod effects;
#[cfg(test)]
mod enchant_tests;
pub(super) mod execute;
pub(super) mod extra;
mod give;
mod item_count;
pub(in crate::game) mod items;
mod misc;
mod modprofile;
mod place;
mod players;
mod position_args;
pub(super) mod pregen;
mod profiles;
mod raid;
mod selector_bounds;
mod selectors;
mod targets;
pub(in crate::game) mod terrain_cmds;
mod test_locate;
mod test_runs;
mod text_commands;
pub(in crate::game) mod world_cmds;

use super::{Dim, Game, Screen, chat::PLAYER_NAME, vehicle_teleport::TeleportMotion};
use mod_api::CommandFeedback;
use sunrise_client::{
    command::{
        Arg, CommandError, Context, Reader,
        args::{BlockInput, Coordinates, GameMode, ItemInput, Selector},
        vanilla::DIFFICULTIES,
    },
    gamerules::{self, Value},
    items::ItemStack,
    text::{self, Style, Text},
};
use sunrise_core::{geometry::Vec3, java_number::float_to_string, random::RandomSource};
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_world::registry::BlockStateId;

pub(super) type R<T> = Result<T, CommandError>;

/// An entity a selector found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Target {
    /// The player in place: the host's, or the guest whose turn it is.
    Player,
    /// Another player of a hosted world or server: a guest by connection
    /// (game/administration/commands/players.rs).
    Guest(u32),
    Mob(u32),
    Item(u32),
    /// An experience orb, by its entity id.
    Orb(u32),
    /// A projectile, by its entity id.
    Projectile(u32),
    Stand(u32),
    Frame(u32),
    Painting(u32),
    Boat(u32),
    Cart(u32),
    /// An entity of a type Sunrise does not simulate (markers, displays,
    /// interactions...): kept as its saved data at a position.
    Other(u32),
    /// Active evoker fangs, including command-loaded ones.
    Fangs(u32),
    /// An End crystal, by its entity id (`minecraft:end_crystal`).
    Crystal(u32),
    /// The Ender Dragon, by its entity id (`minecraft:ender_dragon`).
    Dragon(u32),
    /// One of the dragon's eight `EnderDragonPart`s, by its own entity id (the
    /// eight after the dragon's). Only a mod's handle names one: a part is no
    /// selector's, command's or save's entity.
    DragonPart(u32),
    /// A primed TNT, by its entity id (`minecraft:tnt`).
    Tnt(u32),
    /// A cushion, by its entity id (`minecraft:cushion`).
    Cushion(u32),
    /// A falling block, by its entity id (`minecraft:falling_block`).
    Falling(u32),
    /// A firework rocket, by its entity id (`minecraft:firework_rocket`).
    Rocket(u32),
    /// A thrown trident, by its entity id (`minecraft:trident`).
    Trident(u32),
    /// An ominous item spawner, by its entity id (`minecraft:ominous_item_spawner`).
    Spawner(u32),
    /// A simulated lingering, creeper or dragon area-effect cloud.
    Cloud(u32),
    /// A live lightning bolt (`minecraft:lightning_bolt`).
    Bolt(u32),
    /// A general entity row, by its entity id (#792).
    General(u32),
    /// A cast fishing hook (`minecraft:fishing_bobber`), which lives with the
    /// player who cast it. Only a mod's handle names one.
    Hook(u32),
    /// The fence knot a lead tie made, by entity id.
    Knot(u32),
}
macro_rules! target_entity_mut {
    ($game:expr, $target:expr, $e:ident => $body:expr) => {{
        use super::entity::Entity as _;
        let handle = $game.entity_handles().find(|h| {
            super::entity::with_entity!($game, *h, entity => entity.selector_target() == Some($target))
        });
        if let Some(handle) = handle {
            super::entity::with_entity_mut!($game, handle, $e => $body);
        }
    }};
}

/// `Entity.setXRot`: wrap before clamping the pitch.
pub(super) fn entity_pitch(pitch: f32) -> f32 {
    (pitch % 360.0).clamp(-90.0, 90.0)
}

/// `PositionMoveRotation.calculateAbsolute` and `TeleportCommand.performTeleport`.
pub(super) fn teleport_velocity(velocity: Vec3, relative: [bool; 3], fall_flying: bool) -> Vec3 {
    Vec3::new(
        if relative[0] { velocity.x } else { 0.0 },
        if relative[1] && fall_flying {
            velocity.y
        } else {
            0.0
        },
        if relative[2] { velocity.z } else { 0.0 },
    )
}

/// An entity of a type Sunrise does not simulate: its compound lives in the
/// entity memory (`persist.raw`), its position and rotation here.
#[derive(Clone, Debug)]
pub(in crate::game) struct OtherEntity {
    /// Entity-owned registration metadata; content clones are detached.
    pub(super) attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub(in crate::game) join: sunrise_client::join_seq::JoinSeq,
    pub id: u32,
    pub kind: String,
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// A display's typed `SynchedEntityData`, not a saved or network mirror.
    pub display: Option<std::sync::Arc<sunrise_client::text_display::resident::DisplayState>>,
    /// A `minecraft:interaction` saved settings; None for every other kind.
    pub interaction: Option<super::interactions::InteractionState>,
    /// Only explicitly loaded leash knots own this BlockAttachedEntity check counter.
    pub knot_check: Option<u8>,
    /// A summoned or loaded `minecraft:eye_of_ender`'s flight; None for every other kind.
    pub eye: Option<super::end::SummonedEye>,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub(in crate::game) size_answer: Option<sunrise_client::pose::SizeAnswer>,
}

sunrise_client::impl_joined!(OtherEntity);
sunrise_mod_runtime::impl_attachment_lifetime!(OtherEntity);

pub(super) fn err(key: &str, args: &[&dyn std::fmt::Display]) -> CommandError {
    CommandError::tr(key, args)
}

/// Format and validate the failure as the sink sees it, including any timestamp.
/// Both sinks use the 512-container depth from `NbtAccounter`, even for trusted
/// SystemChat. The command result is recorded before output.
fn formatted_command_failure(message: Text, timestamp: Option<Text>) -> Text {
    use serde_json::Value;
    use sunrise_core::nbt;

    let format = |message| {
        let failure = Text::empty().append(message).with_color(text::RED);
        match &timestamp {
            Some(prefix) => prefix.clone().append(failure),
            None => failure,
        }
    };
    let output = format(message);
    let serializes = || {
        let component = output.component_view();
        let mut limits = nbt::Limits::default();
        // Check before recursing into the component converters. The writer checks
        // the full tag, including synthetic list wrappers not present in the JSON.
        let mut pending = vec![(component.as_ref(), 0)];
        while let Some((node, depth)) = pending.pop() {
            match node {
                Value::Object(_) | Value::Array(_) if depth >= limits.max_depth => return false,
                Value::Object(fields) => pending.extend(fields.values().map(|v| (v, depth + 1))),
                Value::Array(values) => pending.extend(values.iter().map(|v| (v, depth + 1))),
                _ => {}
            }
        }
        let tag = if timestamp.is_some() {
            sunrise_client::scoreboard::component_tag(&component)
        } else {
            sunrise_client::lan::data::component_nbt(&component)
        };
        if timestamp.is_some() {
            // The saved component sits under three containers: the chunk root,
            // the block_entities or Entities list, and the block or cart entry.
            // NbtAccounter counts those before it reads LastOutput.
            limits.max_depth = nbt::Limits::saved().max_depth - 3;
        }
        // Use the strict writer so overlong strings are refused, not saved empty.
        // It also counts wrappers introduced when encoding heterogeneous lists.
        nbt::write_network(&tag, limits).is_ok()
    };
    if serializes() {
        output
    } else {
        format(Text::tr("command.failed", &[]))
    }
}

/// A natural fence knot is only read by commands here: one that would move
/// or remove it fails as a whole instead of skipping it, so it never
/// reports a change it did not make (#13036).
/// The targets a command that moves or kills entities acts on: a natural leash knot is
/// listed for selectors and `/data` but not changed yet, so it is left out, as before it was
/// listed. A selector that found only knots finds no entity.
pub(in crate::game) fn without_knots(targets: Vec<Target>) -> R<Vec<Target>> {
    let found = !targets.is_empty();
    let kept: Vec<Target> = targets
        .into_iter()
        .filter(|t| !matches!(t, Target::Knot(_)))
        .collect();
    if found && kept.is_empty() {
        return Err(err("argument.entity.notfound.entity", &[]));
    }
    Ok(kept)
}
/// `EntityType.getDescriptionId`: `entity.<namespace>.<path>`.
fn entity_name_key(id: &str) -> String {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("entity.{namespace}.{}", path.replace('/', "."))
}
fn texts(args: &[Text]) -> Vec<Text> {
    args.to_vec()
}
/// String.format(Locale.ROOT, "%.<decimals>f"): Java rounds the shortest
/// decimal text half-up, which `{:.N}` (exact value, half-even) does not.
pub(super) fn percent_f(v: f64, decimals: usize) -> String {
    sunrise_core::java_number::format_fixed(v, decimals)
}
/// String.format(Locale.ROOT, "%f").
fn format_double(v: f64) -> String {
    percent_f(v, 6)
}
/// `EntitySelectorParser.createRotationPredicate`: the wrapped angle lies in
/// the wrapped range, which runs through 180 when its minimum is above its
/// maximum (defaults 0 and 359).
fn rotation_in((lo, hi): (Option<f64>, Option<f64>), angle: f32) -> bool {
    let min = wrap_degrees(lo.unwrap_or(0.0) as f32);
    let max = wrap_degrees(hi.unwrap_or(359.0) as f32);
    let angle = wrap_degrees(angle);
    if min > max {
        angle >= min || angle <= max
    } else {
        angle >= min && angle <= max
    }
}
/// `Mth.wrapDegrees`.
fn wrap_degrees(angle: f32) -> f32 {
    let wrapped = angle % 360.0;
    if wrapped >= 180.0 {
        wrapped - 360.0
    } else if wrapped < -180.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

impl Game<'_> {
    /// `native_player_level_change` for a level that was `from` before an experience change.
    pub(super) fn native_player_level_changed(&mut self, from: i32) {
        let to = self.experience.level;
        if from != to
            && mod_api::__private::listened(mod_api::HookId::PlayerLevelChange)
            && let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone())
        {
            self.native_player_level_change(player, &name, from, to);
        }
    }

    /// `PlayerXpEvent.XpChange` for a `Player.giveExperiencePoints` of `amount`
    /// points: the amount to give, or `None` when a mod cancelled it.
    pub(super) fn xp_change(&mut self, amount: i32) -> Option<i32> {
        if !mod_api::__private::listened(mod_api::HookId::PlayerXpChange) {
            return Some(amount);
        }
        let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone())
        else {
            return Some(amount);
        };
        let (mut amount, mut cancel) = (amount, false);
        let player_entity_id = self
            .lan_acting_entity
            .map_or_else(|| self.own_entity_id(), |id| id as i32);
        self.native_player_xp_change(player, player_entity_id, &name, &mut amount, &mut cancel);
        (!cancel).then_some(amount)
    }

    /// Commands.performCommand for a line without its slash. Client dispatcher
    /// handling belongs to the chat entry points, not server or console execution.
    pub(super) fn run_command(&mut self, command: &str) -> bool {
        let d = self.ui.dispatcher.0.clone();
        let parse = d.parse(command, &self.ui.registries);
        let result = match parse.executable() {
            Err(e) => {
                self.syntax_failure(&e, command);
                return false;
            }
            Ok((ctx, _)) => {
                let ctx = ctx.clone();
                let root = parse.context.clone();
                self.exec = Default::default();
                self.run_chain(&root, &ctx)
            }
        };
        let ok = result.is_ok();
        if let Err(e) = result {
            if e.input.is_some() {
                self.syntax_failure(&e, command);
            } else {
                self.command_failure(e.message);
            }
        }
        // InventoryChangeTrigger fires as slots change, not on the next tick.
        if self.player.is_some() {
            self.check_inventory_trigger();
        }
        ok
    }
    /// `run_command` with its output suppressed
    /// (`CommandSourceStack.withSuppressedOutput`): the player's own source,
    /// rights and checks, no feedback and no failure in the chat.
    pub(super) fn run_command_quietly(&mut self, command: &str) -> bool {
        let quiet = execute::Source {
            silent: true,
            ..self.player_source()
        };
        let outer = self.command_source.replace(quiet);
        let ok = self.run_command(command);
        self.command_source = outer;
        ok
    }
    /// Runs `f` and reports the commands it ran as their source's
    /// `CommandResultCallback`s saw them, in the pair `execute store` stores:
    /// whether any succeeded (the rule of a command block, `run_as_game_master`:
    /// an `execute as @a run ...` forks and calls back once per fork) and the
    /// last callback's result. None when no command reached a callback (a
    /// syntax error, an unknown command, an `execute` forking to no target).
    pub(in crate::game) fn recording_command_outcome<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> T,
    ) -> (T, Option<(bool, i32)>) {
        let outer = self.cmds.results.replace(Vec::new());
        let out = f(self);
        let results = std::mem::replace(&mut self.cmds.results, outer).unwrap_or_default();
        let outcome = results
            .last()
            .map(|&(_, result)| (results.iter().any(|&(success, _)| success), result));
        (out, outcome)
    }
    pub(in crate::game) fn syntax_failure(&mut self, e: &CommandError, command: &str) {
        self.command_failure(e.message.clone());
        if let Some(context) = Self::command_syntax_context(e, command) {
            self.command_failure(context);
        }
    }

    /// Brigadier's clickable cursor context, shared by client and server failure reporting.
    pub(in crate::game) fn command_syntax_context(e: &CommandError, command: &str) -> Option<Text> {
        let window = e.context_window()?;
        let suggest = Style {
            click: Some(text::ClickEvent::SuggestCommand(format!("/{command}"))),
            ..Style::default()
        };
        let mut context = Text::empty();
        if window.ellipsis {
            context.push("...");
        }
        context.push(window.before.as_str());
        if !window.after.is_empty() {
            context.push(
                Text::literal(window.after)
                    .with_color(text::RED)
                    .underlined(),
            );
        }
        context.push(
            Text::translate("command.context.here", &[])
                .with_color(text::RED)
                .italic(),
        );
        Some(context.with_color(text::GRAY).with_style(&suggest))
    }
    /// CommandSourceStack.sendFailure: red text.
    pub(super) fn command_failure(&mut self, message: Text) {
        if self.command_source.as_ref().is_some_and(|s| s.silent) {
            return;
        }
        if let Some(block) = self.command_source.as_ref().and_then(|s| s.block) {
            self.command_block_output(block, |timestamp| {
                formatted_command_failure(message, Some(timestamp))
            });
            return;
        }
        if let Some(cart) = self.command_source.as_ref().and_then(|s| s.cart) {
            self.cart_command_output(cart, |timestamp| {
                formatted_command_failure(message, Some(timestamp))
            });
            return;
        }
        self.send_command_output(
            CommandFeedback::Failure,
            formatted_command_failure(message, None),
        );
    }
    /// `CommandSourceStack.sendSystemMessage`: no success-feedback rule. Unlike `sendSuccess`
    /// and `sendFailure`, it addresses the executing player when there is one.
    pub(super) fn command_system_message(&mut self, message: Text) {
        if self.command_source.as_ref().is_some_and(|s| s.silent) {
            return;
        }
        if self.native_console_source() {
            let conn = match self.source_entity() {
                Some(Target::Player) => self.lan_acting,
                Some(Target::Guest(conn)) => Some(conn),
                _ => {
                    self.send_command_output(CommandFeedback::System, message);
                    return;
                }
            };
            if let Some(conn) = conn {
                if let Some(host) = &self.lan {
                    host.hub
                        .send_to(conn, &super::native_server::system_chat(&message));
                }
            } else {
                self.system_message(message);
            }
            return;
        }
        self.send_command_output(CommandFeedback::System, message);
    }
    /// `sendSuccess(message, false)`: the source is told (when it takes
    /// success feedback), the operators are not.
    pub(super) fn feedback_unbroadcast(&mut self, message: Text) {
        self.send_success(message, false);
    }
    /// `CommandSource.acceptsSuccess`: a player's source takes success feedback while
    /// `send_command_feedback` is on; the server console's and an RCON client's always do.
    fn accepts_success(&self) -> bool {
        self.native_console_source()
            || self.console_broadcast.is_some()
            || self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::SEND_COMMAND_FEEDBACK)
    }
    /// A vanilla command's success line: `sendSuccess(message, true)`, except the
    /// replies vanilla sends with `broadcast` false (`admin::QUIET`).
    pub(super) fn feedback(&mut self, message: Text) {
        let broadcast = !message
            .translation_key()
            .is_some_and(|k| admin::QUIET.contains(&k));
        self.send_success(message, broadcast);
    }
    /// `CommandSourceStack.sendSuccess(message, broadcast)`, the success path of
    /// vanilla's commands and of mods' (`Commands::feedback`). A silent source
    /// sends nothing. The line goes to the origin when it accepts success: a
    /// player under `send_command_feedback`, the console and RCON always, a
    /// command block or minecart that tracks output under `send_command_feedback`;
    /// never to the `execute as` entity. Independently, with `broadcast`,
    /// `broadcastToAdmins` tells the other operators and the log.
    ///
    /// Runs with the executing level in place (`source.dimension`). A block or
    /// minecart origin is read in its own level (`source.origin_dimension`),
    /// which `execute in` does not move.
    pub(in crate::game) fn send_success(&mut self, message: Text, broadcast: bool) {
        if self.command_source.as_ref().is_some_and(|s| s.silent) {
            return;
        }
        let admins = broadcast.then(|| message.clone());
        if let Some(block) = self.command_source.as_ref().and_then(|s| s.block) {
            self.in_origin_level(|g| g.command_block_success(block, message));
        } else if let Some(cart) = self.command_source.as_ref().and_then(|s| s.cart) {
            self.in_origin_level(|g| g.cart_command_success(cart, message));
        } else if self.accepts_success() {
            self.send_command_output(CommandFeedback::Success, message);
        }
        if let Some(message) = admins {
            self.broadcast_to_admins(message);
        }
    }
    /// `f` with the command's origin level in place (`source.origin_dimension`):
    /// where its command block or minecart is, whatever level `execute in` named.
    fn in_origin_level<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let origin = self.command_source.as_ref().map(|s| s.origin_dimension);
        let back = origin.and_then(|dim| self.enter_command_level(dim));
        let out = f(self);
        if let Some(back) = back {
            self.enter_command_level(back);
        }
        out
    }
    /// `CommandSource.shouldInformAdmins` of the command's origin: a player's
    /// always (`ServerPlayer`); a command block's or minecart's while it tracks
    /// output and `command_block_output` is on (`BaseCommandBlock`, which reads
    /// the block itself: looked up in its own level); the console's
    /// `broadcast-console-to-ops` (`DedicatedServer`) and RCON's
    /// `broadcast-rcon-to-ops` (`RconConsoleSource`).
    fn should_inform_admins(&mut self) -> bool {
        let origin = self
            .command_source
            .as_ref()
            .and_then(|s| s.block.map(Ok).or(s.cart.map(Err)));
        let Some(origin) = origin else {
            return self.console_broadcast.unwrap_or(true);
        };
        let tracks = self.in_origin_level(|g| match origin {
            Ok(pos) => g.command_block(pos).is_some_and(|c| c.track_output),
            Err(id) => g.carts.iter().any(|c| {
                c.id == id
                    && c.kind == super::minecarts::CartKind::CommandBlock
                    && c.command.track_output
            }),
        });
        tracks
            && self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::COMMAND_BLOCK_OUTPUT)
    }
    /// `CommandSourceStack.getDisplayName` for the admin line: the executing
    /// entity's name, read where `execute in` put the command (`source.dimension`,
    /// in place); a command-block minecart executing as itself, and a command
    /// block's own name, in the origin's level; the server's for the console.
    fn admin_display_name(&mut self) -> Text {
        let Some((entity, own_cart)) = self.command_source.as_ref().map(|source| {
            (
                source.entity,
                source.cart.is_some() && source.entity == source.cart.map(Target::Cart),
            )
        }) else {
            return if self.console_broadcast.is_some() || self.native_console_source() {
                self.server_name()
            } else {
                self.source_display_name()
            };
        };
        match entity {
            Some(_) if !own_cart => self.source_display_name(),
            _ => self.in_origin_level(|g| g.source_display_name()),
        }
    }
    /// `CommandSourceStack.broadcastToAdmins` when the origin informs the
    /// operators: `[name: message]` (`chat.type.admin`, gray and italic, with
    /// the source's display name: the `execute as` entity's, else the block's,
    /// minecart's or server's). Under `send_command_feedback` every other
    /// operator gets it (`PlayerList.isOp`, queued while the command runs;
    /// `send_admin_feedback` sends it). Under `log_admin_commands` the server's
    /// log gets it, unless the server's console is the source. Neither rule
    /// gates the other.
    pub(super) fn broadcast_to_admins(&mut self, message: Text) {
        if !self.should_inform_admins() {
            return;
        }
        let rules = &self.ui.rules;
        let ops = rules.bool_at(sunrise_client::gamerules::vanilla::SEND_COMMAND_FEEDBACK);
        // `source != server`: RCON is a source of its own and is logged.
        let console_source =
            (self.console_broadcast.is_some() || self.native_console_source()) && !self.rcon_source;
        let log = !console_source
            && rules.bool_at(sunrise_client::gamerules::vanilla::LOG_ADMIN_COMMANDS);
        if !ops && !log {
            return;
        }
        let who = self.admin_display_name();
        let line = Text::translate("chat.type.admin", &[who, message])
            .with_color(text::GRAY)
            .italic();
        if log {
            self.log_server_system_message(&line);
        }
        if ops {
            // The player whose command it is never hears it back; a block's,
            // minecart's or console's line has no such player.
            let origin_is_player = self.console_broadcast.is_none()
                && self.command_source.as_ref().is_none_or(|s| s.origin_player);
            let source = if origin_is_player {
                self.lan_acting
            } else {
                None
            };
            self.admin_feedback.push((line, source));
        }
    }
    /// `MinecraftServer.sendSystemMessage`: a line in the server's log, as the
    /// console's replies are logged. Kept out of an RCON reply, which carries
    /// only the source's own output.
    pub(in crate::game) fn log_server_system_message(&self, line: &Text) {
        let message = format!("System chat: {}", line.plain());
        if self.is_dedicated_server() {
            super::dedicated::log_outside_rcon_reply(self.rcon_source, &message);
        } else {
            log::info!("{message}");
        }
    }
    /// Player.getDisplayName: the name (team-formatted), suggesting a
    /// `/tell`, showing the player on hover, inserting the name.
    pub(super) fn player_display_name(&self) -> Text {
        let name = self.acting_player_name();
        if let Some(uuid) = self.lan_acting_uuid {
            return self.player_text(name, uuid);
        }
        let mut displayname = self.target_name(Target::Player);
        sunrise_client::lan::hub::native_player_name_format(
            &self.loop_context,
            &Text::literal(name),
            &mut displayname,
        );
        displayname
    }
    pub(super) fn source_position(&self) -> [f64; 3] {
        if let Some(s) = &self.command_source {
            return s.pos;
        }
        match &self.player {
            Some(p) => [p.position.x, p.position.y, p.position.z],
            None => self.server_position(),
        }
    }
    /// MinecraftServer.createCommandSourceStack's position: the corner of
    /// the respawn block (`Vec3.atLowerCornerOf`).
    pub(super) fn server_position(&self) -> [f64; 3] {
        self.spawn.map_or([0.0, 64.0, 0.0], |s| {
            [s.x.floor(), s.y.floor(), s.z.floor()]
        })
    }
    pub(super) fn source_rotation(&self) -> [f32; 2] {
        self.command_source
            .as_ref()
            .map_or([self.yaw, self.pitch], |s| s.rot)
    }
    /// LocalCoordinates use the anchored position; world coordinates the position.
    pub(super) fn resolve_coords(&self, c: &Coordinates) -> [f64; 3] {
        let origin = match (c, &self.command_source) {
            (Coordinates::Local(_), Some(s)) => self.anchor_position(s),
            _ => self.source_position(),
        };
        c.position(origin, self.source_rotation())
    }
    pub(super) fn coords(&self, ctx: &Context, name: &str) -> Option<Coordinates> {
        match ctx.get(name) {
            Some(Arg::Coords(c)) => Some(c.clone()),
            _ => None,
        }
    }
    pub(super) fn position_arg(&self, ctx: &Context, name: &str) -> Option<[f64; 3]> {
        self.coords(ctx, name).map(|c| self.resolve_coords(&c))
    }
    fn int(ctx: &Context, name: &str) -> Option<i32> {
        match ctx.get(name) {
            Some(Arg::Int(v)) => Some(*v),
            _ => None,
        }
    }
    // ---- entities -------------------------------------------------------

    /// The name of the player the game acts for: a guest's own while their
    /// command or action runs (`Player.getName`).
    pub(super) fn acting_player_name(&self) -> &str {
        self.lan_acting_name.as_deref().unwrap_or(PLAYER_NAME)
    }
    /// Entity.getDisplayName.
    pub(super) fn target_name(&self, t: Target) -> Text {
        match t {
            Target::Player => {
                let name = self.acting_player_name();
                if let Some(uuid) = self.lan_acting_uuid {
                    return self.player_text(name, uuid);
                }
                match self.score.board.players_team(name) {
                    // PlayerTeam.formatNameForTeam.
                    Some(team) => team.formatted_name(Text::literal(name)),
                    None => Text::literal(name),
                }
            }
            Target::Guest(conn) => match self.command_guest(conn) {
                Some(g) => self.player_text(&g.name, g.uuid),
                None => Text::empty(),
            },
            Target::Mob(id) => self.mob_command_name(id),
            other => self.entity_display_name(other),
        }
    }
    /// `type=`: every positive must match, no negative may match.
    fn selector_types_match(&self, s: &Selector, ty: &str) -> bool {
        s.types.iter().all(|(want, neg)| {
            let matches = if let Some(tag) = want.strip_prefix('#') {
                self.entity_tag_matches(tag, ty)
            } else {
                want == ty
            };
            matches != *neg
        })
    }
    /// The candidates of the level in place that pass the selector
    /// (EntitySelector.getPredicate).
    pub(super) fn select_in_level(
        &mut self,
        candidates: Vec<Target>,
        s: &Selector,
        origin: [f64; 3],
    ) -> Vec<(Target, f64)> {
        let level = self.experience.level;
        // EntitySelector tests the type before any predicate. With no scores=
        // or team= (whose holders may be made as they are read), the rest of
        // the extra filter is a pure test of each candidate, so the ones of
        // another type leave first: `@e[type=marker,tag=…]` no longer reads
        // the tags of every falling block and item in the level.
        let candidates = if !s.types.is_empty() && s.scores.is_empty() && s.teams.is_empty() {
            candidates
                .into_iter()
                .filter(|&t| self.selector_types_match(s, &self.target_type(t)))
                .collect()
        } else {
            candidates
        };
        let passing = self.selector_extra_filter(&candidates, s);
        candidates
            .into_iter()
            .filter_map(|t| {
                let p = self.target_position(t)?;
                let ty = self.target_type(t);
                if !self.selector_types_match(s, &ty) {
                    return None;
                }
                if !s.include_entities
                    && !t.is_player()
                    && s.types.iter().all(|(w, n)| *n || w != &ty)
                    && s.kind != 'e'
                    && s.kind != 'n'
                {
                    return None;
                }
                for (name, neg) in &s.names {
                    let n = self.target_plain_name(t);
                    if (n == *name) == *neg {
                        return None;
                    }
                }
                if !passing.contains(&t) {
                    return None;
                }
                let player = match t {
                    // ServerPlayer.gameMode: the real mode, not `creative`.
                    Target::Player => Some((level, self.mode.name())),
                    Target::Guest(conn) => self.guest_level_and_mode(conn),
                    _ => None,
                };
                if let Some((lo, hi)) = s.level
                    && player.is_none_or(|(level, _)| {
                        lo.is_some_and(|l| level < l) || hi.is_some_and(|h| level > h)
                    })
                {
                    return None;
                }
                for (mode, neg) in &s.gamemodes {
                    if player.is_none_or(|(_, current)| (current == mode) == *neg) {
                        return None;
                    }
                }
                let d2 = (p[0] - origin[0]).powi(2)
                    + (p[1] - origin[1]).powi(2)
                    + (p[2] - origin[2]).powi(2);
                if let Some((lo, hi)) = s.distance
                    && (lo.is_some_and(|l| d2 < l * l) || hi.is_some_and(|h| d2 > h * h))
                {
                    return None;
                }
                // x_rotation is the pitch, y_rotation the yaw (getXRot, getYRot).
                let [yaw, pitch] = self.target_rotation(t);
                if let Some(range) = s.x_rotation
                    && !rotation_in(range, pitch)
                {
                    return None;
                }
                if let Some(range) = s.y_rotation
                    && !rotation_in(range, yaw)
                {
                    return None;
                }
                if s.delta.iter().any(Option::is_some) {
                    let d = s.delta.map(|v| v.unwrap_or(0.0));
                    if let Some(bounds) = self.selector_target_bounds(t) {
                        if !selector_bounds::selector_box(origin, d).intersects(bounds) {
                            return None;
                        }
                    } else {
                        // Unmodeled boxes retain their previous point fallback.
                        for i in 0..3 {
                            let (a, b) = (
                                origin[i].min(origin[i] + d[i]),
                                origin[i].max(origin[i] + d[i]) + 1.0,
                            );
                            if p[i] < a || p[i] >= b {
                                return None;
                            }
                        }
                    }
                }
                Some((t, d2))
            })
            .collect()
    }
    /// `EntityLookup.getEntities`: insertion order for the joined families in
    /// unbounded arbitrary queries. Spatial queries keep their existing path;
    /// their section membership order is a separate traversal.
    /// With only the `Entity::isAlive` ones when `alive_only` (`@e` and `@n`).
    pub(in crate::game) fn global_selector_candidates(&self, alive_only: bool) -> Vec<Target> {
        use super::entity::{Entity as _, entity_lists_of, with_entity};
        let original = self.level_candidates(alive_only);
        // Counts preserve every occurrence, including nonwalk fallback targets.
        // The map is never iterated to choose order and is not an order store.
        let mut remaining = std::collections::HashMap::<Target, usize>::new();
        for target in &original {
            *remaining.entry(*target).or_default() += 1;
        }
        let mut ordered = Vec::with_capacity(original.len());
        let walked = entity_lists_of!(self).walk_in_join_order(|handle, _| {
            if let Some(target) = with_entity!(self, handle, entity => entity.selector_target())
                && let Some(count) = remaining.get_mut(&target)
                && *count > 0
            {
                *count -= 1;
                ordered.push(target);
            }
        });
        // Invalid/unjoined storage retains the whole old path, never a prefix.
        if walked.is_err() {
            return original;
        }
        for target in original {
            let count = remaining.get_mut(&target).expect("an original candidate");
            if *count > 0 {
                *count -= 1;
                ordered.push(target);
            }
        }
        ordered
    }

    /// The entities of the level in place a selector can find: one walk over
    /// the entity lists (a kind added there joins `@e` once it names its
    /// `Target`), then the kinds that are not on the walk. An
    /// unsorted selector lists them in that order.
    /// With only the `Entity::isAlive` ones when `alive_only`:
    /// `EntitySelectorParser` gives `@e` and `@n` that predicate, so a killed
    /// mob through its death animation, a stand or a dragon at no health is
    /// not among them.
    pub(in crate::game) fn level_candidates(&self, alive_only: bool) -> Vec<Target> {
        use super::entity::{Entity as _, entity_lists_of, for_each_entity};
        let mut candidates = Vec::new();
        for_each_entity!(entity_lists_of!(self), entity => {
            if !alive_only || entity.is_alive() {
                candidates.extend(entity.selector_target());
            }
        });
        candidates.extend(self.item_frames.iter().map(|f| Target::Frame(f.id)));
        candidates.extend(self.paintings.iter().map(|p| Target::Painting(p.id)));
        candidates.extend(self.cushions.iter().map(|c| Target::Cushion(c.id)));
        candidates.extend(self.others.iter().map(|o| Target::Other(o.id)));
        candidates.extend(self.fangs.list.iter().map(|f| Target::Fangs(f.id)));
        // The End crystals and the dragon (EndCrystal and EnderDragon
        // are entities of their own).
        candidates.extend(self.crystals.iter().map(|c| Target::Crystal(c.id)));
        // EnderDragon.isAlive is its health's, through the dying flight too.
        if let Some(d) = self.dragon.as_ref()
            && (!alive_only || d.health > 0.0)
        {
            candidates.push(Target::Dragon(d.id));
        }
        candidates.extend(self.ominous_spawners.iter().map(|s| Target::Spawner(s.id)));
        candidates.extend(
            self.breath_clouds
                .iter()
                .filter(|c| c.id != 0)
                .map(|c| Target::Cloud(c.id)),
        );
        candidates.extend(
            self.bolts
                .iter()
                .filter(|b| b.id != 0)
                .map(|b| Target::Bolt(b.id)),
        );
        // `FishingHook`: every cast bobber of this level (#6100). A guest's is here with its
        // guest (`fishing_hooks_here` keeps those); the player in place's hook is not swapped
        // with the level, so it counts only in the level that player is in, as vanilla's
        // `EntitySelector.findEntities` finds a bobber only in its own level.
        let player_here = self
            .current_player_resident_location()
            .is_some_and(|location| location.owner.dimension == self.dim.key());
        candidates.extend(
            self.fishing_hooks_here()
                .filter(|(_, caster)| player_here || *caster != Target::Player)
                .map(|(hook, _)| Target::Hook(hook.id())),
        );
        candidates.extend(self.natural_knot_ids().into_iter().map(Target::Knot));
        // `isRemoved` from a mod's call on, though the sweep has not taken it out.
        candidates.retain(|t| !self.target_removed(t));
        candidates
    }
    /// `Level.getEntity`: the entity of the level in place with a network
    /// id, players aside.
    pub(in crate::game) fn level_entity(&self, id: i32) -> Option<Target> {
        let id = id as u32;
        match self.server_resident_entity(id) {
            sunrise_client::join_seq::residency::table::EntityLookup::Found { location, value }
                if location.owner.dimension == self.dim.key()
                    && !matches!(
                        location.owner.kind,
                        sunrise_client::join_seq::residency::Kind::Player
                            | sunrise_client::join_seq::residency::Kind::Hook
                            | sunrise_client::join_seq::residency::Kind::DragonPart
                    ) =>
            {
                Some(value)
            }
            // A client's own copy of a row its server shows it.
            _ => self
                .remote
                .as_ref()
                .is_some_and(|r| r.general.rows.contains_key(&(id as i32)))
                .then_some(Target::General(id)),
        }
    }
    /// `ServerLevel.getEntityOrPart` for a part: the `EnderDragonPart` of the
    /// level in place with this network id. `level_entity` does not find it,
    /// as `Level.getEntity` does not: no selector or command reaches a part.
    pub(in crate::game) fn level_dragon_part(&self, id: u32) -> Option<Target> {
        let part = Target::DragonPart(id);
        self.level_holds(part).then_some(part)
    }
    /// `ServerLevel.getEntity`: whether the level in place still holds the
    /// entity a target names. Discard removes mobs from lookup immediately.
    fn level_holds(&self, t: Target) -> bool {
        let Some(id) = self.resident_target_id(t) else {
            return false;
        };
        match self.server_resident_entity(id) {
            sunrise_client::join_seq::residency::table::EntityLookup::Found { location, value }
                if location.owner.dimension == self.dim.key() =>
            {
                value == t
                    || matches!(t, Target::Guest(conn) if self.lan_acting == Some(conn) && value == Target::Player)
            }
            _ => false,
        }
    }
    /// Run `f` with the level that holds the entity a target names in place
    /// (the entity ids are unique across the loaded levels) and put the
    /// level back afterwards. `f` does not run when no level holds the
    /// entity.
    pub(in crate::game) fn in_target_level<R>(
        &mut self,
        t: Target,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        self.with_level_in_place(self.resident_target_dimension(t)?, f)
    }
    pub(in crate::game) fn entity_tag_matches(&self, tag: &str, ty: &str) -> bool {
        let tag = sunrise_client::command::args::normalized(tag);
        self.items().entity_type_tags.contains(&tag, ty)
    }
    /// EntityArgument.getEntities / getPlayers / getEntity / getPlayer.
    pub(super) fn entities(&mut self, ctx: &Context, name: &str, players: bool) -> R<Vec<Target>> {
        let Some(Arg::Entity(s)) = ctx.get(name).cloned() else {
            return Ok(vec![self.implied_player()]);
        };
        self.resolve_entity_argument(&s, players)
    }
    /// Resolve an already parsed entity argument against the current command source.
    pub(super) fn resolve_entity_argument(
        &mut self,
        selector: &Selector,
        players: bool,
    ) -> R<Vec<Target>> {
        let found: Vec<Target> = self
            .select(selector)
            .into_iter()
            .filter(|t| !players || t.is_player())
            .collect();
        if found.is_empty() {
            return Err(err(
                if players {
                    "argument.entity.notfound.player"
                } else {
                    "argument.entity.notfound.entity"
                },
                &[],
            ));
        }
        Ok(found)
    }
    // ---- dispatch -------------------------------------------------------

    fn execute_command(&mut self, id: &str, ctx: &Context) -> R<i32> {
        match id {
            "native_mod_command" => self.run_native_command(ctx.clone()),
            "help" => self.cmd_help(ctx),
            "gamemode" => self.cmd_gamemode(ctx),
            "defaultgamemode" => {
                let Some(Arg::GameMode(m)) = ctx.get("gamemode") else {
                    return Ok(0);
                };
                self.set_default_game_mode(match m {
                    sunrise_client::command::args::GameMode::Survival => 0,
                    sunrise_client::command::args::GameMode::Creative => 1,
                    sunrise_client::command::args::GameMode::Adventure => 2,
                    sunrise_client::command::args::GameMode::Spectator => 3,
                });
                self.feedback(Text::translate(
                    "commands.defaultgamemode.success",
                    &[Text::translate(&format!("gameMode.{}", m.name()), &[])],
                ));
                Ok(0)
            }
            "give" => self.cmd_give(ctx),
            "teleport.location"
            | "teleport.destination"
            | "teleport.targets.location"
            | "teleport.targets.destination" => self.cmd_teleport(id, ctx),
            id if id.starts_with("time.") => self.cmd_time(id, ctx),
            id if id.starts_with("raid.") => self.cmd_raid(id, ctx),
            "weather" => self.cmd_weather(ctx),
            "kill" => self.cmd_kill(ctx),
            "summon" => self.cmd_summon(ctx),
            "effect.give" => self.cmd_effect_give(ctx),
            "effect.clear" => self.cmd_effect_clear(ctx),
            "enchant" => self.cmd_enchant(ctx),
            "tellraw" => self.cmd_tellraw(ctx),
            "title" => self.cmd_title(ctx),
            "setblock" => self.cmd_setblock(ctx),
            "fill" => self.cmd_fill(ctx),
            "clone" => self.cmd_clone(ctx),
            "experience" => self.cmd_experience(ctx),
            "advancement" => self.cmd_advancement(ctx),
            "gamerule" => self.cmd_gamerule(ctx),
            "difficulty" => self.cmd_difficulty(ctx),
            "seed" => {
                let seed = self.level.seed.to_string();
                let value = Text::copyable(seed);
                self.feedback_unbroadcast(Text::translate("commands.seed.success", &[value]));
                Ok(self.level.seed as i32)
            }
            "spawnpoint" => self.cmd_spawnpoint(ctx),
            "setworldspawn" => self.cmd_setworldspawn(ctx),
            "clear" => self.cmd_clear(ctx),
            "item" => self.cmd_item(ctx),
            "item.modify" => self.cmd_item_modify(ctx),
            "spectate" => self.cmd_spectate(ctx),
            id if id.starts_with("waypoint.") => self.cmd_waypoint(id, ctx),
            id if id.starts_with("dialog.") => self.cmd_dialog(id, ctx),
            id if id.starts_with("posteffect.") => self.cmd_posteffect(id, ctx),
            "kick" => self.cmd_kick(ctx),
            "op" | "deop" | "ban" | "pardon" => self.cmd_profiles(id, ctx),
            "ban-ip" | "pardon-ip" => self.cmd_ip_bans(id, ctx),
            id if id.starts_with("banlist") => self.cmd_banlist(id),
            id if id.starts_with("whitelist.") => self.cmd_whitelist(id, ctx),
            "setidletimeout" => self.cmd_setidle_timeout(ctx),
            "stop" => self.cmd_stop(),
            "save-all" => self.cmd_save_all(),
            "save-off" | "save-on" => self.cmd_auto_save(id == "save-on"),
            "transfer" => self.cmd_transfer(ctx),
            "fetchprofile" => self.cmd_fetchprofile(ctx),
            "publish" => self.cmd_publish(ctx),
            "unpublish" => self.cmd_unpublish(),
            id if id.starts_with("modprofile.") => self.cmd_modprofile(id),
            "debug" => self.cmd_debug(ctx),
            "jfr" => self.cmd_jfr(ctx),
            "version" => self.cmd_version(),
            id if id.starts_with("test.") => self.cmd_test(id, ctx),
            "compute" => self.cmd_compute(ctx),
            "place" => self.cmd_place(ctx),
            "place.feature" | "place.jigsaw" | "place.structure" => {
                self.cmd_place_generated(id, ctx)
            }
            "recipe.give" | "recipe.take" => self.cmd_recipe(id, ctx),
            "damage" => self.cmd_damage(ctx),
            "rotate" => self.cmd_rotate(ctx),
            "swing" => self.cmd_swing(ctx),
            "playsound" => self.cmd_playsound(ctx),
            "stopsound" => self.cmd_stopsound(ctx),
            "particle" => self.cmd_particle(ctx),
            id if id.starts_with("attribute.") => {
                let result = self.cmd_attribute(id, ctx);
                // AttributeInstance.setDirty: the entity reacts when it ticks.
                if result.is_ok()
                    && matches!(ctx.get("attribute"), Some(Arg::Resource(a)) if a == "minecraft:waypoint_transmit_range")
                    && !id.ends_with("get")
                    && let Ok(t) = self.entities(ctx, "target", false)
                {
                    // Players already have a live attribute dirty callback.
                    if !matches!(t[0], Target::Player | Target::Guest(_)) {
                        self.cmds.waypoint_dirty.push(t[0]);
                    }
                }
                result
            }
            "ride.mount" | "ride.dismount" => self.cmd_ride(id, ctx),
            "spreadplayers" => self.cmd_spreadplayers(ctx),
            id if id.starts_with("forceload.") => self.cmd_forceload(id, ctx),
            id if id.starts_with("pregen.") => self.cmd_pregen(id, ctx),
            "fillbiome" => self.cmd_fillbiome(ctx),
            id if id.starts_with("worldborder.") => self.cmd_worldborder(id, ctx),
            id if id.starts_with("loot.") => self.cmd_loot(id, ctx),
            "teammsg" => self.cmd_teammsg(ctx),
            id if id.starts_with("stopwatch.") => self.cmd_stopwatch(id, ctx),
            id if id.starts_with("tick.") => self.cmd_tick(id, ctx),
            id if id.starts_with("random.") => self.cmd_random(id, ctx),
            "function" => self.cmd_function(ctx),
            "return.value" | "return.fail" => self.cmd_return(id, ctx),
            "schedule.function" | "schedule.clear" => self.cmd_schedule(id, ctx),
            "reload" => self.cmd_reload(),
            id if id.starts_with("datapack.") => self.cmd_datapack(id, ctx),
            id if id.starts_with("bossbar.") => self.cmd_bossbar(id, ctx),
            id if id.starts_with("data.") => self.cmd_data(id, ctx),
            id if id.starts_with("tag.") => self.cmd_tag(id, ctx),
            id if id.starts_with("scoreboard.")
                || id.starts_with("team.")
                || id.starts_with("trigger") =>
            {
                self.cmd_scoreboard(id, ctx)
            }
            "locate.structure" | "locate.biome" => self.cmd_locate(id, ctx),
            "locate.poi" => self.cmd_locate_poi(ctx),
            "warden_spawn_tracker.clear" | "warden_spawn_tracker.set" => {
                let level = Self::int(ctx, "warning_level");
                self.cmd_warden_spawn_tracker(level);
                Ok(1)
            }
            "say" | "me" | "msg" => self.cmd_say(id, ctx),
            "list" => self.cmd_list(ctx),
            _ => Err(err("command.failed", &[])),
        }
    }
    fn cmd_help(&mut self, ctx: &Context) -> R<i32> {
        let d = self.ui.dispatcher.0.clone();
        let level = self.ui.registries.permission;
        match ctx.get("command") {
            None => {
                let usage = d.smart_usage(0, level);
                let n = usage.len() as i32;
                for (_, line) in usage {
                    self.feedback_unbroadcast(Text::literal(format!("/{line}")));
                }
                Ok(n)
            }
            Some(Arg::Str(command)) => {
                let parse = d.parse(command, &self.ui.registries);
                if parse.context.nodes.is_empty() {
                    return Err(err("commands.help.failed", &[]));
                }
                let last = parse.context.nodes.last().map(|n| n.0).unwrap_or(0);
                let usage = d.smart_usage(last, level);
                let prefix = &command[..parse.context.range.1.min(command.len())];
                for (_, line) in &usage {
                    self.feedback_unbroadcast(Text::literal(format!("/{prefix} {line}")));
                }
                if usage.is_empty() {
                    self.feedback_unbroadcast(Text::literal(format!("/{prefix}")));
                }
                Ok(usage.len().max(1) as i32)
            }
            _ => Ok(0),
        }
    }
    pub(super) fn set_game_mode(&mut self, mut mode: GameMode) -> bool {
        if self.mode == mode {
            return false;
        }
        // A server player's change is heard by the mods first, who may keep or redirect it.
        if let Some(uuid) = self.lan_acting_uuid {
            let name = self.acting_player_name().to_owned();
            let mut cancel = false;
            self.native_player_game_mode_changed(uuid, &name, self.mode, &mut mode, &mut cancel);
            if cancel || self.mode == mode {
                return false;
            }
        }
        self.set_mode(mode);

        true
    }
    fn cmd_gamemode(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::GameMode(mode)) = ctx.get("gamemode").cloned() else {
            return Ok(0);
        };
        let targets = if ctx.get("target").is_some() {
            self.entities(ctx, "target", true)?
        } else {
            vec![self.implied_player()]
        };
        let mut count = 0;
        for t in targets {
            let mode_name = Text::translate(&format!("gameMode.{}", mode.name()), &[]);
            // GameModeCommand.logGamemodeChange: the source's own entity.
            let own = self.source_entity() == Some(t);
            let told = !own
                && self
                    .ui
                    .rules
                    .bool_at(sunrise_client::gamerules::vanilla::SEND_COMMAND_FEEDBACK);
            let changed = self.for_player(t, |g| {
                if !g.set_game_mode(mode) {
                    return false;
                }
                if told {
                    g.system_message(Text::translate(
                        "gameMode.changed",
                        std::slice::from_ref(&mode_name),
                    ));
                }
                true
            });
            if changed != Some(true) {
                continue;
            }
            count += 1;
            if own {
                self.feedback(Text::translate(
                    "commands.gamemode.success.self",
                    &[mode_name],
                ));
            } else {
                self.feedback(Text::translate(
                    "commands.gamemode.success.other",
                    &[self.target_name(t), mode_name],
                ));
            }
        }
        Ok(count)
    }
    /// `ItemStack.getDisplayName`: the hover name (custom name in italics,
    /// a written book's title, a potion's effect name, a lodestone compass, else the item's
    /// translated name) in brackets, in the rarity colour and showing the
    /// stack on hover unless it is empty.
    pub(super) fn stack_display_name(&self, s: &ItemStack) -> Text {
        let item = &*s.item;
        let empty = item == "minecraft:air" || s.count == 0;
        let custom = s.data().and_then(|data| data.custom_name.as_ref());
        let hover = self.stack_hover_name_text(s);
        // Component.empty().append(getHoverName()), italic for a custom name.
        let mut name = Text::empty().append(hover);
        if custom.is_some() {
            name = name.italic();
        }
        let wrapped = Text::translate("chat.square_brackets", &[name]);
        if empty {
            return wrapped;
        }

        // HoverEvent.ShowItem: the count only when it is not one.
        let mut shown = serde_json::json!({"action": "show_item", "id": item});
        if s.count != 1 {
            shown["count"] = s.count.into();
        }
        // ItemStackTemplate.fromNonEmptyStack keeps the stack's component patch.
        if let Some(patch) = sunrise_client::item_data::encode_components(s, self.items())
            && let Ok(components) = sunrise_core::json_nbt::tag_to_json(&patch)
        {
            shown["components"] = components;
        }
        wrapped
            .with_color(self.stack_rarity_color(s))
            .with_component_fields(vec![("hover_event".into(), shown)])
    }
    fn cmd_give(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Item(ItemInput { id, components, .. })) = ctx.get("item").cloned() else {
            return Ok(0);
        };
        let (damage, extra) = match components
            .as_deref()
            .map(sunrise_client::item_data::command_components)
        {
            Some(Ok(tag)) => sunrise_client::item_data::decode_components(Some(&tag)),
            Some(Err(e)) => return Err(CommandError::new(Text::literal(e))),
            None => (0, None),
        };
        let prototype = ItemStack {
            item: id.as_str().into(),
            count: 1,
            damage,
            extra,
        };
        let count = Self::int(ctx, "count").unwrap_or(1);
        let targets = self.entities(ctx, "targets", true)?;
        let max_stack = self.items().get(&id).map_or(64, |i| i.max_stack) as i32;
        // A native mod's item can stack far past vanilla's 99: saturate rather
        // than overflow on a size near i32::MAX.
        let limit = max_stack.saturating_mul(100);
        if count > limit {
            return Err(CommandError::new(Text::translate(
                "commands.give.failed.toomanyitems",
                &[
                    Text::literal(limit.to_string()),
                    self.stack_display_name(&prototype),
                ],
            )));
        }
        // An air stack is empty: nothing is given (but the message still is).
        let empty = id == "minecraft:air";
        for &t in targets.iter().filter(|_| !empty) {
            // The player's UUID, which an item they could not hold is set to.
            let conn = match t {
                Target::Guest(conn) => Some(conn),
                _ => self.lan_acting,
            };
            let uuid = match conn {
                Some(conn) => self
                    .command_guest(conn)
                    .map(|g| sunrise_client::lan::playerdata::uuid_ints(g.uuid)),
                None => Some(self.level.ensure_player_uuid()),
            };
            self.for_player(t, |g| {
                // An acting player in another level than the command's (`execute in`) gets
                // what does not fit dropped, and the trigger's rewards, in their own.
                let theirs = conn
                    .and_then(|conn| g.command_guest(conn))
                    .map(|guest| Dim::from_id(&guest.dimension));
                let back = theirs.and_then(|dim| g.enter_command_level(dim));
                let mut remaining = count;
                while remaining > 0 {
                    let size = remaining.min(max_stack);
                    remaining -= size;
                    let rest = g.give_acting_stack(
                        ItemStack {
                            count: size as u32,
                            ..prototype.clone()
                        },
                        uuid,
                    );
                    debug_assert!(rest.is_none(), "the player's own level is in place");
                }
                // containerMenu.broadcastChanges: InventoryChangeTrigger
                // (and its announcement) comes before the command's message.
                if g.player.is_some() {
                    g.check_inventory_trigger();
                }
                if let Some(back) = back {
                    g.enter_command_level(back);
                }
            });
        }
        let name = self.stack_display_name(&prototype);
        let message = if targets.len() == 1 {
            Text::translate(
                "commands.give.success.single",
                &[
                    Text::literal(count.to_string()),
                    name,
                    self.target_name(targets[0]),
                ],
            )
        } else {
            Text::translate(
                "commands.give.success.multiple",
                &texts(&[
                    Text::literal(count.to_string()),
                    name,
                    Text::literal(targets.len().to_string()),
                ]),
            )
        };
        self.feedback(message);
        Ok(targets.len() as i32)
    }
    /// `Entity.teleportTo(level, x, y, z, relatives, yRot, xRot, ...)`, as
    /// `/tp` and `/spreadplayers` call it: its transition has no motion and no
    /// relative delta, so `teleportSetPosition` leaves the moving entity still.
    /// (An end gateway's or a vehicle's transition carries its motion.)
    pub(super) fn teleport_stops(&mut self, t: Target) {
        let _ = self.in_target_level(t, |g| {
            target_entity_mut!(g, t, e => *e.velocity_mut() = Vec3::ZERO);
            // The dragon's flight builds on the motion it carries.
            if let Target::Dragon(id) = t
                && let Some(d) = g.dragon.as_mut().filter(|d| d.id == id)
            {
                d.teleport_motion([false; 3]);
            }
        });
    }
    /// `TeleportCommand.performTeleport` after a successful `teleportTo` in
    /// the target's own level: unless it is a fall-flying living entity, its
    /// vertical motion is zeroed and it is on the ground; a mob's navigation
    /// stops (`PathfinderMob`; the others hold no path). The players' own
    /// ground is their clients'.
    fn tp_lands(&mut self, t: Target) {
        // Player placement already handles motion and fall-flying ground state.
        if matches!(t, Target::Player | Target::Guest(_)) {
            return;
        }
        let _ = self.in_target_level(t, |g| {
            if let Target::Mob(id) = t
                && let Some(m) = g.mobs.iter_mut().find(|m| m.id == id)
            {
                m.nav_stop();
                if m.extra.fall_flying {
                    return;
                }
            }
            target_entity_mut!(g, t, e => {
                e.velocity_mut().y = 0.0;
                if matches!(e.update_retained_ground(&g.world, g.collision, &world_items!(g).entity_type_tags, true, None), Ok(false)) {
                    e.set_on_ground(true);
                }
            });
        });
    }
    pub(in crate::game) fn teleport_to(
        &mut self,
        t: Target,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        turn_riders: bool,
    ) {
        self.teleport_to_relative(t, pos, rotation, [false; 3], turn_riders);
    }
    fn teleport_to_relative(
        &mut self,
        t: Target,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        relative: [bool; 3],
        turn_riders: bool,
    ) {
        // An entity in another level is teleported in the level that holds it.
        let _ = self.in_target_level(t, |g| {
            g.teleport_to_in_level(t, pos, rotation, relative, turn_riders)
        });
    }
    pub(super) fn teleport_to_in_level(
        &mut self,
        t: Target,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        relative: [bool; 3],
        turn_riders: bool,
    ) {
        let from = self.place_teleport_filed(t, pos, rotation, relative, turn_riders);
        if let (Some(entity), Some(to)) = (self.network_entity_id(t), self.target_position(t))
            && let (true, Some(from)) = (self.options.dedicated.is_some(), from)
        {
            self.native_entity_teleport(self.dim.id(), entity as u32, from, to);
        }
    }

    /// `ServerPlayer.teleport` via `ServerGamePacketListenerImpl.teleport`: place the acting
    /// player's body and send the teleport at the end of the guest turn. Returns its old feet
    /// position, without adding command-only chunk holds or notifications.
    pub(super) fn place_player_teleport_in_level(
        &mut self,
        at: Vec3,
        rotation: Option<[f32; 2]>,
        relative: [bool; 3],
    ) -> Option<[f64; 3]> {
        self.stop_riding();
        let from = self.player.as_mut().map(|p| {
            let from = [p.position.x, p.position.y, p.position.z];
            p.position = at;
            p.velocity = teleport_velocity(p.velocity, relative, p.fall_flying);
            p.teleported = true;
            p.teleported_delta = relative;
            from
        });
        self.previous_position = at;
        if let Some([yaw, pitch]) = rotation {
            if yaw.is_finite() {
                self.yaw = yaw;
            }
            if !pitch.is_nan() {
                // PositionMoveRotation.calculateAbsolute clamps pitch before setXRot.
                self.pitch = pitch.clamp(-90.0, 90.0);
            }
        }
        from
    }

    /// `Entity.teleportSameDimension`: shared entity/passenger placement.
    /// Returns its old position for the command caller's completion notification.
    pub(super) fn place_teleport_in_level(
        &mut self,
        t: Target,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        relative: [bool; 3],
        turn_riders: bool,
    ) -> Option<[f64; 3]> {
        use super::entity::Entity;
        let from = self.target_position(t);
        let v = Vec3::new(pos[0], pos[1], pos[2]);
        // The riders turn from the unclamped angles (calculatePassengerTransition
        // adds their offset to the transition's, each rider's own setXRot clamps).
        let requested = rotation;
        // Entity.teleportSameDimension teleports the riders before their vehicle.
        let riding = self.teleport_riders_first(t, v, relative, requested.filter(|_| turn_riders));
        // PositionMoveRotation.calculateAbsolute clamps pitch before setXRot.
        let rotation = if matches!(t, Target::Player | Target::Guest(_)) {
            requested // Player placement applies the clamp itself.
        } else {
            requested.map(|[yaw, pitch]| [yaw, pitch.clamp(-90.0, 90.0)])
        };
        match t {
            Target::General(id) => {
                if let Some(entity) = self.entity(id) {
                    self.entity_stop_riding(entity);
                }
                if let Some(row) = self.resident_entities.general.row_mut(id) {
                    row.position = v;
                    row.previous = v;
                    if let Some([yaw, pitch]) = rotation {
                        if yaw.is_finite() {
                            row.rotation[0] = yaw;
                        }
                        if !pitch.is_nan() {
                            row.rotation[1] = entity_pitch(pitch);
                        }
                    }
                    // Entity.teleportSetPosition ends with setOldPosAndRot.
                    row.previous_rotation = row.rotation;
                }
            }
            Target::Guest(_) => {
                self.for_player(t, |g| {
                    g.teleport_to_relative(Target::Player, pos, rotation, relative, turn_riders)
                });
            }
            Target::Player => {
                let from = self.place_player_teleport_in_level(v, rotation, relative);
                // A server's player teleported within their level is placed
                // there at once: the arrival they were waiting for elsewhere
                // in it (whose chunks lose their tickets) is abandoned.
                if let Some(conn) = self.lan_acting {
                    self.hold_for_chunks(conn, v);
                }
                // TeleportCommand.performTeleport grounds a player unless they're gliding.
                if let Some(p) = self.player.as_mut()
                    && !p.fall_flying
                {
                    p.ground.on_ground = true;
                }
                // A server player's teleport is told to the mods once their
                // move is in the hub (the put-back at the end of `guest_turn`).
                if let (Some(conn), Some(uuid), Some(name), Some(from)) = (
                    self.lan_acting,
                    self.lan_acting_uuid,
                    self.lan_acting_name.clone(),
                    from,
                ) {
                    self.teleport_notes
                        .push(super::server_travel::TeleportNote {
                            conn,
                            uuid,
                            name,
                            from,
                            to: pos,
                            dimension: self.dim.id().to_owned(),
                        });
                }
            }
            Target::Mob(id) => {
                // Entity.teleport: stopRiding() before the entity is placed,
                // which a command's transition always does (a command's
                // transition is never asPassenger). A rider moved without this
                // is seated again by the next sync, so nothing moves.
                self.dismount_mob(id);
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                    m.ai.nav.stop();
                    m.velocity = teleport_velocity(m.velocity, relative, m.extra.fall_flying);
                    if !m.extra.fall_flying {
                        m.ground.on_ground = true;
                    }
                    sunrise_client::mobs::living::teleports::snap_to(m, v);
                    m.needs_sync = true;
                    // Entity.teleportSetPosition: setYRot, setYHeadRot, setXRot
                    // (clamped to -90..90), then setOldRot.
                    if let Some([yaw, pitch]) = rotation {
                        if yaw.is_finite() {
                            m.yaw = yaw;
                            m.body_yaw = yaw;
                            m.head_yaw = yaw;
                        }
                        if !pitch.is_nan() {
                            m.head_pitch = entity_pitch(pitch);
                        }
                        m.previous_yaw = m.yaw;
                        m.previous_head_pitch = m.head_pitch;
                    }
                }
            }
            Target::Item(id) => {
                if let Some(i) = self.dropped.iter_mut().find(|i| i.id == id) {
                    i.position = v;
                    i.previous = v;
                    if let Some(r) = rotation {
                        Entity::set_rotation(i, r);
                    }
                }
            }
            Target::Orb(id) => {
                if let Some(o) = self.orbs.iter_mut().find(|o| o.id == id) {
                    o.position = v;
                    o.previous = v;
                    if let Some(r) = rotation {
                        Entity::set_rotation(o, r);
                    }
                }
            }
            Target::Projectile(id) => {
                if let Some(i) = self.creatures.projectiles.iter().position(|a| a.id == id) {
                    self.creatures.projectiles.update(i, |a| {
                        a.position = v;
                        a.previous = v;
                        if let Some(r) = rotation {
                            Entity::set_rotation(a, r);
                        }
                    });
                }
            }
            Target::Stand(id) => {
                if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id) {
                    a.position = v;
                    a.previous = v;
                    // Entity.teleport's stopRiding(), as for a mob: a stand a
                    // server seats on a vehicle (a name tag hologram) leaves it.
                    a.vehicle = None;
                    // TeleportCommand uses the current view angles when rotation
                    // is omitted; teleportSetPosition still updates head yaw.
                    let angles = rotation.unwrap_or_else(|| a.rotation());
                    Entity::set_rotation(a, angles);
                }
            }
            Target::Boat(id) => {
                if let Some(i) = self.boats.iter().position(|b| b.id == id) {
                    self.boats.update(i, |b| {
                        b.position = v;
                        b.previous = v;
                        if let Some(r) = rotation {
                            Entity::set_rotation(b, r);
                        }
                    });
                }
            }
            Target::Cart(id) => {
                if let Some(i) = self.carts.iter().position(|c| c.id == id) {
                    self.carts.update(i, |c| {
                        c.position = v;
                        c.previous = v;
                        if let Some(r) = rotation {
                            Entity::set_rotation(c, r);
                        }
                    });
                }
            }
            Target::Fangs(id) => {
                if let Some(i) = self.fangs.list.iter().position(|e| e.id == id) {
                    self.fangs.list.update(i, |f| {
                        f.pos = [v.x, v.y, v.z];
                        if let Some([yaw, _]) = rotation.filter(|r| r[0].is_finite()) {
                            f.yaw = yaw;
                        }
                    });
                }
            }
            Target::Other(id) => {
                if let Some(i) = self.others.iter().position(|e| e.id == id) {
                    self.others.update(i, |o| {
                        o.position = v;
                        if let Some([yaw, pitch]) = rotation {
                            if yaw.is_finite() {
                                o.yaw = yaw;
                            }
                            if !pitch.is_nan() {
                                o.pitch = entity_pitch(pitch);
                            }
                        }
                    });
                }
            }
            Target::Crystal(id) => {
                if let Some(c) = self.crystals.iter_mut().find(|c| c.id == id) {
                    (c.position, c.fall.0) = (v, true); // performTeleport: setOnGround(true)
                    c.previous = v;
                }
            }
            Target::Dragon(id) => {
                if let Some(d) = self.dragon.as_mut().filter(|d| d.id == id) {
                    d.position = v;
                    d.previous = v;
                }
            }
            Target::Tnt(id) => {
                if let Some(t) = self.tnts.iter_mut().find(|t| t.id == id) {
                    // `PrimedTnt.teleport` includes same-level command teleports.
                    t.used_portal = true;
                    t.position = v;
                    t.previous = v;
                    if let Some(r) = rotation {
                        Entity::set_rotation(t, r);
                    }
                }
            }
            Target::Cushion(id) => {
                // BlockAttachedEntity.setPos: the block it rests on follows the position.
                if let Some(c) = self.cushions.iter_mut().find(|c| c.id == id) {
                    c.position = v;
                    c.pos = [v.x.floor() as i32, v.y.floor() as i32, v.z.floor() as i32];
                    if let Some(r) = rotation {
                        c.yaw = r[0];
                    }
                }
            }
            Target::Falling(id) => {
                if let Some(i) = self.falling.iter().position(|f| f.id == id) {
                    self.falling.update(i, |f| {
                        f.position = v;
                        f.previous = v;
                        if let Some(r) = rotation {
                            Entity::set_rotation(f, r);
                        }
                    });
                }
            }
            Target::Rocket(id) => {
                if let Some(r) = self.rockets.iter_mut().find(|r| r.id == id) {
                    r.position = v;
                    r.previous = v;
                    if let Some(rot) = rotation {
                        Entity::set_rotation(r, rot);
                    }
                }
            }
            Target::Trident(id) => {
                if let Some(t) = self.tridents.iter_mut().find(|t| t.id == id) {
                    t.position = v;
                    t.previous = v;
                    if let Some(r) = rotation {
                        Entity::set_rotation(t, r);
                    }
                }
            }
            Target::Spawner(id) => {
                if let Some(i) = self.ominous_spawners.iter().position(|e| e.id == id) {
                    self.ominous_spawners.update(i, |s| {
                        s.position = [v.x, v.y, v.z];
                    });
                }
            }
            Target::Cloud(id) => {
                if let Some(i) = self.breath_clouds.iter().position(|e| e.id == id) {
                    self.breath_clouds.update(i, |c| {
                        c.at = pos;
                    });
                }
            }
            Target::Bolt(id) => {
                if let Some(i) = self.bolts.iter().position(|e| e.id == id) {
                    self.bolts.update(i, |b| {
                        b.at = pos;
                        if let Some([yaw, pitch]) = rotation {
                            if yaw.is_finite() {
                                b.rotation[0] = yaw;
                            }
                            if !pitch.is_nan() {
                                b.rotation[1] = entity_pitch(pitch);
                            }
                        }
                    });
                }
            }
            // Wall-mounted entities do not move; a hook follows its own tick and a
            // dragon part its dragon's.
            Target::Frame(_) | Target::Painting(_) => self.teleport_hanging(t, v),
            Target::Hook(_) | Target::DragonPart(_) | Target::Knot(_) => {}
        }
        if !matches!(t, Target::Player | Target::Guest(_) | Target::Mob(_)) {
            target_entity_mut!(self, t, entity => {
                let velocity = entity.velocity_mut();
                *velocity = teleport_velocity(*velocity, relative, false);
            });
            if let Target::Dragon(id) = t
                && let Some(dragon) = self.dragon.as_mut().filter(|dragon| dragon.id == id)
            {
                dragon.teleport_motion(relative);
            }
        }
        self.announce_vehicle_teleport(t, &riding);
        from
    }

    /// `t` teleported into another level than its own: an entity crosses at
    /// once (`teleport_target_across`), a server's player is queued to move
    /// after the command (ServerPlayer.teleport). False when `t` cannot
    /// change level or is already in `level`.
    pub(in crate::game) fn teleport_across(
        &mut self,
        t: Target,
        level: super::Dim,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        turn_riders: bool,
        motion: TeleportMotion,
    ) -> R<bool> {
        match self.teleport_target_across(t, level, pos, rotation, turn_riders, motion) {
            Ok(Some(moved)) => return Ok(moved),
            Ok(None) => {}
            Err(error) => {
                log::error!("entity teleport into {}: {error}", level.id());
                return Err(err("command.failed", &[]));
            }
        }
        let conn = match t {
            Target::Player => self.lan_acting,
            Target::Guest(conn) => Some(conn),
            _ => None,
        };
        let at = self.target_level(t);
        let Some(conn) = conn.filter(|_| at != level) else {
            return Ok(false);
        };
        // A fake player stays in its level. True: the teleport is taken, so no
        // same-level teleport stands in for it.
        if Some(conn) == self.lan_acting && self.fake_blocks(mod_api::FakeEffect::ChangeDimension) {
            return Ok(true);
        }
        // A canceled travel keeps the player where they are.
        if !self.server_player_travel_allowed(
            conn,
            level,
            super::server_travel::TravelCause::Teleport,
        ) {
            return Ok(true);
        }
        self.level_teleports
            .push(super::server_travel::LevelTeleport {
                conn,
                dim: level,
                at: pos,
                rotation,
                vehicle: None,
            });
        Ok(true)
    }
    /// `ServerPlayer.lookAt`: besides turning the player, tell their client
    /// with a `ClientboundPlayerLookAtPacket` (`from` anchor, the point, and
    /// the entity with its anchor when the target is one). Only a connected
    /// player has a client to tell. The entity form is sent whenever the
    /// target is an entity with a network id (`LookAt.LookAtEntity.perform`);
    /// a player whose id is not known here (a guest's turn, with the host
    /// out of reach) falls back to the point, which a client takes the same
    /// unless the entity moves first.
    /// `Entity.getId` as the players' clients know it: a player's is the one the
    /// hub gave it, any other entity's is its own id.
    pub(in crate::game) fn network_entity_id(&self, t: Target) -> Option<i32> {
        let host = self.lan.as_ref();
        match t {
            Target::Player => self.player_entity_id_by_connection(self.lan_acting),
            Target::Guest(conn) => host?.hub.guest(conn).map(|g| g.entity_id),
            other => targets::non_player_entity_id(other),
        }
    }
    pub(super) fn send_player_look_at(
        &mut self,
        t: Target,
        from_eyes: bool,
        target: [f64; 3],
        entity: Option<(Target, bool)>,
    ) {
        let packet = C::PlayerLookAt(play::PlayerLookAt {
            from: i32::from(from_eyes),
            target,
            entity: entity.and_then(|(e, eyes)| {
                Some(play::LookAtEntity {
                    id: self.network_entity_id(e)?,
                    anchor: i32::from(eyes),
                })
            }),
        });
        match t {
            Target::Player if self.lan_acting.is_some() => self.guest_packets.push(packet),
            Target::Guest(conn) => {
                if let Some(host) = self.lan.as_ref() {
                    host.hub.send_to(conn, &packet);
                }
            }
            _ => {}
        }
    }
    /// `TeleportCommand.performTeleport`'s `EntityTeleportEvent.TeleportCommand`:
    /// where `t` goes in `level`, or None when a mod canceled the teleport.
    /// A target that is not a number keeps the command's. Only a server's
    /// entities with a network id are asked.
    fn teleport_command_target(
        &mut self,
        t: Target,
        level: super::Dim,
        pos: [f64; 3],
    ) -> Option<[f64; 3]> {
        let (Some(entity), Some(from)) = (self.network_entity_id(t), self.target_position(t))
        else {
            return Some(pos);
        };
        if !self.native_authoritative() {
            return Some(pos);
        }
        let mut to = pos;
        let mut cancel = false;
        self.native_teleport_command(level.id(), entity as u32, from, &mut to, &mut cancel);
        if cancel {
            return None;
        }
        Some(if to.iter().all(|v| v.is_finite()) {
            to
        } else {
            pos
        })
    }
    fn cmd_teleport(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = if id.starts_with("teleport.targets") {
            without_knots(self.entities(ctx, "targets", false)?)?
        } else {
            vec![
                self.source_entity()
                    .ok_or_else(|| err("permissions.requires.entity", &[]))?,
            ]
        };
        if id.ends_with("destination") {
            let dest = self.entities(ctx, "destination", false)?[0];
            let (pos, level, rotation, destination_name) = self
                .in_target_level(dest, |game| {
                    (
                        game.target_position(dest).unwrap_or(game.source_position()),
                        game.target_level(dest),
                        Some(game.target_rotation(dest).map(wrap_degrees)),
                        game.target_name(dest),
                    )
                })
                .ok_or_else(|| err("argument.entity.notfound.entity", &[]))?;
            let target_name = (targets.len() == 1).then(|| {
                self.in_target_level(targets[0], |game| game.target_name(targets[0]))
                    .unwrap_or_else(Text::empty)
            });
            for &t in &targets {
                let Some(pos) = self.teleport_command_target(t, level, pos) else {
                    continue;
                };
                if !self.teleport_across(t, level, pos, rotation, true, TeleportMotion::Stop)? {
                    // TeleportCommand.teleportToEntity: the destination's angles,
                    // which the riders turn to as well.
                    self.teleport_to(t, pos, rotation, true);
                    self.tp_lands(t);
                }
            }
            let message = if targets.len() == 1 {
                Text::translate(
                    "commands.teleport.success.entity.single",
                    &[target_name.unwrap(), destination_name],
                )
            } else {
                Text::translate(
                    "commands.teleport.success.entity.multiple",
                    &[Text::literal(targets.len().to_string()), destination_name],
                )
            };
            self.feedback(message);
            return Ok(targets.len() as i32);
        }
        let pos = self
            .position_arg(ctx, "location")
            .unwrap_or(self.source_position());
        if !(-30_000_000.0..30_000_000.0).contains(&pos[0])
            || !(-30_000_000.0..30_000_000.0).contains(&pos[2])
            || !(-20_000_000.0..20_000_000.0).contains(&pos[1])
        {
            return Err(err("commands.teleport.invalidPosition", &[]));
        }
        let relative = match ctx.get("location") {
            Some(Arg::Coords(Coordinates::World(coords))) => coords.each_ref().map(|c| c.relative),
            Some(Arg::Coords(Coordinates::Local(_))) => [true; 3],
            _ => [false; 3],
        };
        let rotation = match ctx.get("rotation") {
            Some(Arg::Coords(c @ Coordinates::World(axes))) => Some((
                c.rotation(self.source_rotation()),
                [axes[0].relative, axes[1].relative],
            )),
            _ => None,
        };
        // facing: look from the new position at a location or entity.
        let facing = self.position_arg(ctx, "facingLocation").or_else(|| {
            let Some(Arg::Entity(_)) = ctx.get("facingEntity") else {
                return None;
            };
            None
        });
        let mut facing_entity = None;
        let facing = match (facing, ctx.get("facingEntity")) {
            (Some(f), _) => Some(f),
            (None, Some(_)) => {
                let e = self.entities(ctx, "facingEntity", false)?[0];
                // EntityAnchorArgument.Anchor.apply: the feet, or the eyes.
                let eyes = matches!(ctx.get("facingAnchor"), Some(Arg::Anchor(true)));
                facing_entity = Some((e, eyes));
                let eye_height = if eyes { self.target_eye_height(e) } else { 0.0 };
                self.target_position(e)
                    .map(|p| [p[0], p[1] + eye_height, p[2]])
            }
            _ => None,
        };
        // Entity.lookAt: from the source's anchor (the feet, unless `anchored
        // eyes`) on the teleported entity, so the eyes depend on the target.
        let anchored_eyes = self.command_source.as_ref().is_some_and(|s| s.eyes);
        let look_at = |this: &Self, t: Target, at: [f64; 3]| {
            let f = facing?;
            let eye = if anchored_eyes {
                at[1] + this.target_eye_height(t)
            } else {
                at[1]
            };
            Some(world_cmds::look_at([at[0], eye, at[2]], f))
        };
        // A server's player in another level than the source's (`execute
        // in`, the console's Overworld) moves there (ServerPlayer.teleport
        // into another level), after the command.
        let source_level = match self.command_source.as_ref().and_then(|s| s.level) {
            Some(level) => level,
            None if self.source_entity().is_none() => super::Dim::Overworld,
            None => self.dim,
        };
        let target_name = (targets.len() == 1).then(|| {
            self.in_target_level(targets[0], |game| game.target_name(targets[0]))
                .unwrap_or_else(Text::empty)
        });
        for &t in &targets {
            let Some(pos) = self.teleport_command_target(t, source_level, pos) else {
                continue;
            };
            let looked = look_at(self, t, pos);
            let turn_riders = looked.is_none();
            let rotation = looked.or_else(|| {
                let (angles, relative) = rotation?;
                let current = self
                    .in_target_level(t, |game| game.target_rotation(t))
                    .unwrap_or([0.0, 0.0]);
                // performTeleport wraps relative deltas before calculateAbsolute
                // adds them back; wrapping the resolved pitch changes its clamp.
                Some(std::array::from_fn(|axis| {
                    if relative[axis] {
                        current[axis] + wrap_degrees(angles[axis] - current[axis])
                    } else {
                        wrap_degrees(angles[axis])
                    }
                }))
            });
            if self.teleport_across(
                t,
                source_level,
                pos,
                rotation,
                turn_riders,
                TeleportMotion::Stop,
            )? {
                continue;
            }
            self.teleport_to_relative(t, pos, rotation, relative, turn_riders);
            // LookAt.perform after a successful teleportTo.
            if let Some(f) = facing {
                self.send_player_look_at(t, anchored_eyes, f, facing_entity);
            }
            self.tp_lands(t);
        }
        let [x, y, z] = pos.map(|v| Text::literal(format_double(v)));
        let message = if targets.len() == 1 {
            Text::translate(
                "commands.teleport.success.location.single",
                &[target_name.unwrap(), x, y, z],
            )
        } else {
            Text::translate(
                "commands.teleport.success.location.multiple",
                &[Text::literal(targets.len().to_string()), x, y, z],
            )
        };
        self.feedback(message);
        Ok(targets.len() as i32)
    }
    /// `TimeCommand.getDefaultClock`: explicit registered clocks and dimension defaults are
    /// separate; built-in timelines and markers belong only to Overworld.
    fn cmd_time(&mut self, id: &str, ctx: &Context) -> R<i32> {
        if id == "time.query.gametime" {
            let t = self.world.game_time;
            self.feedback_unbroadcast(Text::tr("commands.time.query.gametime", &[&t]));
            return Ok((t % i64::from(i32::MAX)) as i32);
        }
        // `time of <clock>` names the clock; otherwise the dimension's default clock.
        let clock = match ctx.get("clock") {
            Some(Arg::Resource(c)) => c.clone(),
            _ => match self.dimension_type().default_clock.clone() {
                Some(clock) => clock,
                None => return Err(err("commands.time.no_default_clock", &[&self.dim.id()])),
            },
        };
        let total = self.command_clock_time(&clock);
        match id {
            "time.set" => {
                let t = i64::from(Self::int(ctx, "time").unwrap_or(0));
                if total == t {
                    return Err(err("commands.time.set.already_at_time", &[&clock, &t]));
                }
                let mut cancel = false;
                self.native_time_skip(self.dim.id(), total, t, "command", &mut cancel);
                if !cancel {
                    self.set_command_clock_time(&clock, t);
                    self.command_clock_state(&clock).partial = 0.0;
                }
                self.feedback(Text::tr("commands.time.set.absolute", &[&clock, &t]));
                Ok(t as i32)
            }
            "time.set.marker" => {
                let Some(Arg::Resource(marker)) = ctx.get("timemarker").cloned() else {
                    return Ok(0);
                };
                let Some((m, target)) = self
                    .ui
                    .markers
                    .get(&(clock.clone(), marker.clone()))
                    .copied()
                    .and_then(|m| Some((m, m.resolve(total)?)))
                else {
                    return Err(err(
                        "commands.time.no_time_marker_found",
                        &[&marker, &clock],
                    ));
                };
                if m.occurs_at(total) {
                    return Err(err(
                        "commands.time.set.already_at_time_marker",
                        &[&clock, &marker],
                    ));
                }
                let mut cancel = false;
                self.native_time_skip(self.dim.id(), total, target, "command", &mut cancel);
                if !cancel {
                    self.set_command_clock_time(&clock, target);
                    self.command_clock_state(&clock).partial = 0.0;
                }
                self.feedback(Text::tr(
                    "commands.time.set.time_marker",
                    &[&clock, &marker],
                ));
                Ok((self.command_clock_time(&clock) % i64::from(i32::MAX)) as i32)
            }
            "time.add" => {
                let t = i64::from(Self::int(ctx, "time").unwrap_or(0));
                let target = (total + t).max(0);
                let mut cancel = false;
                self.native_time_skip(self.dim.id(), total, target, "command", &mut cancel);
                if !cancel {
                    self.set_command_clock_time(&clock, target);
                }
                let now = self.command_clock_time(&clock);
                self.feedback(Text::tr("commands.time.set.absolute", &[&clock, &now]));
                Ok((now % i64::from(i32::MAX)) as i32)
            }
            "time.pause" | "time.resume" => {
                let pause = id == "time.pause";
                if self.command_clock_state(&clock).paused == pause {
                    return Err(err(
                        if pause {
                            "commands.time.pause.already_paused"
                        } else {
                            "commands.time.pause.already_running"
                        },
                        &[&clock],
                    ));
                }
                self.command_clock_state(&clock).paused = pause;
                self.feedback(Text::tr(
                    if pause {
                        "commands.time.pause"
                    } else {
                        "commands.time.resume"
                    },
                    &[&clock],
                ));
                Ok(1)
            }
            "time.rate" => {
                let rate = match ctx.get("rate") {
                    Some(Arg::Float(r)) => *r,
                    _ => 1.0,
                };
                if self.command_clock_state(&clock).rate == rate {
                    return Err(err(
                        "commands.time.rate.already_same",
                        &[&clock, &float_to_string(rate)],
                    ));
                }
                self.command_clock_state(&clock).rate = rate;
                self.feedback(Text::tr(
                    "commands.time.rate",
                    &[&clock, &float_to_string(rate)],
                ));
                Ok(1)
            }
            "time.query.time" => {
                self.feedback_unbroadcast(Text::tr(
                    "commands.time.query.absolute",
                    &[&clock, &total],
                ));
                Ok((total % i64::from(i32::MAX)) as i32)
            }
            "time.query.timeline" | "time.query.repetition" => {
                let Some(Arg::Resource(timeline)) = ctx.get("timeline").cloned() else {
                    return Ok(0);
                };
                let Some(period) = self
                    .ui
                    .timelines
                    .get(&timeline)
                    .filter(|(c, _)| c == &clock)
                    .map(|(_, period)| *period)
                else {
                    return Err(err(
                        "commands.time.wrong_timeline_for_clock",
                        &[&timeline, &clock],
                    ));
                };
                if id == "time.query.timeline" {
                    let ticks = period.map_or(total, |p| total.rem_euclid(p));
                    self.feedback_unbroadcast(Text::tr(
                        "commands.time.query.timeline",
                        &[&timeline, &ticks],
                    ));
                    Ok(ticks as i32)
                } else {
                    let reps = period.map_or(0, |p| total / p);
                    self.feedback_unbroadcast(Text::tr(
                        "commands.time.query.timeline.repetitions",
                        &[&timeline, &reps],
                    ));
                    Ok(reps as i32)
                }
            }
            _ => Ok(0),
        }
    }
    fn cmd_weather(&mut self, ctx: &Context) -> R<i32> {
        let duration = Self::int(ctx, "duration");
        let sample = |min: i32, max: i32, r: &mut sunrise_core::random::LegacyRandom| {
            min + r.next_int_bound(max - min + 1).unwrap_or(0)
        };
        let (key, clear, time, raining, thundering) = if ctx.has("clear") {
            (
                "commands.weather.set.clear",
                duration.unwrap_or_else(|| sample(12_000, 180_000, &mut self.world.random)),
                0,
                false,
                false,
            )
        } else if ctx.has("rain") {
            (
                "commands.weather.set.rain",
                0,
                duration.unwrap_or_else(|| sample(12_000, 24_000, &mut self.world.random)),
                true,
                false,
            )
        } else {
            (
                "commands.weather.set.thunder",
                0,
                duration.unwrap_or_else(|| sample(3_600, 15_600, &mut self.world.random)),
                true,
                true,
            )
        };
        if !self.set_weather_parameters(self.dim.id(), clear, time, raining, thundering) {
            return Err(err("command.failed", &[]));
        }
        self.feedback(Text::translate(key, &[]));
        Ok(duration.unwrap_or(-1))
    }
    pub(super) fn kill_target(&mut self, t: Target) -> bool {
        // An entity in another level is killed in the level that holds it.
        self.in_target_level(t, |g| g.kill_target_in_level(t))
            .unwrap_or(false)
    }

    pub(super) fn can_kill_target_in_level(&self, t: Target) -> bool {
        if self.target_removed(&t) || self.persist.preparing_vehicle_removals.contains(&t) {
            return false;
        }
        match t {
            Target::DragonPart(_) => false,
            Target::Player => self
                .player
                .as_ref()
                .is_some_and(|p| p.health.health() > 0.0),
            Target::Guest(conn) => self.command_guest(conn).is_some_and(|g| !g.dead),
            Target::Mob(id) => self
                .mobs
                .iter()
                .any(|m| m.id == id && !m.removed && m.alive()),
            _ => self.target_position(t).is_some(),
        }
    }

    pub(super) fn kill_target_in_level(&mut self, t: Target) -> bool {
        if !self.can_kill_target_in_level(t) {
            return false;
        }
        // Entity.kill (BlockAttachedEntity.kill; ArmorStand's generic_kill hit,
        // which bypasses its invulnerability; a vehicle is no LivingEntity):
        // removed, then ENTITY_DIE by the entity itself. Mobs post theirs as
        // they die, the dragon on its own path, and EndCrystal.kill posts it
        // after onDestroyedBy. The source is captured before the removal: an
        // armor stand is living (wardens can't target it), and a dropped item
        // keeps its stack's ItemEntity.dampensVibrations.
        let died = (!matches!(
            t,
            Target::Player
                | Target::Guest(_)
                | Target::Mob(_)
                | Target::Crystal(_)
                | Target::Dragon(_)
        ))
        .then(|| self.target_position(t))
        .flatten();
        let mut context = self
            .network_entity_id(t)
            .map(|id| self.entity_event_context(id as u32, None))
            .unwrap_or_default();
        context.living = matches!(t, Target::Stand(_));
        if let Target::Item(id) = t {
            context.dampens = self.dropped.iter().find(|i| i.id == id).is_some_and(|i| {
                self.res
                    .items()
                    .item_in_tag("minecraft:dampens_vibrations", &i.stack.item)
            });
        }
        // `Entity.kill` (and `EnderDragon.kill`, `ArmorStand.kill`,
        // `BlockAttachedEntity.kill`) removes the entity immediately with
        // `RemovalReason.KILLED`, so `ServerLevel.EntityCallbacks.onDestroyed`
        // runs `Scoreboard.entityRemoved` before `/kill` formats its reply.
        // Players are never removed from teams, and living mobs stay in the
        // level through `tickDeath`.
        let destroyed_score = (!matches!(
            t,
            Target::Player
                | Target::Guest(_)
                | Target::Mob(_)
                | Target::Knot(_)
                | Target::DragonPart(_)
        ))
        .then(|| {
            super::native_entities::target_id(t)
                .and_then(|id| self.persist.uuid_of(id).map(|uuid| (id, uuid)))
        })
        .flatten();
        match t {
            Target::General(id) => {
                let Some(entity) = self.entity(id) else {
                    return false;
                };
                self.remove_target_now(entity, t);
            }
            Target::Guest(_) => {
                return self
                    .for_player(t, |g| g.kill_target_in_level(Target::Player))
                    .unwrap_or(false);
            }
            Target::Player => {
                if self
                    .player
                    .as_mut()
                    .is_none_or(|p| p.set_health(0.0, false).is_err())
                {
                    return false;
                }
                self.ui.death_cause = Some("death.attack.genericKill".into());
            }
            Target::Mob(id) => {
                let mut killed = None;
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                    // LivingEntity.kill: a generic_kill hit of Float.MAX_VALUE,
                    // which its death message tells.
                    let alive = m.alive();
                    if alive {
                        let source = sunrise_client::mobs::Damage::of(
                            sunrise_client::mobs::DamageKind::GenericKill,
                        );
                        m.record_damage(source, f32::MAX);
                        m.last_damage = Some((source, i64::from(m.age)));
                    }
                    m.health = 0.0;
                    killed = alive.then(|| m.clone());
                }
                // The hit runs LivingEntity.die before the command returns:
                // its ENTITY_DIE, loot and experience come now, not in the
                // next tick's deaths, which skip it (as a frog's bite does).
                let Some(m) = killed.filter(|m| !m.removed && self.died.insert(id)) else {
                    return false;
                };
                self.mob_die(&m);
                if self
                    .mobs
                    .iter()
                    .any(|m| m.id == id && !m.removed && (m.alive() || m.death_canceled))
                {
                    return false;
                }
            }
            Target::Item(id) => {
                if let Some(i) = self.dropped.iter().position(|i| i.id == id) {
                    self.remove_dropped(i);
                }
            }
            // Entity.kill removes the orb.
            Target::Orb(id) => self.orbs.retain(|o| o.id != id),
            Target::Projectile(id) => self.remove_projectile(id),
            Target::Stand(id) => {
                if !self.remove_armor_stand_after(id, |_| {}) {
                    return false;
                }
            }
            Target::Frame(id) => {
                // ItemFrame.onKilled.
                if let Some(frame) = self.item_frames.iter().find(|f| f.id == id).cloned() {
                    self.remove_framed_map(&frame);
                }
                self.item_frames.retain(|f| f.id != id);
            }
            Target::Painting(id) => self.paintings.retain(|p| p.id != id),
            Target::Boat(_) | Target::Cart(_) => {
                if !self.kill_vehicle_object(t) {
                    return false;
                }
            }
            Target::Other(id) => {
                if self
                    .with_destroying_vehicle(t, |game| {
                        game.others.retain(|o| o.id != id);
                        game.forget_entity(id);
                    })
                    .is_none()
                {
                    return false;
                }
            }
            Target::Fangs(id) => self.fangs.list.retain(|f| f.id != id),
            Target::Crystal(id) => self.kill_crystal(id),
            Target::Dragon(id) => self.kill_dragon(id),
            Target::Tnt(id) => {
                if self.take_removed_tnt(id).is_none() {
                    return false;
                }
            }
            Target::Cushion(id) => {
                if self.take_removed_cushion(id).is_none() {
                    return false;
                }
            }
            Target::Falling(id) => self.falling.retain(|f| f.id != id),
            Target::Rocket(id) => self.rockets.retain(|r| r.id != id),
            Target::Trident(id) => self.tridents.retain(|t| t.id != id),
            Target::Spawner(id) => self.remove_ominous_spawner(id),
            Target::Cloud(id) => self.remove_breath_cloud(id),
            Target::Bolt(id) => self.remove_bolt(id),
            Target::Hook(id) => self.remove_fishing_hook(id),
            Target::Knot(id) => {
                if !self.kill_natural_knot(id) {
                    return false;
                }
            }
            // No command names a part, and `kill` on one would not stop its dragon.
            Target::DragonPart(_) => return false,
        }
        if let Some((id, uuid)) = destroyed_score {
            self.score_destroyed_entity_once(id, uuid);
        }
        if let Some(at) = died {
            self.post_game_event("minecraft:entity_die", at, context);
        }
        self.native_leaves_flush();
        true
    }
    fn cmd_kill(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            without_knots(self.entities(ctx, "targets", false)?)?
        } else {
            vec![self.implied_player()]
        };
        let single_display = (targets.len() == 1).then(|| {
            let t = targets[0];
            // The identity of an entity in another level is read there before
            // `kill_target` removes it from that level.
            match self.in_target_level(t, |g| g.capture_kill_display(t)) {
                Some(captured) => captured,
                None => self.capture_kill_display(t),
            }
        });
        for &t in &targets {
            self.kill_target(t);
        }
        let message = if let Some(captured) = single_display {
            let name = self.finish_kill_display(captured);
            Text::translate("commands.kill.success.single", &[name])
        } else {
            Text::tr("commands.kill.success.multiple", &[&targets.len()])
        };
        self.feedback(message);
        Ok(targets.len() as i32)
    }
    /// `Level.isInSpawnableBounds(BlockPos.containing(pos))`: shared by
    /// `/summon` and `/execute summon` (`SummonCommand.createEntity`).
    pub(super) fn check_summon_bounds(pos: [f64; 3]) -> R<()> {
        if !(-30_000_000.0..30_000_000.0).contains(&pos[0])
            || !(-20_000_000.0..20_000_000.0).contains(&pos[1])
            || !(-30_000_000.0..30_000_000.0).contains(&pos[2])
        {
            return Err(err("commands.summon.invalidPosition", &[]));
        }
        Ok(())
    }
    /// `SummonCommand.createEntity`: Peaceful admission uses the entity-type
    /// policy for both forms, independently of the mob category.
    fn cmd_summon(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(kind)) = ctx.get("entity").cloned() else {
            return Ok(0);
        };
        // ResourceArgument.getSummonableEntityType.
        if matches!(
            kind.as_str(),
            "minecraft:player" | "minecraft:fishing_bobber"
        ) {
            return Err(err("entity.not_summonable", &[&kind]));
        }
        let pos = self
            .position_arg(ctx, "pos")
            .unwrap_or(self.source_position());
        Self::check_summon_bounds(pos)?;
        let mob_type = self.res.mob_types().get(&kind);
        if self.difficulty_id() == 0 && mob_type.is_some_and(|t| !t.allowed_in_peaceful()) {
            return Err(err("commands.summon.failed.peaceful", &[]));
        }
        let short = kind.trim_start_matches("minecraft:").to_owned();
        if let Some(Arg::Str(nbt)) = ctx.get("nbt").cloned() {
            // With an NBT argument the entity is built from it, unfinalized.
            let before = self.spawned_mark();
            let name = match self.summon_with_nbt(&kind, pos, &nbt)? {
                Some(target) => self.target_name(target),
                // Spawned as a kind no command target stands for (primed
                // TNT, falling blocks, projectiles): its type's name.
                None if self.spawned_mark() != before => data::summon_type_name(&kind),
                None => return Err(err("commands.summon.failed", &[])),
            };
            self.feedback(Text::translate("commands.summon.success", &[name]));
            return Ok(1);
        }
        let name;
        if kind == "minecraft:ender_dragon" || mob_type.is_some() {
            let id = self
                .spawn_mob_spawned(
                    &short,
                    pos,
                    super::creatures::SpawnSite::Finalizing(
                        sunrise_client::mobs::spawn_rules::Reason::Command,
                    ),
                )
                .ok_or_else(|| err("commands.summon.failed.uuid", &[]))?;
            name = self.target_name(Target::Mob(id));
        } else {
            match short.as_str() {
                "experience_orb" => {
                    // `EntityType`'s factory, `new ExperienceOrb(EntityType, Level)`,
                    // which only `snapTo`s: the position is kept, even in a block.
                    self.next_entity += 1;
                    let id = self.next_entity;
                    let uuid = sunrise_client::save::insecure_uuid(
                        &mut sunrise_client::random_source::entity(),
                    );
                    self.persist.uuids.insert(id, uuid);
                    let at = Vec3::new(pos[0], pos[1], pos[2]);
                    let orb = sunrise_client::entities::ExperienceOrb::new(id, at, Vec3::ZERO, 0);
                    self.orbs.push(orb);
                    // tryAddFreshEntityWithPassengers refused: ERROR_DUPLICATE_UUID.
                    if !self.native_joins_flush(false, id) {
                        return Err(err("commands.summon.failed.uuid", &[]));
                    }
                    name = self.target_name(Target::Orb(id));
                }
                "lightning_bolt" => {
                    let target = self
                        .summon_with_nbt(&kind, pos, "{}")?
                        .ok_or_else(|| err("commands.summon.failed", &[]))?;
                    name = self.target_name(target);
                }
                "end_crystal" => {
                    // EndCrystal's constructor takes an id from the entity
                    // counter, and the crystal is shown with its base.
                    self.next_entity += 1;
                    let id = self.next_entity;
                    let at = Vec3::new(pos[0], pos[1], pos[2]);
                    let (drawn, uuid) = super::dragon::Crystal::own_draws();
                    self.persist.uuids.insert(id, uuid);
                    let time = super::dragon::Crystal::start_time(at, drawn, self.clock.still());
                    let crystal = super::dragon::Crystal::new(id, at, true, time);
                    self.crystals.push(crystal);
                    // tryAddFreshEntityWithPassengers refused: ERROR_DUPLICATE_UUID.
                    if !self.native_joins_flush(false, id) {
                        return Err(err("commands.summon.failed.uuid", &[]));
                    }
                    name = self.target_name(Target::Crystal(id));
                }
                _ if super::minecarts::CartKind::from_id(&kind).is_some() => {
                    let cart_kind = super::minecarts::CartKind::from_id(&kind)
                        .unwrap_or(super::minecarts::CartKind::Rideable);
                    self.next_entity += 1;
                    let mut cart = super::minecarts::Minecart::new(
                        self.next_entity,
                        cart_kind,
                        Vec3::new(pos[0], pos[1], pos[2]),
                    );
                    // SummonCommand.createEntity loads even an empty compound, so a
                    // spawner minecart's BaseSpawner.load runs: SpawnPotentials becomes
                    // WeightedList.of(new SpawnData()).
                    if cart.spawner.is_some() {
                        use sunrise_client::mobs::spawner::{Spawner, SpawnerExt};
                        let empty = sunrise_core::nbt::Tag::Compound(Default::default());
                        cart.spawner = Some(Box::new(Spawner::load(&empty)));
                    }
                    let id = cart.id;
                    let uuid = sunrise_client::save::insecure_uuid(
                        &mut sunrise_client::random_source::entity(),
                    );
                    self.persist.uuids.insert(id, uuid);
                    self.carts.push(cart);
                    if !self.native_joins_flush(false, id) {
                        return Err(err("commands.summon.failed.uuid", &[]));
                    }
                    name = self.target_name(Target::Cart(id));
                }
                // SummonCommand.createEntity: any other type from an empty compound.
                _ => {
                    let before = self.spawned_mark();
                    name = match self.summon_with_nbt(&kind, pos, "{}")? {
                        Some(target) => self.target_name(target),
                        // Spawned as a kind no command target stands for
                        // (primed TNT, falling blocks, projectiles).
                        None if self.spawned_mark() != before => data::summon_type_name(&kind),
                        None => return Err(err("commands.summon.failed", &[])),
                    };
                }
            }
        }
        self.feedback(Text::translate("commands.summon.success", &[name]));
        Ok(1)
    }
    /// What changes when an entity spawns, kinds that take their id only
    /// when first tracked (primed TNT, falling blocks, projectiles)
    /// included: a summon compares it before and after.
    fn spawned_mark(&self) -> (u32, usize) {
        (
            self.next_entity,
            self.tnts.len()
                + self.falling.len()
                + self.creatures.projectiles.len()
                + self.rockets.len()
                + self.tridents.len(),
        )
    }
    /// BlockStateArgument -> registry state.
    pub(super) fn block_state(&self, b: &BlockInput) -> R<BlockStateId> {
        let state = sunrise_world::chunk::BlockState {
            name: b.id.clone(),
            properties: b.properties.iter().cloned().collect(),
        };
        self.pack
            .assets
            .registry
            .resolve(&state)
            .map_err(|_| err("argument.block.id.invalid", &[&b.id]))
    }
    /// BlockPredicateArgument test of the block at `p`: its state, then any
    /// `{...}` against the block entity's `saveWithFullMetadata`
    /// (`NbtUtils.compareNbt(…, true)`); no block entity fails.
    fn block_matches(&self, b: &BlockInput, p: [i32; 3]) -> bool {
        let state = self.world.block(p[0], p[1], p[2]);
        if !self.command_block_state_matches(b, state) {
            return false;
        }
        let Some(nbt) = &b.nbt else { return true };
        // (the argument's own grammar, `TagParser`, as the predicate was read)
        let Ok(pattern) = sunrise_client::command::snbt::parse_fully(nbt) else {
            return false;
        };
        sunrise_client::nbt_path::compare_nbt(&pattern, self.block_entity_nbt(p).as_ref())
    }
    /// The state part of `block_matches`: name or tag, then properties.
    fn command_block_state_matches(&self, b: &BlockInput, state: BlockStateId) -> bool {
        block_predicate_matches_state(
            self.pack.assets.registry,
            [&self.res.items().block_tags, &self.items().block_tags],
            &b.id,
            b.tag,
            b.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            state,
        )
    }
    /// `BlockInput.overwriteWithDefinedProperties`: restore explicit command
    /// properties after normalizing the state against neighbouring blocks.
    fn block_input_state(
        &mut self,
        pos: [i32; 3],
        input: &BlockInput,
        state: BlockStateId,
        strict: bool,
    ) -> R<BlockStateId> {
        if strict {
            return Ok(state);
        }
        let updated = self.with_level(|b, level, ticks| {
            let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
            let updated = b.update_from_neighbour_shapes(level, ticks, at, state);
            if b.is_air(updated) { state } else { updated }
        });
        let registry = self.pack.assets.registry;
        let mut descriptor = registry
            .state(updated)
            .expect("registered block state")
            .clone();
        for (key, value) in &input.properties {
            if descriptor.properties.contains_key(key) {
                descriptor.properties.insert(key.clone(), value.clone());
            }
        }
        registry
            .resolve(&descriptor)
            .map_err(|_| err("argument.block.id.invalid", &[&input.id]))
    }
    /// Place a state like BlockInput.place: false when nothing changed.
    /// `BlockInput.place` for a block of a command. `command`: `/setblock` and `/fill` place with
    /// `UPDATE_CLIENTS | UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS`, so `Level.setBlock` notifies no neighbour,
    /// and send that notice themselves once the shape updates are done (`update_neighbours_on_block_set`):
    /// a block those shape updates pop off is gone before its neighbours are told of it.
    pub(super) fn place_state(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        strict: bool,
        command: bool,
    ) -> R<bool> {
        let previous = self.world.block(pos[0], pos[1], pos[2]);
        if previous == state {
            return Ok(false);
        }
        self.ensure_stateful_entity_table();
        self.sculk_prepare();
        // A different block replaces the block entity (Level.setBlock).
        let (old, new) = (
            self.res.facts().get(previous).name.clone(),
            self.res.facts().get(state).name.clone(),
        );
        // The command clears the old block entity without its side effects
        // (UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS); the new block gets a fresh one.
        let key = (pos[0], pos[1], pos[2]);
        if old != new {
            self.jukebox_removed(pos);
            self.world.block_entities.remove(&key);
        }
        if strict {
            self.with_level(|b, level, ticks| {
                b.set_block(
                    level,
                    ticks,
                    sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                    state,
                    sunrise_world::level::flags::UPDATE_CLIENTS
                        | sunrise_world::level::flags::UPDATE_SKIP_ALL_SIDEEFFECTS,
                );
            });
            self.process_level_events()
                .map_err(|e| CommandError::new(Text::literal(e.to_string())))?;
        } else if command {
            self.level_set_with_flags(
                pos,
                state,
                sunrise_world::level::flags::UPDATE_CLIENTS
                    | sunrise_world::level::flags::UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS,
            )
            .map_err(|e| CommandError::new(Text::literal(e.to_string())))?;
        } else {
            self.level_set(pos, state)
                .map_err(|e| CommandError::new(Text::literal(e.to_string())))?;
        }
        // LevelChunk.setBlockState creates it when the block, not the state, is still
        // there after onPlace: a powered hopper turns itself `enabled=false`.
        let landed = self.world.block(pos[0], pos[1], pos[2]);
        let needs_entity = !self.world.block_entities.stateful_entity(landed.0) && old != new;
        if needs_entity
            && self.res.facts().get(landed).name == new
            && !self.sculk_place_entity(landed)
            && let Some(entity) = self.new_block_entity_for_state(landed)
        {
            self.world.block_entities.insert(key, entity);
        }
        Ok(true)
    }
    fn cmd_setblock(&mut self, ctx: &Context) -> R<i32> {
        let pos = self.block_pos_arg(ctx, "pos")?;
        let Some(Arg::Block(b)) = ctx.get("block").cloned() else {
            return Ok(0);
        };
        let state = self.block_state(&b)?;
        // SetBlockCommand.setBlock: `ServerLevel.isDebug` fails it.
        if self.world_debug {
            return Err(err("commands.setblock.failed", &[]));
        }
        let current = self.world.block(pos[0], pos[1], pos[2]);
        if ctx.has("keep") && !self.res.facts().is_air(current) {
            return Err(err("commands.setblock.failed", &[]));
        }
        // SetBlockCommand.setBlock skips placement when destruction already
        // leaves an air state and the requested state is also air.
        let place_needed = if ctx.has("destroy") {
            self.destroy_block_dropping(pos)?;
            let remaining = self.world.block(pos[0], pos[1], pos[2]);
            !(self.res.facts().is_air(state) && self.res.facts().is_air(remaining))
        } else {
            true
        };
        if place_needed {
            let state = self.block_input_state(pos, &b, state, ctx.has("strict"))?;
            // SetBlockCommand.setBlock reads the old state after a destroy.
            let old_state = self.world.block(pos[0], pos[1], pos[2]);
            let placed = self.place_state(pos, state, ctx.has("strict"), true)?;
            let loaded = match &b.nbt {
                Some(nbt) => self.load_block_entity_nbt(pos, nbt),
                None => false,
            };
            if !placed && !loaded {
                return Err(err("commands.setblock.failed", &[]));
            }
            if !ctx.has("strict") {
                self.update_neighbours_on_block_set(pos, old_state)?;
            }
        } else if !ctx.has("strict") {
            // ServerLevel.updateNeighboursOnBlockSet still runs when placing
            // air was unnecessary. The unchanged air has no analog output or
            // removal side effects, so only its neighbours need notification.
            let remaining = self.world.block(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| {
                b.update_neighbors_at(
                    level,
                    ticks,
                    sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                    b.block(remaining),
                );
            });
            self.process_level_events()
                .map_err(|e| CommandError::new(Text::literal(e.to_string())))?;
        }
        self.feedback(Text::tr(
            "commands.setblock.success",
            &[&pos[0], &pos[1], &pos[2]],
        ));
        Ok(1)
    }
    /// `ServerLevel.updateNeighboursOnBlockSet` (`Behaviours::update_neighbours_on_block_set`) for the
    /// block now at `pos`, which replaced `old`.
    fn update_neighbours_on_block_set(&mut self, pos: [i32; 3], old: BlockStateId) -> R<()> {
        self.with_level(|b, level, ticks| {
            b.update_neighbours_on_block_set(
                level,
                ticks,
                sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                old,
            );
        });
        self.process_level_events()
            .map_err(|e| CommandError::new(Text::literal(e.to_string())))
    }
    /// `Level.destroyBlock(pos, true)` through the behaviour library: the
    /// destroy event, the block's drops without a tool or breaker, then its
    /// fluid's block, the block entity spilling its contents on the way.
    pub(super) fn destroy_block_dropping(&mut self, pos: [i32; 3]) -> R<()> {
        self.destroy_block_by(pos, None)
    }
    /// `Level.destroyBlock(pos, true, entity)`: [`Self::destroy_block_dropping`]
    /// with `entity` as the loot's `LootContextParams.THIS_ENTITY`.
    pub(super) fn destroy_block_by(&mut self, pos: [i32; 3], entity: Option<u32>) -> R<()> {
        let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        let entity = self.loot_entity_ref(entity);
        self.with_level(|b, level, ticks| {
            level.drop_entity = entity;
            level.drop_breaker_at = entity.map(|_| at);
            b.destroy_block(level, ticks, at, true)
        });
        self.process_level_events()
            .map_err(|e| CommandError::new(Text::literal(e.to_string())))
    }
    /// The block entity part of `BlockInput.place`: load the block entity
    /// from the argument's tag; true when its saved data changed.
    pub(super) fn load_block_entity_nbt(&mut self, pos: [i32; 3], nbt: &str) -> bool {
        let Ok((tag, _)) = sunrise_core::nbt::parse_snbt(nbt) else {
            return false;
        };
        self.load_block_entity_tag(pos, tag)
    }

    /// `RandomizableContainer.unpackLootTable` for a player who opens a
    /// container: a roll the container still holds fires
    /// `player_generates_container_loot` for its table, and from then on the
    /// container saves its items, not the table.
    pub(super) fn unpack_opened_loot(&mut self, key: (i32, i32, i32), _name: &str) {
        let player = self.native_acting_player();
        self.unpack_loot(crate::game::container_loot::LootTarget::Block(key), player);
        if let Some((other, _)) = self.chest_partner(key) {
            self.unpack_loot(
                crate::game::container_loot::LootTarget::Block(other),
                player,
            );
        }
    }
    /// Install a command's table without rolling. Raw NBT reads keep it pending.
    pub(super) fn take_loot_table(
        &mut self,
        key: (i32, i32, i32),
        tag: &sunrise_core::nbt::Tag,
        entity: &mut sunrise_client::containers::BlockEntity,
    ) -> bool {
        let block = self.data_block_name([key.0, key.1, key.2]);
        let pending = sunrise_client::save::randomizable_container(entity, &block)
            .then(|| sunrise_client::save::PendingContainerLoot::from_tag(tag))
            .flatten();
        let mut changed = false;
        self.world.with_chunk_extras(key, |extras| {
            changed = extras.pending_loot.get(&key) != pending.as_ref();
            extras.pending_loot.remove(&key);
            extras.pending_loot.extend(pending.map(|p| (key, p)));
        });
        changed
    }
    /// Raw NBT inspection overlays metadata without unpacking the inventory: a container's
    /// table, which a slotless mod block entity at `block` never has (its `LootTable` field is
    /// its own).
    pub(super) fn write_pending_loot(
        world: &sunrise_client::world::World,
        key: (i32, i32, i32),
        block: &str,
        tag: &mut sunrise_core::nbt::Tag,
    ) {
        if !world
            .block_entities
            .get(&key)
            .is_some_and(|entity| sunrise_client::save::randomizable_container(entity, block))
        {
            return;
        }
        if let sunrise_core::nbt::Tag::Compound(map) = tag {
            map.remove(&"LootTable".into());
            map.remove(&"LootTableSeed".into());
            if let Some(pending) = world
                .chunks
                .get(&(key.0 >> 4, key.2 >> 4))
                .and_then(|c| c.saved.as_ref())
                .and_then(|s| s.pending_loot.get(&key))
            {
                let items = map.get(&"Items".into()).cloned();
                pending.write_tags(map);
                if world
                    .block_entities
                    .get(&key)
                    .is_some_and(|e| e.slots().iter().any(Option::is_some))
                {
                    map.extend(items.map(|items| ("Items".into(), items)));
                }
            }
        }
    }
    fn cmd_fill(&mut self, ctx: &Context) -> R<i32> {
        let a = self.block_pos_arg(ctx, "from")?;
        let b = self.block_pos_arg(ctx, "to")?;
        let Some(Arg::Block(block)) = ctx.get("block").cloned() else {
            return Ok(0);
        };
        let filter = match ctx.get("filter") {
            Some(Arg::Block(f)) => Some(f.clone()),
            _ => None,
        };
        let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
        let area = (0..3)
            .map(|i| i64::from(max[i] - min[i] + 1))
            .product::<i64>();
        let limit = self
            .ui
            .rules
            .int_at(sunrise_client::gamerules::vanilla::MAX_BLOCK_MODIFICATIONS);
        if area > i64::from(limit) {
            return Err(err("commands.fill.toobig", &[&limit, &area]));
        }
        // FillCommand.fillBlocks: `ServerLevel.isDebug` fails it.
        if self.world_debug {
            return Err(err("commands.fill.failed", &[]));
        }
        let state = self.block_state(&block)?;
        let (hollow, outline, destroy, strict, keep) = (
            ctx.has("hollow"),
            ctx.has("outline"),
            ctx.has("destroy"),
            ctx.has("strict"),
            ctx.has("keep"),
        );
        let normalize_note = !strict
            && self
                .res
                .behaviours()
                .data(state)
                .is(sunrise_world::level::classes::Class::Note);
        let mut count = 0;
        // What was placed, and what it replaced, for the notices after the last block.
        let mut updated: Vec<([i32; 3], BlockStateId)> = Vec::new();
        // Whether the filter takes a state, worked out once per state: a fill visits up to 32768 blocks of a handful
        // of states, and matching a tag is a name lookup.
        let mut matched: std::collections::HashMap<BlockStateId, bool> =
            std::collections::HashMap::new();
        // A tag can change a block entity whose block is already in place.
        let plain = !(destroy || hollow || outline) && block.nbt.is_none();
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                for x in min[0]..=max[0] {
                    let current = self.world.block(x, y, z);
                    if keep && !self.res.facts().is_air(current) {
                        continue;
                    }
                    // Already the block it would become: nothing to place (place_state would do nothing either).
                    // `BlockInput.place` can change a note's instrument even
                    // when the raw input equals the current state.
                    if plain && current == state && !normalize_note {
                        continue;
                    }
                    if let Some(f) = &filter {
                        // (a filter with NBT reads each block's own block entity)
                        let ok = if f.nbt.is_some() {
                            self.block_matches(f, [x, y, z])
                        } else {
                            match matched.get(&current) {
                                Some(&ok) => ok,
                                None => {
                                    let ok = self.command_block_state_matches(f, current);
                                    matched.insert(current, ok);
                                    ok
                                }
                            }
                        };
                        if !ok {
                            continue;
                        }
                    }
                    let edge = x == min[0]
                        || x == max[0]
                        || y == min[1]
                        || y == max[1]
                        || z == min[2]
                        || z == max[2];
                    let mut affected = false;
                    if destroy && !self.res.facts().is_air(current) {
                        self.destroy_block_dropping([x, y, z])?;
                        affected = true;
                    }
                    let core = (outline || hollow) && !edge;
                    let target = if core {
                        if outline {
                            if affected {
                                count += 1;
                            }
                            continue;
                        }
                        self.world.air
                    } else {
                        state
                    };
                    let target = if normalize_note && target == state {
                        self.block_input_state([x, y, z], &block, target, strict)?
                    } else {
                        target
                    };
                    let previous = self.world.block(x, y, z);
                    let placed = self.place_state([x, y, z], target, strict, true)?;
                    // BlockInput.place loads the tag into the block entity now
                    // there; a tag that changed it counts the block as placed.
                    // A hollow fill's core is plain air.
                    let loaded = !core
                        && block
                            .nbt
                            .as_deref()
                            .is_some_and(|nbt| self.load_block_entity_nbt([x, y, z], nbt));
                    if placed || loaded {
                        count += 1;
                        if !strict {
                            updated.push(([x, y, z], previous));
                        }
                    } else if affected {
                        count += 1;
                    }
                }
            }
        }
        // FillCommand.fillBlocks: every block is in before any neighbour hears of one.
        for (pos, previous) in updated {
            self.update_neighbours_on_block_set(pos, previous)?;
        }
        if count == 0 {
            return Err(err("commands.fill.failed", &[]));
        }
        self.feedback(Text::tr("commands.fill.success", &[&count]));
        Ok(count)
    }
    /// `CloneCommands`: `from <sourceDimension>` is resolved as `begin` is
    /// read, `to <targetDimension>` as `destination` is
    /// (`DimensionArgument.getDimension`), each position checked loaded in its
    /// own level (`BlockPosArgument.getLoadedBlockPos`): an unknown source fails
    /// before an unknown target. The blocks are read in the source level and
    /// placed in the target level. The command's own level is back in place,
    /// whatever happened, before the command reports.
    fn cmd_clone(&mut self, ctx: &Context) -> R<i32> {
        let here = self.dim;
        let result = self.clone_levels(ctx);
        if self.dim != here {
            self.enter_command_level(here);
        }
        if self.dim != here {
            log::error!("/clone could not return to {}", here.id());
            return Err(err("commands.clone.failed", &[]));
        }
        let count = result?;
        self.feedback(Text::tr("commands.clone.success", &[&count]));
        Ok(count)
    }
    fn clone_levels(&mut self, ctx: &Context) -> R<i32> {
        let here = self.dim;
        let source_level = self
            .command_source
            .as_ref()
            .map_or(self.dim, |s| s.dimension);
        // The coordinates, from the command's own source, before any level moves.
        let position = |name: &str| -> R<[i32; 3]> {
            let c = self
                .coords(ctx, name)
                .ok_or_else(|| err("argument.pos.missing.int", &[]))?;
            Ok(self.resolve_coords(&c).map(|v| v.floor() as i32))
        };
        let (a, b, dest) = (
            position("begin")?,
            position("end")?,
            position("destination")?,
        );
        let from_dim = self.clone_dimension(ctx, "sourceDimension")?;
        self.clone_loaded(from_dim, here, source_level, a)?;
        self.clone_loaded(from_dim, here, source_level, b)?;
        let to_dim = self.clone_dimension(ctx, "targetDimension")?;
        self.clone_loaded(to_dim, here, source_level, dest)?;
        self.clone_between(
            ctx,
            a,
            b,
            dest,
            from_dim.unwrap_or(here),
            to_dim.unwrap_or(here),
        )
    }
    /// `BlockPosArgument.getLoadedBlockPos` (`Level.isInWorldBounds`) in the
    /// level a dimension argument names, or, without one, the command's own.
    fn clone_loaded(
        &mut self,
        dim: Option<Dim>,
        here: Dim,
        source_level: Dim,
        p: [i32; 3],
    ) -> R<()> {
        match dim {
            Some(dim) => self.clone_enter(dim)?,
            None => {
                self.clone_enter(here)?;
                if source_level != self.dim {
                    return Err(err("argument.pos.unloaded", &[]));
                }
            }
        }
        if !self.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4)) {
            return Err(err("argument.pos.unloaded", &[]));
        }
        if p[1] < self.world.shape.min_y || p[1] >= self.world.shape.max_y() {
            return Err(err("argument.pos.outofworld", &[]));
        }
        Ok(())
    }
    /// `DimensionArgument.getDimension`: a built-in dimension, or
    /// `argument.dimension.invalid`; None when the argument is absent.
    fn clone_dimension(&self, ctx: &Context, name: &str) -> R<Option<Dim>> {
        let Some(Arg::Resource(id)) = ctx.get(name) else {
            return Ok(None);
        };
        if !matches!(
            id.as_str(),
            "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
        ) {
            return Err(err("argument.dimension.invalid", &[id]));
        }
        Ok(Some(Dim::from_id(id)))
    }
    /// Put `dim`'s level in place for the clone, or fail it when it cannot be.
    fn clone_enter(&mut self, dim: Dim) -> R<()> {
        if self.dim != dim {
            self.enter_command_level(dim);
        }
        if self.dim != dim {
            return Err(err("commands.clone.failed", &[]));
        }
        Ok(())
    }
    fn clone_between(
        &mut self,
        ctx: &Context,
        a: [i32; 3],
        b: [i32; 3],
        dest: [i32; 3],
        from_dim: Dim,
        to_dim: Dim,
    ) -> R<i32> {
        let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
        let max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
        let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
        let dmax = [dest[0] + size[0], dest[1] + size[1], dest[2] + size[2]];
        let overlap = (0..3).all(|i| dest[i] <= max[i] && dmax[i] >= min[i]);
        let moving = ctx.has("move");
        // Mode.canOverlap: only the normal mode refuses overlapping regions.
        if !ctx.has("force") && !moving && from_dim == to_dim && overlap {
            return Err(err("commands.clone.overlap", &[]));
        }
        let area = (0..3).map(|i| i64::from(size[i] + 1)).product::<i64>();
        let limit = self
            .ui
            .rules
            .int_at(sunrise_client::gamerules::vanilla::MAX_BLOCK_MODIFICATIONS);
        if area > i64::from(limit) {
            return Err(err("commands.clone.toobig", &[&limit, &area]));
        }
        // CloneCommands.clone: every column of both boxes must be loaded, in each level.
        self.clone_boxes_loaded([(from_dim, a, b), (to_dim, dest, dmax)])?;
        // CloneCommands.clone: the destination level being a Debug world fails it.
        if self.world_debug {
            return Err(err("commands.clone.failed", &[]));
        }
        let filter = match ctx.get("filter") {
            Some(Arg::Block(f)) => Some(f.clone()),
            _ => None,
        };
        let masked = ctx.has("masked");
        self.clone_enter(from_dim)?;
        let mut copies = Vec::new();
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                for x in min[0]..=max[0] {
                    let s = self.world.block(x, y, z);
                    if masked && self.res.facts().is_air(s) {
                        continue;
                    }
                    if let Some(f) = &filter
                        && !self.block_matches(f, [x, y, z])
                    {
                        continue;
                    }
                    copies.push((
                        [x, y, z],
                        [
                            dest[0] + x - min[0],
                            dest[1] + y - min[1],
                            dest[2] + z - min[2],
                        ],
                        s,
                        self.clone_block_payload((x, y, z)),
                    ));
                }
            }
        }
        if moving {
            for (from, ..) in &copies {
                self.place_state(*from, self.world.air, true, false)?;
            }
        }
        self.clone_enter(to_dim)?;
        let strict = ctx.has("strict");
        // CloneCommands.clone sets every destination to a barrier first, so
        // each copied state but a barrier counts as placed.
        let barrier = self.pack.assets.registry.default_state("minecraft:barrier");
        let mut count = 0;
        for (_, to, s, payload) in copies {
            self.discard_clone_loot(to);
            self.place_state(to, s, strict, false)?;
            // CloneCommands.clone counts a block only when Level.setBlock returned true.
            if self.clone_writable(to) && Some(s) != barrier {
                count += 1;
            }
            self.apply_clone_payload(to, payload);
        }
        // LevelTicks.copyAreaFrom: the block ticks pending anywhere in the
        // source box go to the same offset in the destination, before the
        // command can fail on an empty count.
        {
            use sunrise_core::position::BlockPos;
            self.block_ticks.blocks.copy_area(
                sunrise_core::ticks::TickArea::new(
                    BlockPos::new(min[0], min[1], min[2]),
                    BlockPos::new(max[0], max[1], max[2]),
                ),
                BlockPos::new(dest[0] - min[0], dest[1] - min[1], dest[2] - min[2]),
            );
        }
        if count == 0 {
            return Err(err("commands.clone.failed", &[]));
        }
        Ok(count)
    }
    /// AdvancementCommands.perform / performCriterion.
    fn cmd_advancement(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", true)?;
        self.load_advancements();
        let tree = self.adv.tree.clone();
        let grant = ctx.has("grant");
        let key = if grant { "grant" } else { "revoke" };
        let index = match ctx.get("advancement") {
            Some(Arg::Resource(id)) => Some(
                tree.find_id(id)
                    .ok_or_else(|| err("advancement.advancementNotFound", &[id]))?,
            ),
            _ => None,
        };
        if let (Some(i), Some(Arg::Str(criterion))) = (index, ctx.get("criterion")) {
            let a = &tree.list[i];
            let name = super::advancements::adv_name(a);
            if !a.criteria.iter().any(|c| &c.name == criterion) {
                return Err(CommandError::new(Text::translate(
                    "commands.advancement.criterionNotFound",
                    &[name, Text::literal(criterion.clone())],
                )));
            }
            // AdvancementCommands.performCriterion: each selected player's own turn.
            let results = self.advancement_turns(&targets, |g| {
                i32::from(if grant {
                    g.grant_advancement(i, Some(criterion))
                } else {
                    g.revoke_advancement(i, Some(criterion))
                })
            });
            let (who, many) = self.advancement_subject(&results);
            let to = if many { "many" } else { "one" };
            let args = [Text::literal(criterion.clone()), name, who];
            if results.iter().map(|r| r.1).sum::<i32>() == 0 {
                return Err(CommandError::new(Text::translate(
                    &format!("commands.advancement.{key}.criterion.to.{to}.failure"),
                    &args,
                )));
            }
            self.feedback(Text::translate(
                &format!("commands.advancement.{key}.criterion.to.{to}.success"),
                &args,
            ));
            return Ok(results.iter().map(|r| r.1).sum());
        }
        // getAdvancements: parents first (nearest up), the target, then
        // the subtree depth first.
        let list: Vec<usize> = match index {
            None => (0..tree.list.len()).collect(),
            Some(i) => {
                let mut out = Vec::new();
                if ctx.has("until") || ctx.has("through") {
                    let mut p = tree.list[i].parent.as_ref();
                    while let Some(pi) = p.and_then(|id| tree.find_id(id)) {
                        out.push(pi);
                        p = tree.list[pi].parent.as_ref();
                    }
                }
                out.push(i);
                if ctx.has("from") || ctx.has("through") {
                    let kids = tree.children();
                    fn add(kids: &[Vec<usize>], i: usize, out: &mut Vec<usize>) {
                        for &c in &kids[i] {
                            out.push(c);
                            add(kids, c, out);
                        }
                    }
                    add(&kids, i, &mut out);
                }
                out
            }
        };
        let quiet = index.is_none();
        // AdvancementCommands.perform: Action.perform for each selected player.
        let results = self.advancement_turns(&targets, |g| {
            let was_quiet = std::mem::replace(&mut g.adv.quiet, quiet);
            let mut count = 0;
            for &i in &list {
                let changed = if grant {
                    g.grant_advancement(i, None)
                } else {
                    g.revoke_advancement(i, None)
                };
                count += i32::from(changed);
            }
            g.adv.quiet = was_quiet;
            count
        });
        let total: i32 = results.iter().map(|r| r.1).sum();
        let (who, many) = self.advancement_subject(&results);
        let to = if many { "many" } else { "one" };
        let (what, first) = if list.len() == 1 {
            ("one", super::advancements::adv_name(&tree.list[list[0]]))
        } else {
            ("many", Text::literal(list.len().to_string()))
        };
        let args = [first, who];
        if total == 0 {
            return Err(CommandError::new(Text::translate(
                &format!("commands.advancement.{key}.{what}.to.{to}.failure"),
                &args,
            )));
        }
        self.feedback(Text::translate(
            &format!("commands.advancement.{key}.{what}.to.{to}.success"),
            &args,
        ));
        Ok(total)
    }
    /// Run `f` as each selected player's own turn and collect what it changed;
    /// a target without a turn counts as unchanged, like in `cmd_clear`.
    fn advancement_turns(
        &mut self,
        targets: &[Target],
        f: impl Fn(&mut Self) -> i32,
    ) -> Vec<(Target, i32)> {
        targets
            .iter()
            .map(|&t| (t, self.for_player(t, &f).unwrap_or(0)))
            .collect()
    }
    /// The player argument of the advancement feedback: with the tracker's
    /// NON_ZERO elements on success and every element on failure, the one
    /// player's name, else the count (CommandResponseTracker.dispatch).
    fn advancement_subject(&self, results: &[(Target, i32)]) -> (Text, bool) {
        let changed: Vec<Target> = results.iter().filter(|r| r.1 != 0).map(|r| r.0).collect();
        let picked: Vec<Target> = if changed.is_empty() {
            results.iter().map(|r| r.0).collect()
        } else {
            changed
        };
        match picked.as_slice() {
            [t] => (self.target_name(*t), false),
            _ => (Text::literal(picked.len().to_string()), true),
        }
    }
    fn cmd_experience(&mut self, ctx: &Context) -> R<i32> {
        let levels = ctx.has("levels");
        let unit = if levels { "levels" } else { "points" };
        if ctx.has("query") {
            let target = self.entities(ctx, "target", true)?[0];
            // ExperienceCommand.queryExperience: the target's own level or points.
            let value = self
                .for_player(target, |g| {
                    let e = &g.experience;
                    if levels {
                        e.level
                    } else {
                        (e.progress * sunrise_client::entities::Experience::needed(e.level) as f32)
                            as i32
                    }
                })
                .unwrap_or(0);
            self.feedback_unbroadcast(Text::translate(
                &format!("commands.experience.query.{unit}"),
                &[self.target_name(target), Text::literal(value.to_string())],
            ));
            return Ok(value);
        }
        let targets = self.entities(ctx, "target", true)?;
        let amount = Self::int(ctx, "amount").unwrap_or(0);
        let add = ctx.has("add");
        let op = if add { "add" } else { "set" };
        // Every target changes (a set of points can be refused for some).
        let changed: Vec<Target> = targets
            .iter()
            .copied()
            .filter(|&t| {
                self.for_player(t, |g| g.change_experience(add, levels, amount)) == Some(true)
            })
            .collect();
        if changed.is_empty() {
            return Err(err("commands.experience.set.points.invalid", &[]));
        }
        let (key, who) = match changed[..] {
            [only] => ("single", self.target_name(only)),
            _ => ("multiple", Text::literal(changed.len().to_string())),
        };
        self.feedback(Text::translate(
            &format!("commands.experience.{op}.{unit}.success.{key}"),
            &[Text::literal(amount.to_string()), who],
        ));
        Ok(changed.len() as i32)
    }
    /// ExperienceCommand's add and set on the player in place: false when
    /// the points to set do not fit the level.
    fn change_experience(&mut self, add: bool, levels: bool, amount: i32) -> bool {
        if add {
            if levels {
                // Player.giveExperienceLevels handles the strict negative
                // boundary and emits the level-change hook itself.
                self.give_levels(amount);
                return true;
            } else {
                self.give_experience_points(amount);
            }
            return true;
        }
        if levels {
            self.set_experience_levels(amount);
        } else {
            let needed = sunrise_client::entities::Experience::needed(self.experience.level);
            if amount > needed - 1 {
                return false;
            }
            self.experience.progress = amount as f32 / needed as f32;
        }
        true
    }

    /// `ServerPlayer.setExperienceLevels`: an absolute write is not a level grant.
    pub(super) fn set_experience_levels(&mut self, level: i32) {
        let old_level = self.experience.level;
        self.experience.level = level;
        self.native_player_level_changed(old_level);
    }
    /// `GameRules.set` as the `/gamerule` command and the management protocol call it: the rule changes, the
    /// world follows it, and a dedicated server tells the event stream (`MinecraftServer.onGameRuleChanged`
    /// tells the management protocol's clients).
    pub(super) fn set_game_rule(
        &mut self,
        rule: &gamerules::Rule,
        value: Value,
        actor: Option<mod_api::PlayerRef>,
    ) {
        if self.ui.rules.set(&rule.name, value) {
            self.game_rule_changed(&rule.name, value, actor);
        }
        self.sync_rules();
        if self.is_dedicated_server() {
            let (value_type, json) = match value {
                Value::Bool(b) => ("boolean", serde_json::json!(b)),
                Value::Int(i) => ("integer", serde_json::json!(i)),
            };
            super::dedicated::events::record_server(
                "gamerule",
                serde_json::json!({
                    "key": rule.name.clone(),
                    "valueType": value_type,
                    "value": json,
                }),
            );
        }
    }
    fn cmd_gamerule(&mut self, ctx: &Context) -> R<i32> {
        let Some(name) = ctx.literals.get(1).cloned() else {
            return Ok(0);
        };
        let Some(rule) = self.ui.rules.rule(&name).cloned() else {
            return Ok(0);
        };
        // GameRule.id(): only the default namespace is omitted in messages.
        let rule_id = rule.name.strip_prefix("minecraft:").unwrap_or(&rule.name);
        let current = self.ui.rules.get(&rule.name).unwrap_or(rule.default);
        let value = match ctx.get("value") {
            Some(Arg::Bool(b)) => Value::Bool(*b),
            Some(Arg::Int(i)) => Value::Int(*i),
            _ => {
                self.feedback_unbroadcast(Text::tr(
                    "commands.gamerule.query",
                    &[&rule_id, &current],
                ));
                return Ok(match current {
                    Value::Bool(b) => i32::from(b),
                    Value::Int(i) => i,
                });
            }
        };
        if current == value {
            return Err(err("commands.gamerule.not_set", &[&rule_id, &value]));
        }
        let actor = self.native_lifecycle.epoch().and_then(|epoch| {
            Some(mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(epoch, self.lan_acting_entity?),
                self.lan_acting_uuid?,
            ))
        });
        self.set_game_rule(&rule, value, actor);
        self.feedback(Text::tr("commands.gamerule.set", &[&rule_id, &value]));
        Ok(match value {
            Value::Bool(b) => i32::from(b),
            Value::Int(i) => i,
        })
    }
    fn cmd_spawnpoint(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", true)?
        } else {
            vec![Target::Player]
        };
        let pos = if ctx.get("pos").is_some() {
            self.spawnable_pos_arg(ctx, "pos")?
        } else {
            let p = self.source_position();
            [
                p[0].floor() as i32,
                p[1].floor() as i32,
                p[2].floor() as i32,
            ]
        };
        let rot = match ctx.get("rotation") {
            Some(Arg::Coords(c)) => c.rotation(self.source_rotation()),
            _ => [0.0, 0.0],
        };
        let yaw = ((rot[0] + 180.0).rem_euclid(360.0)) - 180.0;
        let pitch = rot[1].clamp(-90.0, 90.0);
        // SetSpawnCommand: a forced point in the source's level, for each target.
        let level = self.dim;
        for &t in &targets {
            self.for_player(t, |g| {
                g.set_respawn_point(Some(super::respawn_anchor::RespawnConfig::forced(
                    level, pos, yaw, pitch,
                )));
            });
        }
        let dim = Text::literal(self.dim.id());
        let p = pos.map(|v| Text::literal(v.to_string()));
        let mut args = vec![
            p[0].clone(),
            p[1].clone(),
            p[2].clone(),
            Text::literal(float_to_string(yaw)),
            Text::literal(float_to_string(pitch)),
            dim,
        ];
        let key = if targets.len() == 1 {
            args.push(self.target_name(targets[0]));
            "commands.spawnpoint.success.single"
        } else {
            args.push(Text::literal(targets.len().to_string()));
            "commands.spawnpoint.success.multiple"
        };
        self.feedback(Text::translate(key, &args));
        Ok(targets.len() as i32)
    }
    /// `LevelData.setRespawnData(RespawnData.of(..))`: the world's spawn (level.dat `spawn`)
    /// moves to `pos`, its yaw wrapped and its pitch clamped, and `spawn_change` fires.
    pub(in crate::game) fn set_world_spawn(
        &mut self,
        dimension: Dim,
        pos: [i32; 3],
        [yaw, pitch]: [f32; 2],
    ) -> [f32; 2] {
        let rot = [wrap_degrees(yaw), pitch.clamp(-90.0, 90.0)];
        let previous_point = self.world_spawn_position().map(|v| v.floor() as i32);
        let old_angle = self.level.world_spawn_angle;
        let old_level = self.world_spawn_level();
        let old = self.level.world_spawn.replace(pos);
        self.level.world_spawn_angle = rot;
        self.level.world_spawn_dimension =
            (dimension != Dim::Overworld).then(|| dimension.id().to_owned());
        if previous_point != pos
            || old_angle.map(f32::to_bits) != rot.map(f32::to_bits)
            || old_level != dimension
        {
            self.lan_broadcast_world_spawn();
        }
        self.native_spawn_change(dimension.id(), pos, rot);
        // A player whose fallback is the world spawn (no spawnpoint of its
        // own; a bed or an anchor that is gone falls back to it) respawns at
        // the new one.
        let floor = |s: Vec3| [s.x.floor() as i32, s.y.floor() as i32, s.z.floor() as i32];
        if self.spawn.map(floor) == old {
            self.spawn = Some(Vec3::new(
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]),
                f64::from(pos[2]) + 0.5,
            ));
        }
        rot
    }
    fn cmd_setworldspawn(&mut self, ctx: &Context) -> R<i32> {
        // SetWorldSpawnCommand: BlockPosArgument.getSpawnablePos, in the source's level.
        let pos = if ctx.get("pos").is_some() {
            self.spawnable_pos_arg(ctx, "pos")?
        } else {
            let p = self.source_position();
            [
                p[0].floor() as i32,
                p[1].floor() as i32,
                p[2].floor() as i32,
            ]
        };
        let rot = match ctx.get("rotation") {
            Some(Arg::Coords(c)) => c.rotation(self.source_rotation()),
            _ => [0.0, 0.0],
        };
        let dimension = self
            .command_source
            .as_ref()
            .map_or(self.dim, |s| s.dimension);
        let [yaw, pitch] = self.set_world_spawn(dimension, pos, rot);
        let args = [
            Text::literal(pos[0].to_string()),
            Text::literal(pos[1].to_string()),
            Text::literal(pos[2].to_string()),
            Text::literal(float_to_string(yaw)),
            Text::literal(float_to_string(pitch)),
            Text::literal(dimension.id()),
        ];
        self.feedback(Text::translate("commands.setworldspawn.success", &args));
        Ok(1)
    }
    fn item_matches(&self, predicate: &ItemInput, item: &str) -> bool {
        if predicate.any {
            return true;
        }
        if predicate.tag {
            let tag = predicate.id.trim_start_matches("minecraft:");
            return self
                .res
                .items()
                .item_tags
                .get(tag)
                .or_else(|| self.items().item_tags.get(&predicate.id))
                .is_some_and(|s| {
                    s.contains(item) || s.contains(item.trim_start_matches("minecraft:"))
                });
        }
        predicate.id == item
    }
    /// `ItemPredicateArgument`: the item test and its `[...]` component
    /// tests (presence, `=` equality, `!` negation, `|` alternatives).
    pub(super) fn stack_matches(&self, predicate: &ItemInput, stack: &ItemStack) -> bool {
        if !self.item_matches(predicate, &stack.item) {
            return false;
        }
        let Some(tests) = predicate.components.as_deref() else {
            return true;
        };
        let inner = tests
            .trim()
            .strip_prefix('[')
            .and_then(|t| t.strip_suffix(']'))
            .unwrap_or("");
        let components = match sunrise_client::item_data::component_snapshot(stack) {
            Some(sunrise_core::nbt::Tag::Compound(c)) => c,
            _ => Default::default(),
        };
        let test = |term: &str| -> bool {
            let term = term.trim();
            let (negated, term) = match term.strip_prefix('!') {
                Some(t) => (true, t.trim()),
                None => (false, term),
            };
            let split = term.find(['=', '~']).unwrap_or(term.len());
            let id = term[..split].trim();
            let id = if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            };
            let present = components.get(&id.as_str().into());
            let result = match term[split..].chars().next() {
                // Both count wrappers in `ItemPredicateArgument` use the live
                // stack count and the same integer bounds matcher.
                Some('=' | '~') if id == "minecraft:count" => {
                    let value = sunrise_core::nbt::parse_snbt(term[split + 1..].trim());
                    value.is_ok_and(|(v, _)| item_count::matches(stack.count, &v))
                }
                Some('=') => {
                    let value = sunrise_core::nbt::parse_snbt(term[split + 1..].trim())
                        .ok()
                        .map(|(t, _)| t);
                    match (present, value) {
                        (Some(have), Some(want)) => {
                            sunrise_client::nbt_path::compare_nbt(&want, Some(have))
                                && sunrise_client::nbt_path::compare_nbt(have, Some(&want))
                        }
                        _ => false,
                    }
                }
                Some('~') => {
                    // Component sub-predicates.
                    let value = sunrise_core::nbt::parse_snbt(term[split + 1..].trim())
                        .ok()
                        .map(|(t, _)| t);
                    match (id.as_str(), value) {
                        // `MinMaxBounds.Ints.matches`: DamagePredicate's damage-only
                        // Int bounds shape; effective DAMAGE presence is required.
                        // Other fields keep the entire existing predicate path.
                        ("minecraft:damage", Some(ref v))
                            if matches!(v, sunrise_core::nbt::Tag::Compound(c) if c.len() == 1)
                                && match v.get("damage") {
                                    Some(sunrise_core::nbt::Tag::Int(_)) => true,
                                    Some(sunrise_core::nbt::Tag::Compound(bounds)) => {
                                        bounds.iter().all(|(key, value)| {
                                            matches!(key.to_string_lossy().as_str(), "min" | "max")
                                                && matches!(value, sunrise_core::nbt::Tag::Int(_))
                                        })
                                    }
                                    _ => false,
                                } =>
                        {
                            let removed = stack
                                .data()
                                .is_some_and(|data| data.other("!minecraft:damage").is_some());
                            let has_damage = !removed
                                && (present.is_some()
                                    || self.items().get(&stack.item).is_some_and(|item| {
                                        item.components.iter().any(|c| c == "minecraft:damage")
                                    }));
                            let n = i64::from(stack.damage);
                            has_damage
                                && match v.get("damage").unwrap() {
                                    sunrise_core::nbt::Tag::Int(exact) => n == i64::from(*exact),
                                    bounds => {
                                        let get = |key| match bounds.get(key) {
                                            Some(sunrise_core::nbt::Tag::Int(value)) => {
                                                Some(i64::from(*value))
                                            }
                                            _ => None,
                                        };
                                        get("min").is_none_or(|min| n >= min)
                                            && get("max").is_none_or(|max| n <= max)
                                    }
                                }
                        }
                        (_, Some(v)) => present.is_some_and(|have| {
                            sunrise_client::nbt_path::compare_nbt(&v, Some(have))
                        }),
                        _ => false,
                    }
                }
                // Count is a pseudo-component, not a catalog component.
                _ if id == "minecraft:count" => present.is_some(),
                _ => self.stack_component_present(stack, &id),
            };
            result != negated
        };
        split_top_level(inner, ',')
            .iter()
            .filter(|t| !t.trim().is_empty())
            .all(|term| split_top_level(term, '|').iter().any(|alt| test(alt)))
    }
    /// `ItemPredicateArgument.createComponentExistencePredicate`: the stack's
    /// effective component presence, with its patch ahead of its prototype.
    fn stack_component_present(&self, stack: &ItemStack, id: &str) -> bool {
        // Read the modeled patch before saved-default elision. A removal wins
        // even when the actual item prototype supplies the component.
        if let Some(patch) = sunrise_client::item_data::components_tag(stack.damage, stack.data()) {
            if patch.get(&format!("!{id}")).is_some() {
                return false;
            }
            if patch.get(id).is_some() {
                return true;
            }
        }
        self.items()
            .get(&stack.item)
            .is_some_and(|item| item.components.iter().any(|key| key == id))
    }
    fn cmd_clear(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", true)?
        } else {
            vec![Target::Player]
        };
        let predicate = match ctx.get("item") {
            Some(Arg::Item(i)) => i.clone(),
            _ => ItemInput {
                id: "*".into(),
                tag: false,
                any: true,
                components: None,
            },
        };
        let max = Self::int(ctx, "maxCount").unwrap_or(-1);
        let counting = max == 0;
        // ClearInventoryCommands.clearInventory: each player on their own.
        let mut removed = 0i32;
        let mut found = Vec::new();
        for &t in &targets {
            let n = self
                .for_player(t, |g| g.clear_matching(&predicate, max))
                .unwrap_or(0);
            removed += n;
            if n != 0 {
                found.push(t);
            }
        }
        if removed == 0 {
            return Err(if targets.len() == 1 {
                CommandError::new(Text::translate(
                    "clear.failed.single",
                    &[self.target_name(targets[0])],
                ))
            } else {
                err("clear.failed.multiple", &[&targets.len()])
            });
        }
        let kind = if counting { "test" } else { "success" };
        let (key, who) = match found[..] {
            [only] => ("single", self.target_name(only)),
            _ => ("multiple", Text::literal(found.len().to_string())),
        };
        self.feedback(Text::translate(
            &format!("commands.clear.{kind}.{key}"),
            &[Text::literal(removed.to_string()), who],
        ));
        Ok(removed)
    }
    /// Inventory.clearOrCountMatchingItems for the player in place: `max`
    /// of the matching items go (all when negative, none and only counted
    /// when 0). Returns how many matched.
    fn clear_matching(&mut self, predicate: &ItemInput, max: i32) -> i32 {
        let counting = max == 0;
        // Inventory slots 0..40: main, armor feet to head, off hand;
        // then clearOrCountMatchingItems visits the crafting grid and carried stack.
        let mut removed = 0i32;
        let mut budget = if max < 0 { i32::MAX } else { max };
        let all = |g: &Self| -> Vec<Option<ItemStack>> {
            let mut v: Vec<Option<ItemStack>> = g.inventory.clone();
            v.extend(g.armor.iter().rev().cloned());
            v.push(g.offhand.clone());
            v.extend(g.craft_grid.iter().cloned());
            v.push(g.carried.clone());
            v
        };
        let matching: Vec<bool> = all(self)
            .iter()
            .map(|s| s.as_ref().is_some_and(|s| self.stack_matches(predicate, s)))
            .collect();
        let mut slots: Vec<&mut Option<ItemStack>> = Vec::new();
        slots.extend(self.inventory.iter_mut());
        slots.extend(self.armor.iter_mut().rev());
        slots.push(&mut self.offhand);
        slots.extend(self.craft_grid.iter_mut());
        slots.push(&mut self.carried);
        for (slot, matches) in slots.into_iter().zip(matching) {
            if !matches {
                continue;
            }
            let Some(stack) = slot.as_mut() else { continue };
            let take = if counting {
                stack.count as i32
            } else {
                (stack.count as i32).min(budget)
            };
            removed += take;
            if !counting {
                budget -= take;
                stack.count -= take as u32;
                if stack.count == 0 {
                    *slot = None;
                }
                if budget == 0 {
                    break;
                }
            }
        }
        removed
    }
    /// `LocateCommand.locatePoi`: `PoiManager.findClosestWithType` within
    /// 256 blocks of the source, over the loaded chunks.
    fn cmd_locate_poi(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(poi)) = ctx.get("poi").cloned() else {
            return Ok(0);
        };
        let wanted: Vec<String> = match poi.strip_prefix('#') {
            Some(tag) => self.tag_entries("point_of_interest_type", tag),
            None => vec![poi.clone()],
        };
        let from = self.source_position().map(|v| v.floor() as i32);
        const RANGE: i32 = 256;
        let mut cache: std::collections::HashMap<u32, Option<&str>> =
            std::collections::HashMap::new();
        let mut best: Option<(i64, [i32; 3], &str)> = None;
        let (cx0, cz0) = ((from[0] - RANGE) >> 4, (from[2] - RANGE) >> 4);
        let (cx1, cz1) = ((from[0] + RANGE) >> 4, (from[2] + RANGE) >> 4);
        for cx in cx0..=cx1 {
            for cz in cz0..=cz1 {
                let Some(chunk) = self.world.chunks.get(&(cx, cz)) else {
                    continue;
                };
                for (i, section) in chunk.sections.iter().enumerate() {
                    let mut wanted_state = |state: u32| {
                        cache
                            .entry(state)
                            .or_insert_with(|| {
                                self.poi_type(sunrise_world::registry::BlockStateId(state))
                                    .filter(|t| wanted.iter().any(|w| w == t))
                            })
                            .is_some()
                    };
                    if !section.blocks.maybe_has(&mut wanted_state) {
                        continue;
                    }
                    let base_y = self.world.bottom_y() + 16 * i as i32;
                    for j in 0..4096 {
                        let state = section.get(j);
                        let Some(kind) = cache.get(&state).copied().flatten() else {
                            continue;
                        };
                        let p = [
                            cx * 16 + (j & 15) as i32,
                            base_y + (j >> 8) as i32,
                            cz * 16 + ((j >> 4) & 15) as i32,
                        ];
                        let d = [p[0] - from[0], p[1] - from[1], p[2] - from[2]]
                            .map(|d| i64::from(d) * i64::from(d))
                            .iter()
                            .sum::<i64>();
                        if d <= i64::from(RANGE * RANGE) && best.is_none_or(|(b, _, _)| d < b) {
                            best = Some((d, p, kind));
                        }
                    }
                }
            }
        }
        let Some((_, p, kind)) = best else {
            return Err(CommandError::new(Text::tr(
                "commands.locate.poi.not_found",
                &[&poi],
            )));
        };
        let link = Text::teleport_link(p[0], None, p[2]);
        let (dx, dz) = (p[0] - from[0], p[2] - from[2]);
        let distance = ((dx * dx + dz * dz) as f32).sqrt().floor() as i32;
        let shown = if poi.starts_with('#') {
            format!("{poi} ({kind})")
        } else {
            poi
        };
        self.feedback(Text::translate(
            "commands.locate.poi.success",
            &[
                Text::literal(shown),
                link,
                Text::literal(distance.to_string()),
            ],
        ));
        Ok(distance)
    }
    fn cmd_locate(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let origin = self.source_position();
        if id == "locate.biome" {
            let Some(Arg::Resource(biome)) = ctx.get("biome").cloned() else {
                return Ok(0);
            };
            // LocateCommand.locateBiome: BiomeSource.findClosestBiome3d with
            // a 6400-block radius, 32-block columns and 64-block layers.
            let from = origin.map(|v| v.floor() as i32);
            let allowed = |name: &str| self.biome_matches(&biome, name);
            let search = sunrise_world::worldgen::generator::BiomeSearch {
                allowed: &allowed,
                origin: from,
                radius: 6400,
                horizontal: 32,
                vertical: 64,
                min_y: self.world.bottom_y(),
                max_y: self.world.top_y() - 1,
            };
            let found = self
                .end
                .locator()
                .and_then(|t| t.find_closest_biome(&search));
            let Some((name, p)) = found else {
                return Err(CommandError::new(Text::tr(
                    "commands.locate.biome.not_found",
                    &[&biome],
                )));
            };
            let d2 = [p[0] - from[0], p[1] - from[1], p[2] - from[2]]
                .map(|d| f64::from(d) * f64::from(d))
                .iter()
                .sum::<f64>();
            let shown = if biome.starts_with('#') {
                format!("{biome} ({name})")
            } else {
                biome.clone()
            };
            let link = Text::teleport_link(p[0], Some(p[1]), p[2]);
            let distance = (d2 as f32).sqrt().floor() as i32;
            self.feedback(Text::translate(
                "commands.locate.biome.success",
                &[
                    Text::literal(shown),
                    link,
                    Text::literal(distance.to_string()),
                ],
            ));
            return Ok(distance);
        }
        let Some(Arg::Resource(s)) = ctx.get("structure").cloned() else {
            return Ok(0);
        };
        let names = self.structure_names(&s);
        let from = origin.map(|v| v.floor() as i32);
        let found = self.end.locator().and_then(|t| t.locate(&names, from, 100));
        let Some((name, p)) = found else {
            return Err(CommandError::new(Text::tr(
                "commands.locate.structure.not_found",
                &[&s],
            )));
        };
        let link = Text::teleport_link(p[0], None, p[2]);
        let (dx, dz) = (f64::from(p[0] - from[0]), f64::from(p[2] - from[2]));
        let distance = (dx * dx + dz * dz).sqrt().floor() as i32;
        let shown = if s.starts_with('#') {
            format!("{s} ({name})")
        } else {
            name
        };
        self.feedback(Text::translate(
            "commands.locate.structure.success",
            &[
                Text::literal(shown),
                link,
                Text::literal(distance.to_string()),
            ],
        ));
        Ok(distance)
    }
    /// The structures `LocateCommand.locateStructure` searches for: an id, or
    /// a `#tag`'s members.
    pub(super) fn structure_names(&self, structure: &str) -> Vec<String> {
        match structure.strip_prefix('#') {
            Some(tag) => self.structure_tag(tag),
            None => vec![structure.to_owned()],
        }
    }
    /// The structures of a `worldgen/structure` tag, nested tags expanded.
    pub(super) fn structure_tag(&self, tag: &str) -> Vec<String> {
        self.tag_entries("worldgen/structure", tag)
    }
    /// The entries of a data pack tag of `registry`, nested tags expanded.
    pub(super) fn tag_entries(&self, registry: &str, tag: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut pending = vec![tag.to_owned()];
        let mut seen = std::collections::HashSet::new();
        while let Some(t) = pending.pop() {
            if !seen.insert(t.clone()) {
                continue;
            }
            let (ns, path) = t.split_once(':').unwrap_or(("minecraft", &t));
            let file = self
                .options
                .resources
                .join(format!("data/{ns}/tags/{registry}/{path}.json"));
            // The game's file and the world's packs' both count, entries
            // joined in resource-stack order; a pack file with `replace`
            // drops what the lower sources put in (`TagLoader.load`).
            let packed = self
                .fns
                .store
                .files
                .get(&format!("tags/{ns}:{registry}/{path}"))
                .into_iter()
                .flatten()
                .filter_map(|b| serde_json::from_slice::<serde_json::Value>(b).ok());
            let parsed = std::fs::read(file)
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .into_iter()
                .chain(packed);
            let mut entries = Vec::new();
            for v in parsed {
                if v["replace"].as_bool() == Some(true) {
                    entries.clear();
                }
                entries.extend(v["values"].as_array().into_iter().flatten().cloned());
            }
            for e in &entries {
                let id = e.as_str().or_else(|| e["id"].as_str()).unwrap_or_default();
                match id.strip_prefix('#') {
                    Some(nested) => pending.push(nested.to_owned()),
                    None if !id.is_empty() => out.push(id.to_owned()),
                    None => {}
                }
            }
        }
        out
    }
    /// ListPlayersCommand: every online player in join order (a single-player
    /// owner is one of them), with the count and the max.
    fn cmd_list(&mut self, ctx: &Context) -> R<i32> {
        let players: Vec<(u128, String)> = self
            .lan
            .iter()
            .flat_map(|h| h.hub.online_players())
            .map(|g| (g.uuid, g.name.clone()))
            .collect();
        let max = self
            .lan
            .as_ref()
            .map_or(sunrise_client::lan::MAX_PLAYERS, |h| {
                h.hub.lan.shared.max_players()
            });
        let uuids = ctx.has("uuids");
        let mut names = Text::empty();
        for (k, (uuid, name)) in players.iter().enumerate() {
            if k > 0 {
                // ComponentUtils.formatList: gray commas.
                names.push(Text::colored(", ", text::GRAY));
            }
            if uuids {
                names.push(Text::translate(
                    "commands.list.nameAndId",
                    &[
                        Text::literal(name.as_str()),
                        Text::literal(sunrise_protocol::crypto::uuid_string(*uuid)),
                    ],
                ));
            } else {
                names.push(self.player_text(name, *uuid));
            }
        }
        let count = players.len();
        self.feedback_unbroadcast(Text::translate(
            "commands.list.players",
            &[
                Text::literal(count.to_string()),
                Text::literal(max.to_string()),
                names,
            ],
        ));
        Ok(count as i32)
    }
    /// `SayCommand.register`: resolve the source name and broadcast unsigned announcements.
    fn cmd_say(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let raw = match ctx.get(if id == "me" { "action" } else { "message" }) {
            Some(Arg::Str(s)) => s.clone(),
            _ => String::new(),
        };
        match id {
            "say" | "me" => {
                let message = self.resolve_chat_message(&raw);
                let name = self.source_display_name();
                let sender = match self.source_entity() {
                    Some(Target::Player) => self.lan_acting_uuid.unwrap_or_else(|| self.own_uuid()),
                    Some(Target::Guest(conn)) => {
                        self.command_guest(conn).map_or(0, |guest| guest.uuid)
                    }
                    _ => 0,
                };
                if !self.lan_broadcast_command_chat(sender, &name, id, &message, None) {
                    return Err(err("command.failed", &[]));
                }
                Ok(1)
            }
            _ => {
                let message = self.expand_selectors(&raw);
                let name = self.source_display_name();
                let targets = self.entities(ctx, "targets", true)?;
                let count = targets.len() as i32;
                // ChatType.bind throws for a missing type before anything is sent.
                let incoming_type = match &self.lan {
                    Some(host) => {
                        let (Some(incoming), Some(_)) = (
                            host.hub.chat_type_id("msg_command_incoming"),
                            host.hub.chat_type_id("msg_command_outgoing"),
                        ) else {
                            return Err(err("command.failed", &[]));
                        };
                        Some(incoming)
                    }
                    None => None,
                };
                for target in targets {
                    let target_name = match target {
                        Target::Player => self.player_display_name(),
                        other => self.target_name(other),
                    };
                    let out = Text::translate(
                        "commands.message.display.outgoing",
                        &[target_name, message.clone()],
                    )
                    .with_color(text::GRAY)
                    .italic();
                    // MsgCommand.sendMessage -> CommandSourceStack.sendChatMessage:
                    // a non-player source receives the decorated outgoing line via
                    // sendSystemMessage; a player source receives it as its own message.
                    self.command_system_message(out);
                    let incoming = Text::translate(
                        "commands.message.display.incoming",
                        &[name.clone(), message.clone()],
                    )
                    .with_color(text::GRAY)
                    .italic();
                    let conn = match target {
                        Target::Guest(conn) => Some(conn),
                        Target::Player => self.lan_acting,
                        _ => None,
                    };
                    if let Some(conn) = conn {
                        if self.lan_acting == Some(conn)
                            && let Some(capture) = self.chat_capture.as_mut()
                        {
                            capture.push(incoming);
                        } else if let Some(host) = &self.lan
                            && let Some(t) = host.hub.guests.connection_index(conn)
                        {
                            // The target's copy is private chat with the incoming whisper
                            // type, shown only to players who show chat (`Hub::whisper`).
                            let bound = sunrise_protocol::packets::play::ChatTypeBound {
                                chat_type: incoming_type
                                    .expect("a hub's incoming type was checked above")
                                    .network_id() as i32,
                                name: sunrise_client::lan::data::component_nbt(&name.component()),
                                target: None,
                            };
                            host.hub.send_disguised(
                                t,
                                sunrise_client::lan::data::component_nbt(&message.component()),
                                bound,
                            );
                        }
                    } else {
                        if let Some(capture) = self.chat_capture.as_mut() {
                            capture.push(incoming.clone());
                        }
                        let focused = self.screen == Screen::Chat;
                        self.ui.chat.add(
                            &self.gui.atlas,
                            incoming,
                            None,
                            sunrise_client::chat::ChatSource::Player,
                            self.visual_ticks(),
                            focused,
                        );
                    }
                }
                Ok(count)
            }
        }
    }
    /// `MessageArgument.resolveChatMessage`: only sources with entity selector permission
    /// resolve selectors. Other players' text remains literal.
    pub(super) fn resolve_chat_message(&mut self, raw: &str) -> Text {
        if self.ui.registries.permission >= 2 {
            self.expand_selectors(raw)
        } else {
            Text::literal(raw)
        }
    }

    /// MessageArgument: `@p`-style selectors in a message become names.
    pub(super) fn expand_selectors(&mut self, raw: &str) -> Text {
        // `MessageArgument.Message.resolveComponent`: no selector part leaves the whole text as one
        // literal; otherwise the text before the first part is the literal root and each part and
        // each non-empty stretch after it is appended as a sibling.
        let mut out: Option<Text> = None;
        let mut pending = String::new();
        let mut rest = raw;
        while let Some(at) = rest.find('@') {
            pending.push_str(&rest[..at]);
            let mut reader = Reader::new(&rest[at..]);
            match (sunrise_client::command::ArgType::Entity {
                single: false,
                players: false,
            })
            .parse(&mut reader, &self.ui.registries)
            {
                Ok(Arg::Entity(s)) if reader.cursor > 1 => {
                    let found = self.select(&s);
                    let text =
                        out.get_or_insert_with(|| Text::literal(std::mem::take(&mut pending)));
                    if !pending.is_empty() {
                        text.push(std::mem::take(&mut pending));
                    }
                    for (i, t) in found.iter().enumerate() {
                        if i > 0 {
                            text.push(", ");
                        }
                        text.push(self.target_name(*t));
                    }
                    rest = &rest[at + reader.cursor..];
                }
                _ => {
                    pending.push('@');
                    rest = &rest[at + 1..];
                }
            }
        }
        pending.push_str(rest);
        match out {
            None => Text::literal(pending),
            Some(mut text) => {
                if !pending.is_empty() {
                    text.push(pending);
                }
                text
            }
        }
    }
}

/// Split at a separator outside brackets, braces and quotes.
fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut escaped, mut start) = (0i32, None::<char>, false, 0);
    for (i, ch) in s.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// BlockPredicateArgument test, against the registry and the block tags:
/// a `#tag` is looked up by its path in `tags[0]`, then by its whole id in
/// `tags[1]` (as `Game::block_matches` reads them).
pub(super) fn block_predicate_matches_state<'p>(
    registry: &sunrise_world::registry::BlockRegistry,
    tags: [&std::collections::HashMap<String, std::collections::HashSet<String>>; 2],
    id: &str,
    tag: bool,
    mut properties: impl Iterator<Item = (&'p str, &'p str)>,
    state: BlockStateId,
) -> bool {
    let Some(s) = registry.state(state) else {
        return false;
    };
    let name_ok = if tag {
        let path = id.trim_start_matches("minecraft:");
        tags[0]
            .get(path)
            .or_else(|| tags[1].get(id))
            .is_some_and(|set| {
                set.contains(&s.name) || set.contains(s.name.trim_start_matches("minecraft:"))
            })
    } else {
        s.name == id
    };
    name_ok && properties.all(|(k, v)| s.properties.get(k).map(String::as_str) == Some(v))
}

/// The names `native_difficulty_changed` hears for a change from `from` to
/// `to`: the four difficulties of `options.difficulty.*`, the index kept in
/// range as the command and the options keep it.
fn difficulty_names(from: u8, to: u8) -> (&'static str, &'static str) {
    (
        DIFFICULTIES[usize::from(from) & 3],
        DIFFICULTIES[usize::from(to) & 3],
    )
}

#[cfg(test)]
mod clone_writes_tests;

#[cfg(test)]
mod air_filter_tests;
#[cfg(test)]
mod teleport_pitch_tests;

#[cfg(test)]
mod difficulty_tests {
    use super::difficulty_names;

    #[test]
    fn a_difficulty_change_names_both_ends() {
        assert_eq!(difficulty_names(0, 3), ("peaceful", "hard"));
        assert_eq!(difficulty_names(2, 1), ("normal", "easy"));
        // The four difficulties are the only indexes the command and the
        // options keep; an index outside them still names one.
        assert_eq!(difficulty_names(7, 4), ("hard", "peaceful"));
    }
}

#[cfg(test)]
#[path = "summon_item_orb_tests.rs"]
pub(in crate::game) mod summon_item_orb_tests;

#[cfg(test)]
#[path = "summon_bolt_tests.rs"]
pub(in crate::game) mod summon_bolt_tests;

#[cfg(test)]
pub(in crate::game) mod peaceful_tests {
    use super::{Arg, Context, Game};
    use crate::game::{execution_role::ExecutionRole, native_entity_refs::tests as fixture};
    use std::sync::{Arc, OnceLock};
    use sunrise_client::{
        chunk::Chunk,
        items::ItemStack,
        mod_content::{self, ContentMod},
    };
    use sunrise_command::ParsedArg;
    use sunrise_core::nbt::Tag;
    use sunrise_mod_package::{Content, Package, enabled::EnabledMods};

    pub(in crate::game) const TRADER: &str = "peaceful_test:trader";

    pub(in crate::game) fn trader_egg() -> ItemStack {
        let mut stack = ItemStack::new("minecraft:cow_spawn_egg", 1);
        stack.data_mut().set_other(
            "minecraft:entity_data",
            Some(Tag::compound([("id", Tag::String(TRADER.into()))])),
        );
        stack
    }

    pub(in crate::game) fn with_types(role: ExecutionRole, test: impl FnOnce(&mut Game<'_>)) {
        // All cases use the same data-only fixture. Build its overlay once; never edit the base.
        static PACKAGE: OnceLock<(tempfile::TempDir, std::path::PathBuf)> = OnceLock::new();
        let (_, root) = PACKAGE.get_or_init(|| {
            let scratch = tempfile::tempdir().unwrap();
            let path = scratch.path().join("peaceful.smod");
            let manifest = r#"id = "peaceful_test"
name = "Peaceful test"
version = "1.0"
api = "0.2"
side = "both"
capabilities = ["registries"]
"#;
            let rows: [(&str, &[u8]); 3] = [
                (
                    "data/peaceful_test/sunrise/entity_type/trader.json",
                    br#"{"template":"minecraft:wandering_trader","allowed_in_peaceful":false}"#,
                ),
                (
                    "data/peaceful_test/sunrise/entity_type/zombie.json",
                    br#"{"template":"minecraft:zombie","allowed_in_peaceful":true}"#,
                ),
                (
                    "data/peaceful_test/sunrise/entity_type/inherited_zombie.json",
                    br#"{"template":"minecraft:zombie"}"#,
                ),
            ];
            std::fs::write(&path, Package::write_files(manifest, &rows).unwrap()).unwrap();
            let package = Package::read(&path).unwrap();
            let mods = [ContentMod {
                id: "peaceful_test".into(),
                content: Content::read(&path, &package.manifest).unwrap(),
                jars: Vec::new(),
            }];
            let inventory = EnabledMods::new([("peaceful_test".into(), mod_api::Side::Both)]);
            let root = mod_content::resource_root(
                &fixture::fixture_root(),
                scratch.path(),
                &mods,
                scratch.path(),
                &inventory,
            )
            .unwrap();
            (scratch, root)
        });
        mod_platform::testing::with_mods(Arc::from([]), || {
            fixture::with_game_role(root.clone(), role, |game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.build_registries();
                game.set_difficulty_forced(0);
                test(game);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_persisted_mod_type_despawns_when_difficulty_switches_to_peaceful() {
        with_types(ExecutionRole::IntegratedServer, |game| {
            // Restoring a saved entity still works in Peaceful. Its own
            // despawn check, not the save loader, enforces the type's rule.
            let saved = Tag::compound([
                ("id", Tag::String(TRADER.into())),
                (
                    "Pos",
                    Tag::List(
                        [4.5_f64, 64.0, 4.5]
                            .map(|v| Tag::Double(v.to_bits()))
                            .to_vec(),
                    ),
                ),
                ("PersistenceRequired", Tag::Byte(1)),
                ("NoAI", Tag::Byte(1)),
            ]);
            let id = game.load_mob(saved, None).unwrap();
            let trader = game.mobs.iter().find(|mob| mob.id == id).unwrap();
            assert!(trader.persistent && trader.no_ai);
            game.tickets
                .update(Some(sunrise_client::ticket_levels::Sources {
                    forced: vec![(0, 0)],
                    ..Default::default()
                }));

            game.set_difficulty_forced(2);
            game.tick_creature_entities().unwrap();
            assert!(game.mobs.iter().any(|mob| mob.id == id));
            game.set_difficulty_forced(0);
            game.tick_creature_entities().unwrap();
            assert!(!game.mobs.iter().any(|mob| mob.id == id));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn summon_checks_the_mod_type_instead_of_its_template() {
        with_types(ExecutionRole::IntegratedServer, |game| {
            let context = |id: &str, nbt: Option<&str>| {
                let arg = |name: &str, value| ParsedArg {
                    name: name.into(),
                    value,
                    range: (0, 0),
                };
                let mut args = vec![arg("entity", Arg::Resource(id.into()))];
                if let Some(nbt) = nbt {
                    args.push(arg("nbt", Arg::Str(nbt.into())));
                }
                Context {
                    root: 0,
                    nodes: vec![],
                    args,
                    command: None,
                    executor: None,
                    range: (0, 0),
                    child: None,
                    literals: vec![],
                }
            };
            for id in [TRADER, "peaceful_test:inherited_zombie", "minecraft:zombie"] {
                for nbt in [None, Some("{NoAI:1b}")] {
                    let error = game.cmd_summon(&context(id, nbt)).unwrap_err();
                    assert_eq!(
                        error.key, "commands.summon.failed.peaceful",
                        "{id}, NBT {nbt:?}"
                    );
                }
            }
            assert!(game.mobs.is_empty());
            assert_eq!(
                game.cmd_summon(&context("peaceful_test:zombie", Some("{UUID:[I;1,2,3,4]}"))),
                Ok(1)
            );
            assert!(
                game.mobs
                    .iter()
                    .any(|mob| mob.kind.id == "peaceful_test:zombie")
            );
            for uuid in ["", ",UUID:[I;1,2,3,4]"] {
                // The already loaded zombie owns the second rider's UUID.
                // The skipped rider's own permitted rider must not spawn either.
                let nbt = format!(
                    "{{Passengers:[{{id:\"{TRADER}\"{uuid},Passengers:[{{id:\"minecraft:pig\"}}]}}]}}"
                );
                let before = game.mobs.len();
                assert_eq!(
                    game.cmd_summon(&context("minecraft:cow", Some(&nbt))),
                    Ok(1)
                );
                assert_eq!(game.mobs.len(), before + 1);
                let cow = &game.mobs[before];
                assert_eq!(cow.kind.id, "minecraft:cow");
                assert!(cow.extra.rider.is_none());
            }
        });
    }
}

#[cfg(test)]
mod summon_tests {
    #[test]
    fn a_summoned_entity_is_named_from_its_own_namespace() {
        use super::entity_name_key;
        assert_eq!(
            entity_name_key("minecraft:snowball"),
            "entity.minecraft.snowball"
        );
        assert_eq!(entity_name_key("ruby:bolt"), "entity.ruby.bolt");
        assert_eq!(entity_name_key("ruby:spell/bolt"), "entity.ruby.spell.bolt");
    }
}

#[cfg(test)]
mod look_at_tests {
    use super::Target;
    use super::targets::non_player_entity_id;

    #[test]
    fn look_at_entity_form_names_any_entity_by_its_id() {
        for t in [
            Target::Mob(7),
            Target::Item(7),
            Target::Orb(7),
            Target::Projectile(7),
            Target::Boat(7),
            Target::Cart(7),
            Target::Crystal(7),
            Target::Dragon(7),
            Target::Tnt(7),
            Target::Cushion(7),
            Target::Falling(7),
            Target::Rocket(7),
            Target::Trident(7),
            Target::Spawner(7),
            Target::Cloud(7),
            Target::Bolt(7),
        ] {
            assert_eq!(non_player_entity_id(t), Some(7), "{t:?}");
        }
        // Players are the hub's to name; a dragon part's id is no entity id.
        assert_eq!(non_player_entity_id(Target::Player), None);
        assert_eq!(non_player_entity_id(Target::Guest(3)), None);
        assert_eq!(non_player_entity_id(Target::Mob(u32::MAX - 4096)), None);
    }
}

#[cfg(test)]
mod percent_f_tests {
    use super::percent_f;

    #[test]
    fn rounds_the_shortest_decimal_half_up_like_java() {
        // WorldBorderCommand: "%.1f" distance, "%.2f" buffer, "%.0f" size.
        assert_eq!(percent_f(10.35, 1), "10.4");
        assert_eq!(percent_f(5.125, 2), "5.13");
        assert_eq!(percent_f(10.5, 0), "11");
        assert_eq!(percent_f(2.5, 0), "3");
        // TeleportCommand.formatDouble: "%f".
        assert_eq!(percent_f(0.1234565, 6), "0.123457");
        assert_eq!(percent_f(-1.005, 2), "-1.01");
        assert_eq!(percent_f(3.0, 2), "3.00");
    }
}

#[cfg(test)]
#[path = "teleport_mob_tests.rs"]
mod teleport_mob_tests;

#[cfg(test)]
#[path = "teleport_item_tests.rs"]
pub(in crate::game) mod teleport_item_tests;

#[cfg(test)]
#[path = "teleport_level_tests.rs"]
pub(in crate::game) mod teleport_level_tests;

#[cfg(test)]
#[path = "teleport_player_rider_tests.rs"]
mod teleport_player_rider_tests;

#[cfg(test)]
mod selector_bounds_tests {
    use super::*;
    use crate::game::{Audio, Options, Progress, Settings, load_resources};
    use sunrise_client::{entities::ArmorStand, mobs::Mob};
    use sunrise_core::geometry::Vec3;

    #[test]
    #[ignore = "daily"]
    fn actual_selector_uses_body_overlap_but_distance_stays_at_the_origin() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            eprintln!("Prepared resources unavailable; Game selector coverage did not run");
            return;
        }
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        game.mobs
            .push(Mob::new(30, kind, Vec3::new(10.5, 220.0, 10.5), 0.0));
        let target = Target::Mob(30);
        let mut selector = Selector {
            kind: 'e',
            include_entities: true,
            limit: usize::MAX,
            delta: [Some(0.0); 3],
            ..Default::default()
        };
        let origin = [9.5, 220.0, 10.5];
        assert_eq!(
            game.select_in_level(vec![target], &selector, origin),
            vec![(target, 1.0)]
        );
        selector.distance = Some((None, Some(0.9)));
        assert!(
            game.select_in_level(vec![target], &selector, origin)
                .is_empty()
        );
        selector.distance = None;
        assert!(
            game.select_in_level(vec![target], &selector, [11.0, 220.0, 10.5])
                .is_empty()
        );
        selector.delta = [Some(-1.0), None, None];
        assert_eq!(
            game.select_in_level(vec![target], &selector, [11.5, 220.0, 10.5])
                .len(),
            1
        );
        game.armor_stands
            .push(ArmorStand::new(31, Vec3::new(30.5, 220.0, 10.5), 0.0));
        selector.delta = [Some(0.0); 3];
        for (x, count) in [(29.5, 1), (29.25, 0), (30.75, 0), (29.250000000000004, 1)] {
            assert_eq!(
                game.select_in_level(vec![Target::Stand(31)], &selector, [x, 220.0, 10.5])
                    .len(),
                count
            );
        }
    }
}

#[cfg(test)]
#[path = "local_coordinate_command_tests.rs"]
mod local_coordinate_command_tests;

#[cfg(test)]
mod query_feedback_tests {
    #[test]
    #[ignore = "daily"]
    fn gamerule_feedback_uses_short_ids_without_changing_registry_identity() {
        use super::{Arg, Context};
        use crate::game::native_entity_refs::tests::with_game;
        use std::sync::Arc;
        use sunrise_client::{
            command::ParsedArg,
            gamerules::{GameRules, Rule, RuleRegistry, Value},
        };
        with_game(|game| {
            let registry = RuleRegistry::with_mod_rules(
                &["test:feedback".into()],
                &[Rule {
                    name: "test:spawn_monsters".into(),
                    category: "test:feedback".into(),
                    default: Value::Bool(true),
                    min: 0,
                    max: 0,
                }],
            )
            .unwrap();
            game.ui.rules = GameRules::with_registry(Arc::new(registry), &[]);
            for (input, display, stored) in [
                (
                    "spawn_monsters",
                    "spawn_monsters",
                    "minecraft:spawn_monsters",
                ),
                (
                    "minecraft:spawn_monsters",
                    "spawn_monsters",
                    "minecraft:spawn_monsters",
                ),
                (
                    "test:spawn_monsters",
                    "test:spawn_monsters",
                    "test:spawn_monsters",
                ),
            ] {
                game.ui.rules.set(stored, Value::Bool(true));
                let mut context = Context {
                    root: 0,
                    nodes: Vec::new(),
                    args: Vec::new(),
                    command: None,
                    executor: None,
                    range: (0, 0),
                    child: None,
                    literals: vec!["gamerule".into(), input.into()],
                };
                game.chat_capture = Some(Vec::new());
                assert_eq!(game.cmd_gamerule(&context).unwrap(), 1);
                let lines = game.chat_capture.take().unwrap();
                assert_eq!(lines.len(), 1);
                let component = lines[0].component();
                assert_eq!(component["translate"], "commands.gamerule.query");
                assert_eq!(component["with"][0]["text"], display);
                context.args.push(ParsedArg {
                    name: "value".into(),
                    value: Arg::Bool(false),
                    range: (0, 0),
                });
                game.chat_capture = Some(Vec::new());
                assert_eq!(game.cmd_gamerule(&context).unwrap(), 0);
                let lines = game.chat_capture.take().unwrap();
                assert_eq!(lines.len(), 1);
                let component = lines[0].component();
                assert_eq!(component["translate"], "commands.gamerule.set");
                assert_eq!(component["with"][0]["text"], display);
                let error = game.cmd_gamerule(&context).unwrap_err();
                assert_eq!(error.key, "commands.gamerule.not_set");
                assert_eq!(error.message.component()["with"][0]["text"], display);
                assert_eq!(game.ui.rules.get(stored), Some(Value::Bool(false)));
                assert_eq!(game.ui.rules.rule(input).unwrap().name, stored);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_silent_source_drops_query_results_but_still_stores_them() {
        use crate::game::{
            Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        assert!(game.run_command("time set 1234"));
        assert!(game.run_command("scoreboard objectives add q dummy"));
        let queries = [
            "time query time",
            "time query day",
            "time query day repetition",
            "time query gametime",
            "seed",
            "gamerule keep_inventory",
            "difficulty",
            "list",
            "help seed",
        ];
        // A function's source (silent, like `run_command_quietly`) is told nothing.
        for command in queries {
            game.chat_capture = Some(Vec::new());
            assert!(game.run_command_quietly(command), "{command}");
            let lines = game.chat_capture.take().unwrap();
            assert!(lines.is_empty(), "{command}: {:?}", lines[0].plain());
        }
        // The player's own source still sees each result.
        for command in queries {
            game.chat_capture = Some(Vec::new());
            assert!(game.run_command(command), "{command}");
            assert_eq!(game.chat_capture.take().unwrap().len(), 1, "{command}");
        }
        // The result still reaches `execute store`.
        assert!(game.run_command_quietly("execute store result score #t q run time query day"));
        game.chat_capture = Some(Vec::new());
        assert!(game.run_command("scoreboard players get #t q"));
        let lines = game.chat_capture.take().unwrap();
        assert!(lines[0].plain().contains("1234"), "{:?}", lines[0].plain());
    }
}

#[cfg(test)]
mod forced_respawn_tests {
    #[test]
    #[ignore = "daily"]
    fn setworldspawn_reports_normalized_rotation() {
        use crate::game::native_entity_refs::tests::with_game;
        use std::sync::Arc;
        use sunrise_client::{chunk::Chunk, motion::PlayerMotion, text::Text};
        use sunrise_core::geometry::Vec3;

        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(5.5, 64.0, 5.5)).unwrap());
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.chat_capture = Some(Vec::new());

            assert!(game.run_command("setworldspawn 5 64 5 270 120"));
            assert_eq!(game.level.world_spawn_angle, [-90.0, 90.0]);
            let expected = Text::translate(
                "commands.setworldspawn.success",
                &["5", "64", "5", "-90.0", "90.0", "minecraft:overworld"].map(Text::literal),
            );
            let lines = game.chat_capture.take().unwrap();
            assert_eq!(lines.len(), 1);
            assert_eq!(lines[0].plain(), expected.plain());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn overworld_spawnpoint_is_validated_at_both_player_blocks() {
        use crate::game::{native_guest_turn_tests::with_guests, respawn_anchor::RespawnConfig};
        use sunrise_client::chunk::Chunk;

        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_guests(|game| {
                game.build_registries();
                for x in -2..=2 {
                    for z in -2..=2 {
                        game.streaming
                            .insert_remote(&mut game.world, Chunk::empty(x, z));
                    }
                }
                game.level.world_spawn = Some([0, 80, 0]);
                game.level.world_spawn_angle = [-30.0, -5.0];
                let registry = game.pack.assets.registry;
                let air = registry.default_state("minecraft:air").unwrap();
                // Liquid is a block property: aquatic plants such as kelp still permit
                // a forced point, while a bubble column does not, at feet or head.
                for (block, allowed) in [
                    ("air", true),
                    ("water", false),
                    ("lava", false),
                    ("bubble_column", false),
                    ("kelp", true),
                    ("stone", false),
                ] {
                    let state = registry
                        .default_state(&format!("minecraft:{block}"))
                        .unwrap();
                    for height in [0, 1] {
                        game.world.set(5, 64, 5, air).unwrap();
                        game.world.set(5, 65, 5, air).unwrap();
                        game.world.set(5, 64 + height, 5, state).unwrap();
                        assert!(
                            game.as_console(|g| g.run_command("spawnpoint guest12 5 64 5 45 10"))
                        );
                        let config = game
                            .command_guest(12)
                            .unwrap()
                            .respawn_elsewhere
                            .as_ref()
                            .and_then(RespawnConfig::from_saved)
                            .unwrap();
                        assert!(config.forced);
                        assert_eq!(config.pos, [5, 64, 5]);
                        game.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                            g.dead = true;
                            g.health = 0.0;
                        });
                        game.guest_respawn(12);
                        let guest = game.command_guest(12).unwrap();
                        assert!(!guest.dead, "{block} at height {height}");
                        assert_eq!(
                            guest.respawn_elsewhere.is_some(),
                            allowed,
                            "{block} at height {height}"
                        );
                        assert_eq!(game.level.world_spawn, Some([0, 80, 0]));
                        if allowed {
                            assert_eq!(guest.position, [5.5, 64.1, 5.5]);
                            assert_eq!([guest.yaw, guest.pitch], [45.0, 10.0]);
                        } else {
                            // The missing point must not supply the fallback's facing.
                            assert_eq!([guest.yaw, guest.pitch], [-30.0, -5.0]);
                            assert_eq!(game.level.world_spawn_angle, [-30.0, -5.0]);
                        }
                    }
                }
            });
        });
    }
}

#[cfg(test)]
mod effective_presence_tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    #[test]
    #[ignore = "daily"]
    fn registered_presence_uses_actual_catalog_and_stack_patch() {
        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            let key = "minecraft:jukebox_playable";
            assert!(
                game.items()
                    .get("minecraft:music_disc_cat")
                    .unwrap()
                    .components
                    .iter()
                    .any(|k| k == key)
            );
            assert!(
                !game
                    .items()
                    .get("minecraft:stone")
                    .unwrap()
                    .components
                    .iter()
                    .any(|k| k == key)
            );
            // Supply the synthetic chunk before forceload's synchronous load barrier.
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            assert!(game.run_command("forceload add 0 0 15 15"));
            assert!(game.world.loaded(4, 4));
            assert!(game.run_command("setblock 4 199 4 stone"));
            assert!(game.run_command("setblock 4 200 4 chest{Items:[{Slot:0b,id:\"minecraft:music_disc_cat\",count:1},{Slot:1b,id:\"minecraft:stone\",count:1},{Slot:2b,id:\"minecraft:stone\",count:1,components:{\"minecraft:jukebox_playable\":\"minecraft:cat\",\"minecraft:custom_data\":{h193:1}}},{Slot:3b,id:\"minecraft:music_disc_cat\",count:1,components:{\"!minecraft:jukebox_playable\":{},\"minecraft:custom_data\":{h193:1}}}]}"));
            assert!(game.run_command("scoreboard objectives add h193 dummy"));
            let clock = game.world.game_time;
            let sunrise_client::containers::BlockEntity::Chest(stacks) =
                game.world.block_entities.get(&(4, 200, 4)).unwrap()
            else {
                panic!("actual chest missing")
            };
            let stacks = stacks.clone();
            assert_eq!(stacks.len(), 27);
            assert!(
                stacks[0].as_ref().unwrap().extra.is_none(),
                "plain modeled stack has no patch"
            );
            assert!(
                stacks[2]
                    .as_ref()
                    .unwrap()
                    .extra
                    .as_ref()
                    .unwrap()
                    .other(key)
                    .is_some()
            );
            assert!(
                stacks[3]
                    .as_ref()
                    .unwrap()
                    .extra
                    .as_ref()
                    .unwrap()
                    .other("!minecraft:jukebox_playable")
                    .is_some()
            );
            let saved = game.block_entity_nbt([4, 200, 4]).unwrap();
            let mut failures = Vec::new();
            for (slot, present) in [true, false, true, false].into_iter().enumerate() {
                let stack = stacks[slot].as_ref().unwrap();
                assert_eq!(stack.count, 1);
                for neg in [false, true] {
                    let expected = present != neg;
                    let term = format!("{}minecraft:jukebox_playable", if neg { "!" } else { "" });
                    let predicate = ItemInput {
                        id: String::new(),
                        tag: false,
                        any: true,
                        components: Some(format!("[{term}]")),
                    };
                    if game.stack_matches(&predicate, stack) != expected {
                        failures.push(format!("direct slot{slot} {term}"));
                    }
                    for sign in ["if", "unless"] {
                        for (name, n) in [("r", 91), ("s", 17)] {
                            assert!(
                                game.run_command(&format!(
                                    "scoreboard players set #{name} h193 {n}"
                                ))
                            );
                        }
                        let accepted = game.run_command(&format!("execute store result score #r h193 store success score #s h193 {sign} items block 4 200 4 container.{slot} *[{term}]"));
                        let yes = if sign == "if" { expected } else { !expected };
                        let score = |name| game.score.board.score(name, "h193").unwrap().value;
                        if (accepted, score("#r"), score("#s"))
                            != (yes, i32::from(yes), i32::from(yes))
                        {
                            failures.push(format!("command slot{slot} {sign} {term}"));
                        }
                    }
                }
            }
            let plain = ItemStack::new("minecraft:music_disc_cat", 1);
            assert!(plain.extra.is_none());
            let p = ItemInput {
                id: String::new(),
                tag: false,
                any: true,
                components: Some("[minecraft:jukebox_playable]".into()),
            };
            if !game.stack_matches(&p, &plain) {
                failures.push("no-raw modeled prototype".into());
            }
            assert_eq!(
                game.block_entity_nbt([4, 200, 4]).unwrap(),
                saved,
                "read-only saved inventory"
            );
            assert_eq!(game.world.game_time, clock, "zero actor ticks");
            assert!(game.world.loaded(4, 4));
            assert!(
                failures.is_empty(),
                "actual effective component presence mismatches: {failures:?}"
            );
        });
    }
}

#[cfg(test)]
mod fill_note_instrument_tests {
    use super::*;
    use crate::game::{Audio, Options, Progress, Settings, load_resources};
    use std::sync::Arc;
    use sunrise_client::{chunk::Chunk, motion::PlayerMotion};
    use sunrise_core::position::ChunkPos;
    fn with_game(test: impl FnOnce(&mut Game<'_>)) {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join(format!("h164-{:?}", std::thread::current().id())),
        );
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("tick freeze"));
        assert!(game.run_command("fill 0 198 0 15 199 15 stone"));
        test(&mut game);
    }

    fn instrument(game: &Game<'_>, x: i32, z: i32) -> String {
        let state = game
            .pack
            .assets
            .registry
            .state(game.world.block(x, 200, z))
            .unwrap();
        assert_eq!(state.name, "minecraft:note_block");
        state.properties["instrument"].clone()
    }

    #[test]
    #[ignore = "daily"]
    fn ordinary_fill_normalizes_a_note_block_against_real_stone() {
        with_game(|game| {
            let clock = game.world.game_time;
            assert!(
                game.world.chunks.contains_key(&(0, 0)),
                "actual fill chunk is resident"
            );
            assert!(game.run_command("execute if block 4 199 4 stone"));
            assert!(game.run_command("execute if block 4 200 4 air"));
            assert!(game.run_command("setblock 8 200 4 note_block"));
            assert_eq!(
                instrument(game, 8, 4),
                "basedrum",
                "positive setblock counterpart"
            );
            assert!(game.run_command("fill 4 200 4 4 200 4 note_block"));
            assert_eq!(
                instrument(game, 4, 4),
                "basedrum",
                "actual registered fill must normalize instrument"
            );
            assert!(game.run_command("execute if block 4 200 4 note_block[note=0,powered=false]"));
            assert_eq!(game.world.game_time, clock);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_existing_default_state_does_not_skip_note_normalization() {
        with_game(|game| {
            let clock = game.world.game_time;
            assert!(game.run_command("setblock 4 200 4 note_block strict"));
            assert_eq!(instrument(game, 4, 4), "harp");
            assert!(
                game.run_command("fill 4 200 4 4 200 4 note_block"),
                "valid fill changes existing harp to basedrum"
            );
            assert_eq!(instrument(game, 4, 4), "basedrum");
            assert_eq!(game.world.game_time, clock);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn strict_explicit_and_filtered_out_inputs_keep_their_existing_contracts() {
        with_game(|game| {
            let clock = game.world.game_time;
            assert!(game.run_command("fill 4 200 4 4 200 4 note_block strict"));
            assert_eq!(instrument(game, 4, 4), "harp");
            assert!(game.run_command("fill 8 200 4 8 200 4 note_block[instrument=banjo,note=7]"));
            assert_eq!(instrument(game, 8, 4), "banjo");
            assert!(game.run_command("execute if block 8 200 4 note_block[note=7,powered=false]"));
            assert!(game.run_command("fill 12 200 4 12 200 4 note_block[instrument=harp,note=7]"));
            assert_eq!(instrument(game, 12, 4), "harp");
            assert!(!game.run_command("fill 4 200 4 4 200 4 note_block[instrument=banjo] keep"));
            assert!(
                !game
                    .run_command("fill 4 200 4 4 200 4 note_block[instrument=banjo] replace glass")
            );
            assert_eq!(instrument(game, 4, 4), "harp");
            assert_eq!(game.world.game_time, clock);
        });
    }
}

#[cfg(test)]
mod item_damage_range_tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use sunrise_core::position::BlockPos;

    #[test]
    #[ignore = "daily"]
    fn actual_chest_damage_range_uses_live_damage_and_effective_presence() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            game.build_registries();
            game.set_allow_commands(true);
            game.block_ticks
                .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("setblock 4 199 4 stone"));
            assert!(game.run_command(r#"setblock 4 200 4 chest{Items:[{Slot:0b,id:"minecraft:diamond_sword",count:1,components:{"minecraft:custom_data":{marker:195}}},{Slot:1b,id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":3,"minecraft:custom_data":{marker:195}}},{Slot:2b,id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":4,"minecraft:custom_data":{marker:195}}},{Slot:3b,id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5,"minecraft:custom_data":{marker:195}}},{Slot:4b,id:"minecraft:stone",count:1,components:{"minecraft:custom_data":{marker:195}}},{Slot:5b,id:"minecraft:diamond_sword",count:1,components:{"!minecraft:damage":{},"minecraft:custom_data":{marker:195}}}]}"#));
            assert!(game.run_command("scoreboard objectives add h195 dummy"));
            assert!(game.world.loaded(4, 4));
            assert!(game.run_command("execute if block 4 199 4 stone"));
            let clock = game.world.game_time;
            let stacks = match game.world.block_entities.get(&(4, 200, 4)).unwrap() {
                sunrise_client::containers::BlockEntity::Chest(slots) => slots.clone(),
                _ => panic!("actual loaded chest missing"),
            };
            for (slot, damage) in [0, 3, 4, 5, 0, 0].into_iter().enumerate() {
                let stack = stacks[slot].as_ref().unwrap();
                assert_eq!(stack.damage, damage);
                assert_eq!(stack.count, 1);
                assert_eq!(
                    &*stack.item,
                    if slot == 4 {
                        "minecraft:stone"
                    } else {
                        "minecraft:diamond_sword"
                    }
                );
                assert_eq!(
                    stack.custom_data().unwrap().get(&"marker".into()),
                    Some(&sunrise_core::nbt::Tag::Int(195))
                );
                assert!(game.run_command(&format!(
                    "execute if items block 4 200 4 container.{slot} {}",
                    stack.item
                )));
            }
            assert!(stacks[6].is_none());
            assert!(
                game.items()
                    .get("minecraft:diamond_sword")
                    .unwrap()
                    .components
                    .iter()
                    .any(|c| c == "minecraft:damage")
            );
            assert!(
                !game
                    .items()
                    .get("minecraft:stone")
                    .unwrap()
                    .components
                    .iter()
                    .any(|c| c == "minecraft:damage")
            );
            let removed =
                sunrise_client::item_data::component_snapshot(stacks[5].as_ref().unwrap()).unwrap();
            assert!(
                removed.get("!minecraft:damage").is_some(),
                "actual removal patch must survive load before evaluating it"
            );
            assert!(game.run_command(
                "execute unless data block 4 200 4 Items[{Slot:0b}].components.\"minecraft:damage\""
            ));
            assert!(game.run_command(
                "execute if data block 4 200 4 Items[{Slot:2b,components:{\"minecraft:damage\":4}}]"
            ));
            assert!(game.run_command(
                "execute if data block 4 200 4 Items[{Slot:5b}].components.\"!minecraft:damage\""
            ));
            let mut cases = Vec::new();
            for (slot, damage) in [(1, 3), (2, 4), (3, 5)] {
                for (shape, expected) in [
                    ("4", damage == 4),
                    ("{min:4}", damage >= 4),
                    ("{max:4}", damage <= 4),
                    ("{min:3,max:4}", (3..=4).contains(&damage)),
                ] {
                    cases.push((
                        slot,
                        format!("minecraft:damage~{{damage:{shape}}}"),
                        expected,
                    ));
                }
            }
            cases.extend([
                (0, "damage~{damage:0}".into(), true),
                (0, "damage~{damage:{min:1}}".into(), false),
                (4, "damage~{damage:0}".into(), false),
                (5, "damage~{damage:0}".into(), false),
                (2, "!damage~{damage:4}".into(), false),
                (1, "!damage~{damage:4}".into(), true),
                (1, "damage=3".into(), true),
                (1, "damage=4".into(), false),
                (1, "count~1".into(), true),
                (1, "custom_data~{marker:195}".into(), true),
            ]);
            let mut mismatches = Vec::new();
            for (slot, term, expected) in cases {
                let stack = stacks[slot].as_ref().unwrap();
                let predicate = ItemInput {
                    id: stack.item.to_string(),
                    tag: false,
                    any: false,
                    components: Some(format!("[{term}]")),
                };
                let actual = game.stack_matches(&predicate, stack);
                if actual != expected {
                    mismatches.push(format!(
                        "direct slot{slot} {term}: {actual}, expected{expected}"
                    ));
                }
                assert!(game.run_command("scoreboard players set #r h195 91"));
                assert!(game.run_command("scoreboard players set #s h195 17"));
                let command = format!(
                    "execute store result score #r h195 store success score #s h195 if items block 4 200 4 container.{slot} {}[{term}]",
                    stack.item
                );
                game.run_command(&command);
                let r = game.score.board.score("#r", "h195").map(|s| s.value);
                let s = game.score.board.score("#s", "h195").map(|s| s.value);
                if (r, s) != (Some(i32::from(expected)), Some(i32::from(expected))) {
                    mismatches.push(format!("command {term}: {r:?}/{s:?}, expected{expected}"));
                }
                assert!(game.run_command("scoreboard players set #branch h195 23"));
                game.run_command(&format!("execute unless items block 4 200 4 container.{slot} {}[{term}] run scoreboard players set #branch h195 7",stack.item));
                let branch = game.score.board.score("#branch", "h195").map(|s| s.value);
                if branch != Some(if expected { 23 } else { 7 }) {
                    mismatches.push(format!("unless {term}: {branch:?}"));
                }
            }
            assert!(!game.run_command(
                "execute if items block 4 200 4 container.4 diamond_sword[damage~{damage:4}]"
            ));
            assert!(!game.run_command(
                "execute if items block 4 200 4 container.6 diamond_sword[damage~{damage:4}]"
            ));
            let actual = match game.world.block_entities.get(&(4, 200, 4)).unwrap() {
                sunrise_client::containers::BlockEntity::Chest(slots) => slots,
                _ => panic!("source chest changed"),
            };
            assert_eq!(
                actual.as_slice(),
                stacks.as_slice(),
                "queries must leave the authoritative loaded stacks unchanged"
            );
            assert_eq!(game.world.game_time, clock);
            assert!(
                mismatches.is_empty(),
                "damage-only command predicate mismatches: {mismatches:#?}"
            );
        });
    }
}

#[cfg(test)]
#[path = "vehicle_kill_tests.rs"]
mod vehicle_kill_tests;

#[cfg(test)]
mod global_selector_order_tests {
    use super::{Selector, Target};
    use sunrise_client::command::args::Sort;
    use sunrise_client::join_seq::Joined;
    use sunrise_core::nbt::Tag;
    use sunrise_core::position::ChunkPos;

    #[test]
    #[ignore = "daily"]
    fn actual_registered_global_selector_keeps_interleaved_insertion_and_limits() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            game.build_registries();
            game.set_allow_commands(true);
            for x in 0..2 {
                for z in 0..2 {
                    game.world.chunks.insert(
                        (x, z),
                        std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(x, z)),
                    );
                    game.block_ticks
                        .add_empty_chunk(ChunkPos::new(x, z))
                        .unwrap();
                }
            }
            assert!(game.run_command("fill 0 199 0 31 199 31 stone"));
            assert!(game.run_command("scoreboard objectives add h268 dummy"));
            let clock = game.world.game_time;
            let actors = [
                ("pig", 41, [17.5, 200.0, 16.25]),
                ("armor_stand", 7, [15.75, 200.0, 15.5]),
                ("item", 93, [15.5, 200.0, 17.0]),
                ("pig", 2, [17.0, 200.0, 15.75]),
                ("armor_stand", 68, [18.5, 200.0, 16.5]),
                ("item", 11, [14.0, 200.0, 16.75]),
            ];
            let mut targets = Vec::new();
            let mut last_join = 0;
            for (i, (kind, uuid, pos)) in actors.iter().enumerate() {
                let extra = match *kind {
                    "pig" => ",NoAI:1b,Health:10f,PersistenceRequired:1b",
                    "armor_stand" => ",Health:20f",
                    _ => ",Item:{id:\"minecraft:stone\",count:1},PickupDelay:32767s",
                };
                assert!(game.run_command(&format!(
                "summon {kind} {:?} {:?} {:?} {{UUID:[I;268,0,0,{uuid}],Tags:[\"h268\"],NoGravity:1b,Motion:[0d,0d,0d]{extra}}}",
                pos[0], pos[1], pos[2]
            )));
                let target = game
                    .target_by_uuid((268u128 << 96) | *uuid as u128)
                    .unwrap();
                assert_eq!(game.target_type(target), format!("minecraft:{kind}"));
                assert_eq!(game.target_position(target), Some(*pos));
                assert!(game.world.loaded(pos[0] as i32, pos[2] as i32));
                let join = match target {
                    Target::Mob(id) => game.mobs.iter().find(|m| m.id == id).unwrap().join_seq(),
                    Target::Stand(id) => game
                        .armor_stands
                        .iter()
                        .find(|m| m.id == id)
                        .unwrap()
                        .join_seq(),
                    Target::Item(id) => {
                        game.dropped.iter().find(|m| m.id == id).unwrap().join_seq()
                    }
                    _ => panic!("actual admitted family"),
                };
                assert!(
                    join.is_joined() && join.get() > last_join,
                    "actual registered creation chronology {i}"
                );
                last_join = join.get();
                targets.push(target);
                assert!(game.run_command("scoreboard players set #r h268 91"));
                assert!(game.run_command(
                    "execute store result score #r h268 run execute if entity @e[tag=h268]"
                ));
                assert_eq!(
                    game.score.board.score("#r", "h268").unwrap().value,
                    i as i32 + 1
                );
            }
            let selector = Selector {
                kind: 'e',
                include_entities: true,
                limit: usize::MAX,
                tags: vec![("h268".into(), false)],
                ..Default::default()
            };
            // Existing sorted paths remain controls, with no equal-distance ties.
            let mut nearest = selector.clone();
            nearest.sort = Sort::Nearest;
            nearest.pos = [Some(16.0), Some(200.0), Some(16.0)];
            assert_eq!(
                game.select(&nearest),
                [1, 3, 2, 0, 5, 4].map(|i| targets[i])
            );
            nearest.sort = Sort::Furthest;
            assert_eq!(
                game.select(&nearest),
                [4, 5, 0, 2, 3, 1].map(|i| targets[i])
            );
            let mut spatial = selector.clone();
            spatial.distance = Some((None, Some(3.0)));
            spatial.pos = [Some(16.0), Some(200.0), Some(16.0)];
            let old_spatial = game.select(&spatial);
            let mut failures = Vec::new();
            for (name, options, expected) in [
                ("all", "sort=arbitrary", vec![41, 7, 93, 2, 68, 11]),
                ("two", "sort=arbitrary,limit=2", vec![41, 7]),
                (
                    "filtered",
                    "type=!minecraft:armor_stand,sort=arbitrary,limit=3",
                    vec![41, 93, 2],
                ),
            ] {
                assert!(game.run_command(&format!(
                    "data modify storage h268:proof {name} set value []"
                )));
                assert!(game.run_command("scoreboard players set #r h268 91"));
                assert!(game.run_command("scoreboard players set #s h268 17"));
                assert!(game.run_command(&format!("execute positioned 16.0 200.0 16.0 store result score #r h268 store success score #s h268 as @e[tag=h268,{options}] run data modify storage h268:proof {name} append from entity @s UUID")));
                assert_eq!(game.score.board.score("#r", "h268").unwrap().value, 1);
                assert_eq!(game.score.board.score("#s", "h268").unwrap().value, 1);
                let actual = game.score.storage.get("h268:proof").get(name).cloned();
                let wanted = Some(Tag::List(
                    expected
                        .iter()
                        .map(|v| Tag::IntArray(vec![268, 0, 0, *v]))
                        .collect(),
                ));
                if actual != wanted {
                    failures.push(format!("{name}: {actual:?} != {wanted:?}"));
                }
            }
            assert_eq!(
                game.select(&spatial),
                old_spatial,
                "unchanged spatial path within the scene"
            );
            assert_eq!(game.world.game_time, clock);
            for (target, (_, _, pos)) in targets.iter().zip(actors.iter()) {
                assert_eq!(game.target_position(*target), Some(*pos));
            }
            assert!(
                failures.is_empty(),
                "actual registered selector order/limits: {failures:#?}"
            );
        });
    }
}

#[cfg(test)]
#[path = "incoming_tests/knot_identity.rs"]
mod knot_identity_tests;

#[cfg(test)]
mod mob_display_name_tests;

#[cfg(test)]
#[path = "item_count_predicate_tests.rs"]
mod item_count_predicate_tests;
