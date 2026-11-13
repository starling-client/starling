"""What native_mod_adapt.py changes in a mod's code (packaging/android/native_mod_adapt.py)."""
import importlib.util
import unittest
from pathlib import Path

ADAPT = Path(__file__).resolve().parents[1] / "packaging" / "android" / "native_mod_adapt.py"
spec = importlib.util.spec_from_file_location("native_mod_adapt", ADAPT)
adapt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(adapt)

OLD_STATE = """Break(ItemRenderState {
    oversized_in_gui: resolved.oversized_in_gui,
    animated,
})"""
UPDATED_STATE = """Break(ItemRenderState {
    oversized_in_gui: resolved.oversized_in_gui,
    animated,
    cit_glint: None,
    hdr_tier: 0,
    hdr_light: 0,
})"""


class Code(unittest.TestCase):
    def test_a_mod_from_before_the_new_fields_gets_them(self):
        self.assertEqual(adapt.code(OLD_STATE).count("cit_glint: None"), 1)

    def test_a_mod_that_already_has_them_stays_as_it_is(self):
        # Eating Animation 0.2.0: a second copy is E0062 and no build.
        self.assertEqual(adapt.code(UPDATED_STATE), UPDATED_STATE)


class Calls(unittest.TestCase):
    def test_a_mod_calling_content_root_again_passes_what_it_now_takes(self):
        said = adapt.code("Break(content_root(root, enabled, &with))")
        self.assertEqual(said, "Break(content_root(root, enabled, &with, None, &mut |_| {}))")


class Batch(unittest.TestCase):
    def test_an_old_batch_takes_the_last_batchs_gui_scale(self):
        # Wherever it is: a method of `Game` (`self`), a helper with `game`, or the renderer.
        said = adapt.code("let batch = HudBatch::new([r.config.width, r.config.height]);")
        self.assertEqual(said, "let batch = HudBatch::new_old([r.config.width, r.config.height]);")

    def test_a_new_batch_stays(self):
        line = "let b = HudBatch::new(size, game.settings.gui_scale_inputs());"
        self.assertEqual(adapt.code(line), line)


class Moved(unittest.TestCase):
    def test_an_untinted_sprite_goes_to_the_old_helper(self):
        said = adapt.code("game.native_sprite(batch, &skin, [8.0, 8.0, 8.0, 8.0], dst);")
        self.assertEqual(said, "game.native_sprite_old(batch, &skin, [8.0, 8.0, 8.0, 8.0], dst);")
        line = "game.native_sprite(batch, &skin, src, dst, Some(tint));"
        self.assertEqual(adapt.code(line), line)

    def test_a_hover_is_shared(self):
        said = adapt.code("hover: Some(\n    Box::new(Text::literal(\"x\")),\n),")
        self.assertEqual(said, "hover: Some(\n    std::sync::Arc::new(Text::literal(\"x\")),\n),")

    def test_the_mob_aimed_at_gives_three_things(self):
        said = adapt.code("let Some((index, distance)) = self.mob_hit() else {")
        self.assertEqual(said, "let Some((index, distance, _)) = self.mob_hit() else {")
        line = "if let Some((i, d, at)) = self.mob_hit() {"
        self.assertEqual(adapt.code(line), line)

    def test_a_players_uuid_is_a_method(self):
        said = adapt.code("game.native_acting_player().map(|p| p.uuid),")
        self.assertEqual(said, "game.native_acting_player().map(|p| p.uuid()),")
        line = "game.native_acting_player().map(|p| p.uuid()),"
        self.assertEqual(adapt.code(line), line)

    def test_a_mods_fluid_goes_with_no_fluid(self):
        said = adapt.code("let b = match f.fluid {\n    Fluid::None => 0,\n    Fluid::Water => 1,\n    Fluid::Lava => 2,\n};")
        self.assertIn("Fluid::None | Fluid::Modded => 0,", said)
        said = adapt.code("match f.fluid { horizon_client::blocks::Fluid::None => return, _ => {} }")
        self.assertIn("horizon_client::blocks::Fluid::None | horizon_client::blocks::Fluid::Modded =>", said)
        # A mod's own `Fluid`, or code that already names a mod's fluid, stays.
        own = "enum Fluid { None, Oil }\nmatch x { Fluid::None => 0, Fluid::Oil => 1 }"
        self.assertEqual(adapt.code(own), own)

    def test_old_menu_calls_go_to_the_old_helpers(self):
        said = adapt.code("game.menu_click(Some(i), 0, horizon_client::menu::Input::Pickup);")
        self.assertEqual(said, "game.menu_click_old(Some(i), 0, horizon_client::menu::Input::Pickup);")
        said = adapt.code(".map_while(|s| Game::menu_slot(&menu, s))")
        self.assertEqual(said, ".map_while(|s| Game::menu_slot_old(&menu, s))")
        line = "game.menu_click(menu, &[click])"
        self.assertEqual(adapt.code(line), line)

    def test_a_translation_with_a_fallback_goes_to_translate_or(self):
        said = adapt.code('on: game.translate("options.on", "ON"),')
        self.assertEqual(said, 'on: game.translate_or("options.on", "ON"),')
        line = 'let t = Text::translate("key", &[]);'
        self.assertEqual(adapt.code(line), line)

    def test_a_names_closure_becomes_the_names(self):
        said = adapt.code("let names = |id: &str| game.pack.names.display_item(id);")
        self.assertEqual(said, "let names = &game.pack.names;")

    def test_the_equipment_slot_is_cores(self):
        said = adapt.code("m.item_by_slot(horizon_world::player_inventory::EquipmentSlot::Chest)")
        self.assertEqual(said, "m.item_by_slot(horizon_core::equipment_slot::EquipmentSlot::Chest)")


