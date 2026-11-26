#!/usr/bin/env python3
"""Migrate the level, tracking, tick-ring and rule fixtures to scoped visits."""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CLIENT = "crates/sunrise-client/src/"
MODDING = CLIENT + "game/modding/"
VOCAB = "crates/sunrise-mod-vocab/src/"


def read(path):
    return (ROOT / path).read_text()


def write(path, source):
    (ROOT / path).write_text(source)


def replace(source, old, new, count=1):
    if source.count(old) != count:
        raise ValueError(f"expected {count} occurrences of {old!r}")
    return source.replace(old, new)


def migrate_tracking():
    path = MODDING + "native_level_players.rs"
    source = read(path)
    source = replace(source, "    use mod_api::{ChunkRef, LevelReadError, LevelRef, PlayerRef, WorldEpoch};",
                     "    use mod_api::{ChunkRef, EntityReadError, LevelReadError, LevelRef, PlayerRef, WorldEpoch};")
    source = replace(source, """    fn uuids(players: Option<Vec<PlayerRef>>) -> Option<Vec<u128>> {
        players.map(|players| players.into_iter().map(PlayerRef::uuid).collect())
    }""", """    fn uuids<E>(
        enumerate: impl FnOnce(&mut dyn FnMut(PlayerRef) -> ControlFlow<()>) -> Result<ControlFlow<()>, E>,
    ) -> Result<Vec<u128>, E> {
        let mut uuids = Vec::new();
        assert_eq!(enumerate(&mut |player| {
            uuids.push(player.uuid());
            ControlFlow::Continue(())
        })?, ControlFlow::Continue(()));
        Ok(uuids)
    }""")
    source = re.sub(r"uuids\(game\.entity_trackers\((pig|own)\)\)",
                    r"uuids(|visit| \1.visit_trackers(game, visit))", source)
    source = replace(source, "game.entity_trackers(stale), None",
                     "uuids(|visit| stale.visit_trackers(game, visit)), Err(EntityReadError::Stale)")
    source = replace(source, "game.entity_trackers(parked), None",
                     "uuids(|visit| parked.visit_trackers(game, visit)), Err(EntityReadError::Unavailable)")
    source = replace(source, "uuids(chunk.watchers(game))", "uuids(|visit| chunk.visit_watchers(game, visit))", 6)
    for chunk in ("ChunkRef::new(nether, [0, 0])", "ChunkRef::new(level, [i32::MIN, i32::MAX])"):
        source = replace(source, f"uuids({chunk}.watchers(game))", f"uuids(|visit| {chunk}.visit_watchers(game, visit))")
    source = replace(source, "stale.watchers(game), None",
                     "uuids(|visit| stale.visit_watchers(game, visit)), Err(LevelReadError::Stale)")
    start = source.index("    fn entity_trackers_are_the_guests_that_hold_the_entity()")
    source = source[:start] + source[start:].replace("Some(vec![", "Ok(vec![")
    write(path, source)


def migrate_streaming_callers():
    path = MODDING + "native_level_refs.rs"
    write(path, replace(read(path), "self.native_level_streaming(chunk.level()).is_none()",
                        "self.native_level_streaming(chunk.level()).is_err()", 2))


def migrate_chunks():
    path = MODDING + "native_level_refs.rs"
    source = replace(read(path), "    use mod_api::{ChunkRef, LevelRef, LightLayer, WorldEpoch};", """    use core::ops::ControlFlow;
    use mod_api::{ChunkRef, LevelReadError, LevelRef, LightLayer, WorldEpoch};""")
    source = replace(source, "    #[test]\n    #[ignore = \"daily\"]\n    fn loaded_chunks_lists_the_level_in_order_without_loading()", """    fn collect_chunks<T>(
        enumerate: impl FnOnce(&mut dyn FnMut(T) -> ControlFlow<()>) -> Result<ControlFlow<()>, LevelReadError>,
    ) -> Result<Vec<T>, LevelReadError> {
        let mut entries = Vec::new();
        assert_eq!(enumerate(&mut |entry| {
            entries.push(entry);
            ControlFlow::Continue(())
        })?, ControlFlow::Continue(()));
        Ok(entries)
    }

    #[test]
    #[ignore = "daily"]
    fn loaded_chunks_lists_the_level_in_order_without_loading()""")
    source = replace(source, "level.loaded_chunks(game)", "collect_chunks(|visit| level.visit_loaded_chunks(game, visit))", 2)
    source = replace(source, "assert_eq!(nether.loaded_chunks(game), None, \"not opened yet\");",
                     "assert_eq!(collect_chunks(|visit| nether.visit_loaded_chunks(game, visit)), Err(LevelReadError::Stale), \"not opened yet\");")
    source = re.sub(r"game\.level_forced_chunks\((\w+)\)", r"collect_chunks(|visit| \1.visit_forced_chunks(game, visit))", source)
    source = re.sub(r"(collect_chunks\(\|visit\| \w+\.visit_forced_chunks\(game, visit\)\), )Some\(", r"\1Ok(", source)
    source = replace(source, "collect_chunks(|visit| stale_level.visit_forced_chunks(game, visit)), None",
                     "collect_chunks(|visit| stale_level.visit_forced_chunks(game, visit)), Err(LevelReadError::Stale)")
    source = replace(source, "    fn in_mod(id: &'static str)", """    fn collect_mod_tickets(
        game: &super::Game<'_>,
        level: LevelRef,
        controller: TicketController,
    ) -> Result<Vec<(TicketOwner, [i32; 2], bool)>, LevelReadError> {
        collect_chunks(|visit| level.visit_mod_forced_chunks(game, controller, &mut |owner, pos, natural| {
            visit((owner, pos, natural))
        }))
    }

    fn in_mod(id: &'static str)""")
    source = re.sub(r"game\.level_mod_forced_chunks\(level, (quarry\(\)|vault|drill|c)\)",
                    r"collect_mod_tickets(game, level, \1)", source)
    source = re.sub(r"(collect_mod_tickets\(game, level, (?:quarry\(\)|vault|drill)\),\s*)Some\(", r"\1Ok(", source)
    source = replace(source, "collect_mod_tickets(game, level, quarry()), None",
                     "collect_mod_tickets(game, level, quarry()), Err(LevelReadError::Unavailable)")
    write(path, source)


