"""Unit tests for tools/datafix_worlds.py: the Java runtime choice, the bot's configuration phase, the server stop."""
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from tools import datafix_worlds as worlds


class JavaRuntimeTests(unittest.TestCase):
    def java_for(self, component, platform_name, home_name):
        with tempfile.TemporaryDirectory() as temp:
            home = Path(temp, 'java', home_name)
            (home / 'bin').mkdir(parents=True)
            (home / 'bin' / 'java').write_text('')
            pin = worlds.LOCK[platform_name]['java-runtime-gamma']
            (home / '.complete').write_text(pin['manifest_sha1'])
            with mock.patch.object(worlds, 'LOCAL', Path(temp)), mock.patch.object(worlds, 'platform', return_value=platform_name), \
                    mock.patch.object(worlds, 'version_meta', return_value={'javaVersion': {'component': component}}), \
                    mock.patch.object(worlds, 'install_runtime', side_effect=AssertionError('no download')):
                return worlds.java('x'), home

    def test_a_component_no_platform_pins_is_an_error(self):
        with self.assertRaisesRegex(ValueError, 'java-runtime-zeta'):
            self.java_for('java-runtime-zeta', 'linux', 'java-runtime-gamma-linux')

    def test_a_component_this_platform_lacks_uses_gamma_as_the_lock_says(self):
        exe, home = self.java_for('jre-legacy', 'mac-os-arm64', 'java-runtime-gamma-mac-os-arm64')
        self.assertEqual(exe, str(home / 'bin' / 'java'))


class UnpinnedPlatformTests(unittest.TestCase):
    def test_a_platform_the_lock_does_not_pin_is_refused_by_name_before_any_download(self):
        for plat in ('windows-x64', 'mac-os'):
            with self.subTest(plat), mock.patch.object(worlds, 'platform', return_value=plat), \
                    mock.patch.object(worlds, 'version_meta', return_value={'javaVersion': {'component': 'java-runtime-gamma'}}), \
                    mock.patch.object(worlds, 'install_runtime', side_effect=AssertionError('no download')), \
                    self.assertRaisesRegex(ValueError, f'platform {plat};'):
                worlds.java('x')

    def test_an_unpinned_platform_is_refused_before_version_metadata_is_read(self):
        with mock.patch.object(worlds, 'platform', return_value='windows-x64'), \
                mock.patch.object(worlds, 'version_meta', side_effect=AssertionError('no metadata download')), \
                self.assertRaisesRegex(ValueError, 'platform windows-x64;'):
            worlds.java('x')

    def test_platform_names_what_mojang_calls_the_machine(self):
        for system, machine, expected in (('win32', 'AMD64', 'windows-x64'), ('win32', 'ARM64', 'windows-arm64'),
                                          ('darwin', 'x86_64', 'mac-os'), ('darwin', 'arm64', 'mac-os-arm64'), ('linux', 'x86_64', 'linux'), ('linux', 'i686', 'linux-i386')):
            with self.subTest(system, machine=machine), mock.patch.object(worlds, 'sys', SimpleNamespace(platform=system)), \
                    mock.patch.object(worlds, 'host', SimpleNamespace(machine=lambda: machine)):
                self.assertEqual(worlds.platform(), expected)


class UnsupportedMachineTests(unittest.TestCase):
    def test_a_machine_mojang_has_no_runtime_for_is_refused_by_name(self):
        for system, machine in (('linux', 'aarch64'), ('win32', 'ia64')):
            with self.subTest(system, machine=machine), mock.patch.object(worlds, 'sys', SimpleNamespace(platform=system)), \
                    mock.patch.object(worlds, 'host', SimpleNamespace(machine=lambda: machine)), \
                    self.assertRaisesRegex(ValueError, f'machine {machine}'):
                worlds.platform()


