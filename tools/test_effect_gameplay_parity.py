#!/usr/bin/env python3
"""Compare player effect attributes and the lifetime of a saved hidden effect."""
import sys

import test_command_exec_parity as runner
from command_exec_corpus import cmd, setup


FUNCTIONS = {
    'parity:clear_night_vision': '''effect give @s minecraft:night_vision 30
attribute @s minecraft:max_health base set 16.5
execute unless data entity @s {Health:20.0f} run return 2
effect clear @s minecraft:night_vision
execute unless data entity @s {Health:16.5f} run return fail
return 1
''',
    'parity:update_night_vision': '''effect give @s minecraft:night_vision 30
attribute @s minecraft:max_health base set 12.5
execute unless data entity @s {Health:16.5f} run return 2
effect give @s minecraft:night_vision 60
execute unless data entity @s {Health:12.5f} run return fail
return 1
''',
}


def cases():
    yield 'first waypoint list', [
        cmd('/effect give @s minecraft:invisibility infinite'),
        cmd('/attribute @s minecraft:waypoint_transmit_range get', expect_result=0),
        cmd('/waypoint list', expect_result=1),
        {'op': 'tick', 'n': 1},
        cmd('/waypoint list', expect_result=0),
        cmd('/effect clear @s minecraft:invisibility'),
        cmd('/waypoint list', expect_result=1),
    ]
    yield 'modifierless effect refresh', [
        cmd('/function parity:clear_night_vision', expect_result=1),
        cmd('/function parity:update_night_vision', expect_result=1),
        *setup('/effect clear @s minecraft:night_vision',
               '/attribute @s minecraft:max_health base reset'),
    ]
    yield 'player absorption', [
        *setup('/attribute @s minecraft:max_absorption base set 2.5'),
        cmd('/effect give @s minecraft:absorption infinite 1'),
        cmd('/data get entity @s AbsorptionAmount', expect_result=8),
        cmd('/effect clear @s minecraft:absorption'),
        cmd('/data get entity @s AbsorptionAmount', expect_result=2),
        cmd('/effect give @s minecraft:invisibility infinite'),
        cmd('/data get entity @s AbsorptionAmount', expect_result=2),
        cmd('/effect clear @s minecraft:invisibility'),
        cmd('/data get entity @s AbsorptionAmount', expect_result=2),
    ]
    yield 'player waypoint', [
        *setup('/attribute @s minecraft:waypoint_transmit_range base set 5.25'),
        {'op': 'tick', 'n': 1},
        cmd('/attribute @s minecraft:waypoint_transmit_range get', expect_result=5),
        cmd('/waypoint list', expect_result=1),
        cmd('/effect give @s minecraft:invisibility infinite'),
        cmd('/attribute @s minecraft:waypoint_transmit_range get', expect_result=0),
        cmd('/attribute @s minecraft:waypoint_transmit_range modifier value get '
            'minecraft:effect.waypoint_transmit_range_hide', expect_result=-1),
        cmd('/execute if data entity @s attributes[{id:"minecraft:waypoint_transmit_range"}]'
            '.modifiers[{id:"minecraft:effect.waypoint_transmit_range_hide"}]', expect_result=1),
        {'op': 'tick', 'n': 1},
        cmd('/waypoint list', expect_result=0),
        cmd('/effect clear @s minecraft:invisibility'),
        cmd('/attribute @s minecraft:waypoint_transmit_range get', expect_result=5),
        cmd('/waypoint list', expect_result=1),
    ]
    yield 'saved hidden zero duration', [
        *setup('/forceload add 0 0', '/fill 4 199 4 12 199 12 stone', '/tp @s 8.5 200 8.5'),
        {'op': 'settle'},
        cmd('/summon minecraft:pig 6.5 200 6.5 {NoAI:1b,Tags:["effect_zero"],active_effects:'
            '[{id:"minecraft:speed",amplifier:1,duration:2,hidden_effect:{duration:0}}]}',
            expect_result=1),
        cmd('/execute if data entity @e[tag=effect_zero,limit=1] active_effects[0].hidden_effect', expect_result=1),
        {'op': 'tick', 'n': 1},
        cmd('/execute if data entity @e[tag=effect_zero,limit=1] active_effects[0].hidden_effect', expect_result=1),
        cmd('/execute unless data entity @e[tag=effect_zero,limit=1] '
            'active_effects[0].hidden_effect{duration:-1}', expect_result=1),
        {'op': 'tick', 'n': 1},
        cmd('/execute if entity @e[tag=effect_zero,limit=1] unless data entity '
            '@e[tag=effect_zero,limit=1] active_effects[0]', expect_result=1),
    ]


def main():
    runner.corpus_module.groups = lambda: [('setup', runner.corpus_module.group_setup()), *cases()]
    runner.corpus_module.function_return_fixtures = lambda _groups: FUNCTIONS
    if '--work-dir' not in sys.argv:
        sys.argv += ['--work-dir', str(runner.lab.LOCAL / 'effect-gameplay-parity')]
    return runner.main()


if __name__ == '__main__':
    sys.exit(main())
