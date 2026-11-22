#!/usr/bin/env python3
"""What a client answers when a server probes it, vanilla 26.3's client and a Sunrise build side by side.

A vanilla 26.3 dedicated server has no plugin to send probes, so the logging proxy (tools/net_capture.py) writes them
into the server's stream, once the client is in the state each belongs to, and records what the client sends back. The
probes' answers are the clients' serverbound packets after each injected one; the proxy swallows them, so the real
server never sees a packet it did not expect.

  python3 tools/join_probe.py record vanilla --out .local/join-probe/vanilla
  python3 tools/join_probe.py record sunrise --bin $PWD/.local/bin-official --out .local/join-probe/official
  python3 tools/join_probe.py diff .local/join-probe/vanilla .local/join-probe/official

Probes, by phase (the serverbound packet each should draw is in brackets):
- login: a custom query [custom_query_answer, no payload].
- configuration: ping [pong], store_cookie then cookie_request [cookie_response with the stored bytes],
  minecraft:register and minecraft:unregister, an unknown channel, `sunrise:payloads` Query, a server brand
  `sunrise`, custom_report_details, server_links.
- play: ping [pong], minecraft:register, and a sign whose lines hold a vanilla translation key, a Sunrise lang key, an
  unknown key, a keybind and a vanilla keybind, opened in the sign editor and closed with Escape [sign_update: the
  lines the client resolved and shows].

`--scenario` picks the group: `base` (everything but the Sunrise signals), `sunrise-signals` (the Query and the brand,
which make a Sunrise client announce more, and are meant to).
"""
import argparse
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import net_capture  # noqa: E402
import test_rust_parity as base  # noqa: E402
from join_announce import Headless, output_folder, vanilla_client, PLAYER  # noqa: E402
from test_dedicated_server_parity import Server, free_port  # noqa: E402
from vanilla_wire_e2e import PROPERTIES  # noqa: E402

NAMES = net_capture.load_names()
SIGN = (2, -40, 2)
OAK_SIGN_STATE = 6892  # minecraft:oak_sign, rotation 0, not waterlogged (the datagen block report)
SIGN_BLOCK_ENTITY = 7  # minecraft:sign in the block entity type registry
LINES = [
    {'translate': 'key.sneak'},
    {'translate': 'sunrise.mods.screen.sunrise.authors'},
    {'translate': 'sunrise.extras.zoom'},
    {'keybind': 'key.jump'},
    # Four lines is the sign's limit: the next probe is a second sign.
]
LINES_2 = [
    {'keybind': 'key.sunrise.zoom'},
    {'translate': 'no.such.key', 'fallback': 'fallback text'},
    {'translate': 'chat.type.text', 'with': [{'keybind': 'key.attack'}, {'translate': 'sunrise.extras.sort'}]},
    {'text': 'plain'},
]


def varint(n):
    out = b''
    while True:
        b = n & 0x7F
        n >>= 7
        out += bytes([b | (0x80 if n else 0)])
        if not n:
            return out


def string(s):
    data = s.encode()
    return varint(len(data)) + data


def nbt_string(s):
    data = s.encode()
    return struct.pack('>H', len(data)) + data


def nbt_component(c):
    """A text component as a network NBT compound's body (strings only, which is all the probes use)."""
    out = b''
    for k, v in c.items():
        if isinstance(v, str):
            out += b'\x08' + nbt_string(k) + nbt_string(v)
        else:
            out += b'\x09' + nbt_string(k) + b'\x0a' + struct.pack('>i', len(v))
            for item in v:
                out += nbt_component(item)
    return out + b'\x00'


def sign_nbt(lines):
    def text():
        messages = b'\x09' + nbt_string('messages') + b'\x0a' + struct.pack('>i', 4)
        for line in lines:
            messages += nbt_component(line)
        return messages + b'\x01' + nbt_string('has_glowing_text') + b'\x00' + b'\x08' + nbt_string('color') + nbt_string('black') + b'\x00'
    return (b'\x0a' + b'\x0a' + nbt_string('front_text') + text() + b'\x0a' + nbt_string('back_text') + text()
            + b'\x01' + nbt_string('is_waxed') + b'\x00' + b'\x00')


