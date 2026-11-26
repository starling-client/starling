#!/usr/bin/env python3
"""Make sample worlds with older vanilla servers, for the DataFixer harness.

For each listed version, the official server jar (cached under
.local/datafix/servers/VERSION/server.jar, in the private writable root) generates a default world around spawn,
gets a scene of block entities, entities and items in that version's own save
format (tools/datafix_scene.py), and a bot player joins (with its inventory filled)
so that player data exists. The saved world is kept in datafix/worlds/VERSION in the private writable root
(default .local/datafix; override with SUNRISE_DATAFIX_ROOT).

  datafix_worlds.py [--force] [VERSION...]      (default: every version below)
"""
import argparse
import hashlib
import json
import math
import os
import platform as host
import queue
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
import zipfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from tools import datafix_paths, datafix_scene as scene, datafix_scene_12 as scene12
from tools.lab import free_port, offline_uuid, parse_varint, varint

LOCAL = datafix_paths.writable_root()


def worlds_dir(name):
    return LOCAL / 'worlds-pre-1.17' if name.split('-')[0] in LEGACY else LOCAL / 'worlds'


# The versions the harness covers, oldest first: a choice, not something any file states.
VERSIONS = ['1.8.9', '1.9.4', '1.10.2', '1.11.2', '1.12.2', '1.13.2', '1.14.4', '1.15.2', '1.16.1', '1.16.5', '1.17.1', '1.18.1', '1.18.2', '1.19', '1.19.2', '1.19.3', '1.19.4', '1.20.1', '1.20.2', '1.20.4', '1.20.5', '1.20.6', '1.21.1',
            '1.21.3', '1.21.4', '1.21.5', '1.21.6', '1.21.8', '1.21.9', '1.21.10', '1.21.11', '26.1', '26.1.2', '26.2']
# Extra worlds of one version with another world type, for the level.dat generator settings:
# name -> (version, server.properties). They get no scene, only a joined player.
VARIANTS = {
    '1.13.2-flat': ('1.13.2', {'level-type': 'flat', 'generator-settings':
                               '3;minecraft:bedrock,2*minecraft:dirt,minecraft:grass_block;1;village,oceanmonument,mansion(distance=30)'}),
    '1.14.4-flat': ('1.14.4', {'level-type': 'flat', 'generator-settings':
                               '3;minecraft:bedrock,3*minecraft:stone,minecraft:grass_block;2;stronghold(distance=20 count=5),village(distance=12),'
                               'oceanmonument(spacing=10 separation=4)'}),
    '1.14.4-largebiomes': ('1.14.4', {'level-type': 'largebiomes'}),
    '1.14.4-amplified': ('1.14.4', {'level-type': 'amplified'}),
    '1.13.2-debug': ('1.13.2', {'level-type': 'debug_all_block_states'}),
    '1.14.4-buffet': ('1.14.4', {'level-type': 'buffet', 'generator-settings':
                                 '{"chunk_generator":{"type":"minecraft:surface"},"biome_source":{"type":"minecraft:fixed","options":'
                                 '{"biomes":["minecraft:desert"]}}}'}),
}
# Worlds saved before 1.17 go in worlds-pre-1.17: every version of VERSIONS before this one.
FIRST_MODERN = '1.17.1'
LEGACY = set(VERSIONS[:VERSIONS.index(FIRST_MODERN)])
MANIFEST = 'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json'
LOCK = json.loads((ROOT / 'research-lock.json').read_text())['java_runtimes']
BOT = 'Upgrader'
# Versions whose default world also gets every block state the server can save, on a grid beside the scene.
STATE_GRID = {'1.13.2'}
UUID_BASE = 0x6000
# The clientbound configuration packet id of select_known_packs, by protocol: 14 from 1.20.5 (766) to 26.2 (776); 26.3 (777)
# puts post_effects before it, which makes it 15 (crates/sunrise-protocol/src/ids.rs). The older ids are those of the
# per-version packet tables removed in 9dd33d4e1b2 (crates/sunrise-protocol/src/adapter/v*.rs, from the packets reports).
SELECT_KNOWN_PACKS_ID = {**dict.fromkeys(range(766, 777), 14), 777: 15}


