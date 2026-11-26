/// Install block sound answers for the game-thread test.
pub fn sound_type_game_thread() {
    struct Answer {
        sound: BlockSoundType,
        players_only: bool,
        at_y: Option<i32>,
    }
    #[mod_api::sunrise_part]
    impl BlockPart for Answer {
        fn sound_type(&self, _: crate::LevelSide<'_>, event: &mut SoundTypeEvent<'_>) {
            let player = |(_, kind): (u32, &str)| kind == "minecraft:player";
            if self.players_only && !event.entity.is_some_and(player)
                || self.at_y.is_some_and(|y| event.pos.y != y)
            {
                return;
            }
            *event.sound = self.sound.clone();
        }
    }
    struct EditUnusedPlace(SoundEventKey);
    #[mod_api::sunrise_part]
    impl BlockPart for EditUnusedPlace {
        fn sound_type(&self, _: crate::LevelSide<'_>, event: &mut SoundTypeEvent<'_>) {
            if event.cause == BlockSoundCause::Step {
                event.sound.place = self.0.clone();
            }
        }
    }
    struct PanicAfterEdit;
    #[mod_api::sunrise_part]
    impl BlockPart for PanicAfterEdit {
        fn sound_type(&self, _: crate::LevelSide<'_>, event: &mut SoundTypeEvent<'_>) {
            event.sound.volume = 42.0;
            panic!("block sound test panic");
        }
    }
    struct SoundTypeMod;
    #[mod_api::__subscriptions]
    impl Hooks for SoundTypeMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let key = |name| SoundEventKey::parse(name).unwrap();
            let sound = BlockSoundType {
                identity: crate::BlockSoundTypeIdentity::Other,
                volume: 2.0,
                pitch: 1.5,
                break_sound: key("mymod:block.x.break"),
                step: key("mymod:block.x.step"),
                place: key("mymod:block.x.place"),
                hit: key("mymod:block.x.hit"),
                fall: key("mymod:block.x.fall"),
            };
            for (block, sound, players_only, at_y) in [
                ("minecraft:glass", sound.clone(), false, None),
                ("minecraft:white_wool", sound.clone(), true, None),
                (
                    "minecraft:red_wool",
                    BlockSoundType {
                        volume: -3.0,
                        pitch: 0.25,
                        ..sound.clone()
                    },
                    false,
                    None,
                ),
                (
                    "minecraft:blue_wool",
                    BlockSoundType {
                        step: key("block.amethyst_block.step"),
                        break_sound: key("mymod:missing"),
                        ..sound.clone()
                    },
                    false,
                    None,
                ),
                ("minecraft:snow", sound, false, Some(64)),
            ] {
                let key = r.block_key(block);
                r.block_part(
                    key,
                    Answer {
                        sound,
                        players_only,
                        at_y,
                    },
                );
            }
            let planks = r.block_key("minecraft:oak_planks");
            r.block_part(planks, EditUnusedPlace(key("mymod:unused_place")));
            let wool = r.block_key("minecraft:green_wool");
            r.block_part(wool, PanicAfterEdit);
        }
    }
    struct UnrelatedSoundMod;
    #[mod_api::__subscriptions]
    impl Hooks for UnrelatedSoundMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let wool = r.block_key("minecraft:black_wool");
            let key = SoundEventKey::parse("other_mod:audible").unwrap();
            r.block_part(
                wool,
                Answer {
                    sound: BlockSoundType {
                        identity: crate::BlockSoundTypeIdentity::Other,
                        volume: 2.0,
                        pitch: 1.5,
                        break_sound: key.clone(),
                        step: key.clone(),
                        place: key.clone(),
                        hit: key.clone(),
                        fall: key,
                    },
                    players_only: false,
                    at_y: None,
                },
            );
        }
    }
    let mods: crate::Mods = vec![
        ModEntry::new("sound_type_test", Box::new(SoundTypeMod)),
        ModEntry::new("unrelated_sound_test", Box::new(UnrelatedSoundMod)),
    ]
    .into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
