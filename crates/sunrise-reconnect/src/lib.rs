//! Auto-reconnect (`autoReconnect`, docs/extras.md): after a lost multiplayer connection the
//! Disconnected screen counts down to a new connection to the same server, with backoff, and never
//! after a kick, a ban or a refusal ([`policy`]).
//!
//! A Sunrise crate (docs/layers.md): the game tells it about the window's connection through the
//! platform's hooks ([`HOOKS`]: `connecting`, `joined`, `left`, `ended`) and asks it whether to
//! connect again (`retry`); the engine draws the screen and makes the connection. The official
//! executable (`sunrise-official`) registers it with its switch: its line of `sunrise-extras.txt`
//! ([`SETTING`]) and its row in Extras' Multiplayer group ([`SECTION`]). sunrise-client's own
//! executable never reconnects by itself.
#![forbid(unsafe_code)]
mod policy;

use std::sync::Mutex;
use std::time::Instant;

use sunrise_platform::hooks::connection::{By, Ended, Left, Retry, Server};
use sunrise_platform::hooks::{self, Hooks};
use sunrise_platform::pages::{Act, Label, Page, Place, Row};
use sunrise_platform::settings::Store;

/// The setting's localized label and tooltip, registered only with the crate.
pub static LANG: &sunrise_platform::lang::Files = include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// Its hooks, for `sunrise_platform::hooks::register`.
pub static HOOKS: Reconnect = Reconnect;

pub struct Reconnect;

impl Hooks for Reconnect {
    fn connecting(&self, server: &Server) {
        with_seat(|seat| seat.connecting(server));
    }

    fn joined(&self) {
        with_seat(|seat| seat.joined(&SystemClock));
    }

    fn left(&self, _how: Left) {
        with_seat(Seat::left);
    }

    /// The next attempt, while the switch is on and the reason allows one.
    fn ended(&self, end: &Ended<'_>) {
        with_seat(|seat| seat.ended(end, &SystemClock));
    }

    /// The countdown to the next attempt, then the attempt.
    fn retry(&self) -> Option<Retry> {
        with_seat(|seat| seat.retry(&SystemClock))
    }
}

/// Reads the current instant: the game's clock, or a test's fixed one.
trait Clock {
    fn now(&self) -> Instant;
}

/// The real clock.
struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// One splitscreen player's auto-reconnect (`hooks::seat`): the switch, the attempts at their
/// server and the next one.
#[derive(Default)]
struct Seat {
    on: bool,
    tracker: policy::Tracker,
    planned: Option<policy::Retry>,
}

impl Seat {
    /// A connection starts: a new address counts anew and nothing is planned.
    fn connecting(&mut self, server: &Server) {
        self.planned = None;
        self.tracker
            .connecting(&server.address, &server.name, server.lan);
    }

    fn joined(&mut self, clock: &dyn Clock) {
        self.tracker.joined(clock.now());
    }

    /// The player left on purpose, or cancelled: nothing to retry.
    fn left(&mut self) {
        self.planned = None;
        self.tracker.reset();
    }

    /// Plans the next attempt while the switch is on and the reason allows one.
    fn ended(&mut self, end: &Ended<'_>, clock: &dyn Clock) {
        self.planned = if self.on && end.by != By::Client {
            self.tracker
                .ended(end.reason, end.key, end.by == By::Server, clock.now())
        } else {
            self.tracker.reset();
            None
        };
    }

    /// The countdown to the next attempt, then the attempt.
    fn retry(&self, clock: &dyn Clock) -> Option<Retry> {
        let planned = self.planned.as_ref()?;
        let now = clock.now();
        Some(if planned.due(now) {
            Retry::Now(Server::new(
                planned.address.clone(),
                planned.name.clone(),
                planned.lan,
            ))
        } else {
            Retry::Wait(planned.text(now))
        })
    }
}

static SEATS: Mutex<Vec<Seat>> = Mutex::new(Vec::new());

/// Runs `f` on the active seat's state, growing the list as seats appear.
fn with_seat<T>(f: impl FnOnce(&mut Seat) -> T) -> T {
    let seat = hooks::seat();
    let mut seats = SEATS.lock().unwrap_or_else(|e| e.into_inner());
    if seats.len() <= seat {
        seats.resize_with(seat + 1, Seat::default);
    }
    f(&mut seats[seat])
}

/// The switch's line in `sunrise-extras.txt`, where the engine's Extras kept it before.
const KEY: &str = "autoReconnect";
/// The switch's row, and what it says on hover.
const LABEL: &str = "sunrise.extras.auto_reconnect";
const ABOUT: &str = "sunrise.extras.auto_reconnect.tooltip";

fn on() -> bool {
    with_seat(|seat| seat.on)
}

fn set(on: bool) {
    with_seat(|seat| seat.on = on);
}

/// The switch's line of `sunrise-extras.txt`, for `sunrise_platform::settings::register`.
pub static SETTING: Setting = Setting;

pub struct Setting;

