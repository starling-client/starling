#!/usr/bin/env python3
"""Migrate existing scoreboard and raid fixture calls to the borrowed surface."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MODDING = ROOT / "crates/sunrise-client/src/game/modding"


def migrate(path, replacements):
    source = path.read_text()
    for old, new, count in replacements:
        if source.count(old) != count:
            raise ValueError(f"{path}: expected {count} occurrences of {old!r}")
        source = source.replace(old, new)
    path.write_text(source)


def main():
    migrate(MODDING / "native_scoreboard.rs", [
        ('kills.name(game).as_deref()', 'kills.name(game)', 1),
        ('renamed.name(game).as_deref()', 'renamed.name(game)', 1),
        ('&slot.name()', 'slot.name()', 5),
    ])
    migrate(MODDING / "native_scoreboard_teams.rs", [
        ('red.name(game).as_deref()', 'red.name(game)', 1),
    ])
    migrate(MODDING / "native_raid_refs.rs", [
        ('    type RaidObservation =', '''    fn collect_visit<T, E>(
        enumerate: impl FnOnce(&mut dyn FnMut(T) -> ControlFlow<()>) -> Result<ControlFlow<()>, E>,
    ) -> Result<Vec<T>, E> {
        let mut entries = Vec::new();
        let result = enumerate(&mut |entry| {
            entries.push(entry);
            ControlFlow::Continue(())
        });
        match result {
            Ok(flow) => {
                assert_eq!(flow, ControlFlow::Continue(()));
                Ok(entries)
            }
            Err(error) => {
                assert!(entries.is_empty());
                Err(error)
            }
        }
    }

    type RaidObservation =''', 1),
        ('game.level_raids(level), Some(vec![raid])',
         'collect_visit(|visit| level.visit_raids(game, visit)), Ok(vec![raid])', 1),
        ('level.raids(game), Some(vec![raid])',
         'collect_visit(|visit| level.visit_raids(game, visit)), Ok(vec![raid])', 1),
        ('game.raid_heroes(raid), Some(Vec::new())',
         'collect_visit(|visit| raid.visit_heroes(game, visit)), Ok(vec![])', 1),
        ('game.raid_raiders(raid), Some(vec![first, second])',
         'collect_visit(|visit| raid.visit_raiders(game, visit)), Ok(vec![first, second])', 1),
        ('game.raid_wave_raiders(raid, 2), Some(vec![second])',
         'collect_visit(|visit| raid.visit_wave_raiders(game, 2, visit)), Ok(vec![second])', 1),
        ('game.raid_wave_raiders(raid, 3), Some(Vec::new())',
         'collect_visit(|visit| raid.visit_wave_raiders(game, 3, visit)), Ok(vec![])', 1),
        ('raid.raiders(game), Some(vec![member])',
         'collect_visit(|visit| raid.visit_raiders(game, visit)), Ok(vec![member])', 2),
        ('raid.wave_raiders(game, 2), Some(vec![member])',
         'collect_visit(|visit| raid.visit_wave_raiders(game, 2, visit)), Ok(vec![member])', 1),
        ('raid.wave_raiders(game, 1), Some(vec![])',
         'collect_visit(|visit| raid.visit_wave_raiders(game, 1, visit)), Ok(vec![])', 1),
        ('raid.heroes(game), Some(vec![])',
         'collect_visit(|visit| raid.visit_heroes(game, visit)), Ok(vec![])', 1),
        ('raid.raiders(game), None',
         'collect_visit(|visit| raid.visit_raiders(game, visit)), Err(RaidReadError::Stale)', 1),
        ('game.level_raids(later), None',
         'collect_visit(|visit| later.visit_raids(game, visit)), Err(LevelReadError::Stale)', 1),
        ('game.level_raids(level), None',
         'collect_visit(|visit| level.visit_raids(game, visit)), Err(LevelReadError::Unavailable)', 1),
    ])
    migrate(ROOT / "crates/sunrise-mod-api-gen/unwinding-methods.txt", [
        (f"{name}\n", "", 1)
        for name in (
            "level_saved_data::level_raids",
            "player_recipes::player_discovered_recipes",
            "raids::raid_heroes",
            "raids::raid_raiders",
            "raids::raid_wave_raiders",
            "scoreboard::objective_scores",
            "scoreboard::objective_slots",
            "scoreboard::objectives",
            "scoreboard::teams",
        )
    ])


if __name__ == "__main__":
    main()