class ServerStopTests(unittest.TestCase):
    def test_a_server_that_ignores_stop_is_killed_and_the_timeout_reaches_the_caller(self):
        server = worlds.Server.__new__(worlds.Server)
        server.process = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'], stdin=subprocess.PIPE, text=True)
        self.addCleanup(server.process.kill)
        real_wait = server.process.wait
        calls = []

        def wait(timeout=None):
            calls.append(timeout)
            if timeout == 120:
                raise subprocess.TimeoutExpired('server', 120)
            return real_wait()
        server.process.wait = wait
        with self.assertRaises(subprocess.TimeoutExpired):
            server.stop()
        self.assertIsNotNone(server.process.poll())
        self.assertEqual(calls, [120, None])

    def test_a_server_already_killed_with_its_stdin_closed_stops_without_an_error(self):
        # A server may exit on its own before cleanup.
        server = worlds.Server.__new__(worlds.Server)
        server.process = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'], stdin=subprocess.PIPE, text=True)
        self.addCleanup(server.process.kill)
        server.process.kill()
        server.process.wait()
        server.process.stdin.close()
        server.stop()
        self.assertIsNotNone(server.process.poll())


class KnownPacksTests(unittest.TestCase):
    def configure(self, protocol, received):
        """What the bot sends while it configures against a server that sends `received` (id, payload), then finish."""
        bot = worlds.Bot.__new__(worlds.Bot)
        bot.protocol = protocol
        sent = []
        packets = iter([*received, (3, b'')])
        bot.receive = lambda: next(packets)
        bot.send = lambda packet_id, payload=b'': sent.append((packet_id, payload))
        bot.configure()
        return sent

    def test_the_bot_answers_only_the_select_known_packs_packet_of_its_protocol(self):
        # What select_known_packs carries: one pack, namespace minecraft, id core, version.
        offer = b'\x01\x09minecraft\x04core\x041.21'
        answer = (7, b'\x00')
        finish = (3, b'')
        # 26.2: select_known_packs is 14 and update_tags 15. A bare minecraft-shaped payload on 15 is left alone.
        self.assertEqual(self.configure(776, [(15, offer), (14, offer)]), [answer, finish])
        # 26.3: post_effects comes in before it, so update_tags is 14 and select_known_packs 15.
        self.assertEqual(self.configure(777, [(14, offer), (15, offer)]), [answer, finish])

    def test_a_protocol_with_no_known_id_is_refused(self):
        with self.assertRaisesRegex(ValueError, 'protocol 999'):
            self.configure(999, [])


class BotLeaveTests(unittest.TestCase):
    def test_second_leave_skips_the_wait_and_socket_close(self):
        bot = worlds.Bot.__new__(worlds.Bot)
        bot.stop = threading.Event()
        bot.sock = mock.Mock()
        sleep = mock.Mock()
        with mock.patch.object(worlds, 'time', SimpleNamespace(sleep=sleep)):
            bot.leave()
            bot.leave()
        self.assertTrue(bot.stop.is_set())
        sleep.assert_called_once_with(1.5)
        bot.sock.close.assert_called_once_with()


class BotFailedJoinTests(unittest.TestCase):
    def join_against(self, packets, protocol):
        """Run Bot against a local server that sends `packets` (id, payload) once connected; returns the error and the bot's socket."""
        listener = socket.socket()
        self.addCleanup(listener.close)
        listener.bind(('127.0.0.1', 0))
        listener.listen(1)

        def serve():
            peer, _ = listener.accept()
            with peer:
                for packet_id, payload in packets:
                    body = bytes([packet_id]) + payload
                    peer.sendall(bytes([len(body)]) + body)
                peer.settimeout(5)
                try:
                    while peer.recv(4096):
                        pass
                except OSError:
                    pass
        threading.Thread(target=serve, daemon=True).start()
        made = []
        create = socket.create_connection
        with mock.patch.object(worlds.socket, 'create_connection', lambda *a, **k: made.append(create(*a, **k)) or made[-1]):
            with self.assertRaises((RuntimeError, ValueError)) as caught:
                worlds.Bot(listener.getsockname()[1], protocol, 'x')
        self.addCleanup(made[0].close)
        return caught.exception, made[0]

    def test_a_refused_login_closes_the_socket(self):
        error, sock = self.join_against([(0, b'no')], 770)
        self.assertIsInstance(error, RuntimeError)
        self.assertEqual(sock.fileno(), -1)

    def test_a_failed_configuration_closes_the_socket(self):
        # Login succeeds, then the bot has no select_known_packs id for protocol 999 and refuses.
        error, sock = self.join_against([(2, b'')], 999)
        self.assertIsInstance(error, ValueError)
        self.assertEqual(sock.fileno(), -1)