def migrate_rules():
    path = MODDING + "native_level_world.rs"
    source = replace(read(path), "    use mod_api::{GameRuleChangedEvent, GameRuleValue, Hooks, LevelHost, ModEntry, ServerHost};",
                     "    use core::ops::ControlFlow;\n    use mod_api::{GameRuleChangedEvent, GameRuleValue, Hooks, LevelHost, ModEntry, ServerHost};")
    source = replace(source, """            let rules = host
                .level("minecraft:overworld")
                .and_then(|level| level.game_rules(host).ok())
                .unwrap_or_default();
            let value = |id: &str| rules.iter().find(|(rule, _)| rule == id).map(|(_, v)| *v);""", """            let (mut inventory, mut tick_speed, mut count) = (None, None, 0);
            let level = host.level("minecraft:overworld").unwrap();
            assert_eq!(level.visit_game_rules(host, &mut |id, value| {
                count += 1;
                match id {
                    "minecraft:keep_inventory" => inventory = Some(value),
                    "minecraft:random_tick_speed" => tick_speed = Some(value),
                    _ => {}
                }
                ControlFlow::Continue(())
            }), Ok(ControlFlow::Continue(())));""")
    source = replace(source, """                value("minecraft:keep_inventory"),
                value("minecraft:random_tick_speed"),
                rules.len(),""", """                inventory,
                tick_speed,
                count,""")
    source = replace(source, """            let rules = level.game_rules(game).unwrap();
            assert!(rules.contains(&("minecraft:random_tick_speed".into(), GameRuleValue::Int(7))));""", """            let mut tick_speed = None;
            assert_eq!(level.visit_game_rules(game, &mut |id, value| {
                if id == "minecraft:random_tick_speed" {
                    tick_speed = Some(value);
                }
                ControlFlow::Continue(())
            }), Ok(ControlFlow::Continue(())));
            assert_eq!(tick_speed, Some(GameRuleValue::Int(7)));""")
    for receiver, count in (("handle", 1), ("level", 2)):
        source = replace(source, f"{receiver}.game_rules(game)",
                         f'{receiver}.visit_game_rules(game, &mut |_, _| panic!("refused rule visit"))', count)
    write(path, source)

    path = CLIENT + "game/world/early_game_rules_tests.rs"
    source = replace(read(path), "use mod_api::{Hooks, ModEntry, ServerHost};", """use core::ops::ControlFlow;
use mod_api::{GameRuleValue, Hooks, ModEntry, ServerHost};""")
    source = replace(source, "struct Observe(Arc<Mutex<Vec<&'static str>>>);", """fn collect_rules<E>(
    enumerate: impl FnOnce(&mut dyn FnMut(&str, GameRuleValue) -> ControlFlow<()>) -> Result<ControlFlow<()>, E>,
) -> Vec<(String, GameRuleValue)> where E: core::fmt::Debug {
    let mut rules = Vec::new();
    assert_eq!(enumerate(&mut |id, value| {
        rules.push((id.to_owned(), value));
        ControlFlow::Continue(())
    }).unwrap(), ControlFlow::Continue(()));
    rules
}

struct Observe(Arc<Mutex<Vec<&'static str>>>);""")
    source = replace(source, "let rules = host.game_rules();", "let rules = collect_rules(|visit| host.visit_game_rules(visit));")
    source = replace(source, "level.game_rules(game).unwrap(),\n                            <dyn ServerHost>::game_rules(game)",
                     "collect_rules(|visit| level.visit_game_rules(game, visit)),\n                            collect_rules(|visit| (game as &dyn ServerHost).visit_game_rules(visit))")
    write(path, source)


