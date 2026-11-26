use super::*;
use crate::Host;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
struct FakeHost;
impl Host for FakeHost {}
crate::impl_host_areas!(FakeHost);
impl ClientHost for FakeHost {}
crate::impl_client_host_areas!(FakeHost);

struct StubDraw;
impl DrawContext for StubDraw {
    fn with_scissor(
        &mut self,
        _: [f32; 4],
        _: &mut dyn FnMut(&mut dyn DrawContext),
    ) -> Result<(), crate::GuiClipRefusal> {
        unreachable!("widgets under test do not open a scissor")
    }
}

#[test]
fn button_builder_defaults_modifiers_and_callback_unpack() {
    let pressed = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&pressed);
    let sprites = ButtonSprites {
        highlighted: Some("example:btn_hover".into()),
        disabled: Some("example:btn_disabled".into()),
        label_offset: [2.0, -1.0],
        ..ButtonSprites::new("example:btn")
    };
    let button = Button::new(
        [10.0, 20.0, 120.0, 20.0],
        Text::literal("Save"),
        move |state, _host| {
            state.label = Text::literal("Saved");
            state.active = false;
            seen.store(true, Ordering::Relaxed);
        },
    )
    .active(false)
    .visible(false)
    .sprites(sprites.clone());

    assert_eq!(button.state().rect, [10.0, 20.0, 120.0, 20.0]);
    assert_eq!(button.state().label, Text::literal("Save"));
    assert!(!button.state().active);
    assert!(!button.state().visible);
    assert_eq!(button.state().sprites, Some(sprites));

    let (mut state, on_press) = button.into_parts();
    let mut host = FakeHost;
    on_press.expect("Button::new installs a press callback")(&mut state, &mut host);
    assert!(pressed.load(Ordering::Relaxed));
    assert_eq!(state.label, Text::literal("Saved"));
    assert!(!state.active);

    let inactive = Button::inactive([0.0, 0.0, 80.0, 20.0], Text::literal("Locked"));
    assert!(!inactive.state().active);
    assert!(inactive.state().visible);
    assert_eq!(inactive.state().sprites, None);
    let (inactive_state, inactive_cb) = inactive.into_parts();
    assert!(!inactive_state.active);
    assert!(inactive_cb.is_none());
}

#[test]
fn text_field_builder_defaults_modifiers_and_callbacks_unpack() {
    let changed = Arc::new(AtomicBool::new(false));
    let submitted = Arc::new(AtomicBool::new(false));
    let changed_flag = Arc::clone(&changed);
    let submitted_flag = Arc::clone(&submitted);

    let default_parts = TextField::new([4.0, 8.0, 160.0, 20.0], 64).into_parts();
    assert_eq!(default_parts.rect, [4.0, 8.0, 160.0, 20.0]);
    assert!(default_parts.value.is_empty());
    assert_eq!(default_parts.hint, None);
    assert_eq!(default_parts.max_length, 64);
    assert!(default_parts.editable);
    assert!(default_parts.active);
    assert!(default_parts.visible);
    assert!(default_parts.bordered);
    assert!(default_parts.on_change.is_none());
    assert!(default_parts.on_submit.is_none());

    let parts = TextField::new([12.0, 24.0, 200.0, 20.0], 32)
        .value("hello")
        .hint(Text::literal("Search"))
        .editable(false)
        .active(false)
        .visible(false)
        .bordered(false)
        .on_change(move |text, _host| {
            if text == "world" {
                changed_flag.store(true, Ordering::Relaxed);
            }
        })
        .on_submit(move |text, _host| {
            if text == "world" {
                submitted_flag.store(true, Ordering::Relaxed);
            }
        })
        .into_parts();

    assert_eq!(parts.value, "hello");
    assert_eq!(parts.hint, Some(Text::literal("Search")));
    assert!(!parts.editable);
    assert!(!parts.active);
    assert!(!parts.visible);
    assert!(!parts.bordered);

    let mut host = FakeHost;
    parts.on_change.expect("on_change installed")("world", &mut host);
    parts.on_submit.expect("on_submit installed")("world", &mut host);
    assert!(changed.load(Ordering::Relaxed));
    assert!(submitted.load(Ordering::Relaxed));
}

