#!/usr/bin/env python3
"""The scripted routes of tools/client_packet_parity.py: a test course built once into the world, and for each
route the server console commands that put the player at its start and the input it plays, written as an
input recording (tools/input_recording.py), exactly what a person's recording would hold.

  client_packet_routes.py write routes.jsonl            # every route, one marker each
  client_packet_routes.py write walk.jsonl --route walk

The course is a stone floor at y 100 with one 8 x 40 cell per route (CELLS), so a route never meets another's
blocks; the terrain route walks the generated world beside it. Mouse moves are raw counts: at sensitivity 0.5
one count turns the view 0.15 degrees (MouseHandler.turnPlayer).
"""
import argparse
import re
from pathlib import Path

try:
    from . import input_recording
except ImportError:
    import input_recording

PLAYER = "Steve"
# The opponent of the PvP routes: tools/client_packet_parity.py joins it as sunrise-bots --behaviour stand.
OPPONENT = "Opp0"
OPPONENT_PARKED = f"tp {OPPONENT} 76.5 101 180.5 0 0"
FLOOR = 100
COLUMNS = 7
# Floor beyond the cells, so a route that turns off its own cell never falls.
MARGIN = 24
# A marker's chat command takes this long; a route's own input starts after it.
START = input_recording.MARKER_SECONDS
# Window and GUI scale both clients run with: the inventory route's mouse moves are in these pixels.
WIDTH, HEIGHT, GUI_SCALE = 854, 480, 2
TARGET = ('summon minecraft:husk {x} {y} {z} {{NoAI:1b,Silent:1b,PersistenceRequired:1b,Health:1000f,'
          'attributes:[{{id:"minecraft:max_health",base:1000.0}},'
          '{{id:"minecraft:knockback_resistance",base:1.0}}]}}')


class Events:
    def __init__(self):
        self.list = []

    def key(self, t, code, hold=0.08):
        self.list += [{"t": START + t, "type": "key", "code": code, "down": True},
                      {"t": START + t + hold, "type": "key", "code": code, "down": False}]

    def button(self, t, code="BTN_LEFT", hold=0.08):
        self.list += [{"t": START + t, "type": "button", "code": code, "down": True},
                      {"t": START + t + hold, "type": "button", "code": code, "down": False}]

    def turn(self, t, dx, dy, seconds=0.25):
        steps = max(1, int(seconds / 0.008))
        sent_x = sent_y = 0
        for i in range(1, steps + 1):
            x, y = round(dx * i / steps), round(dy * i / steps)
            if x != sent_x or y != sent_y:
                self.list.append({"t": START + t + i * 0.008, "type": "rel", "dx": x - sent_x, "dy": y - sent_y})
            sent_x, sent_y = x, y

    def wheel(self, t, dy):
        self.list.append({"t": START + t, "type": "wheel", "dy": dy})


