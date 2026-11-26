#!/usr/bin/env python3
"""Split hub packet sequences where gameplay must lend the live command tree."""
import re
from pathlib import Path

CLIENT = Path("crates/sunrise-client")
HUB = CLIENT / "src/lan/hub.rs"


def replace(text, old, new):
    assert text.count(old) == 1, old
    return text.replace(old, new)


def migrate_hub():
    text = HUB.read_text()
    text = replace(text, '''    /// PlayerList.placeNewPlayer for a guest whose saved state is loaded.
    pub fn place_new_player(&mut self, conn: u32, ctx: &mut TickCtx) {
        let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        if !self.guests[i].participates_online() {
            return;
        }
        let (name, uuid, entity_id, mode) = {
            let g = &self.guests[i];
            (g.name.clone(), g.uuid, g.entity_id, g.game_mode)
        };''', '''    /// PlayerList.placeNewPlayer up to the command-tree send.
    pub fn begin_player_join(&mut self, conn: u32, ctx: &TickCtx) -> Option<usize> {
        let i = self.guests.connection_index(conn)?;
        if !self.guests[i].participates_online() {
            return None;
        }''')
    text = replace(text, '''        let held = self.guests[i].held;
        let g = &self.guests[i];
        g.send(&C::Login(play::Login {
            player_id: entity_id,''', '''        let g = &self.guests[i];
        g.send(&C::Login(play::Login {
            player_id: g.entity_id,''')
    text = replace(text, '''        // PlayerList.sendPlayerPermissionLevel, then the recipe book.
        self.send_permission_level(i);
        self.send_initial_recipe_book(i);''', '''        self.send_permission_level(i);
        Some(i)
    }

    /// The rest of PlayerList.placeNewPlayer, after the player's command tree.
    pub fn finish_player_join(&mut self, i: usize, ctx: &TickCtx) {
        let g = &self.guests[i];
        let (conn, name, uuid, mode, held) = (g.conn, g.name.clone(), g.uuid, g.game_mode, g.held);
        self.send_initial_recipe_book(i);''')
    text = replace(text, '''    pub fn respawn(&mut self, i: usize, at: [f64; 3], rotation: Option<[f32; 2]>, ctx: &TickCtx) {''', '''    pub fn begin_respawn(&mut self, i: usize, at: [f64; 3], rotation: Option<[f32; 2]>, ctx: &TickCtx) {''')
    text = replace(text, '''        self.send_permission_level(i);
        // ServerLevel.addRespawnedPlayer''', '''        self.send_permission_level(i);
    }

    /// The rest of PlayerList.respawn, after sending the command tree.
    pub fn finish_respawn(&mut self, i: usize) {
        // ServerLevel.addRespawnedPlayer''')
    text = replace(text, '''        &mut self,
        i: usize,
        at: [f64; 3],
        yaw: f32,
        pitch: Option<f32>,
        portal: bool,
        portal_cooldown: i32,
        ctx: &TickCtx,
    ) {
        let info = self.spawn_info(i, portal_cooldown, ctx.sea_level);''', '''        &mut self,
        i: usize,
        at: [f64; 3],
        yaw: f32,
        pitch: Option<f32>,
        portal_cooldown: i32,
        ctx: &TickCtx,
    ) {
        let info = self.spawn_info(i, portal_cooldown, ctx.sea_level);''')
    text = replace(text, 'pub fn change_level(', 'pub fn begin_level_change(')
    text = replace(text, '''        self.send_permission_level(i);
        if portal {''', '''        self.send_permission_level(i);
    }

    /// Finish the level change after sending the command tree.
    pub fn finish_level_change(&mut self, i: usize, portal: bool, ctx: &TickCtx) {
        if portal {''')
    text = text.replace('[`Self::change_level`]', '[`Self::finish_level_change`]')
    text = replace(text, "        self.send_commands(i);\n    }", "    }")
    text = replace(text, "                self.send_permission_level(i);\n            }", "                self.send_permission_level(i);\n                self.send_commands(i);\n            }")
    text = replace(text, "    fn commands_packet(", "    pub fn commands_packet(")
    HUB.write_text(text)
    path = HUB.parent / "hub/selector_sync.rs"
    text = replace(path.read_text(), "pub fn send_commands(&self, i: usize) {", "pub fn send_commands(&self, i: usize, mut packet: play::Commands) {")
    path.write_text(replace(text, "        let mut packet = self.commands_packet(g.permission);\n", ""))