class BuildCleanupTests(unittest.TestCase):
    """build() with the server, the bot and the scene replaced: what is closed when something fails."""

    def build(self, scene_error, stop_error):
        events = []

        class FakeBot:
            def __init__(self, port, protocol, name):
                events.append('joined')

            def leave(self):
                events.append('left')

        class FakeServer:
            def __init__(self, *args):
                pass

            def stop(self, *, save):
                events.append('stopped')
                if stop_error:
                    raise stop_error

        def scene(server, run, Bot, *args):
            Bot(1, 2, 'x')
            if scene_error:
                raise scene_error

        with tempfile.TemporaryDirectory() as temp, mock.patch.object(worlds, 'LOCAL', Path(temp)), \
                mock.patch.object(worlds, 'server_jar', return_value=(Path('server.jar'), 3700, 770)), \
                mock.patch.object(worlds, 'java', return_value='java'), mock.patch.object(worlds, 'Server', FakeServer), \
                mock.patch.object(worlds, 'free_port', return_value=12345), \
                mock.patch.object(worlds, 'Bot', FakeBot), mock.patch.object(worlds, 'build_scene', scene), \
                mock.patch.object(worlds, 'time', SimpleNamespace(sleep=lambda seconds: None)):
            try:
                worlds.build('1.21.5')
            except Exception as error:
                return events, error
        return events, None

    def test_a_scene_that_fails_still_closes_the_bot_and_stops_the_server(self):
        events, error = self.build(ValueError('scene failed'), None)
        self.assertEqual((events, str(error)), (['joined', 'left', 'stopped'], 'scene failed'))

    def test_a_stop_that_raises_reaches_the_caller_and_the_bot_is_closed(self):
        events, error = self.build(None, RuntimeError('stop failed'))
        self.assertEqual((events, str(error)), (['joined', 'left', 'stopped'], 'stop failed'))


class BuildFlatCleanupTests(unittest.TestCase):
    """build_flat() with the server, the bots and the scene replaced: what is closed when something fails."""

    def build_flat(self, fail_in):
        events = []

        class FakeBot(worlds.Bot):
            def __init__(self, port, protocol, name):
                events.append('joined')
                self.stop = threading.Event()
                self.sock = SimpleNamespace(close=lambda: events.append('left'))

        class FakeServer:
            def __init__(self, *args, **kwargs):
                pass

            def stop(self, *, save):
                events.append('stopped')

        def populate(server, Bot, *args, **kwargs):
            Bot(1, 340, 'x')
            raise ValueError('populate failed')

        def batch(server, commands, problems, **kwargs):
            if fail_in == 'visit' and commands[0].startswith('tp '):
                raise ValueError('visit failed')

        clock = SimpleNamespace(sleep=lambda seconds: None)
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(worlds, 'LOCAL', Path(temp)), \
                mock.patch.object(worlds, 'server_jar', return_value=(Path('server.jar'), 1343, 340)), \
                mock.patch.object(worlds, 'java', return_value='java'), mock.patch.object(worlds, 'Server', FakeServer), \
                mock.patch.object(worlds, 'Bot', FakeBot), mock.patch.object(worlds, 'time', clock), \
                mock.patch.multiple(worlds.scene12, populate=populate, batch=batch, time=clock, free_port=lambda: 12345):
            with self.assertRaises(ValueError) as caught:
                worlds.scene12.build_flat(worlds)
        return events, str(caught.exception)

    def test_a_visit_that_fails_closes_the_visitor_and_stops_the_server(self):
        self.assertEqual(self.build_flat('visit'), (['joined', 'left', 'stopped'], 'visit failed'))

    def test_a_scene_that_fails_closes_every_bot_and_stops_the_server(self):
        events, error = self.build_flat('populate')
        self.assertEqual((events, error), (['joined', 'left', 'joined', 'left', 'stopped'], 'populate failed'))


