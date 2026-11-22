#!/usr/bin/env python3
"""What a client announces on joining: every serverbound packet from the handshake through login,
configuration and the first seconds of play, recorded at a vanilla 26.3 dedicated server's side of
tools/net_capture.py, for vanilla's own client and for a Sunrise build, and the side-by-side diff.

  python3 tools/join_announce.py record vanilla  --out .local/join-announce/vanilla
  python3 tools/join_announce.py record sunrise  --bin .local/bin/official --out .local/join-announce/official
  python3 tools/join_announce.py diff .local/join-announce/vanilla .local/join-announce/official [--md FILE]

`record` writes `capture.jsonl` (the proxy's lines) and `serverbound.json` (the client's packets in order:
state, id, name, payload hex). `--bin` is a folder holding `sunrise-client` (and `sunrise-ctl` beside it).
The server has no plugins and signals nothing Sunrise-specific. Games run in fresh temporary folders
(SUNRISE_HOME, SUNRISE_LAUNCHER_HOME, XDG_*, no audio); the vanilla client runs on a private Xvfb screen.
Needs .local (vanilla jars, prepared resources), and for `sunrise` a built sunrise-client and sunrise-ctl.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import net_capture  # noqa: E402
import test_rust_parity as base  # noqa: E402
from server_pack_e2e import Game  # noqa: E402
from test_dedicated_server_parity import Server, free_port  # noqa: E402
from vanilla_wire_e2e import PROPERTIES  # noqa: E402

PLAYER = 'Steve'
PLAY_SECONDS = 5.0


class Headless(Game):
    """server_pack_e2e's game without a window or GPU."""

    def __init__(self, bins, resources, work):
        self.home = work / 'home'
        for d in ('home', 'launcher', 'data', 'config'):
            (work / d).mkdir(parents=True)
        self.env = lab.game_env(SUNRISE_HOME=self.home, SUNRISE_LAUNCHER_HOME=work / 'launcher',
                                XDG_DATA_HOME=work / 'data', XDG_CONFIG_HOME=work / 'config',
                                SUNRISE_TELEMETRY_URL='off')
        self.base = [str(bins / 'sunrise-ctl'), '--socket', str(work / 'c.sock')]
        self.log = self.home / 'logs' / 'latest.log'
        reply = self('launch', '--game', bins / 'sunrise-client', '--headless', '--resources', resources)
        self.pid = reply.get('pid')


def vanilla_client(address, work, log):
    import vanilla_client
    args = argparse.Namespace(name=PLAYER, join=address, world=None, game_dir=work / 'client', assets=None,
                              heap='2G', width=854, height=480, lan_port=None, backend=None)
    args.game_dir.mkdir(parents=True)
    vanilla_client.write_options(args.game_dir, {'maxFps': '30'})
    command = vanilla_client.command(args)
    env = vanilla_client.environment()
    env.pop('WAYLAND_DISPLAY', None)
    # Xvfb picks a free display and writes its number, so two runs never share a screen.
    read, write = os.pipe()
    xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write), '-screen', '0', '1024x768x24'],
                            stderr=subprocess.DEVNULL, pass_fds=(write,))
    os.close(write)
    with os.fdopen(read) as number:
        display = f':{number.readline().strip()}'
    env['DISPLAY'] = display
    env['LIBGL_ALWAYS_SOFTWARE'] = '1'
    process = subprocess.Popen(command, cwd=args.game_dir, env=env, stdout=open(log, 'w'),
                               stderr=subprocess.STDOUT)
    return process, xvfb, display


def output_folder(path, own):
    """`--out`, emptied of this tool's own files from an earlier run. A folder holding anything
    else is refused, so a mistyped argument never deletes a checkout or a home folder."""
    out = Path(path)
    if out.exists():
        if not out.is_dir():
            sys.exit(f'--out {out} is a file')
        stray = sorted(p.name for p in out.iterdir() if p.name not in own)
        if stray:
            sys.exit(f'--out {out} holds files this tool did not write ({", ".join(stray[:5])}); '
                     'name a new or empty folder')
        for name in own:
            (out / name).unlink(missing_ok=True)
    out.mkdir(parents=True, exist_ok=True)
    return out