impl Store for Setting {
    fn file(&self) -> &'static str {
        "sunrise-extras.txt"
    }
    fn owns(&self, key: &str) -> bool {
        key == KEY
    }
    fn load(&self, _key: &str, value: &str) {
        match value {
            "true" => set(true),
            "false" => set(false),
            _ => {}
        }
    }
    fn save(&self, line: &mut dyn FnMut(&str, &str)) {
        line(KEY, if on() { "true" } else { "false" });
    }
    fn is_default(&self) -> bool {
        !on()
    }
}

/// The switch's row in the Extras page's Multiplayer group, for
/// `sunrise_platform::pages::register`.
pub static SECTION: Section = Section;

pub struct Section;

impl Page for Section {
    fn id(&self) -> &'static str {
        "sunrise-reconnect.extras"
    }
    fn title(&self) -> &'static str {
        "sunrise.extras.header.multiplayer"
    }
    fn place(&self) -> Place {
        Place::Extras
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::toggle(LABEL, on()));
    }
    fn act(&self, row: usize, act: Act) {
        if row == 0 && matches!(act, Act::Press | Act::PressBack) {
            set(!on());
        }
    }
    fn tooltip(&self, _row: usize) -> Option<Label> {
        Some(Label::Key(ABOUT))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A fixed instant, so the orchestration's countdown is reached without sleeping.
    struct FixedClock(Instant);

    impl Clock for FixedClock {
        fn now(&self) -> Instant {
            self.0
        }
    }

    /// The only test that touches the statics: off, a lost connection plans nothing; on, the
    /// screen counts down to the same server; leaving forgets it.
    #[test]
    fn the_switch_decides_and_its_line_round_trips() {
        assert!(SETTING.is_default());
        let server = Server::new("example.org", "Tester", false);
        let lost = Ended::new("Server closed", None, By::Server);
        HOOKS.connecting(&server);
        HOOKS.ended(&lost);
        assert_eq!(HOOKS.retry(), None);
        SECTION.act(0, Act::Press);
        let mut lines = Vec::new();
        SETTING.save(&mut |key, value| lines.push(format!("{key}:{value}")));
        assert_eq!(lines, ["autoReconnect:true"]);
        HOOKS.connecting(&server);
        HOOKS.ended(&lost);
        let Some(Retry::Wait(line)) = HOOKS.retry() else {
            panic!("no countdown");
        };
        assert!(line.ends_with("(attempt 1 of 5)"), "{line}");

        HOOKS.left(Left::Disconnected);
        assert_eq!(HOOKS.retry(), None);
        SETTING.load(KEY, "false");
        assert!(SETTING.is_default());
    }

    #[test]
    fn leaving_one_seat_keeps_the_other_seats_retry() {
        let clock = FixedClock(Instant::now());
        let first_server = Server::new("example.org", "First", false);
        let second_server = Server::new("second.example", "Second", false);
        let lost = Ended::new("Server closed", None, By::Server);
        let mut first = Seat {
            on: true,
            ..Seat::default()
        };
        let mut second = Seat::default();
        first.connecting(&first_server);
        first.ended(&lost, &clock);
        second.connecting(&second_server);
        second.ended(&lost, &clock);
        assert_eq!(second.retry(&clock), None);

        second.on = true;
        second.connecting(&second_server);
        second.ended(&lost, &clock);
        let due = FixedClock(clock.0 + Duration::from_secs(5));
        assert_eq!(second.retry(&due), Some(Retry::Now(second_server)));
        second.left();
        assert_eq!(second.retry(&due), None);
        assert_eq!(first.retry(&due), Some(Retry::Now(first_server)));
    }

    #[test]
    fn a_due_countdown_reconnects_to_the_same_server() {
        let start = Instant::now();
        let clock = FixedClock(start);
        let server = Server::new("example.org", "Tester", true);
        let mut seat = Seat {
            on: true,
            ..Seat::default()
        };
        seat.connecting(&server);
        seat.ended(&Ended::new("Server closed", None, By::Server), &clock);
        assert_eq!(
            seat.retry(&clock),
            Some(Retry::Wait("Reconnecting in 5 s (attempt 1 of 5)".into()))
        );
        let due = start + Duration::from_secs(5);
        assert_eq!(
            seat.retry(&FixedClock(due - Duration::from_nanos(1))),
            Some(Retry::Wait("Reconnecting in 1 s (attempt 1 of 5)".into()))
        );
        assert_eq!(seat.retry(&FixedClock(due)), Some(Retry::Now(server)));
    }

    #[test]
    fn a_client_side_end_never_reconnects_even_with_the_switch_on() {
        let now = Instant::now();
        let clock = FixedClock(now);
        let mut seat = Seat {
            on: true,
            ..Seat::default()
        };
        seat.connecting(&Server::new("example.org", "Tester", false));
        seat.ended(
            &Ended::new("Required resource pack declined", None, By::Client),
            &clock,
        );
        assert_eq!(seat.retry(&clock), None);
    }
}