class Gear(unittest.TestCase):
    def test_a_mobs_old_item_ids_are_its_equipment(self):
        said = adapt.code("hands([m.extra.main_hand.as_deref(), m.extra.off_hand.as_deref()], at)")
        self.assertEqual(
            said,
            "hands([m.extra.equipment.get(horizon_items::equipment::MAINHAND).map(|s| &*s.item).as_deref(),"
            " m.extra.equipment.get(horizon_items::equipment::OFFHAND).map(|s| &*s.item).as_deref()], at)",
        )
        said = adapt.code("let stack = m.extra.armor[1].as_ref();")
        self.assertEqual(said, "let stack = m.extra.equipment[horizon_items::equipment::ARMOR[1]].as_ref();")
        # Writes, and the stack of the held item, stay for the compiler to judge.
        for line in ("m.extra.head = None;", "m.extra.main_hand_stack.clone()"):
            self.assertEqual(adapt.code(line), line)

    def test_what_is_stuck_and_the_saddle(self):
        said = adapt.code("(m.extra.arrow_count, m.extra.stinger_count, m.extra.saddled || m.extra.saddle_stack.is_some())")
        self.assertEqual(
            said,
            "(m.extra.stuck.arrows, m.extra.stuck.stingers, m.extra.saddled()"
            " || m.extra.equipment[horizon_items::equipment::SADDLE].is_some())",
        )
        self.assertEqual(adapt.code("m.extra.saddled()"), "m.extra.saddled()")

    def test_a_stands_hands(self):
        said = adapt.code("let [main, off, ..] = &a.equipment;")
        self.assertEqual(said, "let (main, off) = (&a.equipment[0], &a.equipment[1]);")


class Guests(unittest.TestCase):
    def test_a_loop_changing_guests_is_update_alls_callback(self):
        said = adapt.code("for g in host.hub.guests.iter_mut().filter(|g| g.placed) {\n    g.moved = true;\n}\nafter();")
        self.assertEqual(said, "host.hub.guests.update_all(|g| { if !(g.placed) { return; }\n    g.moved = true;\n});\nafter();")

    def test_a_loop_that_leaves_early_stays(self):
        line = "for g in host.hub.guests.iter_mut() {\n    if !g.placed {\n        continue;\n    }\n}"
        self.assertEqual(adapt.code(line), line)


