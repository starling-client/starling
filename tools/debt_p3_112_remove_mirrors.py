#!/usr/bin/env python3
"""One-shot removal of the retired Game and vocabulary snapshot subgraphs."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CLIENT = ROOT / "crates/sunrise-client/src"
GAME = CLIENT / "game"
MODDING = GAME / "modding"
HOOKS = GAME / "native_hooks"
VOCAB = ROOT / "crates/sunrise-mod-vocab/src"


def replace(source, old, new="", count=1):
    assert source.count(old) == count, old
    return source.replace(old, new)


def cut(source, start, end):
    assert source.count(start) == 1, start
    first = source.index(start)
    return source[:first] + source[source.index(end, first + len(start)):]


path = MODDING / "native_components.rs"
s = cut(path.read_text(), "/// Failure to access a native item component.", "/// Use the attack calculations")
s = cut(s, "// Native mods use these helpers.", "#[cfg(test)]\nmod equippable_slot_tests")
path.write_text(s)

path = MODDING / "native_inventories.rs"
s = path.read_text()
start = s.index("    /// Sends the player")
end = s.index("\n#[cfg(test)]", start)
path.write_text('//! Map pictures sent to individual players.\nuse super::Game;\nuse mod_api::{__private::game::Fields, PlayerRef};\n\nimpl Game<\'_> {\n' + s[start:end])

path = MODDING / "native_menus.rs"
path.write_text(cut(path.read_text(), "/// Adds every stack of `gets`", "/// Server menus, for server mods."))

path = MODDING / "native_mobs.rs"
s = cut(path.read_text(), "//! Mobs for native server mods", "use super::{Game, commands::Target};")
s = "//! Mob and player equipment-change dispatch and server mob actions.\n" + s
s = cut(s, "/// A mob as a native mod reads it.", "fn finite(")
s = cut(s, "// Native mods use these; nothing in Sunrise does.", "    /// Kills mob `id`")
s = s[:s.index("    fn native_mob_of(")] + "}\n"
s = s.replace("    /// Kills mob `id`", "impl Game<'_> {\n    /// Kills mob `id`", 1)
s = replace(s, "        if self.native_mob(dimension, id).is_none() {", """        if self.remote.is_some()
            || dimension != self.dim.id()
            || !self.mobs.iter().find(|m| m.id == id && !m.removed).is_some_and(|m| m.alive())
        {""")
path.write_text(s)

path = MODDING / "native_server.rs"
s = cut(path.read_text(), "/// A sidebar a server mod", "/// The report's name of an operator level")
s = cut(s, "    /// Shows `sidebar` on that player's screen", "    /// The lines over and under that player's list")
s = replace(s, """        self.native_send_all(uuid, vec![packet])
    }
    fn native_send_all(&mut self, uuid: u128, packets: Vec<C>) -> bool {
        if self.lan_acting_uuid == Some(uuid) && !self.lan_acting_fake {
            self.guest_packets.extend(packets);""", """        if self.lan_acting_uuid == Some(uuid) && !self.lan_acting_fake {
            self.guest_packets.push(packet);""")
s = replace(s, """        for packet in &packets {
            host.hub.send_to(g.conn, packet);
        }
""", "        host.hub.send_to(g.conn, &packet);\n")
path.write_text(s)

path = MODDING / "native_recipes.rs"
s = cut(path.read_text(), "//! Every item and recipe", "use mod_api::{LevelRef,")
s = """//! Recipe lookup, placement and client synchronization. The client uses its own
//! recipes, with server-synced types taking precedence, and follows the server's
//! book for known displays.
use super::{Game, api_values::TryIntoApi};
""" + s
s = cut(s, "use std::{collections::HashMap, sync::Arc};", "/// The most recipes a server's recipe content may carry.")
s = s.replace("/// The most recipes", "use std::sync::Arc;\nuse sunrise_client::items::ItemStack;\n\n/// The most recipes", 1)
s = replace(s, "    tables: Option<Arc<Recipes>>,\n")
s = cut(s, "/// Item id -> its index", "/// A stack's components beyond its item's")
s = cut(s, "fn recipe(id: String, kind: RecipeKind)", "impl Game<'_> {")
s = cut(s, "    /// The tables, made on the first call;", "    /// The `book_id`s the player's recipe book holds now.")
s = cut(s, "    fn make_mod_recipes(", "}\n\n/// The window's own recipes outside")
s = replace(s, "\n        self.recipe_tables.tables = None;", "")
s = replace(s, "                self.recipe_tables.tables = None;\n", count=2)
s = replace(s, "                // What was made of the window's own recipes is made again.\n")
path.write_text(s)

path = HOOKS / "shared/system_helpers.rs"
s = cut(path.read_text(), "/// Helpers for native mods", "    pub(in crate::game) fn native_block_collision_in(")
s = s.replace("use super::*;\n\n", "use super::*;\n\nimpl Game<'_> {\n", 1)
s = cut(s, "    /// The window's width and height", "    /// The areas the game itself draws")
s = cut(s, "    /// The player, in a world.", "    /// A native mod's own folder:")
s = s[:s.index("    /// Every item and recipe the game knows,")] + "}\n"
path.write_text(s)

path = HOOKS / "shared/world_helpers.rs"
s = cut(path.read_text(), "    /// The item entities of the server level", "    /// This `Game` is the window's,")
path.write_text(s)

path = HOOKS / "shared.rs"
s = cut(path.read_text(), "/// The player as a native mod reads it.", "/// How the panel lays out a recipe")
s = cut(s, "/// An item of [`NativeRecipes::items`],", "/// The Post a mouse input owes")
s = cut(s, "/// An item entity as `native_item_entities` lists it.", "/// What `mod_screen_open(mod_id)` does")
path.write_text(s)

path = HOOKS / "mod.rs"
path.write_text(replace(path.read_text(), "    chat::ChatSource,\n"))

for path, marker in [(MODDING / "native_visuals.rs", "player"), (HOOKS / "base.rs", "p")]:
    s = replace(path.read_text(), f"let Some({marker}) = self.native_player() else", "let Some(position) = self.player.as_ref().map(|player| player.position) else")
    for axis, index in zip("xyz", range(3)):
        s = replace(s, f"(at[{index}] - {marker}.{axis}).powi(2)", f"(at[{index}] - position.{axis}).powi(2)")
    path.write_text(s)

path = GAME / "interface/extras.rs"
s = replace(path.read_text(), "//! camera; clicks and frame times for mods (`native_clicks`, `native_frame_rate`).", "//! camera.")
s = replace(s, "extras::{Corner, Cps, FpsCounter, fov_target}", "extras::{Corner, fov_target}")
s = replace(s, "use winit::event::MouseButton;\n")
s = cut(s, "    fps: FpsCounter,", "    /// Captured before pre listeners;")
s = cut(s, "    fn extras_now(", "    /// Once per frame:")
s = cut(s, "        // For mods (`render.frame-stats`", "        let (world_partial_tick, partial_tick)")
s = cut(s, "    /// The frame counter for mods:", "    /// A mod's zoom in effect")
path.write_text(s)

path = CLIENT / "game.rs"
path.write_text(replace(path.read_text(), "                            self.extras_mouse(button, pressed);\n"))
path = GAME / "interface/pad/mod.rs"
path.write_text(cut(path.read_text(), "                // What the window path tells mods and the click counters.", "                if !down || playing {"))
path = GAME / "runtime/control.rs"
s = replace(path.read_text(), "                        self.extras_mouse(button, true);\n", count=2)
s = replace(s, "                        self.extras_mouse(button, false);\n", count=2)
path.write_text(s)

path = VOCAB / "entity/types.rs"
path.write_text(cut(path.read_text(), "/// A player on the server, as a server hook sees them.", "/// A persistent entity UUID"))
path = VOCAB / "built.rs"
s = cut(path.read_text(), "// no wire: borrowed game-to-hook adapter; its call fields cross separately.\npub struct ServerPlayer", "#[derive(sunrise_mod_api_macros::Wire)]\npub struct ChannelRef")
path.write_text(s)
path = VOCAB / "exports/built.rs"
path.write_text(replace(path.read_text(), "ScreenView, ServerPlayer, SoundPlay", "ScreenView, SoundPlay"))
path = VOCAB / "item/item_tooltip.rs"
path.write_text(cut(path.read_text(), "    // hidden: a constructor for generated wire code", "    /// Read tooltip visibility from an owned API stack"))
path = ROOT / "crates/sunrise-mod-api-gen/dispatch-tests/src/fixtures/base.rs"
path.write_text(replace(path.read_text(), "    pub(super) struct ServerPlayer;\n"))

path = MODDING / "native_guest_turn_tests.rs"
s = replace(path.read_text(), """/// Native services can reach B during A's turn without exposing displaced
/// inventories or changing ordinary command resolution during nested turns.""", """/// Native services can reach B during A's turn without changing ordinary
/// command resolution during nested turns.""")
s = replace(s, """    use super::{
        commands::Target,
        native_inventories::NativeInventoryKind::{Ender, Player},
    };""", "    use super::commands::Target;")
before, marker, body = s.partition('fn another_guests_services_work_during_a_turn() {')
assert marker
body, end, after = body.partition('/// The body/saddle store')
assert end
s = before + marker + cut(body, '        let stone = ItemStack::new(', '        let a = game.player(12)') + end + after
s = cut(s, '            assert!(game.native_inventory(12, Player).is_err());\n            assert_eq!(', '            assert_eq!(\n                a.trigger_criterion')
s = replace(s, """                assert!(game.native_inventory(12, Player).is_err());
                assert!(game.native_inventory(13, Player).is_err());
""")
s = replace(s, "        assert_eq!(b.ender_items[0], Some(stone));\n")
s = replace(s, "    use super::native_inventories::NativeInventoryKind::Player;\n", count=2)
s = replace(s, """        let holds = |game: &super::Game<'_>, uuid: u128, item: &str| {
            let stack = ItemStack::new(item, 1);
            let slots = game.native_inventory(uuid, Player).unwrap().slots;
            slots.iter().flatten().any(|s| *s == stack)
        };""", """        let holds = |game: &super::Game<'_>, conn: u32, item: &str| {
            let stack = ItemStack::new(item, 1);
            game.lan.as_ref().unwrap().hub.guest(conn).unwrap().inventory
                .iter().flatten().any(|s| *s == stack)
        };""")
s = replace(s, """            let slots = game.native_inventory(13, Player).unwrap().slots;
            slots.iter().flatten().any(|s| *s == stick)""", """            game.lan.as_ref().unwrap().hub.guest(13).unwrap().inventory
                .iter().flatten().any(|s| *s == stick)""")
path.write_text(s)

path = GAME / "network/lan_actions/shared.rs"
path.write_text(replace(path.read_text(), "    /// by `NativeInventoryKind`: Player, Ender, Cursor, Crafting.", "    /// in player inventory, ender chest, cursor and crafting order."))

path = GAME / "items/recipe_book.rs"
path.write_text(replace(path.read_text(), "    /// Every entry of the book, in its order (`native_recipes`).", "    /// Every entry of the book, in its order.\n    #[cfg(test)]"))

path = GAME / "interface/pad/parity_tests.rs"
old = "the_pads_attack_and_use_reach_what_the_mouse_tells_mods"
new = "the_pads_attack_and_use_reach_the_vanilla_key_mappings"
s = replace(path.read_text(), old, new)
path.write_text("".join(line for line in s.splitlines(keepends=True) if "game.native_clicks()" not in line))

path = HOOKS / "mod_block_entity_payload_tests.rs"
# Keep the identity/encoding regression, using the mutation the adapter wrapped.
path.write_text('''#![cfg(test)]

#[test]
fn replacing_mod_block_entity_payload_keeps_its_type_and_saved_identity() {
    use sunrise_client::containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields};
    use sunrise_client::save::{block_entity_encode, decode_block_entity};

    let pos = (1, 64, 1);
    let mut machine = ModBlockEntity::new("test:machine".into(), Default::default());
    let made = machine.revision();
    *machine = ModBlockEntityFields {
        data: vec![7],
        ..Default::default()
    };
    assert_eq!(machine.id(), "test:machine");
    assert_eq!(machine.data, [7]);
    assert_ne!(machine.revision(), made);
    let entity = BlockEntity::Mod(Box::new(machine));
    let saved = block_entity_encode::encode(pos, &entity, "minecraft:stone", &Default::default());
    let (saved_pos, loaded) = decode_block_entity(&saved).unwrap();
    assert_eq!(saved_pos, pos);
    assert_eq!(loaded.id(), "test:machine");
}
''')

path = ROOT / "tools/ci/ignored-tests/daily-sunrise-client-issue-4451.txt"
s = replace(path.read_text(), old, new)
s = replace(s, "sunrise-client game::native_hooks::mod_block_entity_payload_tests::replacing_mod_block_entity_payload_keeps_its_type_and_saved_identity lib daily resources\n")
path.write_text(s)
