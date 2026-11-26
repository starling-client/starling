//! The command registries the game keeps in step with what the world's content and data
//! packs define, outside the one-time build in `chat.rs`.
use super::{Game, chat::TimeMarker};
use std::collections::BTreeMap;

/// Extracts every timeline's `(clock, period)` and every clock's time markers from the world's
/// `minecraft:timeline` table (`ServerClockManager.init`).
pub(super) fn timelines_and_markers(
    table: &sunrise_client::environment::TimelineTable,
) -> (
    BTreeMap<String, (String, Option<i64>)>,
    BTreeMap<(String, String), TimeMarker>,
) {
    let mut timelines = BTreeMap::new();
    let mut markers = BTreeMap::new();
    for (id, timeline) in &table.timelines {
        let period = timeline.period.map(i64::from);
        timelines.insert(id.clone(), (timeline.clock.clone(), period));
        for (marker_id, info) in &timeline.time_markers {
            markers.insert(
                (timeline.clock.clone(), marker_id.clone()),
                TimeMarker {
                    ticks: i64::from(info.ticks),
                    period,
                    show: info.show_in_commands,
                },
            );
        }
    }
    (timelines, markers)
}

impl Game<'_> {
    /// Active `minecraft:timeline` and `minecraft:world_clock` snapshot for `/time` commands
    /// and `wake_time` (`ServerClockManager.init`, `TimeCommand`).
    /// A client without the world's tables (the window of a single-player or multiplayer
    /// session) takes the server's synchronized `minecraft:timeline` and `minecraft:world_clock`
    /// entries.
    pub(in crate::game) fn sync_timeline_registries(&mut self) {
        let synced;
        let table = match (&self.world_tables, &self.remote) {
            (Some(tables), _) => &tables.timelines,
            (None, Some(remote)) => {
                synced = remote.synced_timelines();
                &synced
            }
            (None, None) => {
                synced = Default::default();
                &synced
            }
        };
        let (timelines, markers) = timelines_and_markers(table);
        self.ui.timelines = timelines;
        self.ui.markers = markers;
        let reg = &mut self.ui.registries;
        let timeline_ids: Vec<String> = self.ui.timelines.keys().cloned().collect();
        reg.timelines.clone_from(&timeline_ids);
        reg.by_registry
            .entry("minecraft:timeline".into())
            .or_default()
            .ids = timeline_ids;
        let mut command_markers: Vec<String> = self
            .ui
            .markers
            .iter()
            .filter(|(_, m)| m.show)
            .map(|((_, marker), _)| marker.clone())
            .collect();
        command_markers.sort();
        command_markers.dedup();
        reg.time_markers = command_markers;
        let mut clocks: Vec<String> = if let Some(tables) = &self.world_tables {
            tables
                .packed_registries
                .entries()
                .filter(|(registry, _, admitted)| *registry == "minecraft:world_clock" && *admitted)
                .map(|(_, id, _)| id)
                .collect()
        } else if let Some(remote) = &self.remote {
            remote
                .registries
                .registries
                .get("minecraft:world_clock")
                .map_or_else(Vec::new, |entries| {
                    entries.iter().map(|(name, _)| name.clone()).collect()
                })
        } else {
            Vec::new()
        };
        clocks.sort();
        clocks.dedup();
        reg.clocks = clocks;
        reg.by_registry
            .entry("minecraft:world_clock".into())
            .or_default()
            .ids
            .clone_from(&reg.clocks);
    }

    /// Active enchantment registry snapshot for command codecs and completions.
    /// Replace both ID views; keep the existing registry tags intact.
    pub(super) fn sync_enchantment_registries(&mut self) {
        let ids: Vec<String> = self
            .active_enchantment_definitions()
            .list
            .iter()
            .map(|entry| entry.id.to_string())
            .collect();
        self.ui.registries.set_enchantment_ids(ids);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_client::environment::TimelineTable;
    use sunrise_world::environment_timeline::Timeline;

    #[test]
    fn pack_timeline_markers_bind_to_their_clocks_and_appear_in_time_commands() {
        let day = Timeline::parse(&serde_json::json!({
            "clock": "minecraft:overworld",
            "period_ticks": 24000,
            "time_markers": {
                "minecraft:noon": 6000,
                "minecraft:night": {"ticks": 13000, "show_in_commands": true}
            }
        }))
        .unwrap();
        let festival = Timeline::parse(&serde_json::json!({
            "clock": "minecraft:overworld",
            "period_ticks": 24000,
            "time_markers": {
                "demo:festival_start": {"ticks": 3000, "show_in_commands": true}
            }
        }))
        .unwrap();
        let lunar = Timeline::parse(&serde_json::json!({
            "clock": "demo:moon",
            "period_ticks": 12000,
            "time_markers": {
                "demo:eclipse": {"ticks": 6000, "show_in_commands": true}
            }
        }))
        .unwrap();
        let table = TimelineTable {
            timelines: BTreeMap::from([
                ("minecraft:day".into(), Arc::new(day)),
                ("demo:festival".into(), Arc::new(festival)),
                ("demo:lunar".into(), Arc::new(lunar)),
            ]),
            tags: BTreeMap::new(),
        };
        let (timelines, markers) = timelines_and_markers(&table);
        assert_eq!(
            timelines.get("demo:festival"),
            Some(&("minecraft:overworld".to_owned(), Some(24000)))
        );
        assert_eq!(
            timelines.get("demo:lunar"),
            Some(&("demo:moon".to_owned(), Some(12000)))
        );
        assert_eq!(
            markers
                .get(&("minecraft:overworld".into(), "demo:festival_start".into()))
                .map(|m| (m.ticks, m.period, m.show)),
            Some((3000, Some(24000), true))
        );
        assert_eq!(
            markers
                .get(&("demo:moon".into(), "demo:eclipse".into()))
                .map(|m| (m.ticks, m.period, m.show)),
            Some((6000, Some(12000), true))
        );
        let mut command_markers: Vec<String> = markers
            .iter()
            .filter(|(_, m)| m.show)
            .map(|((_, marker), _)| marker.clone())
            .collect();
        command_markers.sort();
        command_markers.dedup();
        assert_eq!(
            command_markers,
            vec!["demo:eclipse", "demo:festival_start", "minecraft:night"]
        );
    }
}