def position(x, y, z):
    return struct.pack('>q', (x & 0x3FFFFFF) << 38 | (z & 0x3FFFFFF) << 12 | (y & 0xFFF))


def payload(channel, data=b''):
    return string(channel) + data


def swallowed(state, name):
    return (state, name) in {('login', 'custom_query_answer'), ('configuration', 'pong'),
                             ('configuration', 'cookie_response'), ('play', 'pong')}


def plan(scenario, port):
    """(trigger (state, name, direction), [(state, packet name, body)]) in the order they fire."""
    if scenario == 'transfer':
        # The cookie the client keeps, then a transfer to this proxy: the second connection is the client's
        # transfer join (intention intent 3), and what it sends first is what a server sees of a transfer.
        return [(('configuration', 'select_known_packs'), [
            ('configuration', 'store_cookie', string('probe:cookie') + varint(5) + b'hello'),
            ('configuration', 'transfer', string('127.0.0.1') + varint(port)),
        ])]
    if scenario == 'sign2':
        return [(('play', 'chunk_batch_finished'), [
            ('play', 'block_update', position(*SIGN) + varint(OAK_SIGN_STATE)),
            ('play', 'block_entity_data', position(*SIGN) + varint(SIGN_BLOCK_ENTITY) + sign_nbt(LINES_2)),
            ('play', 'open_sign_editor', position(*SIGN) + b'\x01'),
        ])]
    if scenario == 'sunrise-signals':
        return [(('configuration', 'select_known_packs'), [
            ('configuration', 'custom_payload', payload('sunrise:payloads', b'\x00\x01')),
            ('configuration', 'custom_payload', payload('minecraft:brand', string('sunrise'))),
        ])]
    return [
        (('login', 'login_compression'), [('login', 'custom_query', varint(7) + string('probe:query') + b'abc')]),
        (('configuration', 'select_known_packs'), [
            ('configuration', 'ping', struct.pack('>i', 0x1234)),
            ('configuration', 'store_cookie', string('probe:cookie') + varint(5) + b'hello'),
            ('configuration', 'cookie_request', string('probe:cookie')),
            ('configuration', 'custom_payload',
             payload('minecraft:register', b'sunrise:payloads\0horizon:payloads\0minecraft:brand\0probe:x')),
            ('configuration', 'custom_payload', payload('minecraft:unregister', b'probe:x')),
            ('configuration', 'custom_payload', payload('probe:unknown', b'\x01\x02')),
            ('configuration', 'custom_report_details', varint(1) + string('probe') + string('detail')),
        ]),
        (('play', 'chunk_batch_finished'), [
            ('play', 'ping', struct.pack('>i', 0x5678)),
            ('play', 'custom_payload', payload('minecraft:register', b'sunrise:payloads\0probe:x')),
            ('play', 'block_update', position(*SIGN) + varint(OAK_SIGN_STATE)),
            ('play', 'block_entity_data', position(*SIGN) + varint(SIGN_BLOCK_ENTITY) + sign_nbt(LINES)),
            ('play', 'open_sign_editor', position(*SIGN) + b'\x01'),
        ]),
    ]


def frame(threshold, packet):
    if threshold < 0:
        return varint(len(packet)) + packet
    if len(packet) < threshold:
        return varint(len(packet) + 1) + b'\x00' + packet
    body = varint(len(packet)) + zlib.compress(packet)
    return varint(len(body)) + body


