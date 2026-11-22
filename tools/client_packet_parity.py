#!/usr/bin/env python3
"""Client packet parity for anticheats: the vanilla 26.3 client and Sunrise's, each in its own Xvfb display, play
the same input recording on the same vanilla 26.3 server, and their serverbound packets are compared
(tools/client_packet_diff.py). Anticheats (Grim, Vulcan, Polar...) judge a client by exactly these packets.

  client_packet_parity.py world                                 # once: the seeded world with the test course
  client_packet_parity.py run --client vanilla --runs 3         # every route of client_packet_routes.py
  client_packet_parity.py run --client sunrise --runs 3 --bin target/fast
  client_packet_parity.py run --client sunrise --recording mine.jsonl   # a person's own (input_recording.py)
  client_packet_parity.py diff                                  # every run of both: the report
  client_packet_parity.py all --runs 3                          # world, both clients, diff

Every run restores the world snapshot first (course, player data, time and weather), starts the vanilla
dedicated server (offline, from the verified jars in .local, tools/lab.py prepare), puts tools/net_capture.py
between it and the client, and starts the client as Steve on a fresh Xvfb display, never the desktop's. Before
each route the server's console resets the player (place, inventory, effects), the client settles for 40
ticks, and the route's input goes in through XTEST, starting with its marker (the chat command /wire-mark,
where tools/wire_stream.py cuts the stream). Both clients get the same options: window, GUI scale, mouse
sensitivity, raw input, key bindings, frame cap, render distance and minimal graphics (OPTIONS).

The PvP routes (pvp_*) fight a second player, Opp0: sunrise-bots --behaviour stand (build it with
`cargo build --profile fast -p sunrise-bots`), which stands where the console puts it; the knockback the client
takes comes from `damage ... by Opp0` at set times of the route.

Runs land in .local/packet-parity/runs/<client>-<n>/ (run.json, capture.jsonl, logs, a screenshot after each
route); --label puts a set in runs/<label>-<client>-<n>/ (tools/packet_parity_runs.py), and `diff --label` or
`client_packet_diff.py RUNS --a <label>-vanilla --b <label>-sunrise` compares it. `all` compares the runs it just
made. Two sets of one client (`--a x-vanilla --b y-vanilla`) show how much real-time input alone differs: run one
before reading a Sunrise difference as a finding. A run's folder is never replaced unless --replace says so.
run.json is its manifest: what it played (the recording's, settings' and course's hashes, the vanilla server jar's
and the client's own checksums, the routes), whether it finished, and the fixture each route started from; `diff
--strict` compares only finished runs whose manifests match, and fails on any difference. A recording's route that
client_packet_routes.py doesn't define gets no reset beyond a teleport in place, so its setup is neither reset nor
checked: the manifest lists it under `unchecked_routes`.
Run it at low priority beside other work: `nice -n 19 ionice -c3 python3 tools/client_packet_parity.py ...`.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import client_packet_routes as routes  # noqa: E402
import input_recording  # noqa: E402
import lab  # noqa: E402
import net_capture  # noqa: E402
import packet_parity_runs  # noqa: E402
import test_rust_parity as base  # noqa: E402
from test_dedicated_server_parity import Server  # noqa: E402
from test_wire_stream_parity import ClientCounts, settle  # noqa: E402

WORK = lab.LOCAL / "packet-parity"
SNAPSHOT = WORK / "world-snapshot"
PLAYER = routes.PLAYER
SEED = "20261119"
PROPERTIES = {
    "online-mode": "false", "white-list": "false", "level-seed": SEED, "generate-structures": "false",
    "view-distance": "6", "simulation-distance": "6", "spawn-protection": "0", "pause-when-empty-seconds": "-1",
    "gamemode": "survival", "force-gamemode": "true", "difficulty": "easy", "spawn-monsters": "false",
    "enable-command-block": "false", "level-name": "world", "max-tick-time": "-1", "sync-chunk-writes": "false",
}
# One options.txt for both clients: what Sunrise doesn't read it ignores.
OPTIONS = {
    "onboardAccessibility": "false", "skipMultiplayerWarning": "true", "joinedFirstServer": "true",
    "tutorialStep": "none", "pauseOnLostFocus": "false", "renderDistance": "4", "simulationDistance": "5",
    "maxFps": "60", "enableVsync": "false", "inactivityFpsLimit": "minimized", "guiScale": str(routes.GUI_SCALE),
    "mouseSensitivity": "0.5", "rawMouseInput": "true", "autoJump": "false", "toggleCrouch": "false",
    "toggleSprint": "false", "toggleAttack": "false", "toggleUse": "false", "discrete_mouse_scroll": "false",
    "mouseWheelSensitivity": "1.0", "invertYMouse": "false", "narrator": "0", "soundCategory_master": "0.0",
    "realmsNotifications": "false", "chatLinksPrompt": "false", "onlyShowSecureChat": "false",
    "renderClouds": '"false"', "particles": "2", "entityShadows": "false", "mipmapLevels": "0", "ao": "false",
    "biomeBlendRadius": "0", "graphicsPreset": '"fast"', "bobView": "false", "fov": "0.0", "lang": "en_us",
}
GAMERULES = ["advance_time false", "advance_weather false", "spawn_mobs false", "spawn_patrols false",
             "spawn_wandering_traders false", "spawn_phantoms false", "fire_spread_radius_around_player 0",
             "random_tick_speed 0", "respawn_radius 0"]
TERRAIN_CANDIDATES = [(-40, 20), (-40, 60), (-40, 100), (-44, -8), (96, 20), (96, 100)]
LVP = Path("/usr/share/vulkan/icd.d/lvp_icd.json")
# Console answers that say a command did nothing it was meant to (a parse error, an unloaded or too large area).
COMMAND_ERRORS = re.compile(r"<--\[HERE\]|Unknown or incomplete command|Incorrect argument|That position is not "
                            r"loaded|out of the world|Too many blocks in the specified area|Too many chunks")


def server_jar():
    """The verified vanilla server jar every run starts."""
    return base.verify_artifacts()[1]["server"]


def server_command():
    _, jars, libraries = base.verify_artifacts()
    deps = os.pathsep.join(map(str, [jars["server"], *libraries]))
    return ["java", "-Xmx2g", "-cp", deps, "net.minecraft.server.Main", "nogui"]


def server_folder(directory, port):
    directory.mkdir(parents=True, exist_ok=True)
    props = dict(PROPERTIES, **{"server-port": str(port), "server-ip": "127.0.0.1"})
    (directory / "server.properties").write_text("".join(f"{k}={v}\n" for k, v in props.items()),
                                                 encoding="iso-8859-1")
    # The owner accepted Mojang's EULA for these runs (https://aka.ms/MinecraftEULA).
    (directory / "eula.txt").write_text("eula=true\n")


def ask(server, command, pattern, timeout=60):
    """Runs a console command and returns the first new log line matching `pattern`."""
    seen = len(server.lines)
    server.console(command)
    deadline = time.time() + timeout
    regex = re.compile(pattern)
    while time.time() < deadline:
        for line in server.lines[seen:]:
            if regex.search(line):
                return line
        time.sleep(0.1)
    raise RuntimeError(f"no answer to {command!r} matching {pattern!r}:\n" + "\n".join(server.lines[-10:]))


def checked(server, commands, tag):
    """Runs console commands and fails when the server answers any of them with an error (COMMAND_ERRORS)."""
    seen = len(server.lines)
    for command in commands:
        server.console(command)
    ask(server, f"say {tag}-done", rf"\b{tag}-done\b", timeout=120)
    errors = [line for line in server.lines[seen:] if COMMAND_ERRORS.search(line)]
    if errors:
        raise RuntimeError(f"{tag}: the server refused a command:\n" + "\n".join(errors[:10]))


def fixture(server):
    """What a route starts from, as the server reports it: the player's inventory, game mode, and effects by id and
    amplifier (their remaining durations count down while the reset settles)."""
    out = {}
    for key, path in (("inventory", "Inventory"), ("mode", "playerGameType"), ("effects", "active_effects")):
        line = ask(server, f"data get entity {PLAYER} {path}",
                   rf"{PLAYER} has the following entity data|Found no elements matching")
        out[key] = line.split("]: ", 1)[-1]
    out["effects"] = sorted(re.findall(r'amplifier: (-?\d+)b?[^}]*?id: "([^"]+)"|id: "([^"]+)"[^}]*?amplifier: '
                                       r'(-?\d+)b?', out["effects"]))
    return out


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def build_world():
    """The seeded world with the course built, its terrain spot found, saved as the snapshot every run restores."""
    folder = WORK / "world-build"
    shutil.rmtree(folder, ignore_errors=True)
    port = lab.free_port()
    server_folder(folder, port)
    server = Server(server_command(), folder, "world-build")
    try:
        server.wait_for(r"Done \(", timeout=900)
        checked(server, [f"gamerule {rule}" for rule in GAMERULES] + ["time set 6000", "weather clear"], "rules")
        x0, z0, x1, z1 = routes.course_bounds()
        checked(server, [f"forceload add {x0} {z0} {x1} {z1}"], "forceload")
        corners = " ".join(f"if loaded {x} {routes.FLOOR} {z}" for x in (x0, x1) for z in (z0, z1))
        deadline = time.time() + 900
        while True:
            try:
                ask(server, f"execute {corners} run say course-loaded", r"course-loaded", timeout=3)
                break
            except RuntimeError:
                if time.time() > deadline:
                    raise
        checked(server, routes.course_commands(), "course")
        spot = None
        for x, z in TERRAIN_CANDIDATES:
            try:
                ask(server, f"execute positioned {x} 0 {z} positioned over motion_blocking_no_leaves "
                            f"unless block ~ ~-1 ~ #minecraft:replaceable unless block ~ ~-1 ~ minecraft:water "
                            f"unless block ~ ~-1 ~ minecraft:lava run say spot-ok {x} {z}", r"spot-ok", timeout=5)
            except RuntimeError:
                continue
            server.console(f"execute positioned {x} 0 {z} positioned over motion_blocking_no_leaves run "
                           f"summon minecraft:marker ~ ~ ~ {{Tags:[\"pp_spot\"]}}")
            line = ask(server, "data get entity @e[tag=pp_spot,limit=1] Pos", r"entity data: \[")
            y = float(re.search(r"\[[^,]+, ([-\d.]+)d", line).group(1))
            server.console("kill @e[tag=pp_spot]")
            spot = (x, y, z)
            break
        if spot is None:
            raise RuntimeError("no dry terrain spot among TERRAIN_CANDIDATES")
        server.console("forceload remove all")
        ask(server, "save-all flush", r"Saved the game", timeout=300)
    finally:
        server.stop()
    shutil.rmtree(SNAPSHOT, ignore_errors=True)
    shutil.copytree(folder / "world", SNAPSHOT)
    (WORK / "course.json").write_text(json.dumps({"seed": SEED, "terrain": spot, "routes": routes.ORDER,
                                                  "commands": routes.course_commands()}))
    print(f"world snapshot: {SNAPSHOT} (terrain spot {spot})")


class Display:
    """A fresh Xvfb display with GLX, never the desktop's."""

    def __init__(self, log):
        read, write = os.pipe()
        self.process = subprocess.Popen(
            ["Xvfb", "-displayfd", str(write), "-screen", "0", "1280x800x24", "-nolisten", "tcp", "+extension",
             "GLX", "-noreset"], pass_fds=[write], stdout=log, stderr=subprocess.STDOUT)
        os.close(write)
        with os.fdopen(read) as f:
            self.name = f":{f.readline().strip()}"

    def env(self, env):
        env = dict(env)
        env.pop("WAYLAND_DISPLAY", None)
        env["DISPLAY"] = self.name
        env["XDG_SESSION_TYPE"] = "x11"
        env["__GLX_VENDOR_LIBRARY_NAME"] = "mesa"
        env["__EGL_VENDOR_LIBRARY_FILENAMES"] = "/usr/share/glvnd/egl_vendor.d/50_mesa.json"
        if LVP.exists():
            env["VK_ICD_FILENAMES"] = str(LVP)
        return env

    def screenshot(self, path, crop=None):
        """Saves the display's picture (`crop`: a geometry like 854x480+0+0) to `path`; False, with a note on
        stderr, when `import` couldn't."""
        done = subprocess.run(["import", "-display", self.name, "-window", "root", *(["-crop", crop, "+repage"] if crop else []),
                               str(path)], capture_output=True, timeout=30)
        if done.returncode or not path.exists():
            print(f"  screenshot {path.name} missing: import exited {done.returncode}", file=sys.stderr)
            return False
        return True

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(10)
        except subprocess.TimeoutExpired:
            self.process.kill()


