#!/usr/bin/env python3
"""Real first-list and locator packet parity for saved effects and equipment."""
import argparse
import json
from os import pathsep
from pathlib import Path
import subprocess

import test_command_exec_parity as commands
import test_locator_bar_parity as locator


NAME = 'effect-waypoint-parity'
HIDE = ('attributes[{id:"minecraft:waypoint_transmit_range"}]'
        '.modifiers[{id:"minecraft:effect.waypoint_transmit_range_hide"}]')
SAVED = 'active_effects[{id:"minecraft:invisibility"'
FUNCTION = {'parity:refresh_waypoint': '''effect give @s minecraft:invisibility 30 0 true
effect give @s minecraft:invisibility 60 0 true
effect clear @s minecraft:invisibility
'''}
SCRIPT = [
    ('watcher', [], ['chat on', 'join Alice']),
    ('loaded', [], ['join Bob']),
    ('positioned', [], ['cmd Bob tp @s 6.5 -60 0.5']),
    ('promotion-input', [], ['send-cmd Alice execute as Bob if data entity @s '
                            + SAVED + ',hidden_effect:{duration:-1}}] unless data entity @s ' + HIDE
                            + ' run tellraw Alice {"text":"promotion-input-confirmed"}']),
    ('saved-promotion', [], []),
    ('promotion-output', [], ['send-cmd Alice execute as Bob if data entity @s ' + SAVED + ',duration:-1}] '
                             'unless data entity @s active_effects[0].hidden_effect '
                             'if data entity @s ' + HIDE + ' run tellraw Alice {"text":"promotion-output-confirmed"}']),
    ('clear-promoted', [], ['cmd Bob effect clear @s minecraft:invisibility']),
    ('pumpkin-default', [], ['cmd Bob item replace entity @s armor.head with minecraft:carved_pumpkin']),
    ('pumpkin-cleared', [], ['cmd Bob item replace entity @s armor.head with '
                            'minecraft:carved_pumpkin[!minecraft:attribute_modifiers]']),
    ('same-tick-refresh', [], ['cmd Bob function parity:refresh_waypoint']),
]
# A scheduled block runs after placement packets and before the player's equipment tick.
# Save the first real /waypoint list result once Bob is present, without a client race.
JOIN_PROBE = ('execute if entity Bob unless score #first joinobs matches 1.. '
              'store result score #first joinobs run waypoint list')
JOIN_SCRIPT = [
    ('watcher', [], ['chat on', 'join Alice']),
    ('saved-head-setup', [], ['cmd Alice scoreboard objectives add joinobs dummy',
                              'cmd Alice setblock 0 -59 2 minecraft:repeating_command_block'
                              '{auto:1b,TrackOutput:0b,Command:' + json.dumps(JOIN_PROBE) + '}']),
    ('saved-head-join', [], ['join Bob']),
    ('saved-head-list', [], ['cmd Alice scoreboard players get #first joinobs',
                             'cmd Alice execute store result score #later joinobs run waypoint list',
                             'cmd Alice scoreboard players get #later joinobs',
                             'send-cmd Alice execute if items entity Bob armor.head minecraft:carved_pumpkin '
                             'run tellraw Alice {"text":"saved-head-confirmed"}']),
    ('saved-head-clear', [], ['cmd Bob item replace entity @s armor.head with minecraft:air']),
    ('saved-head-restore', [], ['cmd Bob item replace entity @s armor.head with minecraft:carved_pumpkin']),
]
JOIN_STEPS = {'saved-head-join', 'saved-head-clear', 'saved-head-restore'}
CHECKED = {'saved-promotion', 'clear-promoted', 'pumpkin-default', 'pumpkin-cleared', 'same-tick-refresh'}


def recording(lines, steps):
    return [line for line in lines if (parts := line.split())[:1] == ['Alice']
            and len(parts) >= 4 and parts[1].removeprefix('step=') in steps
            and parts[2] in ('track', 'untrack', 'update') and parts[3] == 'Bob']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--work-dir', type=Path)
    args = parser.parse_args()
    name = args.work_dir.resolve() if args.work_dir else NAME
    binary = args.binary.resolve() if args.binary else None
    lock, build, java, native, client = locator.sides(name, binary)
    world = locator.generate_world(java, build)
    # Two servers load the identical synthetic save through PlayerDataStorage.
    playerdata = world / 'players' / 'data'
    playerdata.mkdir(parents=True, exist_ok=True)
    classpath = java[java.index('-cp') + 1]
    source = locator.lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/WaypointEffectSave.java'

    def write_save(case):
        with (build / f'write-{case}-fixture.log').open('w') as log:
            subprocess.run(['java', '-cp', classpath, source, playerdata, case],
                           stdout=log, stderr=subprocess.STDOUT, check=True)

    write_save('effect')
    server_jar = Path(classpath.split(pathsep)[0])
    commands.install_datapack_fixtures(world, {}, FUNCTION, server_jar)

    def run(label, server):
        lines = locator.run_side(label, server, client, build, world, script=SCRIPT,
                                 step_ticks={'saved-promotion': 200})
        assert not any(' error ' in line or 'disconnected' in line for line in lines), label
        assert any(line.startswith('Alice step=positioned ') and ' Bob ' in line
                   and line.split()[2] in ('track', 'update') for line in lines), label
        assert any('promotion-input-confirmed' in line for line in lines), label
        return recording(lines, CHECKED), any('promotion-output-confirmed' in line for line in lines)

    reference, vanilla_promoted = run('vanilla', java)
    assert vanilla_promoted
    expected = [
        ('step=saved-promotion', 'untrack'),
        ('step=clear-promoted', 'track'),
        ('step=pumpkin-default', 'untrack'),
        ('step=pumpkin-cleared', 'track'),
        ('step=same-tick-refresh', 'untrack'),
        ('step=same-tick-refresh', 'track'),
    ]
    assert [tuple(line.split()[1:3]) for line in reference] == expected, reference
    actual, native_promoted = run('sunrise', native)
    write_save('pumpkin')

    def run_join(label, server):
        lines = locator.run_side(label + '-saved-head', server, client, build, world, script=JOIN_SCRIPT)
        assert not any(' error ' in line or 'disconnected' in line for line in lines), label
        assert any('saved-head-confirmed' in line for line in lines), label
        scores = []
        for line in lines:
            prefix = 'Alice step=saved-head-list chat '
            if line.startswith(prefix):
                message = json.loads(line.removeprefix(prefix))
                if isinstance(message, dict) and message.get('translate') == 'commands.scoreboard.players.get.success':
                    scores.append(message['with'][:2])
        return {'scores': scores, 'packets': recording(lines, JOIN_STEPS)}

    vanilla_join = run_join('vanilla', java)
    assert vanilla_join['scores'] == [['#first', 2], ['#later', 1]], vanilla_join
    assert [tuple(line.split()[1:3]) for line in vanilla_join['packets']] == [
        ('step=saved-head-clear', 'track'),
        ('step=saved-head-restore', 'untrack'),
    ], vanilla_join
    sunrise_join = run_join('sunrise', native)
    passed = native_promoted and actual == reference and sunrise_join == vanilla_join
    locator.lab.write_json(build / 'report.json', {'minecraft': lock['minecraft']['version'],
                           'pass': passed, 'promotion': native_promoted,
                           'vanilla_packets': reference, 'sunrise_packets': actual,
                           'vanilla_join': vanilla_join, 'sunrise_join': sunrise_join})
    print(f'Effect waypoint packets: {"PASS" if passed else "FAIL"} — {build / "report.json"}')
    return int(not passed)


if __name__ == '__main__':
    raise SystemExit(main())