def install(steps):
    """Make net_capture's proxy fire each step's packets when its trigger packet comes from the server."""
    pending = list(steps)
    original_record = net_capture.Session.record
    original_init = net_capture.Pipe.__init__

    def init(self, session, direction, src, dst):
        original_init(self, session, direction, src, dst)
        if direction == 'clientbound':
            session.client_socket = dst

    def record(self, direction, packet_id, payload_bytes):
        state = self.state[direction]
        name = self.name(direction, packet_id)
        if direction == 'serverbound' and swallowed(state, name):
            self.swallow = True
        original_record(self, direction, packet_id, payload_bytes)
        if direction != 'clientbound':
            return
        for step in list(pending):
            (trigger_state, trigger_name), packets = step
            if (state, name) != (trigger_state, trigger_name):
                continue
            pending.remove(step)
            for packet_state, packet_name, body in packets:
                names = NAMES[(packet_state, 'clientbound')]
                pid = names.index(packet_name)
                packet = varint(pid) + body
                self.client_socket.sendall(frame(self.threshold, packet))
                entry = {'conn': self.id, 't': round(time.time() - self.start, 4), 'dir': 's2c', 'state': packet_state,
                         'id': pid, 'name': packet_name, 'size': len(body), 'data': body.hex(), 'injected': True}
                with self.lock:
                    self.out.write(json.dumps(entry) + '\n')
                    self.out.flush()
                time.sleep(0.05)

    def run(self):
        buffer = b''
        s = self.session
        try:
            while True:
                chunk = self.src.recv(1 << 16)
                if not chunk:
                    break
                buffer += chunk
                while True:
                    try:
                        length, pos = net_capture.read_varint(buffer, 0)
                    except IndexError:
                        break
                    if len(buffer) < pos + length:
                        break
                    frame_bytes, body, buffer = buffer[:pos + length], buffer[pos:pos + length], buffer[pos + length:]
                    if s.threshold >= 0:
                        size, p = net_capture.read_varint(body, 0)
                        body = zlib.decompress(body[p:]) if size else body[p:]
                    packet_id, p = net_capture.read_varint(body, 0)
                    s.swallow = False
                    s.record(self.direction, packet_id, body[p:])
                    pending_threshold = getattr(s, 'pending_threshold', None)
                    if pending_threshold is not None:
                        s.threshold = pending_threshold
                        s.pending_threshold = None
                    if not (self.direction == 'serverbound' and s.swallow):
                        self.dst.sendall(frame_bytes)
        except (OSError, ValueError, zlib.error):
            pass
        finally:
            for sock in (self.src, self.dst):
                try:
                    sock.shutdown(2)
                except OSError:
                    pass
            s.closed.set()

    net_capture.Session.record = record
    net_capture.Pipe.__init__ = init
    net_capture.Pipe.run = run


def press_escape(client, display):
    if client == 'vanilla':
        subprocess.run(['xdotool', 'mousemove', '400', '300'], env=dict(os.environ, DISPLAY=display), check=False)
        time.sleep(0.3)
        subprocess.run(['xdotool', 'key', 'Escape'], env=dict(os.environ, DISPLAY=display), check=False)


def record(args):
    lock, jars, libraries = base.verify_artifacts()
    deps = os.pathsep.join(map(str, [jars['server'], *libraries]))
    vanilla = ['java', '-Xmx1g', '-cp', deps, 'net.minecraft.server.Main', 'nogui']
    work = Path(tempfile.mkdtemp(prefix='join-probe-'))
    for var, sub in (('SUNRISE_HOME', 'home'), ('SUNRISE_LAUNCHER_HOME', 'launcher'), ('XDG_DATA_HOME', 'data'),
                     ('XDG_CONFIG_HOME', 'config')):
        (work / 'servers' / sub).mkdir(parents=True)
        os.environ[var] = str(work / 'servers' / sub)
    os.environ.update(lab.game_env())
    directory = work / 'server'
    directory.mkdir()
    port = free_port()
    props = dict(PROPERTIES, **{'server-port': str(port), 'server-ip': '127.0.0.1'})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()), encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    out = output_folder(args.out, ('capture.jsonl', 'all.json', 'client.log'))
    capture = out / 'capture.jsonl'
    capture.touch()
    server = Server(vanilla, directory, 'server')
    game = xvfb = process = display = None
    try:
        server.wait_for(r'Done \(', timeout=300)
        listen = free_port()
        install(plan(args.scenario, listen))
        stop = net_capture.start_background(listen, f'127.0.0.1:{port}', capture, full=True)
        address = f'127.0.0.1:{listen}'
        if args.client == 'vanilla':
            process, xvfb, display = vanilla_client(address, work, out / 'client.log')
        else:
            game = Headless(Path(args.bin), lab.RESOURCES, work / 'game')
            game('connect', address, PLAYER)
        last = {'sunrise-signals': 'custom_payload', 'transfer': 'transfer'}.get(args.scenario, 'open_sign_editor')
        deadline = time.time() + 240
        while time.time() < deadline and not any(
                e.get('injected') and e['name'] == last and e['state'] in ('play', 'configuration')
                for e in net_capture.load(capture)):
            time.sleep(0.5)
        time.sleep(4)
        if args.scenario in ('base', 'sign2'):
            if game is not None:
                game('key', 'esc', 'tap', ok=False)
            else:
                press_escape('vanilla', display)
            time.sleep(3)
        stop.set()
    finally:
        if game is not None:
            game('disconnect', ok=False)
            game.quit()
        if process is not None:
            process.terminate()
        if xvfb is not None:
            xvfb.terminate()
        server.stop()
    entries = [e for e in net_capture.load(capture) if 'name' in e and (args.scenario == 'transfer' or e.get('conn') == 1)]
    (out / 'all.json').write_text(json.dumps(entries, indent=1))
    shutil.rmtree(work, ignore_errors=True)
    print(f'{len(entries)} packets in {out / "all.json"}')