def fetch(path, url, sha1, what):
    """Download `url` to `path`, checked against `sha1`. It is written beside `path` and renamed into place, so a
    write cut short (a full disk) never leaves a file that looks complete."""
    path.parent.mkdir(parents=True, exist_ok=True)
    data = urllib.request.urlopen(url).read()
    if hashlib.sha1(data).hexdigest() != sha1:
        sys.exit(f'{what}: checksum mismatch')
    partial = tempfile.NamedTemporaryFile(dir=path.parent, suffix='.part', delete=False)
    try:
        with partial:
            partial.write(data)
        os.replace(partial.name, path)
    finally:
        Path(partial.name).unlink(missing_ok=True)


def version_meta(version):
    path = LOCAL / 'servers' / version / 'version.json'
    if not path.exists():   # fetch() renames a whole file into place, so a file that exists is never cut short
        manifest_path = os.environ.get('SUNRISE_DATAFIX_MANIFEST')
        manifest = json.loads(Path(manifest_path).read_text()) if manifest_path else json.load(urllib.request.urlopen(MANIFEST))
        entry = next(v for v in manifest['versions'] if v['id'] == version)
        fetch(path, entry['url'], entry['sha1'], f'{version}: version metadata')
    return json.loads(path.read_text())


def server_jar(version):
    """The server jar, its DataVersion (None before 1.9: no DataVersion) and protocol."""
    path = LOCAL / 'servers' / version / 'server.jar'
    download = version_meta(version)['downloads']['server']
    # A cached jar that fails its checksum (an empty or cut-short file from a failed run) is downloaded again.
    if not path.exists() or hashlib.sha1(path.read_bytes()).hexdigest() != download['sha1']:
        fetch(path, download['url'], download['sha1'], f'{version}: server.jar')
    jar = zipfile.ZipFile(path)
    if 'version.json' in jar.namelist():
        info = json.loads(jar.read('version.json'))
        return path, info['world_version'], info['protocol_version']
    return path, OLD_VERSIONS[version][0], OLD_VERSIONS[version][1]


# DataVersion and protocol of servers whose jar has no version.json (before 1.14).
OLD_VERSIONS = {
    '1.13.2': (1631, 404), '1.12.2': (1343, 340), '1.11.2': (922, 316), '1.10.2': (512, 210),
    '1.9.4': (184, 110), '1.8.9': (None, 47),
}


# Mojang's runtime platform names (the keys of its java runtime manifest), by Windows machine.
WINDOWS_PLATFORMS = {'AMD64': 'windows-x64', 'ARM64': 'windows-arm64', 'x86': 'windows-x86'}
# and by Linux machine: its manifest has no arm64 Linux runtime, so such a machine is refused.
LINUX_PLATFORMS = {'x86_64': 'linux', 'i386': 'linux-i386', 'i686': 'linux-i386'}


def platform():
    """Mojang's name for this machine in its Java runtime manifest."""
    if sys.platform == 'darwin':
        return 'mac-os-arm64' if host.machine() == 'arm64' else 'mac-os'
    if sys.platform == 'win32':
        if host.machine() not in WINDOWS_PLATFORMS:
            raise ValueError(f'no Mojang Java runtime platform for Windows machine {host.machine()}')
        return WINDOWS_PLATFORMS[host.machine()]
    if host.machine() not in LINUX_PLATFORMS:
        raise ValueError(f'no Mojang Java runtime platform for Linux machine {host.machine()}')
    return LINUX_PLATFORMS[host.machine()]


def pinned_runtimes(plat):
    """The components research-lock.json pins for a platform; a platform it does not pin is refused by name."""
    if plat not in LOCK:
        raise ValueError(f'research-lock.json java_runtimes pins no runtime for platform {plat}; pin its components '
                         f'(manifest_url and manifest_sha1, from Mojang\'s runtime manifest for {plat}) under java_runtimes.{plat}')
    return LOCK[plat]


