#!/usr/bin/env python3
"""One-shot caller migration for the P3-33 living and fluid visitors."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GAME = ROOT / "crates/sunrise-client/src/game"


def main():
    updates = {}

    def replace(rel, old, new, expected=1):
        path = GAME / rel
        text = updates[path] if path in updates else path.read_text(encoding="utf-8")
        count = text.count(old)
        if count != expected:
            raise SystemExit(f"{rel}: expected {expected} occurrences, found {count}: {old}")
        updates[path] = text.replace(old, new)

    fluid = "modding/native_entity_state.rs"
    replace(fluid, '        let (water, lava) = ("minecraft:water".to_owned(), "minecraft:lava".to_owned());', '''        let (water, lava) = ("minecraft:water", "minecraft:lava");
        let touched_fluids = |water, lava| {
            let mut fluids = Vec::new();
            let result = visit_touched_fluids(water, lava, &mut |id, depth| {
                fluids.push((id.to_owned(), depth));
                ControlFlow::Continue(())
            });
            assert_eq!(result, ControlFlow::Continue(()));
            fluids
        };''')
    for name, count in [("water", 3), ("lava", 2)]:
        replace(fluid, f"({name}.clone(),", f"({name}.to_owned(),", count)
    replace(fluid, "assert_eq!(local.fluid_heights(host), Some(Vec::new()));", '''assert_eq!(
                local.visit_fluid_heights(host, &mut |_, _| panic!("dry player visited")),
                Ok(ControlFlow::Continue(()))
            );''')
    replace(fluid, "assert_eq!(zombie.fluid_heights(host), Some(expected));", '''let mut fluids = Vec::new();
                assert_eq!(
                    zombie.visit_fluid_heights(host, &mut |id, depth| {
                        fluids.push((id.to_owned(), depth));
                        ControlFlow::Continue(())
                    }),
                    Ok(ControlFlow::Continue(()))
                );
                assert_eq!(fluids, expected);''')
    replace(fluid, "Some(eye.map(str::to_owned))", "Some(eye)")
    for entity, host, error in [
        ("stand", "host", "Unavailable"), ("stale", "host", "Stale"),
        ("zombie", "&*game as &dyn mod_api::ClientHost", "Stale"),
        ("local", "&*game as &dyn mod_api::ClientHost", "Unavailable"),
    ]:
        ws = "\n                " if host != "host" else " "
        replace(fluid, f"{entity}.fluid_heights({host}),{ws}None",
                f'{entity}.visit_fluid_heights({host}, &mut |_, _| panic!("absent fluid visit")),\n'
                f'                Err(EntityReadError::{error})')

    effects = "modding/native_living_effects.rs"
    replace(effects, '    use mod_api::{ActiveEffect, EntityRef, LivingRef, WorldEpoch};',
            '''    use mod_api::{ActiveEffect, EntityReadError, EntityRef, Host, LivingRef, WorldEpoch};
    use std::ops::ControlFlow;''')
    replace(effects, "    fn effect(id: &'static str, duration: i32, amplifier: i32) -> ActiveEffect {", '''    fn collect_effects(host: &dyn Host, living: LivingRef) -> Result<Vec<ActiveEffect>, EntityReadError> {
        let mut effects = Vec::new();
        assert_eq!(living.visit_effects(host, &mut |effect| {
            effects.push(effect);
            ControlFlow::Continue(())
        })?, ControlFlow::Continue(()));
        Ok(effects)
    }

    fn effect(id: &'static str, duration: i32, amplifier: i32) -> ActiveEffect {''')
    replace(effects, "game.living_effects(zombie).unwrap()", "collect_effects(game, zombie).unwrap()")
    replace(effects, "game.living_effects(living).unwrap()", "collect_effects(game, living).unwrap()")
    replace(effects, "me.effects(game).map(|list| list.len()), Some(1)",
            "collect_effects(game, me).map(|list| list.len()), Ok(1)")
    replace(effects, 'game.living_effects(zombie),\n                Some(vec![effect("minecraft:speed", 200, 0)])',
            'collect_effects(game, zombie),\n                Ok(vec![effect("minecraft:speed", 200, 0)])')
    for entity in ["spider", "zombie"]:
        replace(effects, f"game.living_effects({entity}), Some(Vec::new())",
                f"collect_effects(game, {entity}), Ok(Vec::new())")
    replace(effects, "game.living_effects(stale), None", 'stale.visit_effects(game, &mut |_| panic!("stale effects visited")),\n                Err(EntityReadError::Stale)')

    consume = "modding/consume_effect_dispatch_tests.rs"
    replace(consume, '''        let has_effect = living
            .effects(host)
            .unwrap()
            .iter()
            .any(|effect| effect.id == breathing);''', '''        use std::ops::ControlFlow;
        let has_effect = living
            .visit_effects(host, &mut |effect| {
                if effect.id == breathing {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .unwrap()
            .is_break();''')

    original = "modding/original_player.rs"
    replace(original, '''            let replacement_effects = mod_api::LivingRef::new(replacement.entity())
                .effects(host)
                .unwrap();
            assert_eq!(replacement_effects.len(), usize::from(!self.death));''', '''            let mut replacement_count = 0;
            assert_eq!(
                mod_api::LivingRef::new(replacement.entity()).visit_effects(host, &mut |_| {
                    replacement_count += 1;
                    std::ops::ControlFlow::Continue(())
                }),
                Ok(std::ops::ControlFlow::Continue(()))
            );
            assert_eq!(replacement_count, usize::from(!self.death));''')

    refs = "modding/native_living_refs.rs"
    replace(refs, '''            // The zombie's type has these attributes, and the stand's lacks
            // attack damage.
            let listed = |living: mod_api::LivingRef| living.attributes(game).unwrap();''', '''            let read_names = |game: &dyn mod_api::Host, living: LivingRef| {
                let mut names = Vec::new();
                let result = living.visit_attributes(game, &mut |name| {
                    names.push(name.to_owned());
                    ControlFlow::Continue(())
                });
                assert_eq!(result, Ok(ControlFlow::Continue(())));
                names
            };
            let read_modifiers = |game: &dyn mod_api::Host, living: LivingRef, name: &str| -> Result<Vec<_>, EntityReadError> {
                let mut modifiers = Vec::new();
                assert_eq!(living.visit_attribute_modifiers(game, name, &mut |m| {
                    modifiers.push((m.id.to_owned(), m.amount, m.operation));
                    ControlFlow::Continue(())
                })?, ControlFlow::Continue(()));
                Ok(modifiers)
            };
            // The zombie's type has these attributes, and the stand's lacks attack damage.''')
    replace(refs, "listed(zombie).contains(&Identifier::parse(name).unwrap())",
            "read_names(game, zombie).iter().any(|id| id == name)")
    replace(refs, 'listed(stand).contains(&Identifier::parse("minecraft:attack_damage").unwrap())',
            'read_names(game, stand).iter().any(|id| id == "minecraft:attack_damage")')
    for entity, name, result, count in [
        ("zombie", "speed", "Ok(Vec::new())", 1),
        ("player", "health", "Ok(Vec::new())", 1),
    ]:
        replace(refs, f"{entity}.attribute_modifiers(game, {name}), Some(Vec::new())",
                f"read_modifiers(game, {entity}, {name}), {result}", count)
    replace(refs, '''assert_eq!(zombie.attribute_modifiers(game, "missing:attribute"), None);''',
            '''assert_eq!(read_modifiers(game, zombie, "missing:attribute"), Err(EntityReadError::Unavailable));''')
    replace(refs, '''stand.attribute_modifiers(game, "minecraft:attack_damage"),
                None''', '''read_modifiers(game, stand, "minecraft:attack_damage"),
                Err(EntityReadError::Unavailable)''')
    replace(refs, '''zombie.attribute_modifiers(game, speed),
                Some(vec![
                    modifier("test:fast", 1.0, AddMultipliedBase),
                    modifier("test:boost", 0.5, AddMultipliedTotal)
                ])''', '''read_modifiers(game, zombie, speed),
                Ok(vec![
                    ("test:fast".to_owned(), 1.0, AddMultipliedBase),
                    ("test:boost".to_owned(), 0.5, AddMultipliedTotal),
                ])''')
    for entity, attr, id, value, op in [("zombie", "speed", "test:fast", "2.0", "AddValue"),
                                         ("player", "health", "test:hearty", "0.5", "AddMultipliedBase")]:
        replace(refs, f'{entity}.attribute_modifiers(game, {attr}),\n                Some(vec![modifier("{id}", {value}, {op})])',
                f'read_modifiers(game, {entity}, {attr}),\n                Ok(vec![("{id}".to_owned(), {value}, {op})])')
    replace(refs, 'stand.attribute_modifiers(game, "minecraft:armor").unwrap()',
            'read_modifiers(game, stand, "minecraft:armor").unwrap()')
    replace(refs, '(m.amount - 3.0).abs() < 1e-9', 'm.1 == 3.0')
    replace(refs, 'zombie.attributes(game), None',
            'zombie.visit_attributes(game, &mut |_| panic!("stale attributes visited")),\n                Err(EntityReadError::Stale)')
    for entity, attr in [("zombie", "speed"), ("stand", '"minecraft:armor"')]:
        replace(refs, f'{entity}.attribute_modifiers(game, {attr}), None',
                f'{entity}.visit_attribute_modifiers(game, {attr}, &mut |_| panic!("stale modifier visited")),\n                Err(EntityReadError::Stale)')

    trade = "network/lan_trade_decisions_tests.rs"
    replace(trade, '''    let live = other
        .attribute_modifiers(game, SPEED)
        .unwrap()
        .into_iter()
        .map(|row| row.id.to_string())
        .collect();''', '''    let mut live = Vec::new();
    assert_eq!(other.visit_attribute_modifiers(game, SPEED, &mut |row| {
        live.push(row.id.to_owned());
        std::ops::ControlFlow::Continue(())
    }), Ok(std::ops::ControlFlow::Continue(())));''')

    remote = "network/remote/client_attributes_tests.rs"
    replace(remote, '    use crate::game::native_entity_refs::tests::with_client_game;',
            '''    use crate::game::native_entity_refs::tests::with_client_game;
    use mod_api::{AttributeOperation, EntityReadError};
    use std::ops::ControlFlow;''')
    replace(remote, 'assert_eq!(living.attribute_modifiers(game, health), Some(Vec::new()));',
            '''assert_eq!(living.visit_attribute_modifiers(game, health, &mut |_| panic!("default modifier visited")),
            Ok(ControlFlow::Continue(())));''')
    replace(remote, '''        assert_eq!(
            living.attribute_modifiers(game, health),
            Some(vec![mod_api::AttributeModifier::new(
                mod_api::Identifier::parse("test:bonus").unwrap(),
                5.0,
                mod_api::AttributeOperation::AddValue,
            )])
        );
        let listed = living.attributes(game).unwrap();
        assert!(listed.contains(&mod_api::Identifier::parse(health).unwrap()));''', '''        let mut modifiers = Vec::new();
        assert_eq!(living.visit_attribute_modifiers(game, health, &mut |m| {
            modifiers.push((m.id.to_owned(), m.amount, m.operation));
            ControlFlow::Continue(())
        }), Ok(ControlFlow::Continue(())));
        assert_eq!(modifiers, [("test:bonus".to_owned(), 5.0, AttributeOperation::AddValue)]);
        assert_eq!(living.visit_attributes(game, &mut |name| {
            if name == health {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }), Ok(ControlFlow::Break(())));''')
    replace(remote, 'mob.attribute_modifiers(game, health), None',
            'mob.visit_attribute_modifiers(game, health, &mut |_| panic!("client mob modifier visited")),\n            Err(EntityReadError::Unavailable)')
    replace(remote, 'mob.attributes(game), None',
            'mob.visit_attributes(game, &mut |_| panic!("client mob attribute visited")),\n            Err(EntityReadError::Unavailable)')

    handles = "network/remote/client_handles_tests.rs"
    replace(handles, '        assert_eq!(living.effects(game), Some(Vec::new()));', '''        use std::ops::ControlFlow;
        assert_eq!(living.visit_effects(game, &mut |_| panic!("effect before sync")), Ok(ControlFlow::Continue(())));''')
    replace(handles, '        assert_eq!(living.effects(game).unwrap().len(), 1);', '''        let mut effects = Vec::new();
        assert_eq!(living.visit_effects(game, &mut |effect| {
            effects.push((effect.duration, effect.amplifier));
            ControlFlow::Continue(())
        }), Ok(ControlFlow::Continue(())));
        assert_eq!(effects, [(600, 1)]);''')

    for path, text in updates.items():
        path.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