class LegacyBuildTests(unittest.TestCase):
    def setUp(self):
        work = Path(self.enterContext(tempfile.TemporaryDirectory()))

        def start(*args, cwd, **kwargs):
            (cwd / 'world').mkdir()
            process.returncode = None
            return process

        self.process = process = mock.Mock(stdout=['Done (\n'], returncode=None)
        process.poll.side_effect = lambda: process.returncode
        process.kill.side_effect = lambda: setattr(process, 'returncode', 1)
        self.stop = stop = mock.Mock()

        class RecordingServer(worlds.Server):
            def stop(self, *args, **kwargs):
                stop(*args, **kwargs)
                super().stop(*args, **kwargs)

        self.enterContext(mock.patch.multiple(
            worlds, LOCAL=work, server_jar=mock.Mock(return_value=(Path('server.jar'), 1343, 340)),
            java=mock.Mock(return_value='java'), free_port=lambda: 12345,
            subprocess=SimpleNamespace(Popen=start, PIPE=subprocess.PIPE, STDOUT=subprocess.STDOUT),
            Server=RecordingServer, time=SimpleNamespace(sleep=lambda seconds: None, time=worlds.time.time)))
        self.lab_actions = mock.Mock()
        self.batch = mock.Mock(return_value=[['Time is 12'], ['Time is 12']])
        self.enterContext(mock.patch.multiple(
            worlds.scene12, time=worlds.time, lab_actions=self.lab_actions, batch=self.batch,
            free_port=lambda: 12345, make_player=lambda *args: lambda *args: mock.Mock()))

    def assert_saved_shutdown(self):
        self.stop.assert_called_once_with(save=False)
        self.process.kill.assert_called_once_with()
        self.process.wait.assert_called_once_with()
        self.process.stdin.close.assert_called_once_with()
        self.process.stdin.write.assert_not_called()  # a graceful stop would save over the last tick

    def test_legacy_world_owner_ends_the_server_once_without_resaving(self):
        worlds.build('1.12.2')
        self.assert_saved_shutdown()

    def test_flat_world_owner_ends_the_server_once_without_resaving(self):
        worlds.scene12.build_flat(worlds)
        self.assert_saved_shutdown()

    def test_older_world_does_not_change_the_next_worlds_origin_or_items(self):
        worlds.server_jar.side_effect = [(Path('server.jar'), 922, 316), (Path('server.jar'), 1343, 340)]
        with mock.patch.multiple(worlds.scene12, spawn_origin=lambda *args: (-32, 96), probe_items=lambda *args: {'stick'}):
            worlds.build('1.11.2')
            worlds.build('1.12.2')
        commands = [command for call in self.batch.call_args_list for command in call.args[1]]
        self.assertEqual([c for c in commands if c.startswith('spawnpoint ')],
                         ['spawnpoint Upgrader -32 100 96', 'spawnpoint Upgrader 144 100 144'])
        self.assertEqual([c for c in commands if c.startswith('replaceitem entity Upgrader slot.hotbar.0 ')],
                         ['replaceitem entity Upgrader slot.hotbar.0 minecraft:stick 1 0',
                          'replaceitem entity Upgrader slot.hotbar.0 minecraft:map 1 0'])

    def test_lab_actions_oserror_reaches_the_caller(self):
        self.lab_actions.side_effect = ConnectionResetError('lab bot disconnected')
        with self.assertRaisesRegex(ConnectionResetError, 'lab bot disconnected'):
            worlds.build('1.12.2')


if __name__ == '__main__':
    unittest.main()