#[test]
fn screen_widget_defaults_and_registrar_ids_work() {
    struct MinimalWidget;
    impl ScreenWidget for MinimalWidget {
        fn bounds(&self) -> [f32; 4] {
            [1.0, 2.0, 30.0, 40.0]
        }
    }

    let mut widget = MinimalWidget;
    let mut host = FakeHost;
    let mut draw = StubDraw;
    assert_eq!(widget.bounds(), [1.0, 2.0, 30.0, 40.0]);
    assert!(widget.visible());
    assert!(widget.active());
    assert!(!widget.captures_input());
    widget.render(&host, &mut draw, [0.0, 0.0], false, false, 0.5);
    assert!(!widget.mouse_clicked(&mut host, [5.0, 5.0], 0, 0, false));
    assert!(!widget.mouse_released(&mut host, [5.0, 5.0], 0, 0));
    assert!(!widget.mouse_dragged(&mut host, [5.0, 5.0], 0, [1.0, 0.0], 0));
    assert!(!widget.mouse_scrolled(&mut host, [5.0, 5.0], [0.0, 1.0], 0));
    assert!(!widget.key_pressed(&mut host, "key.keyboard.a", false, 0));
    assert!(!widget.key_released(&mut host, "key.keyboard.a", 0));
    assert!(!widget.char_typed(&mut host, 'a', 0));
    widget.set_focused(true);

    #[derive(Default)]
    struct FakeRegistrar {
        next: u64,
        children: Vec<ScreenChild>,
        initial_focus: Option<ScreenWidgetId>,
    }
    impl ScreenWidgets for FakeRegistrar {
        fn children(&self) -> &[ScreenChild] {
            &self.children
        }

        fn add_button(&mut self, _button: Button) -> ScreenWidgetId {
            self.next += 1;
            let id = ScreenWidgetId::from_raw(self.next);
            self.children.push(ScreenChild::new(id, [0.0; 4], None));
            id
        }
        fn add_text_field(&mut self, _field: TextField) -> ScreenWidgetId {
            self.next += 1;
            let id = ScreenWidgetId::from_raw(self.next);
            self.children.push(ScreenChild::new(id, [0.0; 4], None));
            id
        }
        fn add_widget(&mut self, _widget: Box<dyn ScreenWidget>) -> ScreenWidgetId {
            self.next += 1;
            let id = ScreenWidgetId::from_raw(self.next);
            self.children.push(ScreenChild::new(id, [0.0; 4], None));
            id
        }
        fn set_initial_focus(&mut self, id: ScreenWidgetId) -> bool {
            if self.children.iter().any(|child| child.id == id) {
                self.initial_focus = Some(id);
                true
            } else {
                false
            }
        }
        fn remove(&mut self, id: ScreenWidgetId) -> bool {
            if let Some(pos) = self.children.iter().position(|child| child.id == id) {
                self.children.remove(pos);
                true
            } else {
                false
            }
        }
    }

    let mut reg = FakeRegistrar::default();
    let b = reg.add_button(Button::inactive([0.0, 0.0, 20.0, 20.0], Text::literal("B")));
    let f = reg.add_text_field(TextField::new([0.0, 24.0, 100.0, 20.0], 16));
    let w = reg.add_widget(Box::new(MinimalWidget));
    assert_eq!(b.raw(), 1);
    assert_eq!(f.raw(), 2);
    assert_eq!(w.raw(), 3);
    assert!(reg.set_initial_focus(f));
    assert_eq!(reg.initial_focus, Some(f));
    assert!(reg.remove(w));
    assert!(!reg.remove(w));
    assert!(!reg.set_initial_focus(w));
}