def game_window(display_name, timeout=300):
    """The client's window on the display, given the input focus and the pointer, once it is mapped."""
    from Xlib import X, display as xdisplay
    d = xdisplay.Display(display_name)
    deadline = time.time() + timeout
    while time.time() < deadline:
        for w in d.screen().root.query_tree().children:
            try:
                attrs, geo = w.get_attributes(), w.get_geometry()
            except Exception:
                continue
            if attrs.map_state == X.IsViewable and geo.width >= 400 and geo.height >= 300:
                w.set_input_focus(X.RevertToParent, X.CurrentTime)
                d.screen().root.warp_pointer(geo.x + geo.width // 2, geo.y + geo.height // 2)
                d.sync()
                return d, w
        time.sleep(0.5)
    raise RuntimeError(f"no client window on {display_name} in {timeout} s")


def options_file(folder):
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "options.txt").write_text("".join(f"{k}:{v}\n" for k, v in OPTIONS.items()))


def start_vanilla(run, address, display, log):
    import vanilla_client
    args = argparse.Namespace(name=PLAYER, join=address, world=None, game_dir=run / "client", assets=None,
                              heap="2G", width=routes.WIDTH, height=routes.HEIGHT, lan_port=None, backend=None)
    options_file(args.game_dir)
    vanilla_client.write_options(args.game_dir, OPTIONS)
    env = display.env(vanilla_client.environment())
    return subprocess.Popen(vanilla_client.command(args), cwd=args.game_dir, env=env, stdin=subprocess.DEVNULL,
                            stdout=log, stderr=subprocess.STDOUT)


