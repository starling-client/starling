"""The old mod messages keep every link they need (crates/sunrise-client/src/game/starling/old_mod_net.rs).

The fork replaced Sunrise's `sunrise:mod_net` channel with declared payloads, which the website's mods made
for the old API never declare (ayynoxs_map_pack and elytratrails send and hear their messages with
`native_net_send_to_*` and the `net_from_*` hooks). Sync 7 dropped them until Starling brought them back:
the server's announcement, the client's flag, both ends' receiving, the host's queue and its flushes, and the
old hooks that mods inject into. A sync that loses one of these links fails here.
"""

import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "crates/sunrise-client/src"


def read(path: str) -> str:
    return (SRC / path).read_text(encoding="utf-8")


class OldModNet(unittest.TestCase):
    def test_the_protocol_module_is_there(self):
        lib = (ROOT / "crates/sunrise-protocol/src/lib.rs").read_text(encoding="utf-8")
        self.assertIn("pub mod mod_net;", lib)
        self.assertIn('pub const CHANNEL: &str = "sunrise:mod_net";',
                      (ROOT / "crates/sunrise-protocol/src/mod_net.rs").read_text(encoding="utf-8"))

    def test_the_server_announces_it_and_the_client_hears_it(self):
        self.assertIn("channel: sunrise_protocol::mod_net::CHANNEL.into()", read("lan/conn.rs"))
        net = read("net.rs")
        self.assertIn("payload.channel == sunrise_protocol::mod_net::CHANNEL", net)
        self.assertIn("Event::ModNet", net)
        remote = read("game/network/remote/mod.rs")
        self.assertIn("remote.mod_net = true;", remote)
        # A reconfiguration forgets it, as the other announcements.
        self.assertIn("remote.mod_net = false;", remote)

    def test_both_ends_receive_their_messages(self):
        remote = read("game/network/remote/mod.rs")
        self.assertIn("self.remote_old_mod_net(&p.data);", remote)
        self.assertIn("self.tick_old_mod_net();", remote)
        self.assertIn("self.guest_old_mod_net(conn, &p.data)", read("game/network/lan_actions/packets.rs"))

    def test_the_host_runs_and_sends_them(self):
        lan = read("game/network/lan/mod.rs")
        self.assertIn("self.run_old_mod_net();", lan)
        self.assertIn("self.flush_old_mod_net();", lan)
        # What the mods said as the players left goes out before the server stops.
        for path in ("game/native_hooks/shared.rs", "game/world/integrated.rs", "game/interface/dedicated/mod.rs"):
            self.assertIn("self.flush_old_mod_net();", read(path), path)

    def test_the_old_hooks_and_sends_are_where_mods_find_them(self):
        compat = read("game/starling/native_compat.rs")
        for name in ("native_net_from_client", "native_net_from_server",
                     "native_net_send_to_server", "native_net_send_to_player"):
            self.assertIn(f"pub(in crate::game) fn {name}(", compat, name)
        self.assertIn("self.old_net_send_to_server(channel, data)", compat)
        self.assertIn("self.old_net_send_to_player(player, channel, data)", compat)
        old = read("game/starling/old_mod_net.rs")
        self.assertIn("self.native_net_from_client(player, &m.channel, &m.data);", old)
        self.assertIn("self.native_net_from_server(&m.channel, &m.data)", old)


if __name__ == "__main__":
    unittest.main()
