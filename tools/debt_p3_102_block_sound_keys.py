#!/usr/bin/env python3
"""Migrate the block-sound callers from text fields to resolved sound event keys."""
from pathlib import Path


def change(path, *pairs):
    file = Path('crates') / path
    text = file.read_text()
    for old, new in pairs:
        if text.count(old) != 1:
            raise SystemExit(f'{file}: expected one occurrence of {old!r}')
        text = text.replace(old, new)
    file.write_text(text)


change('sunrise-client/src/game/mobs/sniffers.rs', ('self.audio.play(&hit,', 'self.audio.play(hit.as_str(),'))
change('sunrise-client/src/game/network/remote/fall_sound.rs', ('&sound.fall,', 'sound.fall.as_str(),'))
change('sunrise-client/src/game/network/remote/interaction_actions.rs', ('&sound.place,', 'sound.place.as_str(),'))
change('sunrise-client/src/game/network/remote/tick.rs', ('&sounds.step,', 'sounds.step.as_str(),'))
change('sunrise-client/src/game/network/lan_actions/guest_body.rs',
       ('                        &sound.fall,', '                        sound.fall.as_str(),'),
       ('LevelSound::new(&sound.fall,', 'LevelSound::new(sound.fall.as_str(),'))
change('sunrise-client/src/game/network/lan_actions/effects.rs',
       ('name.trim_start_matches("minecraft:").to_owned()', 'name.as_str().trim_start_matches("minecraft:").to_owned()'))
change('sunrise-client/src/game/native_hooks/play_hit_sound.rs',
       ('let sounds = self.res.facts().get(state).sounds.clone();', 'let sounds = &self.res.facts().get(state).sounds;'),
       ('&sounds.hit,', 'sounds.hit.as_str(),'))
change('sunrise-client/src/mobs/equine.rs',
       ('    let step = sounds\n        .step\n        .strip_prefix("minecraft:")\n        .unwrap_or(&sounds.step);\n'
        '    let place = sounds\n        .place\n        .strip_prefix("minecraft:")\n        .unwrap_or(&sounds.place);\n'
        '    match step {\n'
        '        "block.wood.step" => place == "block.wood.place",\n'
        '        "block.nether_wood.step"\n'
        '        | "block.stem.step"\n'
        '        | "block.cherry_wood.step"\n'
        '        | "block.bamboo_wood.step" => true,',
        '    match sounds.step.as_str() {\n'
        '        "minecraft:block.wood.step" => sounds.place.as_str() == "minecraft:block.wood.place",\n'
        '        "minecraft:block.nether_wood.step"\n'
        '        | "minecraft:block.stem.step"\n'
        '        | "minecraft:block.cherry_wood.step"\n'
        '        | "minecraft:block.bamboo_wood.step" => true,'))
change('sunrise-client/src/mobs/spawn_rules.rs',
       ('use crate::blocks::{RenderShape, Sounds, StateFacts};', 'use crate::blocks::{RenderShape, StateFacts};'),
       ('let sound: Arc<str> = Arc::from("");', 'let sound = mod_api::SoundEventKey::parse("").unwrap();'),
       ('sounds: Sounds {', 'sounds: mod_api::BlockSoundType {'))
change('sunrise-client/src/native_client_hooks_tests.rs',
       ('sound.step = "rules_mod:block.rules.step".into();', 'sound.step = mod_api::SoundEventKey::parse("rules_mod:block.rules.step").unwrap();'))
change('sunrise-client/src/game/modding/native_block_tables.rs',
       ('Some(sunrise_client::block_sounds::own(\n            &self.res.facts().get(state).sounds,\n        ))',
        'Some(self.res.facts().get(state).sounds.clone())'))
