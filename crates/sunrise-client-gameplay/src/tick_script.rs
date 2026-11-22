//! `SUNRISE_TICK_SCRIPT=PATH` (tools/test_wire_stream_parity.py): a server's client plays a tick-indexed
//! script, the one the vanilla client plays in `VanillaTickScript.java`, so the two clients' serverbound
//! streams can be compared tick by tick. The harness writes the scenario to PATH (JSON `{"name", "ticks",
//! "steps": {"TICK": [LINE...]}}`, LINE a sunrise-ctl line: `key w down`, `mouse left click`, `look 90 0`,
//! `click-slot 3`, `close`), then creates PATH with the extension `go`. From the next client tick the script
//! plays: its first tick sends `/wire-mark`, which marks it in the capture, then each tick's lines act
//! before the tick runs, as a player's input in the frame before it; when its ticks have run, the client
//! writes the scenario's name to PATH with the extension `done`.
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Default)]
pub struct TickScript {
    /// PATH, when the variable is set.
    path: Option<PathBuf>,
    running: Option<Running>,
}

pub struct Running {
    name: String,
    ticks: u32,
    steps: BTreeMap<u32, Vec<String>>,
    tick: u32,
}

impl TickScript {
    pub fn from_env() -> Self {
        Self {
            path: std::env::var_os("SUNRISE_TICK_SCRIPT").map(PathBuf::from),
            running: None,
        }
    }
    /// Whether a scenario is playing.
    pub fn playing(&self) -> bool {
        self.running.is_some()
    }
    /// Whether the variable is set (the frame asks before every tick).
    pub fn armed(&self) -> bool {
        self.path.is_some()
    }
    /// PATH, when the variable is set.
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }
    pub fn start(&mut self, script: Running) {
        self.running = Some(script);
    }
    /// This tick's lines, and the scenario's name once its ticks have run; `None` when nothing plays.
    pub fn advance(&mut self) -> Option<(Vec<String>, Option<String>)> {
        let running = self.running.as_mut()?;
        let tick = running.tick;
        running.tick += 1;
        let lines = running.steps.remove(&tick).unwrap_or_default();
        let finished = (running.tick > running.ticks).then(|| running.name.clone());
        Some((lines, finished))
    }
    pub fn finish(&mut self) {
        self.running = None;
    }
}

/// The scenario at `text`: its name, its length in ticks and each tick's lines.
pub fn parse(text: &str) -> Option<Running> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut steps = BTreeMap::new();
    for (tick, lines) in json.get("steps")?.as_object()? {
        let lines = lines
            .as_array()?
            .iter()
            .filter_map(|l| l.as_str().map(str::to_owned))
            .collect();
        steps.insert(tick.parse().ok()?, lines);
    }
    Some(Running {
        name: json.get("name")?.as_str()?.to_owned(),
        ticks: u32::try_from(json.get("ticks")?.as_u64()?).ok()?,
        steps,
        tick: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scenario_names_its_ticks_and_their_lines() {
        let s = parse(r#"{"name":"walk","ticks":40,"steps":{"0":["key w down"],"20":["key w up","look 90 0"]}}"#)
            .unwrap();
        assert_eq!((s.name.as_str(), s.ticks), ("walk", 40));
        assert_eq!(s.steps[&20], ["key w up", "look 90 0"]);
        assert!(parse(r#"{"name":"x","ticks":1,"steps":{"a":[]}}"#).is_none());
    }
}
