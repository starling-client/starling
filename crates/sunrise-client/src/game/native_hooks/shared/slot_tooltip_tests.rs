use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};

const MENU: &str = "tips:chest";

struct SlotTip {
    enabled: bool,
    mode: Arc<AtomicU8>,
    calls: Arc<AtomicUsize>,
}
impl mod_api::MenuScreenFactory for SlotTip {
    fn create(
        &self,
        _: &dyn mod_api::ClientHost,
        open: mod_api::MenuScreenOpen<'_>,
    ) -> Option<mod_api::MenuScreen> {
        let screen = mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
            width: 176.0,
            height: 166.0,
            inventory_x: 8.0,
            inventory_y: 84.0,
            slots: (0..open.backing_slots)
                .map(|index| {
                    Some(mod_api::ScreenSlot {
                        position: [
                            8.0 + (index % 9) as f32 * 18.0,
                            18.0 + (index / 9) as f32 * 18.0,
                        ],
                        no_item_icon: None,
                    })
                })
                .collect(),
            ..Default::default()
        });
        if !self.enabled {
            return Some(screen);
        }
        let (mode, calls) = (self.mode.clone(), self.calls.clone());
        Some(screen.with_tooltip(move |_, frame, slot, lines| {
            // Player inventory slot 9 is menu slot 27 in a three-row chest.
            assert_eq!(slot, Some(27));
            assert_eq!(frame.gui_size, [512, 512]);
            assert_eq!(frame.origin, [168.0, 173.0]);
            assert_eq!(frame.pointer, [184, 265]);
            assert_eq!(frame.at, [16.0, 92.0]);
            assert!(
                lines
                    .iter()
                    .any(|(line, _)| *line == mod_api::Text::literal("item hook tail"))
            );
            calls.fetch_add(1, Ordering::Relaxed);
            match mode.load(Ordering::Relaxed) {
                1 => *lines = vec![(mod_api::Text::literal("replacement"), u32::MAX)],
                2 => lines.clear(),
                3 => lines.push((mod_api::Text::literal("slot hint"), u32::MAX)),
                4 => {
                    lines.clear();
                    panic!("fixture tooltip panic after editing")
                }
                _ => {}
            }
        }))
    }
}
struct SlotImageData;
impl mod_api::TooltipComponentData for SlotImageData {}

struct SlotImageClient(Arc<AtomicUsize>);
impl mod_api::ClientTooltipComponent for SlotImageClient {
    fn height(&self, _: &dyn mod_api::TooltipFont) -> i32 {
        10
    }
    fn width(&self, _: &dyn mod_api::TooltipFont) -> i32 {
        20
    }
    fn draw_image(
        &self,
        _: &dyn mod_api::TooltipFont,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: &mut dyn mod_api::DrawContext,
    ) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Debug, PartialEq)]
enum ObservedComponent {
    Text(mod_api::Text, u32),
    Custom,
}

#[derive(Clone)]
struct Image;
#[mod_api::sunrise_part]
impl mod_api::ItemPart for Image {
    fn tooltip_image(
        &self,
        _: &dyn mod_api::ClientHost,
        _: &mut mod_api::TooltipImageEvent<'_>,
    ) -> Option<mod_api::TooltipComponent> {
        Some(mod_api::TooltipComponent::custom(SlotImageData))
    }
}
struct Listener {
    enabled: bool,
    painted: Arc<AtomicUsize>,
    mode: Arc<AtomicU8>,
    calls: Arc<AtomicUsize>,
    drawn: Arc<Mutex<Vec<Vec<ObservedComponent>>>>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Listener {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let item = r.item_key("minecraft:stone");
        r.item_part(item, Image);
        r.register_menu_screen(
            MENU,
            Arc::new(SlotTip {
                enabled: self.enabled,
                mode: self.mode.clone(),
                calls: self.calls.clone(),
            }),
        )
        .unwrap();
    }
    fn register_client_tooltip_component_factories(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterClientTooltipComponentFactoriesEvent<'_>,
    ) {
        let painted = self.painted.clone();
        event
            .factories
            .register::<SlotImageData, SlotImageClient>(move |_| SlotImageClient(painted.clone()));
    }
    fn item_tooltip(&self, _: &dyn mod_api::ClientHost, event: &mut mod_api::ItemTooltipEvent<'_>) {
        event
            .lines
            .push(mod_api::Text::literal("item hook tail"), 0xFF124578)
            .unwrap();
    }
    fn render_tooltip_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderTooltipPreEvent<'_>,
    ) -> mod_api::Flow {
        let snapshot = event
            .components
            .iter()
            .filter_map(|component| {
                if let Some((text, color)) = component.as_text() {
                    Some(ObservedComponent::Text(text.clone(), color))
                } else if component.downcast_ref::<SlotImageClient>().is_some() {
                    Some(ObservedComponent::Custom)
                } else {
                    None
                }
            })
            .collect();
        self.drawn.lock().unwrap().push(snapshot);
        mod_api::Flow::Continue
    }
    fn render_tooltip_texture(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderTooltipTextureEvent<'_>,
    ) {
        assert!(event.stack.is_some());
        event.background = 0xFF123456;
    }
}