def describe(e):
    data = bytes.fromhex(e.get('data', ''))
    text = f"{e['state']}/{e['name']}"
    if e['name'] == 'custom_payload' and data:
        n = data[0]
        return f"{text} channel={data[1:1 + n].decode(errors='replace')} body={data[1 + n:].hex()}"
    if e['name'] == 'intention':
        version, p = net_capture.read_varint(data, 0)
        n, q = net_capture.read_varint(data, p)
        host = data[q:q + n].decode()
        port = int.from_bytes(data[q + n:q + n + 2], 'big')
        return f"{text} protocol={version} host={host} port={'(proxy port)' if port else 0} intent={data[q + n + 2]}"
    if e['name'] in ('sign_update',):
        # position, the four lines, then the side's flag
        lines, p = [], 8
        for _ in range(4):
            n, q = net_capture.read_varint(data, p)
            lines.append(data[q:q + n].decode(errors='replace'))
            p = q + n
        return f'{text} lines={lines} front={data[p]}'
    return f'{text} {data.hex()[:80]}'


def answers(path):
    """For each injected packet: the injected one and what the client sent between it and the next injection."""
    entries = json.loads((Path(path) / 'all.json').read_text())
    out = []
    for e in entries:
        if e.get('injected'):
            out.append([f"-> {describe(e)}"])
        elif e['dir'] == 'c2s' and out and e['name'] not in ('client_tick_end', 'chunk_batch_received', 'move_player_pos',
                                                              'move_player_status_only', 'move_player_pos_rot',
                                                              'move_player_rot', 'keep_alive'):
            out[-1].append(f"<- {describe(e)}")
    return out


def diff(args):
    a, b = answers(args.a), answers(args.b)
    lines = ['| probe | vanilla | sunrise |', '|---|---|---|']
    differences = 0
    for left, right in zip(a, b):
        same = left == right
        differences += not same
        lines.append(f"| {left[0][3:]} | {'<br>'.join(left[1:]) or 'nothing'} | "
                     f"{'(same)' if same else '<br>'.join(right[1:]) or 'nothing'} |")
    if len(a) != len(b):
        lines.append(f'| probe count | {len(a)} | {len(b)} |')
        differences += 1
    text = '\n'.join(lines)
    print(text)
    if args.md:
        Path(args.md).write_text(text + '\n')
    sys.exit(1 if differences else 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    r = sub.add_parser('record')
    r.add_argument('client', choices=['vanilla', 'sunrise'])
    r.add_argument('--bin', default=str(lab.cargo_target() / 'fast'))
    r.add_argument('--out', required=True)
    r.add_argument('--scenario', choices=['base', 'sign2', 'sunrise-signals', 'transfer'], default='base')
    d = sub.add_parser('diff')
    d.add_argument('a')
    d.add_argument('b')
    d.add_argument('--md')
    args = parser.parse_args()
    (record if args.command == 'record' else diff)(args)


if __name__ == '__main__':
    main()