class Items(unittest.TestCase):
    def test_a_custom_name_is_nbt_text(self):
        said = adapt.code("stack.data_mut().custom_name = Some(WAND_NAME.to_owned());")
        self.assertEqual(said, "stack.data_mut().custom_name = Some(horizon_core::nbt::Tag::String(WAND_NAME.into()));")
        said = adapt.code("stack.data().and_then(|data| data.custom_name.as_deref())\n    == Some(WAND_NAME)")
        self.assertEqual(
            said,
            "stack.data().and_then(|data| data.custom_name.as_ref()) == Some(&horizon_core::nbt::Tag::String(WAND_NAME.into()))",
        )

    def test_a_stack_given_is_added_to_the_inventory(self):
        self.assertEqual(adapt.code("match self.give(item::new_stack()) {"), "match self.add_stack_to_inventory(item::new_stack()) {")
        line = "game.give(key, 1)"
        self.assertEqual(adapt.code(line), line)


class Hud(unittest.TestCase):
    def test_a_system_toast_goes_through_the_hook(self):
        said = adapt.code(
            "let new = horizon_client::toasts::SystemToast::new(title, Some(message));\n"
            'if game.toast_allowed("system", "periodic_notification") {\n'
            "    game.server_packs.load_fail = Some(new);\n}"
        )
        self.assertIn("SystemToast::new(horizon_client::text::Text::literal(title), Some(horizon_client::text::Text::literal(message)))", said)
        self.assertIn("if let Some(new) = game.add_system_toast(new) {", said)

    def test_the_status_rows_height(self):
        said = adapt.code("status_bar_height: game\n    .health_hud\n    .left_height\n    .max(game.health_hud.right_height),")
        self.assertEqual(said, "status_bar_height: game.status_rows_height_old(),")

    def test_options_never_go_back_to_the_game(self):
        said = adapt.code("let (a, b) = (game.options_from_title, game.options_to_game);\ngame.options_to_game = b;")
        self.assertEqual(said, "let (a, b) = (game.options_from_title, false);\nlet _ = b;")


class World(unittest.TestCase):
    def test_a_dimension_as_a_number_is_its_old_number(self):
        self.assertEqual(adapt.code("hp::frame(game.dim as u8, at)"), "hp::frame(game.dim.old_number() as u8, at)")

    def test_an_items_glow_goes_by_its_id(self):
        said = adapt.code("if game.entity_glowing(item.id) {")
        self.assertEqual(said, "if game.tracked_entity_glowing(item.id) {")

    def test_a_map_slot_is_always_claimed(self):
        said = adapt.code("let (slot, old) = game.maps.slots.claim(id);")
        self.assertEqual(said, "let (slot, old) = game.maps.slots.claim_old(id);")

    def test_layers_as_a_function_take_render_withs_parameters(self):
        said = adapt.code("let draw = if v { horizon_client::mobs::render::render_with } else { horizon_client::mobs::render::render_layers };")
        self.assertIn("else { horizon_client::starling_mod_api::render_layers_old }", said)

    def test_a_tooltips_display_comes_from_the_games_stack(self):
        said = adapt.code("!crate::item_tooltip::TooltipDisplay::for_stack(stack).shows(\"minecraft:container\")")
        self.assertEqual(said, "!crate::item_tooltip::tooltip_display(stack).shows(\"minecraft:container\")")


class Texts(unittest.TestCase):
    def test_spans_go_through_texts_own_calls(self):
        said = adapt.code("t.spans.push(Span { text, style: Style::color(color) });")
        self.assertEqual(said, "t.push(Text::from_spans(vec![Span::new(text, Style::color(color))]));")
        said = adapt.code("out.spans.push(Span { text: std::mem::take(literal), style: Style::default() });")
        self.assertEqual(said, "out.push(Text::from_spans(vec![Span::new(std::mem::take(literal), Style::default())]));")
        self.assertEqual(adapt.code("out.spans.extend(arg.spans.iter().cloned());"), "out.push(arg.clone());")
        self.assertEqual(adapt.code("a.spans.extend(b.spans);"), "a.push(b);")
        said = adapt.code("for s in &mut t.spans {\n    if s.style.color.is_none() {\n        s.style.color = Some(color);\n    }\n}")
        self.assertEqual(said, "t = t.with_color(color);")