def migrate_packet_fixtures():
    # These callers drive the hub without gameplay or native callbacks.
    files = [HUB, CLIENT / "src/lan/arrivals.rs", CLIENT / "tests/lan_bot.rs"]
    files.extend(sorted((CLIENT / "src/lan/hub/tests").glob("*.rs")))
    files.extend(sorted((CLIENT / "src/lan/viewed_levels").glob("tests.rs")))
    for path in files:
        old = text = path.read_text()
        text = re.sub(
            r"(?P<hub>(?:[a-z_]+\.)?hub)\.place_new_player\((?P<conn>[^,]+), (?P<ctx>[^)]+)\)",
            r"{ if let Some(i) = \g<hub>.begin_player_join(\g<conn>, \g<ctx>) { "
            r"\g<hub>.send_commands(i); \g<hub>.finish_player_join(i, \g<ctx>); } }", text,
        )
        text = re.sub(
            r"(?P<hub>(?:[a-z_]+\.)?hub)\.respawn\((?P<index>\d), (?P<args>.*?), (?P<ctx>&?ctx)\)",
            r"{ \g<hub>.begin_respawn(\g<index>, \g<args>, \g<ctx>); "
            r"\g<hub>.send_commands(\g<index>); \g<hub>.finish_respawn(\g<index>); }", text,
        )
        text = re.sub(
            r"(?P<hub>(?:[a-z_]+\.)?hub)\.change_level\((?P<index>\d), (?P<args>.*?), "
            r"(?P<portal>true|false), (?P<cooldown>\d+), (?P<ctx>&?ctx)\)",
            r"{ \g<hub>.begin_level_change(\g<index>, \g<args>, \g<cooldown>, \g<ctx>); "
            r"\g<hub>.send_commands(\g<index>); \g<hub>.finish_level_change(\g<index>, \g<portal>, \g<ctx>); }", text,
        )
        text = re.sub(r"^( +)\{ (.*?\.begin_(?:player_join|respawn|level_change)\(.*?) \};$", r"\1\2", text, flags=re.M)
        text = re.sub(r"(\.(?:begin|finish)_player_join\([^,\n]+, )&mut ctx\)", r"\1&ctx)", text)
        if path.name == "arrivals.rs":
            text = replace(text, "        let mut ctx = TickCtx {\n            sea_level: 63,", "        let ctx = TickCtx {\n            sea_level: 63,")
        if text != old:
            path.write_text(text)


def migrate_game():
    paths = {
        "network/lan/mod.rs": [(
            "        host.hub.place_new_player(conn, &mut ctx);",
            "        if let Some(i) = host.hub.begin_player_join(conn, &ctx) {\n"
            "            host.hub.send_commands(i);\n"
            "            host.hub.finish_player_join(i, &ctx);\n"
            "        }",
        ), ("        let mut ctx = TickCtx {\n            sea_level:",
            "        let ctx = TickCtx {\n            sea_level:")],
        "network/lan_actions/respawn.rs": [(
            "            host.hub.respawn(i, spawn, rotation, &ctx);",
            "            host.hub.begin_respawn(i, spawn, rotation, &ctx);\n"
            "            host.hub.send_commands(i);\n"
            "            host.hub.finish_respawn(i);",
        )],
        "world/server_travel.rs": [(
            ".change_level(i, at, yaw, None, true, PLAYER_DELAY, &ctx);",
            ".begin_level_change(i, at, yaw, None, PLAYER_DELAY, &ctx);\n"
            "            host.hub.send_commands(i);\n"
            "            host.hub.finish_level_change(i, true, &ctx);",
        ), (
            ".change_level(i, t.at, yaw, Some(pitch), false, cooldown, &ctx);",
            ".begin_level_change(i, t.at, yaw, Some(pitch), cooldown, &ctx);\n"
            "                host.hub.send_commands(i);\n"
            "                host.hub.finish_level_change(i, false, &ctx);",
        )],
    }
    for filename, replacements in paths.items():
        path = CLIENT / "src/game" / filename
        text = path.read_text()
        for old, new in replacements:
            text = replace(text, old, new)
        path.write_text(text)


if __name__ == "__main__":
    migrate_hub()
    migrate_packet_fixtures()
    migrate_game()
    for path in (*CLIENT.glob("src/**/*.rs"), CLIENT / "tests/lan_bot.rs"):
        old = path.read_text()
        text = re.sub(
            r"(?P<hub>(?:\w+\.)*(?:hub|self))\.send_commands\((?P<i>[i0])\)",
            r"\g<hub>.send_commands(\g<i>, \g<hub>.commands_packet(\g<hub>.guests[\g<i>].permission))",
            old,
        )
        if text != old:
            path.write_text(text)
