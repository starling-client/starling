//! #1988: on a NoAI zombie, `/attribute` reads the modifier Jump Boost,
//! Absorption and Invisibility store when given (`MobEffect.addAttributeModifiers`
//! from `LivingEntity.onEffectAdded`), and none when the effect was loaded
//! from `active_effects` (`readAdditionalSaveData` adds none). The values are
//! `MobEffects`': `minecraft:effect.jump_boost` 1.0 per level and
//! `minecraft:effect.absorption` 4.0 per level, both `add_value`, and
//! `minecraft:effect.waypoint_transmit_range_hide` -1.0, `add_multiplied_total`.
use crate::game::Game;
use crate::game::commands::Target;
use std::sync::Arc;
use sunrise_client::chunk::Chunk;

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        assert!(game.run_command("forceload add 0 0"));
        // A zombie cannot be summoned in peaceful.
        assert!(game.run_command("difficulty normal"));
        test(game);
    });
}

/// The number a successful `/attribute` read says, or None when it failed.
fn read(game: &mut Game<'_>, command: &str) -> Option<String> {
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said: String = game
        .chat_capture
        .take()
        .unwrap_or_default()
        .iter()
        .map(sunrise_client::text::Text::plain)
        .collect();
    ok.then(|| {
        said.split_whitespace()
            .last()
            .unwrap_or_default()
            .to_owned()
    })
}

#[test]
#[ignore = "daily"]
fn attribute_reads_the_modifier_a_given_effect_stored_and_none_for_a_loaded_one() {
    with_game(|game| {
        assert!(game.run_command(
            r#"summon minecraft:zombie 2.5 64 2.5 {NoAI:1b,Tags:["given"],UUID:[I;1988,0,0,1]}"#
        ));
        assert!(game.run_command(
            r#"summon minecraft:zombie 4.5 64 2.5 {NoAI:1b,Tags:["loaded"],AbsorptionAmount:8f,UUID:[I;1988,0,0,2],attributes:[{id:"minecraft:waypoint_transmit_range",base:100d}],active_effects:[{id:"minecraft:jump_boost",amplifier:1b,duration:-1},{id:"minecraft:absorption",amplifier:1b,duration:-1},{id:"minecraft:invisibility",amplifier:0b,duration:-1}]}"#
        ));
        assert!(game.run_command(
            "attribute @e[tag=given,limit=1] minecraft:waypoint_transmit_range base set 100"
        ));
        for effect in [
            "jump_boost infinite 1",
            "absorption infinite 1",
            "invisibility infinite 0",
        ] {
            assert!(game.run_command(&format!(
                "effect give @e[tag=given,limit=1] minecraft:{effect}"
            )));
        }
        let rows = [
            ("safe_fall_distance", "jump_boost", "5.0", "2.0", "3.0"),
            ("max_absorption", "absorption", "8.0", "8.0", "0.0"),
            (
                "waypoint_transmit_range",
                "waypoint_transmit_range_hide",
                "0.0",
                "-1.0",
                "100.0",
            ),
        ];
        for (attribute, modifier, given, amount, loaded) in rows {
            let get =
                |tag: &str| format!("attribute @e[tag={tag},limit=1] minecraft:{attribute} get");
            let value = |tag: &str| {
                format!(
                    "attribute @e[tag={tag},limit=1] minecraft:{attribute} modifier value get minecraft:effect.{modifier}"
                )
            };
            assert_eq!(
                read(game, &get("given")).as_deref(),
                Some(given),
                "{attribute}"
            );
            assert_eq!(
                read(game, &value("given")).as_deref(),
                Some(amount),
                "{attribute}"
            );
            assert_eq!(
                read(game, &get("loaded")).as_deref(),
                Some(loaded),
                "{attribute}"
            );
            assert_eq!(read(game, &value("loaded")), None, "{attribute}");
        }
    });
}

/// `isTransmittingWaypoint` reads the range with Invisibility's modifier:
/// the given zombie's 100 is hidden to 0, so its next waypoint check
/// untracks it, while the zombie that loaded Invisibility keeps transmitting.
#[test]
#[ignore = "daily"]
fn a_given_invisibility_hides_a_mobs_waypoint_and_a_loaded_one_does_not() {
    with_game(|game| {
        assert!(game.run_command(
            r#"summon minecraft:zombie 2.5 64 2.5 {NoAI:1b,Tags:["given"],UUID:[I;1988,0,0,3]}"#
        ));
        assert!(game.run_command(
            r#"summon minecraft:zombie 4.5 64 2.5 {NoAI:1b,Tags:["loaded"],UUID:[I;1988,0,0,4],active_effects:[{id:"minecraft:invisibility",amplifier:0b,duration:-1}]}"#
        ));
        let mob = |game: &Game<'_>, n: i32| {
            let m = game
                .mobs
                .iter()
                .find(|m| game.persist.uuid_of(m.id) == Some([1988, 0, 0, n]))
                .expect("the summoned zombie");
            Target::Mob(m.id)
        };
        let (given, loaded) = (mob(game, 3), mob(game, 4));
        // A range write marks the entity for its next waypoint check.
        let set_range = |game: &mut Game<'_>| {
            for tag in ["given", "loaded"] {
                assert!(game.run_command(&format!(
                    "attribute @e[tag={tag},limit=1] minecraft:waypoint_transmit_range base set 100"
                )));
            }
            game.tick_waypoints();
        };
        set_range(game);
        let tracked = |game: &Game<'_>| game.cmds.waypoints.clone().unwrap_or_default();
        assert!(tracked(game).contains(&given));
        assert!(tracked(game).contains(&loaded));
        assert!(
            game.run_command("effect give @e[tag=given,limit=1] minecraft:invisibility infinite 0")
        );
        set_range(game);
        assert!(!tracked(game).contains(&given), "{:?}", tracked(game));
        assert!(tracked(game).contains(&loaded), "{:?}", tracked(game));
    });
}

/// `AttributeCommand.getAttributeModifier` reads `getModifier`, which finds a
/// transient modifier too: a mod's transient `test:mod` (1.5) on a zombie's
/// `safe_fall_distance` reads 1.5, as it did when the attribute lived in the
/// generic map.
#[test]
#[ignore = "daily"]
fn modifier_value_get_finds_a_mods_transient_modifier() {
    use sunrise_client::mobs::finalize::{self, Attr, Op};
    with_game(|game| {
        assert!(game.run_command(
            r#"summon minecraft:zombie 2.5 64 2.5 {NoAI:1b,Tags:["t"],UUID:[I;1988,0,0,5]}"#
        ));
        let m = game
            .mobs
            .iter_mut()
            .find(|m| m.kind.id == "minecraft:zombie")
            .expect("the summoned zombie");
        finalize::add_transient_modifier(m, Attr::SafeFallDistance, "test:mod", 1.5, Op::AddValue);
        let value =
            "attribute @e[tag=t,limit=1] minecraft:safe_fall_distance modifier value get test:mod";
        assert_eq!(read(game, value).as_deref(), Some("1.5"));
        assert_eq!(
            read(
                game,
                "attribute @e[tag=t,limit=1] minecraft:safe_fall_distance get"
            )
            .as_deref(),
            Some("4.5")
        );
    });
}