def cell(index):
    """The north-west corner (x, z) of a route's course cell."""
    return (index % COLUMNS) * 8, (index // COLUMNS) * 40


def start(ox, oz, y=FLOOR + 1, yaw=0, pitch=0, dz=2.5):
    return f"tp {PLAYER} {ox + 3.5} {y} {oz + dz} {yaw} {pitch}"


def slot_px(menu_slot):
    """InventoryMenu's slot centre in window pixels at WIDTH x HEIGHT, GUI scale GUI_SCALE."""
    left = (WIDTH // GUI_SCALE - 176) // 2
    top = (HEIGHT // GUI_SCALE - 166) // 2
    if 36 <= menu_slot <= 44:
        x, y = 8 + (menu_slot - 36) * 18, 142
    else:
        x, y = 8 + (menu_slot - 9) % 9 * 18, 84 + (menu_slot - 9) // 9 * 18
    return (left + x + 8) * GUI_SCALE, (top + y + 8) * GUI_SCALE


def give(slot, item):
    return f"item replace entity {PLAYER} {slot} with minecraft:{item}"


ROUTES = {}


def route(name, index, build=(), setup=(), during=(), opponent=None, pitch=0, shots=(), expect=None):
    """A route: its cell, blocks, console setup, console commands at times during it (seconds, as its input's),
    and where the opponent stands (blocks ahead of the player), if it fights one. `shots` are (seconds, label)
    screenshots of the window taken while it plays; `expect(ticks, shots)` lists what the route's run did not do
    (client_packet_diff makes each a problem of the run), `shots` being {label: png path}."""
    def wrap(fn):
        ROUTES[name] = {"index": index, "build": build, "setup": setup, "events": fn, "opponent": opponent, "pitch": pitch,
                        "during": [(START + t, c) for t, c in during], "expect": expect,
                        "shots": [(START + t, label) for t, label in shots]}
        return fn
    return wrap


def build_for(index, *commands):
    """Build commands with {x0}, {x1}, {z0} and {F} (the floor), sums like `{F+1}` included, filled in for the route's cell."""
    ox, oz = cell(index)
    names = {"x0": ox, "x1": ox + 7, "z0": oz, "F": FLOOR}
    return [re.sub(r"\{([^}]+)\}", lambda m: str(eval(m.group(1), {}, names)), c) for c in commands]


@route("walk", 0)
def walk(e):
    e.key(0.0, "KEY_W", 2.0)
    e.key(2.4, "KEY_W", 2.0)
    e.turn(2.6, 400, 0, 1.5)
    e.turn(4.8, 0, 200, 0.5)
    e.turn(5.6, 0, -200, 0.5)


@route("sprint", 1)
def sprint(e):
    e.key(0.0, "KEY_LEFTCTRL", 2.5)
    e.key(0.05, "KEY_W", 3.5)


@route("sprint_jump", 2)
def sprint_jump(e):
    e.key(0.0, "KEY_LEFTCTRL", 3.6)
    e.key(0.05, "KEY_W", 3.6)
    for t in (0.5, 1.1, 1.7, 2.3, 2.9):
        e.key(t, "KEY_SPACE", 0.1)


@route("strafe", 3)
def strafe(e):
    e.key(0.0, "KEY_W", 2.0)
    e.key(0.0, "KEY_A", 1.0)
    e.key(1.0, "KEY_D", 1.0)
    e.key(2.3, "KEY_A", 0.5)
    e.key(3.0, "KEY_D", 0.5)
    e.key(3.8, "KEY_S", 1.0)


@route("sneak_edge", 4, build=build_for(4, "fill {x0} {F+1} {z0} {x1} {F+3} {z0+6} minecraft:stone"))
def sneak_edge(e):
    e.key(0.0, "KEY_LEFTSHIFT", 7.8)
    e.key(0.3, "KEY_W", 4.2)
    e.key(4.7, "KEY_A", 1.0)
    e.key(5.9, "KEY_D", 1.5)
    e.key(8.2, "KEY_S", 0.4)


@route("stairs_slabs", 5, build=build_for(
    5, "fill {x0} {F+1} {z0+5} {x1} {F+1} {z0+5} minecraft:smooth_stone_slab[type=bottom]",
    "fill {x0} {F+1} {z0+6} {x1} {F+1} {z0+8} minecraft:stone",
    "fill {x0} {F+2} {z0+7} {x1} {F+2} {z0+7} minecraft:stone_stairs[facing=south]",
    "fill {x0} {F+2} {z0+8} {x1} {F+2} {z0+9} minecraft:stone",
    "fill {x0} {F+1} {z0+9} {x1} {F+1} {z0+9} minecraft:stone",
    "fill {x0} {F+3} {z0+9} {x1} {F+3} {z0+9} minecraft:smooth_stone_slab[type=bottom]"))
def stairs_slabs(e):
    e.key(0.0, "KEY_W", 4.0)
    e.key(2.0, "KEY_SPACE", 0.1)
    e.key(4.5, "KEY_S", 2.5)


@route("ladder", 6, build=build_for(
    6, "fill {x0} {F+1} {z0+5} {x1} {F+6} {z0+5} minecraft:stone",
    "fill {x0+3} {F+1} {z0+4} {x0+3} {F+6} {z0+4} minecraft:ladder[facing=north]"))
def ladder(e):
    e.key(0.0, "KEY_W", 2.0)
    e.key(2.0, "KEY_LEFTSHIFT", 0.8)
    e.key(3.2, "KEY_W", 3.0)
    e.key(6.6, "KEY_S", 0.3)


@route("lava_edge", 7, build=build_for(
    7, "fill {x0} {F-2} {z0+4} {x1} {F} {z0+9} minecraft:stone",
    "fill {x0+1} {F} {z0+5} {x0+6} {F} {z0+8} minecraft:lava"),
       setup=[f"effect give {PLAYER} minecraft:fire_resistance 60 0 true"])
def lava_edge(e):
    e.key(0.0, "KEY_LEFTSHIFT", 2.7)
    e.key(0.2, "KEY_W", 2.3)
    e.key(3.0, "KEY_W", 1.0)
    e.key(4.4, "KEY_S", 1.8)
    e.key(4.4, "KEY_SPACE", 1.8)


@route("swim", 8, build=build_for(
    8, "fill {x0} {F-4} {z0+3} {x1} {F} {z0+15} minecraft:stone",
    "fill {x0+1} {F-3} {z0+4} {x0+6} {F} {z0+14} minecraft:water"))
def swim(e):
    e.key(0.0, "KEY_W", 1.0)
    e.key(1.0, "KEY_SPACE", 1.0)
    e.key(2.2, "KEY_LEFTCTRL", 2.0)
    e.key(2.25, "KEY_W", 1.95)
    e.turn(2.4, 0, 250, 0.4)
    e.turn(3.4, 0, -250, 0.4)
    e.key(4.4, "KEY_W", 2.2)
    e.key(4.4, "KEY_SPACE", 2.2)


@route("ice", 9, build=build_for(9, "fill {x0} {F} {z0+3} {x1} {F} {z0+38} minecraft:packed_ice"))
def ice(e):
    e.key(0.0, "KEY_LEFTCTRL", 2.0)
    e.key(0.05, "KEY_W", 1.95)
    e.key(3.5, "KEY_W", 1.0)
    e.turn(3.5, 600, 0, 1.0)


@route("soul_sand", 10, build=build_for(10, "fill {x0} {F} {z0+3} {x1} {F} {z0+20} minecraft:soul_sand"))
def soul_sand(e):
    e.key(0.0, "KEY_W", 2.5)
    e.key(2.8, "KEY_LEFTCTRL", 1.5)
    e.key(2.85, "KEY_W", 1.45)
    e.key(3.5, "KEY_SPACE", 0.1)


@route("break_block", 11, build=build_for(
    11, "setblock {x0+3} {F+1} {z0+4} minecraft:stone", "setblock {x0+3} {F+2} {z0+4} minecraft:dirt"),
       setup=[give("hotbar.1", "iron_pickaxe")])
def break_block(e):
    e.button(0.3, "BTN_LEFT", 1.5)
    e.key(2.2, "KEY_2")
    e.turn(2.5, 0, 233, 0.3)
    e.button(3.0, "BTN_LEFT", 1.0)
    e.turn(4.3, 0, -600, 0.3)
    e.button(4.8)


@route("place_block", 12, setup=[give("hotbar.0", "stone 64")])
def place_block(e):
    e.turn(0.0, 0, 400, 0.2)
    for t in (0.4, 0.9, 1.4, 1.9):
        e.button(t, "BTN_RIGHT")
    e.key(2.3, "KEY_LEFTSHIFT", 3.4)
    e.button(2.6, "BTN_RIGHT")
    e.key(3.0, "KEY_S", 2.5)
    for i in range(7):
        e.button(3.2 + i * 0.35, "BTN_RIGHT")


@route("eat", 13, setup=[give("hotbar.0", "golden_apple 4")])
def eat(e):
    e.button(0.3, "BTN_RIGHT", 1.9)
    e.button(2.6, "BTN_RIGHT", 0.6)


@route("bow", 14, setup=[give("hotbar.0", "bow"), give("inventory.0", "arrow 16")])
def bow(e):
    e.turn(0.0, 0, -66, 0.2)
    e.button(0.3, "BTN_RIGHT", 1.2)
    e.button(1.9, "BTN_RIGHT", 0.3)
    e.button(2.6, "BTN_RIGHT", 1.2)


@route("shield", 15, setup=[give("weapon.offhand", "shield")])
def shield(e):
    e.button(0.3, "BTN_RIGHT", 1.2)
    e.button(1.8, "BTN_RIGHT", 1.7)
    e.key(2.0, "KEY_W", 1.2)


@route("hit_mob", 16, setup=["{target}", give("hotbar.0", "diamond_sword")])
def hit_mob(e):
    for t in (0.3, 1.0, 1.7, 2.4):
        e.button(t)
    e.key(2.8, "KEY_LEFTCTRL", 0.5)
    e.key(2.8, "KEY_W", 0.5)
    e.button(3.1)
    e.key(3.6, "KEY_S", 0.5)


@route("inventory", 17, setup=[give("hotbar.0", "diamond_sword"), give("hotbar.1", "cooked_beef 16"),
                                give("inventory.0", "golden_apple 8")])
def inventory(e):
    centre = (WIDTH // 2, HEIGHT // 2)
    hot0, hot1, main0 = slot_px(36), slot_px(37), slot_px(9)
    e.key(0.3, "KEY_E")
    at = centre
    for t, target, action in ((0.8, hot0, "click"), (1.5, main0, "click"), (2.2, hot0, "click"),
                              (2.9, hot1, "shift"), (3.6, main0, "right")):
        e.turn(t, target[0] - at[0], target[1] - at[1], 0.15)
        at = target
        if action == "shift":
            e.key(t + 0.2, "KEY_LEFTSHIFT", 0.35)
            e.button(t + 0.35)
        else:
            e.button(t + 0.4, "BTN_RIGHT" if action == "right" else "BTN_LEFT")
    e.turn(4.2, hot0[0] - at[0], hot0[1] - at[1], 0.15)
    e.button(4.6)
    e.key(5.0, "KEY_E")
    e.key(5.6, "KEY_E")
    e.key(6.3, "KEY_ESC")


@route("drop", 18, setup=[give("hotbar.0", "cobblestone 16"), give("hotbar.1", "dirt 16")])
def drop(e):
    e.key(0.3, "KEY_Q")
    e.key(0.8, "KEY_Q")
    e.key(1.3, "KEY_LEFTCTRL", 0.4)
    e.key(1.45, "KEY_Q")
    e.key(2.0, "KEY_2")
    e.key(2.4, "KEY_LEFTCTRL", 0.4)
    e.key(2.55, "KEY_Q")


@route("hotbar_scroll", 19, setup=[give(f"hotbar.{i}", item) for i, item in enumerate(
    ["diamond_sword", "stone 8", "bow", "golden_apple", "shield", "torch 4", "iron_pickaxe", "bread 3",
     "cobblestone 8"])])
def hotbar_scroll(e):
    for i in range(5):
        e.wheel(0.3 + i * 0.25, -1)
    for i in range(3):
        e.wheel(1.7 + i * 0.25, 1)
    for i, k in enumerate("357924681"):
        e.key(2.7 + i * 0.25, f"KEY_{k}")
    e.key(5.2, "KEY_5")
    e.turn(5.2, 0, -400, 0.2)
    e.button(5.6)


@route("terrain", 20, setup=["{terrain}"])
def terrain(e):
    e.key(0.0, "KEY_LEFTCTRL", 4.0)
    e.key(0.05, "KEY_W", 4.5)
    for t in (0.6, 1.3, 2.0, 2.7, 3.4):
        e.key(t, "KEY_SPACE", 0.1)
    e.turn(1.0, 300, 0, 1.0)
    e.turn(2.2, -600, 0, 1.2)


# F3+2 toggles the FPS/TPS charts (the client's debug subscription); op makes the server stream the samples.
CHARTS_ON, CHARTS_OFF = 0.8, 2.6  # when F3+2 is pressed, seconds into the route (KEY_2 lands 0.1 s later)
# ClientDebugSubscriber.tick's packets: the set {dedicated_server_tick_time} (id 0), then the empty set.
SUBSCRIBE, CLEAR = "0100", "00"


def expect_debug_fps_charts(ticks, _shots):
    """The wire stream subscribes once, then clears once. Check receipt in the client fixture."""
    requests = [payload for tick in ticks for _, name, payload in tick["raw"]
                if name == "debug_subscription_request"]
    if requests != [SUBSCRIBE, CLEAR]:
        return [f"expected the subscribe {SUBSCRIBE} then the clear {CLEAR}, saw {requests}"]
    return []


@route("debug_fps_charts", 28, setup=[f"op {PLAYER}"], expect=expect_debug_fps_charts)
def debug_fps_charts(e):
    e.key(0.3, "KEY_F3", 0.1)
    for t in (CHARTS_ON, CHARTS_OFF):
        e.key(t, "KEY_F3", 0.3)
        e.key(t + 0.1, "KEY_2")
    e.key(3.3, "KEY_F3", 0.1)


SWORD = give("hotbar.0", "diamond_sword")
KNOCK = f"damage {PLAYER} 1 minecraft:player_attack by {OPPONENT}"


@route("pvp_hits", 21, setup=[SWORD], opponent=3.0)
def pvp_hits(e):
    for t in (0.3, 1.0, 1.7, 2.4, 3.1):
        e.button(t)


@route("pvp_spam", 22, setup=[SWORD], opponent=3.0)
def pvp_spam(e):
    for i in range(20):
        e.button(0.3 + i * 0.1, hold=0.04)


@route("pvp_wtap", 23, setup=[SWORD], opponent=7.0)
def pvp_wtap(e):
    e.key(0.0, "KEY_LEFTCTRL", 2.6)
    e.key(0.05, "KEY_W", 0.95)
    e.button(0.9)
    e.key(1.15, "KEY_W", 0.55)
    e.button(1.6)
    e.key(1.85, "KEY_W", 0.75)
    e.button(2.3)


@route("pvp_crit", 24, setup=[SWORD], opponent=3.0, pitch=20)
def pvp_crit(e):
    for t in (0.3, 1.6, 2.9):
        e.key(t, "KEY_SPACE", 0.1)
        e.button(t + 0.45)


@route("pvp_knockback", 25, setup=[SWORD], opponent=3.0,
       during=[(0.5, KNOCK), (2.0, KNOCK), (4.2, KNOCK), (5.75, KNOCK)])
def pvp_knockback(e):
    e.key(1.5, "KEY_W", 1.5)
    e.key(3.5, "KEY_LEFTCTRL", 1.5)
    e.key(3.55, "KEY_W", 1.45)
    e.key(5.6, "KEY_SPACE", 0.1)


@route("pvp_shield", 26, setup=[SWORD, give("weapon.offhand", "shield")], opponent=3.0,
       during=[(0.8, KNOCK), (1.4, KNOCK)])
def pvp_shield(e):
    e.button(0.3, "BTN_RIGHT", 1.7)
    e.button(2.3)


@route("pvp_strafe", 27, setup=[SWORD], opponent=3.0)
def pvp_strafe(e):
    e.key(0.2, "KEY_A", 1.2)
    e.button(0.4)
    e.button(1.1)
    e.key(1.6, "KEY_D", 1.2)
    e.button(1.9)
    e.button(2.6)


# By cell index. Lava sets the player alight; the swim route right after it puts the fire out.
ORDER = sorted(ROUTES, key=lambda name: ROUTES[name]["index"])
assert [ROUTES[name]["index"] for name in ORDER] == list(range(len(ORDER)))
PVP = [name for name in ORDER if name.startswith("pvp_")]


def course_bounds():
    """(x0, z0, x1, z1): every block the course builds, its outer floor included."""
    rows = (len(ORDER) + COLUMNS - 1) // COLUMNS
    return -MARGIN, -MARGIN, COLUMNS * 8 + MARGIN - 1, rows * 40 + MARGIN - 1


def course_commands():
    """What the world gets once: every cell's floor, cleared air above it, and the routes' own blocks."""
    x0, z0, x1, z1 = course_bounds()
    out = [f"fill {x0} {FLOOR} {z0} {x1} {FLOOR} {z1} minecraft:stone"]
    for name in ORDER:
        ox, oz = cell(ROUTES[name]["index"])
        out += [f"fill {ox} {FLOOR + 1} {oz} {ox + 7} {FLOOR + 30} {oz + 39} minecraft:air",
                f"fill {ox} {FLOOR} {oz} {ox + 7} {FLOOR} {oz + 39} minecraft:stone"]
    for name in ORDER:
        out += ROUTES[name]["build"]
    return out


def reset_commands(name, terrain_spot):
    """The console commands that put the player at a route's start, as both clients get it."""
    spec = ROUTES[name]
    ox, oz = cell(spec["index"])
    first = [f"kill @e[type=!minecraft:player]", f"clear {PLAYER}", f"effect clear {PLAYER}", f"deop {PLAYER}",
             f"effect give {PLAYER} minecraft:saturation 1 20 true",
             f"effect give {PLAYER} minecraft:instant_health 1 5 true", f"gamemode survival {PLAYER}"]
    setup = []
    for c in spec["setup"]:
        if c == "{target}":
            c = TARGET.format(x=ox + 3.5, y=FLOOR + 1, z=oz + 5.0)
        elif c == "{terrain}":
            x, y, z = terrain_spot
            c = f"tp {PLAYER} {x + 0.5} {y} {z + 0.5} 0 0"
        setup.append(c)
    spot = [] if any(c.startswith("tp ") for c in setup) else [start(ox, oz, y=FLOOR + 4 if name == "sneak_edge"
                                                                      else FLOOR + 1, pitch=spec["pitch"])]
    if spec["opponent"] is None:
        return first + [OPPONENT_PARKED] + setup + spot
    opponent = [f"tp {OPPONENT} {ox + 3.5} {FLOOR + 1} {oz + 2.5 + spec['opponent']} 180 0",
                f"effect give {OPPONENT} minecraft:resistance 1000 4 true",
                f"effect give {OPPONENT} minecraft:instant_health 1 5 true"]
    return first + opponent + setup + spot


def recording(names):
    events, t = [], 0.0
    for name in names:
        e = Events()
        ROUTES[name]["events"](e)
        events.append({"t": t, "type": "marker", "route": name})
        events += [dict(x, t=x["t"] + t) for x in e.list]
        t = max(x["t"] for x in events) + 2.0
    return events


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    w = sub.add_parser("write")
    w.add_argument("out", type=Path)
    w.add_argument("--route", action="append", help="only these routes (default: every route, in ORDER)")
    sub.add_parser("course")
    args = parser.parse_args()
    if args.command == "write":
        input_recording.write(args.out, recording(args.route or ORDER))
    else:
        print("\n".join(course_commands()))


if __name__ == "__main__":
    main()