def start_sunrise(run, address, display, log, bins):
    game = run / "client"
    options_file(game)
    home = run / "home"
    for d in ("home", "launcher", "data", "config"):
        (run / d).mkdir(parents=True, exist_ok=True)
    env = lab.game_env(SUNRISE_HOME=home, SUNRISE_LAUNCHER_HOME=run / "launcher", XDG_DATA_HOME=run / "data",
                       XDG_CONFIG_HOME=run / "config", SUNRISE_TELEMETRY_URL="off")
    env = display.env(env)
    command = [str(lab.binary(bins, "sunrise-client")), "--resources", str(lab.LOCAL / "sunrise-client" / lab.VERSION),
               "--server", address, "--name", PLAYER, "--width", str(routes.WIDTH), "--height", str(routes.HEIGHT)]
    return subprocess.Popen(command, cwd=game, env=env, stdin=subprocess.DEVNULL, stdout=log,
                            stderr=subprocess.STDOUT)


def start_opponent(run, port, bins):
    """The PvP routes' opponent: one sunrise-bots player that stands where the console puts it."""
    log = open(run / "opponent.log", "w")
    command = [str(lab.binary(bins, "sunrise-bots")), "--server", f"127.0.0.1:{port}", "--bots", "1",
               "--behaviour", "stand", "--name", routes.OPPONENT[:-1], "--seconds", "100000", "--view-distance", "4"]
    return subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)