def record(args):
    lock, jars, libraries = base.verify_artifacts()
    deps = os.pathsep.join(map(str, [jars['server'], *libraries]))
    vanilla = ['java', '-Xmx1g', '-cp', deps, 'net.minecraft.server.Main', 'nogui']
    work = Path(tempfile.mkdtemp(prefix='join-announce-'))
    for var, sub in (('SUNRISE_HOME', 'home'), ('SUNRISE_LAUNCHER_HOME', 'launcher'), ('XDG_DATA_HOME', 'data'),
                     ('XDG_CONFIG_HOME', 'config')):
        (work / 'servers' / sub).mkdir(parents=True)
        os.environ[var] = str(work / 'servers' / sub)
    os.environ.update(lab.game_env())
    directory = work / 'server'
    directory.mkdir()
    port = free_port()
    props = dict(PROPERTIES, **{'server-port': str(port), 'server-ip': '127.0.0.1'})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()),
                                                 encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    out = output_folder(args.out, ('capture.jsonl', 'serverbound.json', 'client.log'))
    capture = out / 'capture.jsonl'
    capture.touch()
    server = Server(vanilla, directory, 'server')
    game = xvfb = process = None
    try:
        server.wait_for(r'Done \(', timeout=300)
        listen = free_port()
        stop = net_capture.start_background(listen, f'127.0.0.1:{port}', capture, full=True)
        address = f'127.0.0.1:{listen}'
        if args.client == 'vanilla':
            process, xvfb, _ = vanilla_client(address, work, out / 'client.log')
        else:
            game = Headless(Path(args.bin), lab.RESOURCES, work / 'game')
            game('connect', address, PLAYER)
        deadline = time.time() + 240
        while time.time() < deadline and not any(
                e.get('name') == 'login' and e.get('state') == 'play' for e in net_capture.load(capture)):
            time.sleep(0.5)
        time.sleep(PLAY_SECONDS)
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
    entries = [e for e in net_capture.load(capture) if e.get('conn') == 1 and 'name' in e and e['dir'] == 'c2s']
    sent = [{k: e[k] for k in ('state', 'id', 'name', 'data') if k in e} for e in entries]
    (out / 'serverbound.json').write_text(json.dumps(sent, indent=1))
    shutil.rmtree(work, ignore_errors=True)
    print(f'{len(sent)} serverbound packets in {out / "serverbound.json"}')


def describe(p):
    """One readable line: packet name and, for a custom payload, its channel and body."""
    data = bytes.fromhex(p.get('data', ''))
    name = f"{p['state']}/{p['name']} (id {p['id']})"
    if p['name'] == 'intention':
        # protocol version, then the address string; the port and the intent vary by run, not by client.
        version, pos = net_capture.read_varint(data, 0)
        return f'{name} protocol={version} address={data[pos + 1:pos + 1 + data[pos]].decode()}'
    if p['state'] == 'play':
        return f"{name}"
    if p['name'] == 'custom_payload' and data:
        n = data[0]
        return f"{name} channel={data[1:1 + n].decode(errors='replace')} body={data[1 + n:].hex()}"
    return f"{name} {data.hex()[:80]}"


def diff(args):
    import difflib
    a = json.loads((Path(args.a) / 'serverbound.json').read_text())
    b = json.loads((Path(args.b) / 'serverbound.json').read_text())
    # Play packets repeat (ticks, chunk batches) and their counts and order follow timing: the first of each name.
    def lines(packets):
        out = []
        for p in packets:
            line = describe(p)
            if p['state'] != 'play' or line not in out:
                out.append(line)
        return out
    la, lb = lines(a), lines(b)
    lines = ['| # | vanilla | sunrise |', '|---|---|---|']
    matcher = difflib.SequenceMatcher(None, la, lb, autojunk=False)
    n = 0
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        for k in range(max(i2 - i1, j2 - j1)):
            left = la[i1 + k] if i1 + k < i2 else ''
            right = lb[j1 + k] if j1 + k < j2 else ''
            n += 1
            mark = '' if tag == 'equal' else ' **≠**'
            lines.append(f'| {n}{mark} | {left} | {right} |')
    text = '\n'.join(lines)
    print(text)
    if args.md:
        Path(args.md).write_text(text + '\n')
    differing = sum(1 for t in matcher.get_opcodes() if t[0] != 'equal')
    sys.exit(1 if differing else 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    r = sub.add_parser('record')
    r.add_argument('client', choices=['vanilla', 'sunrise'])
    r.add_argument('--bin', default=str(lab.cargo_target() / 'fast'))
    r.add_argument('--out', required=True)
    d = sub.add_parser('diff')
    d.add_argument('a')
    d.add_argument('b')
    d.add_argument('--md')
    args = parser.parse_args()
    (record if args.command == 'record' else diff)(args)


if __name__ == '__main__':
    main()
