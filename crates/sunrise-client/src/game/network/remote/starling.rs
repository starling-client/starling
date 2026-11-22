//! Starling's own part of the connection: how the stick moves the player on a server (Analog,
//! Keys, or Free Direction's steering), the hits and uses Free Direction holds back for a tick,
//! the log lines that explain a player a server keeps pulling back, and the creative screen's
//! crafting grid. Its state is one field of [`Remote`] (`starling`).
use super::*;
use sunrise_client::actions::Action;
use sunrise_client::gamepad::ServerStick;

/// A gap in the ticks or movement sent to the server worth a log line (an old server's
/// anticheat behind ViaVersion counts on one each tick, and a reminder every 20).
const GAP_LOGGED: Duration = Duration::from_millis(1100);
/// How many movement packets the first setback's log line shows.
const TRAIL: usize = 12;
/// Free stick direction rests this long after a hit, a use or a drop: those go with the view's
/// rotation, and turning away at once after a hit looks like an aimbot to an anticheat.
const STEER_REST: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(super) struct State {
    /// When the last tick ended to the server, and the last movement packet went: an old
    /// server behind ViaVersion (its anticheat) wants one every tick, so gaps are logged.
    tick_end_at: Option<Instant>,
    moved_at: Option<Instant>,
    /// When the play phase began (login or the end of a reconfiguration).
    pub(super) play_began: Option<Instant>,
    /// This second's movement packets and ticks ended, from when: a second over 20 of
    /// either is logged (an old server's anticheat counts them against the time passed).
    rate: (Option<Instant>, u32, u32),
    /// The last movement packets sent: position, on ground, input flags and when. The
    /// first setback logs them, the path an anticheat refused.
    trail: VecDeque<([f64; 3], bool, u8, Instant)>,
    /// Free stick direction (experimental, `stick_steer`): this tick's rotation the server sees
    /// in place of the view's, and when the player last hit, used or dropped something.
    pub(super) steer_yaw: Option<f32>,
    interacted_at: std::cell::Cell<Option<Instant>>,
    /// The look a teleport's acknowledgement told, for `PositionSender::told_rotation`.
    pub(super) told_rotation: std::cell::Cell<Option<(f32, f32)>>,
    /// Hits and uses waiting for a move with the view's look (`Game::send`).
    held_back: std::cell::RefCell<Vec<S>>,
    /// The stick's mode here this tick ([`ServerStick`]): this session's choice, else the one
    /// remembered for the server (read once), else the setting.
    stick_mode: ServerStick,
    stick_choice: Option<ServerStick>,
    stick_remembered: Option<Option<ServerStick>>,
    /// The setting as last seen here: changed while playing, it becomes the mode here.
    stick_setting: Option<ServerStick>,
    /// Setbacks while the stick moved the player, for asking once a session how it should.
    stick_setbacks: VecDeque<Instant>,
    stick_used_at: Option<Instant>,
    stick_asked: bool,
    /// Ticks the player has waited for the chunk under it (logged when long: the player
    /// cannot move meanwhile).
    waiting_ticks: u32,
    /// Server teleports of the player that took it back a short way (an anti-cheat's or the
    /// server's correction), logged so a player who "cannot walk" shows why.
    setbacks: u32,
    /// While a creative screen is open: the inventory menu's crafting grid (slots 1 to 4) as
    /// last sent or received, which its clicks send when they change.
    creative_grid: Option<[Option<ItemStack>; 4]>,
}

impl State {
    /// Count movement packets and ticks ended; a second with more than 20 of either is logged.
    fn count_rate(&mut self, moves: u32, ticks: u32) {
        let now = Instant::now();
        let (since, m, t) = &mut self.rate;
        let start = *since.get_or_insert(now);
        if now - start >= Duration::from_secs(1) {
            if *m > 21 || *t > 21 {
                log::info!(
                    "Sunrise: {m} movement packets and {t} ticks ended in {} ms",
                    (now - start).as_millis()
                );
            }
            *since = Some(now);
            *m = 0;
            *t = 0;
        }
        *m += moves;
        *t += ticks;
    }

    /// A tick ended to the server (ClientTickEnd went).
    pub(super) fn tick_ended(&mut self) {
        self.count_rate(0, 1);
        let now = Instant::now();
        if let Some(gap) = self.tick_end_at.replace(now).map(|at| now - at)
            && gap >= GAP_LOGGED
        {
            log::info!(
                "Sunrise: {} ms between two ticks ended to the server",
                gap.as_millis()
            );
        }
    }

