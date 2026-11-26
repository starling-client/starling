//! The `sound_type` Part method (`IBlockExtension.getSoundType`). The client library asks through
//! `native_client_hooks`; only a state whose block has a subscribing part is converted.
use super::Game;
use super::api_values::block_state;
use super::native_surface::FrictionView;
use mod_api::{BlockSoundCause, BlockSoundType, SoundSource};
use sunrise_client::motion::FrictionLevel;
use sunrise_core::position::BlockPos;
use sunrise_world::{level::data::BlockSound, registry::BlockStateId};

/// The game's side of `sunrise_client::block_sounds::native_sound_type`: the client library decides
/// (the state's own sound type), the game fires the hook with the read-only level, and the
/// answer comes back for the library to apply.
pub(crate) fn sound_type(
    level: FrictionLevel<'_>,
    pos: BlockPos,
    state: BlockStateId,
    entity: Option<(u32, &str)>,
    cause: BlockSoundCause,
    own: &BlockSoundType,
) -> Option<BlockSoundType> {
    if !block_state::block_listens(mod_api::HookId::SoundType, state) {
        return None;
    }
    let mut sound = own.clone();
    FrictionView(level, None).native_sound_type(pos, state, entity, cause, &mut sound);
    Some(sound)
}

impl Game<'_> {
    /// The sound of `state` at `pos` for `cause`, using the block's contextual sound type.
    /// `entity` is the entity stepping, landing, placing or digging, or None for a break or a
    /// placement by no entity.
    pub(in crate::game) fn selected_block_sound(
        &self,
        [x, y, z]: [i32; 3],
        state: BlockStateId,
        entity: Option<(u32, &str)>,
        cause: BlockSoundCause,
    ) -> BlockSound {
        let level = FrictionLevel {
            world: &self.world,
            registry: self.pack.assets.registry,
            context: &self.loop_context,
            side: self,
            coefficient: &|_| None,
        };
        let sound = sunrise_client::block_sounds::sound_type(
            level,
            self.res.facts(),
            BlockPos::new(x, y, z),
            state,
            entity,
            cause,
        );
        BlockSound::from_type(&sound, cause)
    }
    /// [`Game::selected_block_sound`] for the sounds this window's player plays itself.
    pub(in crate::game) fn own_block_sound(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
        cause: BlockSoundCause,
    ) -> BlockSound {
        let player = (self.own_entity_id() as u32, "minecraft:player");
        self.selected_block_sound(pos, state, Some(player), cause)
    }
    /// `LevelEventHandler`'s break sound (level event 2001, which a client's predicted break
    /// also plays): the block's break sound at its centre, at `(volume + 1) / 2` and pitch × 0.8.
    /// None for air, or when a part on the block cancels it (`play_break_sound`).
    pub(in crate::game) fn play_break_sound(&mut self, pos: [i32; 3], state: BlockStateId) {
        if self.res.facts().is_air(state) || self.mod_play_break_sound(state, pos) {
            return;
        }
        let sound = self.selected_block_sound(pos, state, None, BlockSoundCause::Break);
        self.play_audio_on(
            SoundSource::Blocks.index(),
            sound.event.as_str(),
            Some(pos.map(|v| f64::from(v) + 0.5)),
            (sound.volume + 1.0) / 2.0,
            sound.pitch * 0.8,
        );
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
    use mod_api::BlockSoundCause::{Break, Dig, Fall, Place, Step};
    use std::sync::Arc;
    use sunrise_client::mobs::{Mob, MobEvent, equine};
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    use sunrise_world::level::LevelAccess;

    // The executable installs its client callback table once per process, so the fixture runs
    // in a fresh child: this returns true there, and false in the parent once the child passed.
    pub(in crate::game) fn install_sound_fixture_in_child(test: &str) -> bool {
        const CHILD: &str = "SUNRISE_BLOCK_SOUND_TEST_CHILD";
        if run_fresh_cases(test, CHILD, &["child"]) {
            return false;
        }
        crate::entry::install_client_hooks();
        true
    }

    #[test]
    #[ignore = "daily"]
    fn a_block_part_answers_the_sound_type_a_block_plays() {
        if !install_sound_fixture_in_child(
            "game::native_block_sounds::tests::a_block_part_answers_the_sound_type_a_block_plays",
        ) {
            return;
        }
        mod_platform::__private::part_tests::sound_type_game_thread();
        with_game(|game| {
            let registry = game.pack.assets.registry;
            let player = Some((7, "minecraft:player"));
            let creeper = Some((8, "minecraft:creeper"));
            let sniffer = Some((9, "minecraft:sniffer"));
            let sound = |game: &crate::game::Game<'_>, block: &str, entity, cause| {
                let state = registry.default_state(block).unwrap();
                let sound = game.selected_block_sound([5, 64, 5], state, entity, cause);
                (sound.event.as_str().to_owned(), sound.volume, sound.pitch)
            };
            let stone = |event: &str| (format!("minecraft:block.stone.{event}"), 1.0, 1.0);
            let wool = |event: &str| (format!("minecraft:block.wool.{event}"), 1.0, 1.0);
            let answered = |event: &str| (format!("mymod:block.x.{event}"), 2.0, 1.5);
            // No part: the state's own.
            assert_eq!(sound(game, "minecraft:stone", creeper, Step), stone("step"));
            // A part answering every cause and entity, with the mod's own events.
            assert_eq!(
                sound(game, "minecraft:glass", creeper, Step),
                answered("step")
            );
            assert_eq!(
                sound(game, "minecraft:glass", creeper, Fall),
                answered("fall")
            );
            assert_eq!(
                sound(game, "minecraft:glass", player, Place),
                answered("place")
            );
            assert_eq!(
                sound(game, "minecraft:glass", None, Break),
                answered("break")
            );
            assert_eq!(
                sound(game, "minecraft:glass", sniffer, Dig),
                answered("hit")
            );
            // A part answering for a player only.
            assert_eq!(
                sound(game, "minecraft:white_wool", player, Step),
                answered("step")
            );
            assert_eq!(
                sound(game, "minecraft:white_wool", creeper, Step),
                wool("step")
            );
            assert_eq!(
                sound(game, "minecraft:white_wool", None, Break),
                wool("break")
            );
            // The wrapper does not replace levels the mixer will interpret, and an unused
            // event missing from the packs never hides the selected event.
            assert_eq!(
                sound(game, "minecraft:red_wool", creeper, Step),
                ("mymod:block.x.step".to_owned(), -3.0, 0.25)
            );
            assert_eq!(
                sound(game, "minecraft:blue_wool", creeper, Step),
                ("minecraft:block.amethyst_block.step".to_owned(), 2.0, 1.5)
            );
            assert_eq!(
                sound(game, "minecraft:blue_wool", None, Break),
                ("mymod:missing".to_owned(), 2.0, 1.5)
            );
            // The server's placement (`BlockItem.place`) asks with the acting player, and with
            // none when nothing places it (a dispenser).
            let glass = registry.default_state("minecraft:glass").unwrap();
            game.audio.capture = Some(Vec::new());
            game.play_break_sound([5, 64, 5], glass);
            let captured = game.audio.capture.take().unwrap();
            let [broken] = captured.as_slice() else {
                panic!("expected one break sound");
            };
            assert_eq!(broken.event, "mymod:block.x.break");
            assert_eq!(broken.source, mod_api::SoundSource::Blocks.index());
            assert_eq!((broken.volume, broken.pitch), (1.5, 1.2));

            let white_wool = registry.default_state("minecraft:white_wool").unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:creeper")
                .unwrap()
                .clone();
            let creeper_mob = Mob::new(8, kind, Vec3::ZERO, 0.0);
            let step = sunrise_client::mobs::entity_sounds::step(
                &creeper_mob,
                &game.mob_world_base(&[]),
                [5, 64, 5],
                glass,
            )
            .unwrap();
            assert!(matches!(&step.0, std::borrow::Cow::Owned(_)));
            assert_eq!(step.0, "mymod:block.x.step");
            assert_eq!(step.1, 0.3);
            assert_eq!(step.2, 1.5);

            let mut placed = Vec::new();
            for acting in [Some(7), None] {
                placed.extend(game.with_level(|_, level, _| {
                    let place = |state| {
                        let pos = BlockPos::new(5, 64, 5);
                        level.override_place_sound(pos, state, acting).map(|sound| {
                            (sound.event.as_str().to_owned(), sound.volume, sound.pitch)
                        })
                    };
                    [place(glass), place(white_wool)]
                }));
            }
            let placement = Some(answered("place"));
            assert_eq!(
                placed,
                [
                    placement.clone(),
                    placement.clone(),
                    placement,
                    Some(wool("place")),
                ]
            );
            // A horse stepping on stone under a snow layer plays the snow's answer, asked at the
            // stone's position (`AbstractHorse.playStepSound`): volume 2.0 × 0.15, pitch 1.5.
            let stone = registry.default_state("minecraft:stone").unwrap();
            let snow = registry.default_state("minecraft:snow").unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
            game.world.set(5, 64, 5, stone).unwrap();
            game.world.set(5, 65, 5, snow).unwrap();
            let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            let mut horse = Mob::new(12, kind, Vec3::ZERO, 0.0);
            let mut events = Vec::new();
            equine::play_step(
                &mut horse,
                &game.mob_world_base(&[]),
                [5, 64, 5],
                stone,
                &mut events,
            );
            assert!(
                matches!(
                    events.as_slice(),
                    [MobEvent::Sound("entity.horse.step", _, volume, pitch)]
                        if *volume == 2.0 * 0.15 && *pitch == 1.5
                ),
                "{events:?}"
            );
            // Oak's part edited only its place event. The horse still hears WOOD's hoof sound,
            // and a hard crop sharing the wood step event still has an ordinary hoof sound.
            let air = registry.default_state("minecraft:air").unwrap();
            let planks = registry.default_state("minecraft:oak_planks").unwrap();
            let answered = sunrise_client::block_sounds::sound_type(
                sunrise_client::motion::FrictionLevel {
                    world: &game.world,
                    registry: game.pack.assets.registry,
                    context: &game.loop_context,
                    side: &*game,
                    coefficient: &|_| None,
                },
                game.res.facts(),
                BlockPos::new(5, 64, 5),
                planks,
                Some((horse.id, "minecraft:horse")),
                Step,
            );
            assert_eq!(answered.place.as_str(), "mymod:unused_place");
            game.world.set(5, 65, 5, air).unwrap();
            for (block, expected) in [
                ("minecraft:oak_planks", "entity.horse.step_wood"),
                ("minecraft:pumpkin_stem", "entity.horse.step"),
            ] {
                let state = registry.default_state(block).unwrap();
                game.world.set(5, 64, 5, state).unwrap();
                events.clear();
                equine::play_step(
                    &mut horse,
                    &game.mob_world_base(&[]),
                    [5, 64, 5],
                    state,
                    &mut events,
                );
                assert!(
                    matches!(events.as_slice(), [MobEvent::Sound(name, ..)] if *name == expected)
                );
            }
            // A panic drops only the unfinished edit and disables this mod's callback. It
            // neither disables an unrelated mod nor reports the same panic a second time.
            assert_eq!(
                sound(game, "minecraft:green_wool", creeper, Step),
                wool("step")
            );
            assert_eq!(
                sound(game, "minecraft:glass", creeper, Step),
                ("minecraft:block.glass.step".to_owned(), 1.0, 1.0)
            );
            assert_eq!(
                sound(game, "minecraft:green_wool", creeper, Step),
                wool("step")
            );
            assert_eq!(
                sound(game, "minecraft:black_wool", creeper, Step),
                ("other_mod:audible".to_owned(), 2.0, 1.5)
            );
            let notices = mod_api::__private::take_hook_panics();
            let mut ours = notices
                .iter()
                .filter(|notice| notice.mod_id == "sound_type_test");
            let notice = ours.next().expect("sound callback panic notice");
            assert_eq!(notice.hook, "sound_type");
            assert!(ours.next().is_none());
        });
    }
}
