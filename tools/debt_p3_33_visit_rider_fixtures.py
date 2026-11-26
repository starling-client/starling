#!/usr/bin/env python3
"""One-shot exact-match migration of P3-33 rider and multipart fixtures."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GAME = ROOT / "crates/sunrise-client/src/game"
VOCAB = ROOT / "crates/sunrise-mod-vocab/src"
COLLECT = "collect_entity_visit"
IMPORT = f"    use crate::game::native_entity_refs::tests::{COLLECT};"
STALE = "Err(mod_api::EntityReadError::Stale)"


def main():
    updates = {}

    def replace(path, old, new, expected=1):
        source = updates[path] if path in updates else path.read_text(encoding="utf-8")
        count = source.count(old)
        if count != expected:
            raise SystemExit(f"{path.name}: expected {expected} matches, found {count}: {old}")
        updates[path] = source.replace(old, new)

    def answer(path, old, call, value, count=1, whitespace=" "):
        if value is None:
            before, after = "None", STALE
        else:
            before, after = f"Some({value})", f"Ok({value})"
        replace(path, f"{old},{whitespace}{before}",
                f"{COLLECT}(|visit| {call}),{whitespace}{after}", count)

    refs = GAME / "modding/native_entity_refs.rs"
    replace(refs, "    // Keep the handle and direct Game routes separate when collecting for assertions.",
        f"""    /// Collect only in fixtures that compare the whole result of an entity visit.
    pub(in crate::game) fn {COLLECT}(
        enumerate: impl FnOnce(
            &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
        ) -> Result<ControlFlow<()>, mod_api::EntityReadError>,
    ) -> Result<Vec<EntityRef>, mod_api::EntityReadError> {{
        let mut entities = Vec::new();
        let complete = enumerate(&mut |entity| {{
            entities.push(entity);
            ControlFlow::Continue(())
        }})?;
        assert_eq!(complete, ControlFlow::Continue(()));
        Ok(entities)
    }}

    // Keep the handle and direct Game routes separate when collecting for assertions.""")

    riding = GAME / "modding/native_entity_riding.rs"
    anchor = "mod tests {\n    use super::super::{Game, commands::Target};"
    replace(riding, anchor, f"{anchor}\n{IMPORT}")
    for entity, array, count in [
        ("vehicle", "vec![]", 3), ("vehicle", "vec![rider]", 1),
        ("vehicle", "vec![rider, second]", 2),
        ("camel", "vec![first, second]", 2), ("camel", "vec![second]", 1),
        ("camel", "Vec::new()", 1), ("camel", "vec![second, first]", 1),
        ("vehicle", "vec![last, next, driver]", 1),
        ("ghast", "vec![one, cow]", 1), ("ghast", "vec![one, cow, pig]", 1),
        ("ghast", "vec![one, cow, pig, two]", 3), ("ghast", "vec![cow, pig, two]", 1),
        ("cow", "vec![slime]", 1), ("boat", "vec![sailor]", 1),
        ("pig", "vec![player]", 1),
    ]:
        answer(riding, f"{entity}.passengers(game)", f"{entity}.visit_passengers(game, visit)", array, count)
    for entity, array, space in [
        ("camel", "vec![first, top, second]", "\n                "),
        ("first", "vec![top]", " "), ("top", "Vec::new()", " "),
        ("stale(camel)", None, " "), ("camel", "vec![top, second]", " "),
        ("camel", "vec![top]", " "), ("camel", "vec![second, first, top]", "\n                "),
    ]:
        answer(riding, f"{entity}.indirect_passengers(game)",
               f"{entity}.visit_indirect_passengers(game, visit)", array, whitespace=space)
    replace(riding, "root.indirect_passengers(game).unwrap()",
            f"{COLLECT}(|visit| root.visit_indirect_passengers(game, visit)).unwrap()")

    dragons = GAME / "modding/native_dragon_refs.rs"
    replace(dragons, "    use super::super::native_entity_refs::tests::with_game;",
            f"    use super::super::native_entity_refs::tests::{{{COLLECT}, with_game}};")
    for entity, count in [("dragon", 2), ("entity", 1)]:
        replace(dragons, f"game.entity_parts({entity}).unwrap()",
                f"{COLLECT}(|visit| {entity}.visit_parts(game, visit)).unwrap()", count)
    for entity, array in [("*part", "Vec::new()"), ("stale_dragon", None), ("entity", None)]:
        answer(dragons, f"game.entity_parts({entity})", f"game.entity_visit_parts({entity}, visit)", array)
    answer(dragons, "part.passengers(game)", "part.visit_passengers(game, visit)", "Vec::new()")

    remote = GAME / "network/remote/general.rs"
    anchor = "        use mod_api::{ClientHost, EntityRef, WorldEpoch};"
    replace(remote, anchor, f"    {IMPORT}\n{anchor}")
    for entity, host, array in [
        ("vehicle", "host", "vec![]"), ("vehicle", "host", "vec![row, me]"),
        ("unknown", "host", None), ("stale", "host", None),
        ("vehicle", "game", "vec![row, me]"), ("cannot_carry", "game", "vec![]"),
        ("me", "host", "vec![]"), ("vehicle", "host", "vec![row]"),
        ("vehicle", "game", "vec![]"),
    ]:
        answer(remote, f"{entity}.passengers({host})", f"{entity}.visit_passengers({host}, visit)", array)

    saves = GAME / "entities/general/save_load_tests.rs"
    replace(saves, "use crate::game::native_entity_refs::tests::with_game;",
            f"use crate::game::native_entity_refs::tests::{{{COLLECT}, with_game}};")
    answer(saves, "game.entity_passengers(cart)", "game.entity_visit_passengers(cart, visit)", "vec![rider]", 2)
    for vehicle in ["cart", "boat"]:
        answer(saves, f"{vehicle}.passengers(game)", f"{vehicle}.visit_passengers(game, visit)", "vec![rider]")

    replace(GAME / "entities/passengers.rs", "assert!(game.entity_passengers(vehicle).is_none());",
            'assert_eq!(game.entity_visit_passengers(vehicle, &mut |_| panic!("stale vehicle was visited")), '
            f'{STALE});', 3)
    ride_player = GAME / "modding/native_ride_player.rs"
    replace(ride_player, "mod tests {\n", f"mod tests {{\n{IMPORT}\n")
    answer(ride_player, "game.entity_passengers(player)", "game.entity_visit_passengers(player, visit)", "vec![first]")

    replace(VOCAB / "hooks_tests.rs", "    let _: Option<Vec<EntityRef>> = entity.parts(read);",
        """    use core::ops::ControlFlow;
    let _: Result<ControlFlow<()>, EntityReadError> =
        entity.visit_parts(read, &mut |_: EntityRef| ControlFlow::Break(()));
    let _: Result<ControlFlow<()>, EntityReadError> =
        entity.visit_passengers(read, &mut |_: EntityRef| ControlFlow::Continue(()));
    let _: Result<ControlFlow<()>, EntityReadError> =
        entity.visit_indirect_passengers(read, &mut |_: EntityRef| ControlFlow::Continue(()));""")

    for path, source in updates.items():
        path.write_text(source, encoding="utf-8")


if __name__ == "__main__":
    main()