    /// A movement packet went: counted, kept in the trail, and logged when it came late or was
    /// the first.
    pub(super) fn moved(&mut self, position: [f64; 3], on_ground: bool, input: u8) {
        self.count_rate(1, 0);
        if self.trail.len() == TRAIL {
            self.trail.pop_front();
        }
        let now = Instant::now();
        self.trail.push_back((position, on_ground, input, now));
        match self.moved_at.replace(now).map(|at| now - at) {
            Some(gap) if gap >= GAP_LOGGED => log::info!(
                "Sunrise: {} ms between two movement packets",
                gap.as_millis()
            ),
            Some(_) => {}
            None => log::info!(
                "Sunrise: the first movement packet {} ms after the play phase began",
                self.play_began.map_or(0, |at| (now - at).as_millis())
            ),
        }
    }

    /// The chunk under the player is lit or not this tick: a long wait is logged, with what the
    /// connection holds (`chunks`, `skipped` packets).
    pub(super) fn waited_for_chunk(
        &mut self,
        lit: bool,
        at: (i32, i32),
        chunks: usize,
        skipped: u32,
    ) {
        if lit {
            if self.waiting_ticks >= 100 {
                log::info!(
                    "Sunrise: the chunk under the player came after {} s",
                    self.waiting_ticks / 20
                );
            }
            self.waiting_ticks = 0;
            return;
        }
        self.waiting_ticks += 1;
        if self.waiting_ticks % 600 == 100 {
            log::warn!(
                "Sunrise: waiting {} s for the chunk under the player ({}, {}): {chunks} chunks \
                 here, {skipped} packets skipped",
                self.waiting_ticks / 20,
                at.0,
                at.1
            );
        }
    }

    /// A server teleport that took the player back `back` blocks (a correction): counted, and
    /// the first ones logged with what tells the causes apart.
    fn setback(
        &mut self,
        back: f64,
        to: [f64; 3],
        teleport: i32,
        last_sent: [f64; 3],
        state: &str,
    ) {
        self.setbacks += 1;
        if self.setbacks > 5 && self.setbacks % 50 != 0 {
            return;
        }
        let [sx, sy, sz] = last_sent;
        log::info!(
            "Sunrise: the server moved the player back {back:.2} blocks to {:.2} {:.2} {:.2} ({} \
             times; teleport {teleport}; last sent {sx:.2} {sy:.2} {sz:.2}, {} ms ago; {state})",
            to[0],
            to[1],
            to[2],
            self.setbacks,
            self.moved_at
                .map_or(-1, |at| at.elapsed().as_millis() as i64)
        );
        if self.setbacks == 1 || self.setbacks % 50 == 0 {
            let now = Instant::now();
            let trail: Vec<String> = self
                .trail
                .iter()
                .map(|([x, y, z], ground, input, at)| {
                    format!(
                        "-{}ms {x:.3} {y:.3} {z:.3} {}{input:#04x}",
                        (now - *at).as_millis(),
                        if *ground { "ground " } else { "air " }
                    )
                })
                .collect();
            log::info!("Sunrise: the moves before it: {}", trail.join(" | "));
        }
    }
}

/// A hit, a use, a block placed or broken, an item dropped: what Free Direction steering
/// rests after (`Game::send`).
fn is_interaction(packet: &S) -> bool {
    matches!(
        packet,
        S::PlayerAction(_) | S::Attack(_) | S::Interact(_) | S::UseItemOn(_) | S::UseItem(_)
    )
}

/// Whether Free Direction holds `packet` for the next tick (`Game::send`): an interaction
/// made while the last move steered (`steered`), and whatever comes after one held
/// (`queued`), in order. Digging (START_DESTROY_BLOCK to STOP_DESTROY_BLOCK, actions 0 to 3)
/// goes at once: this client has broken the block on its own level already, and this tick's
/// move may go into the space it left (flying up against the block just dug), which the
/// server and its anticheat take only once they have the break, as vanilla sends it before
/// the move. The server reads no look from it, and Grim's RotationBreak checks it against the
/// look of the move after it, the view's (the steering rests from it on).
fn waits_for_the_view(packet: &S, steered: bool, queued: bool) -> bool {
    let digging = matches!(packet, S::PlayerAction(a) if (0..=3).contains(&a.action));
    (is_interaction(packet) && !digging && steered) || queued
}