def wait_in_game(counts, process, timeout=600):
    """Until the client ends its first 40 ticks in the play state."""
    deadline = time.time() + timeout
    while counts.update().ticks < 40:
        if process.poll() is not None:
            raise RuntimeError(f"the client exited ({process.returncode}) before playing")
        if time.time() > deadline:
            raise RuntimeError("the client never reached play")
        time.sleep(0.5)


def scaled(events, scale):
    """Relative mouse moves times `scale`, the remainder carried, for a diagnostic pass past a turning difference."""
    out, carry = [], [0.0, 0.0]
    for e in events:
        if e["type"] == "rel" and scale != 1:
            carry = [carry[0] + e["dx"] * scale, carry[1] + e["dy"] * scale]
            dx, dy = int(carry[0]), int(carry[1])
            carry = [carry[0] - dx, carry[1] - dy]
            if dx or dy:
                out.append(dict(e, dx=dx, dy=dy))
        else:
            out.append(e)
    return out


def client_build(client, bins):
    """The checksum of what the client runs: the verified vanilla client jar, or the sunrise-client binary."""
    if client == "vanilla":
        return lab.digest(base.verify_artifacts()[1]["client"], "sha256")
    return lab.digest(lab.binary(bins, "sunrise-client"), "sha256")


def run_once(client, index, recording, bins, label=None, mouse_scale=1.0, replace=False):
    """One client, one pass over every route of the recording, on a restored world."""
    course_text = (WORK / "course.json").read_text()
    course = json.loads(course_text)
    played_routes = input_recording.routes(recording)
    try:
        run = packet_parity_runs.run_folder(WORK / "runs", client, index, label, replace)
    except (ValueError, FileExistsError) as error:
        raise SystemExit(str(error))
    manifest = {"client": client, "index": index, "label": label, "complete": False,
                "recording": digest(recording), "settings": digest([PROPERTIES, OPTIONS, GAMERULES]),
                "course": hashlib.sha256(course_text.encode()).hexdigest(), "mouse_scale": mouse_scale,
                "diagnostic": mouse_scale != 1.0, "routes": [name for name, _ in played_routes],
                "unchecked_routes": [name for name, _ in played_routes if name not in routes.ROUTES],
                "server_jar": lab.digest(server_jar(), "sha256"), "client_build": client_build(client, bins),
                "missing_screenshots": []}
    (run / "run.json").write_text(json.dumps(manifest, indent=1))
    port, listen = lab.free_port(), lab.free_port()
    server_folder(run / "server", port)
    shutil.copytree(SNAPSHOT, run / "server" / "world")
    capture = run / "capture.jsonl"
    capture.touch()
    logs = {name: open(run / f"{name}.log", "w") for name in ("xvfb", "client")}
    server = Server(server_command(), run / "server", "server")
    display = stop = process = opponent = None
    try:
        server.wait_for(r"Done \(", timeout=900)
        stop = net_capture.start_background(listen, f"127.0.0.1:{port}", capture, full=True)
        if any(routes.ROUTES.get(name, {}).get("opponent") for name, _ in played_routes):
            opponent = start_opponent(run, port, bins)
            server.wait_for(rf"{routes.OPPONENT} joined the game", timeout=120)
        display = Display(logs["xvfb"])
        address = f"127.0.0.1:{listen}"
        process = (start_vanilla(run, address, display, logs["client"]) if client == "vanilla"
                   else start_sunrise(run, address, display, logs["client"], bins))
        game_window(display.name)
        counts = ClientCounts(capture)
        wait_in_game(counts, process)
        _, window = game_window(display.name, timeout=10)
        injector = input_recording.Injector(display.name, window.id)
        at = window.query_tree().root.translate_coords(window, 0, 0)
        window_crop = f"{routes.WIDTH}x{routes.HEIGHT}+{at.x}+{at.y}"
        if not display.screenshot(run / "joined.png"):
            manifest["missing_screenshots"].append("joined.png")
        played = []
        for number, (name, events) in enumerate(played_routes, 1):
            if process.poll() is not None:
                raise RuntimeError(f"the client exited ({process.returncode}) before {name}")
            injector.play([{"t": 0.0, "type": "key", "code": "KEY_1", "down": True},
                           {"t": 0.08, "type": "key", "code": "KEY_1", "down": False}])
            commands = (routes.reset_commands(name, course["terrain"]) if name in routes.ROUTES
                        else [f"tp {PLAYER} {PLAYER}"])
            accepted = counts.update().accepted + sum(c.startswith(f"tp {PLAYER} ") for c in commands)
            checked(server, commands, f"reset-{number}")
            settle(counts, accepted)
            start = fixture(server)
            timers = [threading.Timer(t, server.console, [c]) for t, c in routes.ROUTES.get(name, {}).get("during", [])]
            # The route's screenshots, of the game window, while it plays (its `expect` reads them).
            shot = lambda label: (display.screenshot(run / f"{name}-{label}.png", window_crop)  # noqa: E731
                                  or manifest["missing_screenshots"].append(f"{name}-{label}.png"))
            timers += [threading.Timer(t, shot, [label]) for t, label in routes.ROUTES.get(name, {}).get("shots", [])]
            for timer in timers:
                timer.start()
            seconds = injector.play(scaled(events, mouse_scale), number=number)
            for timer in timers:
                timer.join()
            time.sleep(1.0)
            if process.poll() is not None:
                raise RuntimeError(f"the client exited ({process.returncode}) during {name}")
            if any(f"{PLAYER} lost connection" in line for line in server.lines):
                raise RuntimeError(f"{PLAYER} disconnected during {name}")
            played.append({"route": name, "mark": number, "seconds": round(seconds, 2),
                           "end_tick": counts.update().ticks, "fixture": start})
            if not display.screenshot(run / f"{name}.png"):
                manifest["missing_screenshots"].append(f"{name}.png")
            print(f"  {run.name} {name}: {seconds:.1f} s", flush=True)
        (run / "played.json").write_text(json.dumps(played))
        manifest.update(complete=True, played=played)
        (run / "run.json").write_text(json.dumps(manifest, indent=1))
    finally:
        if opponent and opponent.poll() is None:
            opponent.kill()
            opponent.wait()
        if process and process.poll() is None:
            process.terminate()
            try:
                process.wait(20)
            except subprocess.TimeoutExpired:
                process.kill()
        if stop:
            stop.set()
        if display:
            display.close()
        server.stop()
        for f in logs.values():
            f.close()
    return run


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("world")
    for name in ("run", "all"):
        p = sub.add_parser(name)
        if name == "run":
            p.add_argument("--client", choices=["vanilla", "sunrise"], required=True)
        p.add_argument("--runs", type=int, default=3)
        p.add_argument("--first", type=int, default=1, help="the first run's number")
        p.add_argument("--route", action="append", help="only these scripted routes")
        p.add_argument("--recording", type=Path, help="an input recording instead of the scripted routes")
        p.add_argument("--bin", type=Path, default=lab.ROOT / "target/fast", help="sunrise-client's folder")
        p.add_argument("--label", help="a set of runs of its own, runs/<label>-<client>-N")
        p.add_argument("--mouse-scale", type=float, default=1.0,
                       help="diagnostic: scale relative mouse moves, to look past a known turning difference "
                            "(the run is marked diagnostic and left out of --strict)")
        p.add_argument("--replace", action="store_true", help="run again over existing runs of the same name")
    d = sub.add_parser("diff")
    d.add_argument("--out", type=Path, default=WORK / "report.json")
    d.add_argument("--label", help="compare the set made with this --label")
    d.add_argument("--strict", action="store_true", help="exit 1 unless every packet of every route matches")
    args = parser.parse_args()
    if args.command == "world":
        build_world()
        return
    indices = None
    if args.command in ("run", "all"):
        try:
            names = [packet_parity_runs.prefix(c, args.label) for c in ("vanilla", "sunrise")]
            recording = (input_recording.read(args.recording) if args.recording
                         else routes.recording(args.route or routes.ORDER))
            input_recording.routes(recording)
        except KeyError as error:
            parser.error(f"--route {error}: no such route in client_packet_routes.py")
        except ValueError as error:
            parser.error(f"{error}")
        if args.runs < 1 or args.first < 1:
            parser.error("--runs and --first count from 1")
        if not args.mouse_scale > 0:
            parser.error("--mouse-scale must be above 0")
        if args.command == "all" or not (WORK / "course.json").exists():
            build_world()
        clients = [args.client] if args.command == "run" else ["vanilla", "sunrise"]
        indices = set(range(args.first, args.first + args.runs))
        for client in clients:
            for i in sorted(indices):
                print(f"{client} run {i}", flush=True)
                run_once(client, i, recording, args.bin, args.label, args.mouse_scale, args.replace)
        if args.command == "run":
            return
    else:
        try:
            names = [packet_parity_runs.prefix(c, args.label) for c in ("vanilla", "sunrise")]
        except ValueError as error:
            parser.error(f"{error}")
    import client_packet_diff
    report = client_packet_diff.report_runs(WORK / "runs", tuple(names), indices)
    (args.out if args.command == "diff" else WORK / "report.json").write_text(json.dumps(report, indent=1))
    print(client_packet_diff.describe(report))
    if getattr(args, "strict", False) and not report["strict"]["bit_exact"]:
        sys.exit(1)


if __name__ == "__main__":
    main()