class Panel(unittest.TestCase):
    OLD = (
        "fn view(v: &m::View) -> NativeRecipeView {\n    NativeRecipeView {\n        note: v.note.clone(),\n    }\n}\n"
        "game.native_panel_recipes(m::ID, &title, tabs, tab, views);\n"
        "NativePanelEvent::SlotClicked { stack: s, right, .. } => m::Event::SlotClicked { stack: stack(s), right: *right },"
    )

    def test_an_old_recipe_panel_keeps_its_shape(self):
        said = adapt.code(self.OLD)
        self.assertIn("fn view(v: &m::View) -> NativeRecipeViewOld {\n    NativeRecipeViewOld {", said)
        self.assertIn("game.native_panel_recipes_old(m::ID", said)
        self.assertIn("stack: stack(&horizon_client::starling_mod_api::native_stack_old(s))", said)

    def test_a_panel_with_fluids_is_the_new_one(self):
        new = self.OLD.replace("note: v.note.clone(),", "note: v.note.clone(),\n        fluid_inputs: Vec::new(),")
        self.assertEqual(adapt.code(new), new)


class OldShapes(unittest.TestCase):
    def test_a_call_in_the_old_shape_goes_to_the_old_helper(self):
        said = adapt.code("let w = game.native_text_width(&line.text) + 4.0;")
        self.assertEqual(said, "let w = game.native_text_width_old(&line.text) + 4.0;")

    def test_a_call_in_the_new_shape_stays(self):
        line = "let w = game.native_text_width(batch, f(a, b));"
        self.assertEqual(adapt.code(line), line)

    def test_the_mods_crate_is_horizon_mod_package(self):
        said = adapt.code("if horizon_mods::evanders_config::enabled() {}")
        self.assertEqual(said, "if horizon_mod_package::evanders_config::enabled() {}")

    def test_nbt_is_horizon_cores(self):
        self.assertEqual(adapt.code("use horizon_world::nbt::Tag;"), "use horizon_core::nbt::Tag;")


class OldApi(unittest.TestCase):
    def test_the_old_mod_api_module_is_starlings(self):
        said = adapt.code("use horizon_client::mod_api::NativeScreenView;\nuse crate::mod_api;\n")
        self.assertEqual(
            said,
            "use horizon_client::starling_mod_api::NativeScreenView;\n"
            "use crate::starling_mod_api as mod_api;\n",
        )


GAME = """use super::Game;
impl Game<'_> {
    pub(in crate::game) fn native_tick(&mut self) {}
    /// A frame.
    pub(in crate::game) fn native_frame(&mut self, elapsed: Duration) {
        let _ = elapsed;
    }
    pub(in crate::game) fn native_gui_layer_post(&self, layer: &str, draw: &mut HudBatch) {
        self.fire(layer, draw);
    }
    pub(in crate::game) fn native_gui_layer_post_old(&mut self, layer: &str, draw: &mut HudBatch) {
        let _ = (layer, draw);
    }
    pub(in crate::game) fn native_use_item_on(
        &mut self,
        state: BlockStateId,
        click: &Click,
    ) {
    }
    pub(in crate::game) fn native_use_item_on_old(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
    ) {
    }
}
impl fmt::Debug for Game<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Ok(())
    }
}
pub fn draw_minimap(batch: &mut HudBatch) {}
"""


class Index(unittest.TestCase):
    def test_methods_are_read_with_their_parameters(self):
        found, _ = adapt.module_items(GAME)
        # A one-line body does not hide the method after it.
        self.assertIn(("Game", "native_frame"), found.methods)
        self.assertEqual(found.parameters[("Game", "native_tick")], ("&mut self", []))
        self.assertEqual(found.parameters[("Game", "native_frame")], ("&mut self", ["elapsed"]))
        self.assertEqual(
            found.parameters[("Game", "native_gui_layer_post")], ("&self", ["layer", "draw"])
        )
        self.assertEqual(found.parameters[("Game", "native_use_item_on")][1], ["state", "click"])
        self.assertNotIn(("", "fmt"), found.methods)
        self.assertNotIn(("Game", "fmt"), found.methods)
        self.assertIn("draw_minimap", found.functions)

    def test_mod_lines_keep_their_path(self):
        _, declared = adapt.module_items('#[path = "game/runtime/feature.rs"]\nmod feature;\nmod plain;\n')
        self.assertEqual(declared, [("feature", "game/runtime/feature.rs"), ("plain", None)])