impl Game<'_> {
    /// Free Direction's part of `send`: whether `packet` waits for the next tick. A hit, a use,
    /// a block placed or an item dropped goes where the player looks: a server places stairs,
    /// throws and aims by the rotation it was last sent. While that is the stick's steering
    /// (last tick's move), it waits for the next tick, after this tick's move with the view's
    /// look (`stick_steer` rests after it): a look of its own would be a tick more to an
    /// anticheat (Grim's TickTimer), the move after it the same look twice (AimDuplicateLook).
    /// The swings after it wait too; a block being broken does not (`waits_for_the_view`).
    pub(super) fn held_back_for_the_view(&self, packet: S) -> Option<S> {
        let Some(r) = &self.remote else {
            return Some(packet);
        };
        let interaction = is_interaction(&packet);
        if !(interaction || matches!(packet, S::Punch(_)))
            || r.starling.stick_mode != ServerStick::Free
        {
            return Some(packet);
        }
        if interaction {
            r.starling.interacted_at.set(Some(Instant::now()));
        }
        let mut held = r.starling.held_back.borrow_mut();
        if waits_for_the_view(&packet, r.starling.steer_yaw.is_some(), !held.is_empty()) {
            held.push(packet);
            return None;
        }
        Some(packet)
    }

    /// The hits and uses held back while the stick steered (`send`), the tick after: first in
    /// it, before its move, as a hit one tick later would be (an anticheat's Post check wants no
    /// hit after the move of its tick).
    pub(super) fn send_held_back(&self) {
        if let Some(r) = &self.remote {
            for packet in r.starling.held_back.take() {
                self.send_play(packet);
            }
        }
    }

    /// The stick's mode on the server this tick, when on one.
    pub(in crate::game) fn remote_stick_mode(&self) -> Option<ServerStick> {
        self.remote.as_ref().map(|r| r.starling.stick_mode)
    }

    /// The stick's mode on this connection: Analog in single player and for a LAN host.
    pub(super) fn update_stick_mode(&mut self) -> ServerStick {
        let setting = self.settings.controller.server_stick;
        let dir = Self::options_path()
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        let own = self.integrated.is_some() || self.server_address().is_none();
        let Some(r) = self.remote.as_mut() else {
            return ServerStick::Analog;
        };
        if own {
            r.starling.stick_mode = ServerStick::Analog;
            return ServerStick::Analog;
        }
        let address = r.session.address.clone();
        let s = &mut r.starling;
        let mut remembered = *s
            .stick_remembered
            .get_or_insert_with(|| sunrise_client::stick_memory::get(&dir, &address));
        // The setting changed while playing here: it is this server's mode now, the one
        // remembered for it included.
        if s.stick_setting
            .replace(setting)
            .is_some_and(|seen| seen != setting)
        {
            s.stick_choice = Some(setting);
            if remembered.is_some() {
                sunrise_client::stick_memory::set(&dir, &address, setting);
                remembered = Some(setting);
                s.stick_remembered = Some(remembered);
            }
            log::info!(
                "Sunrise: the stick moves as {} on this server, from the setting",
                setting.name()
            );
        }
        s.stick_mode = s.stick_choice.or(remembered).unwrap_or(setting);
        s.stick_mode
    }

    /// The stick is moving the player this tick.
    pub(super) fn stick_used(&mut self) {
        if let Some(r) = self.remote.as_mut() {
            r.starling.stick_used_at = Some(Instant::now());
        }
    }

    /// A server teleport of the player from `from` to `to`: a short way back is a correction
    /// (the server would not take the move), logged, and counted against the stick's mode.
    pub(super) fn remote_setback(&mut self, from: [f64; 3], to: [f64; 3], teleport: i32) {
        let back = (0..3)
            .map(|i| (from[i] - to[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        if !(back > 0.0 && back < 8.0) {
            return;
        }
        let state = self.player.as_ref().map_or_else(String::new, |player| {
            format!(
                "sprinting {}, on ground {}, input {:#04x}",
                player.sprinting,
                player.ground.on_ground,
                self.remote.as_ref().map_or(0, |r| r.last_input)
            )
        });
        if let Some(r) = self.remote.as_mut() {
            let last_sent = r.position_sender.last_sent();
            r.starling.setback(back, to, teleport, last_sent, &state);
        }
        self.stick_setback();
    }

    /// A setback while the stick moved the player: four in six seconds, in a mode the server
    /// may refuse, and the player is asked once a session how the stick should move here.
    fn stick_setback(&mut self) {
        let now = Instant::now();
        let ask = self.settings.controller.stick_prompt;
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let s = &mut r.starling;
        if !s
            .stick_used_at
            .is_some_and(|at| now - at < Duration::from_secs(1))
        {
            return;
        }
        s.stick_setbacks.push_back(now);
        while s
            .stick_setbacks
            .front()
            .is_some_and(|at| now - *at > Duration::from_secs(6))
        {
            s.stick_setbacks.pop_front();
        }
        if s.stick_setbacks.len() < 4
            || s.stick_asked
            || !ask
            || s.stick_mode == ServerStick::Keys
            || s.stick_choice.is_some()
            || s.stick_remembered.flatten().is_some()
        {
            return;
        }
        s.stick_asked = true;
        log::info!("Sunrise: the server keeps pulling the stick's moves back; asking the player");
        self.ask_stick_mode();
    }

    /// The question: which mode, remembered for this server or not, asked again or not.
    fn ask_stick_mode(&mut self) {
        let t = |key: &str| Tag::String(sunrise_client::lang::get_or(key, key).as_str().into());
        let choice = |key: &str, mode: ServerStick| {
            Tag::compound([
                ("label", t(key)),
                ("width", Tag::Int(300)),
                (
                    "action",
                    Tag::compound([
                        ("type", Tag::String("minecraft:dynamic/custom".into())),
                        ("id", Tag::String("starling:server_stick".into())),
                        (
                            "additions",
                            Tag::compound([("mode", Tag::String(mode.name().into()))]),
                        ),
                    ]),
                ),
            ])
        };
        let toggle = |key: &str, label: &str, initial: bool| {
            Tag::compound([
                ("type", Tag::String("minecraft:boolean".into())),
                ("key", Tag::String(key.into())),
                ("label", t(label)),
                ("initial", Tag::Byte(i8::from(initial))),
            ])
        };
        let dialog = Tag::compound([
            ("type", Tag::String("minecraft:multi_action".into())),
            ("title", t("starling.stick_prompt.title")),
            ("pause", Tag::Byte(0)),
            ("columns", Tag::Int(1)),
            (
                "body",
                Tag::compound([
                    ("type", Tag::String("minecraft:plain_message".into())),
                    ("contents", t("starling.stick_prompt.body")),
                    ("width", Tag::Int(300)),
                ]),
            ),
            (
                "inputs",
                Tag::List(vec![
                    toggle("remember", "starling.stick_prompt.remember", true),
                    toggle("dont_ask", "starling.stick_prompt.dont_ask", false),
                ]),
            ),
            (
                "actions",
                Tag::List(vec![
                    choice("starling.stick_prompt.free", ServerStick::Free),
                    choice("starling.stick_prompt.keys", ServerStick::Keys),
                    choice("starling.stick_prompt.analog", ServerStick::Analog),
                ]),
            ),
        ]);
        self.show_local_dialog(&dialog);
    }

    /// A click in one of the game's own dialogs (`starling:` actions).
    pub(in crate::game) fn starling_dialog_click(&mut self, id: &str, payload: Option<&Tag>) {
        // The link confirmation's Open (`confirm_link`).
        if id == "starling:open_link" {
            if let Some(Tag::String(url)) = payload.and_then(|p| p.get("url")) {
                self.open_link(&url.to_string_lossy());
            }
            return;
        }
        if id != "starling:server_stick" {
            log::warn!("Sunrise: no such dialog action {id}");
            return;
        }
        let field = |key: &str| payload.and_then(|p| p.get(key));
        let mode = match field("mode") {
            Some(Tag::String(s)) => ServerStick::parse(&s.to_string_lossy()),
            _ => return,
        };
        let on = |key: &str| matches!(field(key), Some(Tag::Byte(b)) if *b != 0);
        let path = Self::options_path();
        let dir = path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        if let Some(r) = self.remote.as_mut() {
            r.starling.stick_choice = Some(mode);
            if on("remember") {
                sunrise_client::stick_memory::set(&dir, &r.session.address, mode);
                r.starling.stick_remembered = Some(Some(mode));
            }
        }
        if on("dont_ask") {
            self.settings.controller.stick_prompt = false;
            self.settings.save(&path);
        }
        log::info!(
            "Sunrise: the stick moves as {} on this server{}",
            mode.name(),
            if on("remember") { ", remembered" } else { "" }
        );
    }

    /// Free stick direction (the controller setting, experimental): on a server the stick
    /// moves where it points by turning the rotation the server sees toward it and holding
    /// forward, a move the keys make. Not with keys held, nor while mining, using, riding or
    /// gliding, nor right after a hit or a use, which go with the view's rotation.
    pub(super) fn stick_steer(&self, stick: Option<[f32; 2]>) -> Option<f32> {
        let r = self.remote.as_ref()?;
        let [left, forward] = stick?;
        let keys = [Action::Forward, Action::Left, Action::Back, Action::Right];
        let p = self.player.as_ref()?;
        if left.hypot(forward) <= 0.3
            || keys.iter().any(|k| self.keys.held(*k))
            || self.mining.is_some()
            || self.using.is_some()
            || self.riding.is_some()
            || p.fall_flying
            || r.starling
                .interacted_at
                .get()
                .is_some_and(|at| at.elapsed() < STEER_REST)
        {
            return None;
        }
        // Moving forward at yaw - a is moving at angle a (to the left) of yaw: Entity.moveRelative.
        Some(self.yaw - left.atan2(forward).to_degrees())
    }

    /// CreativeModeInventoryScreen's slot listener for the crafting grid, which the inventory's
    /// sync leaves out (Bedrock Inventory puts a 2x2 grid on the creative screen): each slot a
    /// creative screen's clicks changed is sent as SetCreativeModeSlot, while one is open and
    /// once more as it closes.
    pub(super) fn remote_creative_grid_sync(&mut self) {
        let open = self.creative && self.screen == Screen::Creative;
        let now: [Option<ItemStack>; 4] =
            std::array::from_fn(|i| self.craft_grid.get(i).cloned().flatten());
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let Some(before) = r.starling.creative_grid.as_mut() else {
            if open {
                r.starling.creative_grid = Some(now);
            }
            return;
        };
        let changed: Vec<usize> = (0..4).filter(|&i| before[i] != now[i]).collect();
        *before = now.clone();
        if !open {
            r.starling.creative_grid = None;
        }
        for i in changed {
            let item = self.to_network(&now[i]);
            self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
                slot: 1 + i as i16,
                item,
            }));
        }
    }

    /// The grid as the server last described it (a packet set the player's slots).
    pub(super) fn remote_creative_grid_synced(&mut self) {
        let now: [Option<ItemStack>; 4] =
            std::array::from_fn(|i| self.craft_grid.get(i).cloned().flatten());
        if let Some(r) = self.remote.as_mut()
            && let Some(grid) = r.starling.creative_grid.as_mut()
        {
            *grid = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Empty, S, play, waits_for_the_view};

    #[test]
    fn free_direction_sends_a_break_before_the_move_that_goes_into_its_space() {
        let action = |action| {
            S::PlayerAction(play::PlayerAction {
                action,
                pos: [0, 62, 0],
                direction: 0,
                sequence: 1,
            })
        };
        // Start, change direction, abort and stop go at once right after a steered move.
        for dig in 0..=3 {
            assert!(
                !waits_for_the_view(&action(dig), true, false),
                "action {dig}"
            );
        }
        // A drop (5, DROP_ITEM) and a use wait for a move with the view's look.
        let used = S::UseItem(play::UseItem {
            hand: 0,
            sequence: 2,
            yaw: 0.0,
            pitch: 0.0,
        });
        assert!(waits_for_the_view(&action(5), true, false));
        assert!(waits_for_the_view(&used, true, false));
        assert!(!waits_for_the_view(&used, false, false));
        // After something held everything waits, in order; a swing alone never does.
        assert!(waits_for_the_view(&action(0), true, true));
        assert!(waits_for_the_view(&S::Punch(Empty), false, true));
        assert!(!waits_for_the_view(&S::Punch(Empty), true, false));
    }
}