def java(version):
    """The java executable for a server version: Mojang's runtime of the version's
    component (pinned in research-lock.json), installed in the private writable root."""
    plat = platform()
    runtimes = pinned_runtimes(plat)   # before version_meta, which may download
    component = version_meta(version).get('javaVersion', {}).get('component', 'jre-legacy')
    if component in ('java-runtime-delta', 'java-runtime-epsilon'):
        return 'java'  # 1.20.5 on: the JDK that runs 26.3 runs them
    if component not in {name for other, names in LOCK.items() if other != 'note' for name in names}:
        raise ValueError(f'{version}: javaVersion component {component} is in no platform of research-lock.json java_runtimes')
    if component not in runtimes:
        component = 'java-runtime-gamma'  # research-lock.json java_runtimes note: a component this platform lacks falls back to gamma
    pin = runtimes[component]
    home = LOCAL / 'java' / f'{component}-{plat}'
    stamp = home / '.complete'
    if not stamp.exists() or stamp.read_text() != pin['manifest_sha1']:
        install_runtime(pin, home)
        stamp.write_text(pin['manifest_sha1'])
    exe = next(home.glob('**/bin/java'))
    return str(exe)


def install_runtime(pin, home):
    data = urllib.request.urlopen(pin['manifest_url']).read()
    if hashlib.sha1(data).hexdigest() != pin['manifest_sha1']:
        sys.exit(f'{pin["manifest_url"]}: checksum mismatch')
    shutil.rmtree(home, ignore_errors=True)
    links = []
    for name, entry in sorted(json.loads(data)['files'].items()):
        target = home / name
        if entry['type'] == 'directory':
            target.mkdir(parents=True, exist_ok=True)
        elif entry['type'] == 'link':
            links.append((target, entry['target']))
        else:
            raw = entry['downloads']['raw']
            body = urllib.request.urlopen(raw['url']).read()
            if hashlib.sha1(body).hexdigest() != raw['sha1']:
                sys.exit(f'{name}: checksum mismatch')
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(body)
            if entry.get('executable'):
                target.chmod(0o755)
    for target, to in links:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.symlink_to(to)