def migrate_levels_and_ticks():
    path = MODDING + "native_entity_refs.rs"
    source = replace(read(path), "assert_eq!(host.levels().list_levels(), vec![level, nether]);", """let mut visited = Vec::new();
        assert_eq!(host.levels().visit_levels(&mut |level| {
            visited.push(level);
            ControlFlow::Continue(())
        }), Ok(ControlFlow::Continue(())));
        assert_eq!(visited, [level, nether]);""")
    write(path, source)
    path = MODDING + "native_move_by_tests.rs"
    write(path, replace(read(path), "game.native_list_levels()[0]", 'game.level("minecraft:overworld").unwrap()'))
    path = CLIENT + "clock.rs"
    write(path, replace(read(path), "let times = clock.tick_times();", "let times: Vec<_> = clock.tick_times().collect();", 2))
    path = MODDING + "native_server_distances.rs"
    source = replace(read(path), "mod tests {\n", "mod tests {\n    use core::ops::ControlFlow;\n")
    source = replace(source, "let times = host.tick_times();", """let mut times = Vec::new();
            assert_eq!(host.visit_tick_times(&mut |took| {
                times.push(took);
                ControlFlow::Continue(())
            }), Ok(ControlFlow::Continue(())));""")
    write(path, source)


def migrate_double_and_contracts():
    path = "crates/sunrise-mod-api/src/parts/payloads/group_tests.rs"
    source = replace(read(path), "DimensionKey, EntityRef, Host", "DimensionKey, EntityReadError, EntityRef, Host")
    source = replace(source, """    fn chunk_watchers(&self, _: ChunkRef) -> Option<Vec<PlayerRef>> {
        Some(self.watchers.clone())
    }""", """    fn chunk_visit_watchers(
        &self,
        _: ChunkRef,
        visit: &mut dyn FnMut(PlayerRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        Ok(self.watchers.iter().copied().try_for_each(visit))
    }""")
    source = replace(source, """    fn entity_trackers(&self, entity: EntityRef) -> Option<Vec<PlayerRef>> {
        self.trackers.get(&entity.id()).cloned()
    }""", """    fn entity_visit_trackers(
        &self,
        entity: EntityRef,
        visit: &mut dyn FnMut(PlayerRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        Ok(self.trackers.get(&entity.id()).ok_or(EntityReadError::Stale)?.iter().copied().try_for_each(visit))
    }""")
    write(path, source)
    path = VOCAB + "hooks_tests.rs"
    source = replace(read(path), "    let _: Option<Vec<PlayerRef>> = chunk.watchers(read);", """    use core::ops::ControlFlow;
    let _: Result<ControlFlow<()>, LevelReadError> = chunk.visit_watchers(read, &mut |_: PlayerRef| ControlFlow::Break(()));
    let _: Result<ControlFlow<()>, LevelReadError> = level.visit_loaded_chunks(read, &mut |_: ChunkRef| ControlFlow::Break(()));
    let _: Result<ControlFlow<()>, LevelReadError> = level.visit_forced_chunks(read, &mut |_: [i32; 2]| ControlFlow::Break(()));
    let _: Result<ControlFlow<()>, LevelReadError> = level.visit_game_rules(read, &mut |_: &str, _: GameRuleValue| ControlFlow::Break(()));
    let _: Result<ControlFlow<()>, EntityReadError> = shooter.visit_trackers(read, &mut |_: PlayerRef| ControlFlow::Break(()));
    let _: Result<ControlFlow<()>, ServerReadError> = server.visit_tick_times(&mut |_: u64| ControlFlow::Break(()));
    let _: Result<ControlFlow<()>, ServerReadError> = server.visit_game_rules(&mut |_: &str, _: GameRuleValue| ControlFlow::Break(()));
    let _: Result<ControlFlow<()>, ServerReadError> = server.levels().visit_levels(&mut |_: LevelRef| ControlFlow::Break(()));""")
    write(path, source)
    path = "crates/sunrise-mod-api-gen/unwinding-methods.txt"
    source = read(path)
    for entry in (
        "config::game_rules", "config::tick_times", "entity_identity::entity_trackers",
        "level_chunks::chunk_watchers", "level_chunks::level_loaded_chunks",
        "level_tickets::level_forced_chunks", "level_tickets::level_mod_forced_chunks",
    ):
        source = replace(source, entry + "\n", "")
    write(path, source)


if __name__ == "__main__":
    migrate_tracking()
    migrate_streaming_callers()
    migrate_chunks()
    migrate_rules()
    migrate_levels_and_ticks()
    migrate_double_and_contracts()