#[test]
#[ignore = "daily"]
fn filled_slot_tooltip_replaces_lines_but_keeps_item_image_and_visibility() {
    for enabled in [false, true] {
        let painted = Arc::new(AtomicUsize::new(0));
        let mode = Arc::new(AtomicU8::new(1));
        let calls = Arc::new(AtomicUsize::new(0));
        let drawn = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "tips",
            Box::new(Listener {
                enabled,
                painted: painted.clone(),
                mode: mode.clone(),
                calls: calls.clone(),
                drawn: drawn.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            let mut factories =
                sunrise_client::tooltip_components::TooltipComponentFactories::default();
            {
                crate::game::Game::native_register_client_tooltip_component_factories(
                    &<mod_api::RegistrationContext as mod_api::__private::game::RegistrationContextNew>::new(),
                    &mut factories,
                );
            }
            sunrise_client::tooltip_components::with_test_factories(factories, || {
                mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
                crate::game::native_entity_refs::tests::with_client_game(|game| {
                    let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(pipe, "tips").unwrap();
                    sunrise_client::containers::with_mod_menu_types(
                        [(
                            MENU.into(),
                            "tips".into(),
                            27,
                            0,
                            vec![],
                            vec![],
                            None,
                            None,
                            true,
                            None,
                        )],
                        || {
                            seed_native_menu(game, 1, MENU);
                            let key = (i32::MIN, 0, 0);
                            game.container = Some(key);
                            game.world
                                .block_entities
                                .insert(key, BlockEntity::Chest(vec![None; 27]));
                            assert!(game.open_native_screen(key, None, 27, None));
                            game.set_screen(Screen::Inventory);
                            assert!(game.native_screen_shown());
                            let mut stack = ItemStack::new("minecraft:stone", 1);
                            stack.data_mut().custom_name =
                                Some(sunrise_core::nbt::Tag::compound([
                                    ("text", sunrise_core::nbt::Tag::String("Named stone".into())),
                                    ("color", sunrise_core::nbt::Tag::String("gold".into())),
                                    ("italic", sunrise_core::nbt::Tag::Byte(1)),
                                ]));
                            game.inventory[9] = Some(stack.clone());
                            let mut batch = HudBatch::scratch([512, 512]);
                            let (ox, oy) = game.inventory_origin(&batch);
                            let (x, y) = game.slot_position(9);
                            game.cursor = (ox + x + 8.0, oy + y + 8.0);
                            assert_eq!(game.slot_at(&batch, game.cursor.0, game.cursor.1), Some(9));
                            game.draw_stack_tooltip(
                                &mut batch,
                                &stack,
                                game.cursor.0,
                                game.cursor.1,
                            );
                            let original = drawn.lock().unwrap().pop().unwrap();
                            assert!(
                                original
                                    .iter()
                                    .any(|element| matches!(element, ObservedComponent::Custom))
                            );
                            let before_image_draw = painted.load(Ordering::Relaxed);
                            batch.vertices.clear();
                            game.draw_slot_tooltip(&mut batch);
                            assert!(
                                painted.load(Ordering::Relaxed) > before_image_draw,
                                "the retained image is drawn"
                            );
                            assert!(
                                batch
                                    .vertices
                                    .iter()
                                    .any(|v| v.color == rgba(0x12, 0x34, 0x56, 0xFF)),
                                "the stack-aware box style is drawn"
                            );
                            let actual = drawn.lock().unwrap().pop().unwrap();
                            if enabled {
                                assert!(
                                    matches!(&actual[0], ObservedComponent::Text(text, _) if *text == mod_api::Text::literal("replacement"))
                                );
                                let images = |elements: &[ObservedComponent]| {
                                    elements
                                        .iter()
                                        .filter(|element| {
                                            matches!(element, ObservedComponent::Custom)
                                        })
                                        .cloned()
                                        .collect::<Vec<_>>()
                                };
                                assert!(images(&actual) == images(&original));
                                assert_eq!(actual.len(), 2);
                                mode.store(0, Ordering::Relaxed);
                                game.draw_slot_tooltip(&mut batch);
                                assert!(
                                    drawn.lock().unwrap().pop().unwrap() == original,
                                    "passing preserves styled item lines"
                                );
                                mode.store(3, Ordering::Relaxed);
                                game.draw_slot_tooltip(&mut batch);
                                let appended = drawn.lock().unwrap().pop().unwrap();
                                assert!(
                                    appended[..original.len()] == original,
                                    "adding a hint preserves every styled source entry"
                                );
                                assert!(
                                    matches!(appended.last().unwrap(), ObservedComponent::Text(text, _) if *text == mod_api::Text::literal("slot hint"))
                                );
                                mode.store(2, Ordering::Relaxed);
                                game.draw_slot_tooltip(&mut batch);
                                assert!(drawn.lock().unwrap().pop().unwrap() == images(&original));
                            } else {
                                assert!(
                                    actual == original,
                                    "without a slot listener, use the item's tooltip"
                                );
                            }
                            let before = calls.load(Ordering::Relaxed);
                            game.carried = Some(stack);
                            game.draw_slot_tooltip(&mut batch);
                            assert!(drawn.lock().unwrap().is_empty());
                            assert_eq!(calls.load(Ordering::Relaxed), before);
                            game.carried = None;
                            if enabled {
                                mode.store(4, Ordering::Relaxed);
                                game.draw_slot_tooltip(&mut batch);
                                assert!(drawn.lock().unwrap().pop().unwrap() == original);
                                assert_eq!(calls.load(Ordering::Relaxed), before + 1);
                                mode.store(1, Ordering::Relaxed);
                                game.draw_slot_tooltip(&mut batch);
                                assert!(drawn.lock().unwrap().pop().unwrap() == original);
                                assert_eq!(calls.load(Ordering::Relaxed), before + 1);
                            }
                        },
                    );
                });
            });
        });
    }
}