class Server:
    def __init__(self, directory, jar, port, packs=(), java='java', props_override=None):
        directory.mkdir(parents=True, exist_ok=True)
        self.directory = directory
        (directory / 'eula.txt').write_text('eula=true\n')
        props = {'server-port': port, 'online-mode': 'false', 'level-name': 'world', 'level-seed': '4242',
                 'view-distance': '4', 'simulation-distance': '4', 'spawn-protection': '0', 'sync-chunk-writes': 'true',
                 'enable-rcon': 'false', 'enable-query': 'false', 'max-tick-time': '-1'}
        if packs:
            props['initial-enabled-packs'] = ','.join(['vanilla', *packs])
        props.update(props_override or {})
        (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
        self.process = subprocess.Popen([java, '-Xmx2G', '-jar', str(jar), 'nogui'], cwd=directory,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        self.lines = []
        self.queue = queue.Queue()
        self.counter = 0
        threading.Thread(target=self._pump, daemon=True).start()
        self.wait('Done (', 600)

    def _pump(self):
        for line in self.process.stdout:
            line = line.rstrip('\n')
            self.lines.append(line)
            self.queue.put(line)

    def wait(self, pattern, timeout=60):
        end = time.time() + timeout
        seen = []
        while time.time() < end:
            try:
                line = self.queue.get(timeout=1)
            except queue.Empty:
                if self.process.poll() is not None:
                    break
                continue
            seen.append(line)
            if pattern in line:
                return seen
        raise TimeoutError(f'no "{pattern}" in {timeout}s; last lines: {seen[-5:]}')

    def run(self, command, timeout=60):
        self.counter += 1
        marker = f'@@{self.counter}@@'
        self.process.stdin.write(command + '\nsay ' + marker + '\n')
        self.process.stdin.flush()
        out = self.wait(marker, timeout)
        return [re.sub(r'^\[[0-9:]+\] \[[^\]]*\]: ', '', l) for l in out[:-1]]

    def stop(self, *, save=True):
        """Stop the server; with save=False, keep the last explicit save instead of making another."""
        try:
            if save and self.process.poll() is None:
                self.process.stdin.write('stop\n')
                self.process.stdin.flush()
                self.process.wait(120)
        finally:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait()   # the world folder and port are reused only once the process is gone
            self.process.stdin.close()


def failed(lines):
    return [l for l in lines if re.search(r'(Unknown|Expected|Incorrect|Invalid|Could not|Unable|Failed|That block|'
                                          r'error|Malformed|Can\'t)', l, re.I)]


# ------------------------------------------------------------------ minimal bot

def mcstring(s):
    data = s.encode()
    return varint(len(data)) + data


class Bot:
    """Logs in (offline mode) with the given protocol and stays until told to leave."""

    def __init__(self, port, protocol, name):
        self.sock = socket.create_connection(('127.0.0.1', port), timeout=30)
        try:
            self.threshold = -1
            self.buffer = b''
            self.protocol = protocol
            self.stop = threading.Event()
            self.send(0, varint(protocol) + mcstring('127.0.0.1') + struct.pack('>H', port) + varint(2))
            if protocol <= 758:  # 1.18.2: the name only
                self.send(0, mcstring(name))
            elif protocol == 759:  # 1.19: no signature data
                self.send(0, mcstring(name) + b'\x00')
            elif protocol == 760:  # 1.19.2: no signature data, optional UUID
                self.send(0, mcstring(name) + b'\x00' + b'\x01' + offline_uuid(name).bytes)
            elif protocol <= 763:  # 1.19.3 - 1.20.1: optional UUID
                self.send(0, mcstring(name) + b'\x01' + offline_uuid(name).bytes)
            else:
                self.send(0, mcstring(name) + offline_uuid(name).bytes)
            while True:
                pid, data = self.receive()
                if pid == 3:
                    self.threshold = parse_varint(data, 0)[1]
                elif pid == 2:
                    break
                elif pid == 0:
                    raise RuntimeError(f'login refused: {data!r}')
            if protocol >= 764:
                self.send(3)  # login acknowledged -> configuration
                self.configure()
        except BaseException:
            self.sock.close()   # the caller never gets the bot, so nothing else would close it
            raise
        threading.Thread(target=self.idle, daemon=True).start()

    def configure(self):
        old = self.protocol < 766  # 1.20.2 - 1.20.4 packet ids
        finish, keep_alive, ping, disconnect = (2, 3, 4, 1) if old else (3, 4, 5, 2)
        known_packs = SELECT_KNOWN_PACKS_ID.get(self.protocol)   # None before 1.20.5: no known packs
        if not old and known_packs is None:
            raise ValueError(f'protocol {self.protocol}: the id of select_known_packs is not in SELECT_KNOWN_PACKS_ID')
        while True:
            pid, data = self.receive()
            if pid == finish:
                self.send(2 if old else 3)
                return
            if pid == keep_alive:
                self.send(3 if old else 4, data)
            elif pid == ping:
                self.send(4 if old else 5, data)
            elif pid == disconnect:
                raise RuntimeError(f'disconnected during configuration: {data!r}')
            elif pid == known_packs:
                self.send(7, varint(0))  # claim no packs: the server sends full registries

    def idle(self):
        self.sock.settimeout(1.0)
        while not self.stop.is_set():
            try:
                self.receive()
            except socket.timeout:
                continue
            except (EOFError, OSError):
                break

    def leave(self):
        if self.stop.is_set():
            return
        self.stop.set()
        time.sleep(1.5)
        self.sock.close()

    def read_exact(self, n):
        while len(self.buffer) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise EOFError('server closed the connection')
            self.buffer += chunk
        out, self.buffer = self.buffer[:n], self.buffer[n:]
        return out

    def read_varint(self):
        value = shift = 0
        while True:
            b = self.read_exact(1)[0]
            value |= (b & 0x7F) << shift
            shift += 7
            if not b & 0x80:
                return value

    def send(self, packet_id, payload=b''):
        body = varint(packet_id) + payload
        if self.threshold >= 0:
            body = varint(len(body)) + zlib.compress(body) if len(body) >= self.threshold else varint(0) + body
        self.sock.sendall(varint(len(body)) + body)

    def receive(self):
        length = self.read_varint()
        body = self.read_exact(length)
        if self.threshold >= 0:
            pos, size = parse_varint(body, 0)
            body = zlib.decompress(body[pos:]) if size else body[pos:]
        pos, packet_id = parse_varint(body, 0)
        return packet_id, body[pos:]


# ------------------------------------------------------------------ world

def uuid_nbt(i):
    return f'[I;{UUID_BASE},0,0,{i}]'


def uuid_fields(data_version, i):
    """The UUID of a summoned entity: an int array since 1.16, two longs before."""
    if data_version >= 2514:
        return f'UUID:{uuid_nbt(i)}'
    return f'UUIDMost:{UUID_BASE << 32}L,UUIDLeast:{i + 1}L'


def block_states(jar, java_exe):
    """Every block state of a server jar, as setblock arguments, from the jar's own data generator report."""
    with tempfile.TemporaryDirectory() as scratch:   # the generator writes its own logs/ into its working folder
        subprocess.run([java_exe, '-cp', str(jar), 'net.minecraft.data.Main', '--reports', '--output', 'out'],
                       cwd=scratch, check=True, stderr=subprocess.STDOUT)
        report = json.loads((Path(scratch) / 'out' / 'reports' / 'blocks.json').read_text())
    states = []
    for block, info in report.items():
        # moving_piston's block entity is never made by the block itself, and the server then fails to save its
        # chunk (a null TileEntities entry), so it is the one state left out.
        if block == 'minecraft:moving_piston':
            continue
        for state in info['states']:
            props = state.get('properties')
            states.append(block + ('[' + ','.join(f'{k}={v}' for k, v in props.items()) + ']' if props else ''))
    return states


def place_states(run, states):
    """One block state at each odd x and z of a grid east of the scene, as vanilla's debug world lays them out
    (DebugLevelSource.getBlockStateFor), on a stone floor and between barriers so fluids stay in their cell.
    Unlike the debug world this world ticks: random ticks and fire spread are turned off, but scheduled ticks still
    run, so a few states (flowing fluids with no source, unsupported or unpowered redstone) may settle before the
    save; the harness upgrades whatever was saved."""
    run('gamerule randomTickSpeed 0')
    run('gamerule doFireTick false')
    width = 1 + math.isqrt(len(states) - 1)   # DebugLevelSource.GRID_WIDTH: ceil(sqrt(state count))
    x0, side = 256, 2 * width + 1
    run(f'forceload add {x0} 0 {x0 + side} {side}')
    for z in range(0, side + 1, 64):   # a fill changes at most 32,768 blocks
        z1 = min(z + 63, side)
        run(f'fill {x0} 69 {z} {x0 + side} 69 {z1} minecraft:stone')
        run(f'fill {x0} 70 {z} {x0 + side} 70 {z1} minecraft:barrier')
    for start in range(0, len(states), 500):
        run('\n'.join(f'setblock {x0 + 1 + 2 * (i % width)} 70 {1 + 2 * (i // width)} {state}'
                       for i, state in enumerate(states[start:start + 500], start)))


def build_scene(server, run, Bot, port, protocol, props, era, data_version, states=()):
    """The scene of the generic generator; its bot stays joined until the caller closes it."""
    if not props:  # variants only differ in their world type
        run('forceload add -48 -48 79 79')
        run('fill 0 99 0 47 99 47 minecraft:stone')
        run('fill 0 100 0 47 112 47 minecraft:air')
        for i, (block, nbt) in enumerate(scene.block_entities(era, data_version)):
            x, z = 1 + 3 * (i % 15), 1 + 3 * (i // 15)
            run(f'setblock {x} 100 {z} minecraft:{block}{nbt}')
        for i, (kind, extra) in enumerate(scene.entities(era, data_version)):
            x, z = 1.5 + 3 * (i % 15), 25.5 + 3 * (i // 15)
            fields = f'{uuid_fields(data_version, i)},NoAI:1b,Silent:1b,NoGravity:1b,Invulnerable:1b,PersistenceRequired:1b'
            run(f'summon minecraft:{kind} {x} 100 {z} {{{fields}' + (',' + extra if extra else '') + '}')
        for i, block in enumerate(scene.blocks(data_version)):
            run(f'setblock {1 + 3 * (i % 15)} 100 {19 + 3 * (i // 15)} minecraft:{block}')
        if states:
            place_states(run, states)
    if not props:
        run('setworldspawn 20 101 20')
    Bot(port, protocol, BOT)
    time.sleep(3)
    for command in scene.player_commands(era, BOT, data_version):
        run(command)
    time.sleep(3)
    for command, wait in ([] if props else scene.world_commands(data_version)):
        run(command)
        time.sleep(wait)
    run('save-all flush')
    time.sleep(2)


def tracked_bot(bots):
    """A Bot class that appends each bot that joined to `bots` once __init__ returns (a failed join closes its own socket)."""
    class JoinedBot(Bot):
        def __init__(self, *args):
            super().__init__(*args)
            bots.append(self)
    return JoinedBot


def build(name, force=False):
    version, props = VARIANTS.get(name, (name, None))
    jar, data_version, protocol = server_jar(version)
    data_version = data_version or 0   # 1.8 saves have no DataVersion
    target = worlds_dir(name) / name
    if target.exists() and not force:
        print(f'{version}: kept {target}')
        return []
    era = scene.era(data_version)
    work = LOCAL / 'gen' / name
    shutil.rmtree(work, ignore_errors=True)
    port = free_port()
    java_exe = java(version)
    states = block_states(jar, java_exe) if version in STATE_GRID and not props else ()
    server = Server(work, jar, port, scene.experiments(data_version), java_exe, props)
    if props and props.get('level-type') == 'debug_all_block_states':
        # The debug world's grid (DebugLevelSource.getBlockStateFor: every block state at odd x and z from 1 to
        # 2 * GRID_WIDTH + 1, 187 for 1.13.2's 8,599 states) holds the twelve moving_piston states. Their block has a
        # block entity it cannot make on its own, so 1.13.2 keeps each of those positions with no data and fails to
        # save its chunk (ChunkSerializer adds a null TileEntities entry), and a debug world refuses every block
        # change, so the grid cannot be cleared. The world is wanted for its level.dat alone: its spawn moves far from
        # the grid, the chunks of this first start are dropped, and the world is made again from there, so its saved
        # chunks never hold the grid.
        out = server.run('setworldspawn -4000 70 -4000')
        if not any('Set the world spawn point' in line for line in out):
            server.stop()
            raise RuntimeError(f'{name}: setworldspawn was refused, so the grid would be saved: {out}')
        server.stop()
        shutil.rmtree(work / 'world' / 'region')
        server = Server(work, jar, port, scene.experiments(data_version), java_exe, props)
    problems = []
    bots = []   # every bot that joined, so that the finally below closes it whatever fails
    JoinedBot = tracked_bot(bots)

    def run(command):
        out = server.run(command)
        bad = failed(out)
        if bad:
            problems.append((command[:160], bad[:2]))
        return out

    legacy_scene = data_version < 1631 and not props
    save_on_stop = True
    try:
        try:
            if legacy_scene:
                scene12.populate(server, JoinedBot, port, protocol, BOT, problems, mcstring=mcstring)
                save_on_stop = False  # a second save would overwrite the transient states in the last explicit save
            else:
                build_scene(server, run, JoinedBot, port, protocol, props, era, data_version, states)
        finally:
            for bot in bots:
                bot.leave()
        if save_on_stop:
            time.sleep(2)
    finally:
        server.stop(save=save_on_stop)
    if legacy_scene:
        scene12.fixtures(work / 'world')   # the few statistics a passive bot cannot cause, written by hand
    # Copy aside, then rename: an interrupted copy must not look like a finished world.
    partial = target.with_name(target.name + '.partial')
    shutil.rmtree(partial, ignore_errors=True)
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(work / 'world', partial)
    (partial / 'datafix-source.json').write_text(json.dumps(
        {'version': version, 'data_version': data_version, 'problems': problems}, indent=1))
    shutil.rmtree(target, ignore_errors=True)
    partial.rename(target)
    for command, out in problems:
        print(f'{version}: command failed: {command}\n   {out}')
    print(f'{version}: DataVersion {data_version}, world in {target}')
    return problems


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('versions', nargs='*')
    p.add_argument('--force', action='store_true')
    args = p.parse_args()
    for version in args.versions or [*VERSIONS, *VARIANTS]:
        build(version, args.force)
        if version == '1.12.2':
            scene12.build_flat(sys.modules[__name__], args.force)   # and the flat world 1.12.2-flat


if __name__ == '__main__':
    main()