def tree():
    found, _ = adapt.module_items(GAME)
    return {"horizon_client::game::native_hooks::base": found, "horizon_client::game::native_hooks": adapt.Module()}


class Relocate(unittest.TestCase):
    def test_a_moved_method_is_found_by_its_type_and_name(self):
        self.assertEqual(
            adapt.relocate("horizon_client::game::native_hooks::Game<'_>::native_frame", tree()),
            "horizon_client::game::native_hooks::base::Game<'_>::native_frame",
        )

    def test_a_trait_method_moves_with_its_impl_never_to_a_free_function(self):
        handler, _ = adapt.module_items(
            "impl ApplicationHandler for Game<'_> {\n"
            "    fn window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {\n"
            "    }\n"
            "}\n"
        )
        touch, _ = adapt.module_items("fn window_event(step: Pointer) -> WindowEvent {\n}\n")
        modules = {
            "horizon_client::game": adapt.Module(),
            "horizon_client::game::window_handler": handler,
            "horizon_client_lib::touch": touch,
        }
        self.assertEqual(
            adapt.relocate("horizon_client::game::<Game<'_> as ApplicationHandler>::window_event", modules),
            "horizon_client::game::window_handler::<Game<'_> as ApplicationHandler>::window_event",
        )

    def test_what_is_nowhere_stays_as_it_was(self):
        target = "horizon_client::game::native_hooks::Game<'_>::native_gone"
        self.assertEqual(adapt.relocate(target, tree()), target)


INJECTION = """[[injection]]
id = "hud"
kind = "HEAD"
target = "horizon_client::game::native_hooks::base::Game<'_>::native_gui_layer_post"
helper = "hud.rs"
bindings = [
  { name = "batch", ty = "&mut HudBatch", value = "&mut *batch" },
]
"""


class Rebind(unittest.TestCase):
    def test_a_renamed_parameter_is_bound_by_its_new_name(self):
        said = adapt.rebind(INJECTION, tree(), {})
        self.assertIn('{ name = "batch", ty = "&mut HudBatch", value = "&mut *draw" }', said)

    def test_old_parameters_bind_to_the_old_function_starling_keeps(self):
        old = """[[injection]]
target = "horizon_client::game::native_hooks::base::Game<'_>::native_use_item_on"
bindings = [
  { name = "pos", ty = "[i32; 3]", value = "pos" },
]
"""
        said = adapt.rebind(old, tree(), {})
        self.assertIn('target = "horizon_client::game::native_hooks::base::Game<\'_>::native_use_item_on_old"', said)

    def test_a_game_to_change_binds_to_the_old_function_where_the_new_one_only_reads_it(self):
        mutating = INJECTION.replace(
            "bindings = [\n",
            'bindings = [\n  { name = "game", ty = "&mut crate::game::Game<\'_>", value = "&mut *self" },\n',
        )
        said = adapt.rebind(mutating, tree(), {})
        self.assertIn("native_gui_layer_post_old", said)
        self.assertIn('value = "&mut *draw"', said)
        # Reading the game is fine where the new function reads it.
        self.assertNotIn("native_gui_layer_post_old", adapt.rebind(INJECTION, tree(), {}))


class Includes(unittest.TestCase):
    def test_an_included_file_beside_it_is_written_in(self):
        import tempfile

        with tempfile.TemporaryDirectory() as root:
            folder = Path(root) / "crates/horizon-client/src/game/native_hooks"
            folder.mkdir(parents=True)
            (folder / "mod.rs").write_text('mod fire;\ninclude!(concat!(env!("OUT_DIR"), "/x.rs"));\ninclude!("shared.rs");\nfn after() {}\n')
            (folder / "shared.rs").write_text("fn shared() {}\n")
            adapt.inline_includes(Path(root))
            self.assertEqual(
                (folder / "mod.rs").read_text(),
                'mod fire;\ninclude!(concat!(env!("OUT_DIR"), "/x.rs"));\nfn shared() {}\nfn after() {}\n',
            )
            # Horizon's generator would read its items twice.
            self.assertFalse((folder / "shared.rs").exists())


if __name__ == "__main__":
    unittest.main()
