//! Container screens (docs/layers.md, "Container screens"): input and drawing on any open menu
//! screen (the inventory, chests, furnaces, crafting tables, anvils, merchants, a mount's
//! inventory, a mod's menu...). A crate implements [`ContainerHooks`] and offers it from
//! [`Hooks::containers`](crate::Hooks::containers); the engine hands each call a [`Container`],
//! its view of the open screen, which it builds only when a crate offered hooks.
//!
//! The hooks run at the moments of the screen's own input and drawing events: an input right
//! after the mods' `screen_*_pre` hooks let it through (`ScreenEvent.MouseButtonPressed.Pre` and
//! its siblings) and before anything else reads it, the drawing at `ScreenEvent.Render.Foreground`.
//!
//! Every read reflects the engine's predicted slot view after each click. On a server's menu the
//! server's later answers reconcile it, so a crate reads between its clicks as vanilla's own
//! screen does.
//!
//! The API names no engine type: slots are opaque ids, items are id strings, and the only text is
//! a stack's shown name.

/// A slot of the open screen: opaque, and stable while the screen is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlotId(pub u32);

/// What a slot is for, by the open menu. A crafting grid is `Input`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Hotbar,
    Inventory,
    /// The open container's own slots (a chest's, a mount's chest...).
    Storage,
    /// What a menu works on: a crafting grid, a furnace's ingredient and fuel, an anvil's inputs.
    Input,
    /// What a menu makes (`ResultSlot`, `FurnaceResultSlot`): nothing goes in.
    Result,
    Equipment,
    Other,
}

/// A slot drawn on the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Slot {
    pub id: SlotId,
    pub role: Role,
    /// The slot's top left corner in GUI pixels, from the panel's corner.
    pub x: f32,
    pub y: f32,
}

impl Slot {
    pub fn new(id: SlotId, role: Role, x: f32, y: f32) -> Self {
        Self { id, role, x, y }
    }
}

/// An item stack, as far as an extra needs to read one.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Stack {
    /// The item's id (`minecraft:stone`).
    pub item: String,
    pub count: u32,
    /// How many of this item a slot holds at most.
    pub max: u32,
    /// Equal for the same item and components: two stacks that merge have the same kind.
    pub kind: u64,
    /// This exact stack, for the queries that take one back ([`Container::takes_one`]); valid
    /// for the view that returned it.
    pub id: u64,
}

impl Stack {
    pub fn new(item: String, count: u32, max: u32, kind: u64, id: u64) -> Self {
        Self {
            item,
            count,
            max,
            kind,
            id,
        }
    }
}

/// A mouse button, as the screen's input events name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    Left,
    Right,
    Middle,
    /// Any other button, by its number (GLFW's: 3 and up).
    Other(u16),
}

/// One ordinary slot click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    /// A left or right click: pick up, put down or swap.
    Pick { right: bool },
    /// A Shift click.
    QuickMove,
    /// The hand's stack (`one`: one item of it) into the slot, as a one-slot quick craft drag; a
    /// click would pick the slot's items up when the hand is empty by then.
    Put { one: bool },
}

/// The panel's size and the rows vanilla draws its two labels in, in GUI pixels from its corner.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Panel {
    pub width: f32,
    pub height: f32,
    /// The top of the container's title.
    pub title_y: f32,
    /// The top of the "Inventory" label.
    pub inventory_y: f32,
}

impl Panel {
    pub fn new(width: f32, height: f32, title_y: f32, inventory_y: f32) -> Self {
        Self {
            width,
            height,
            title_y,
            inventory_y,
        }
    }
}

/// The open screen, for the length of one call. The engine implements it.
pub trait Container {
    /// The menu's type id: `inventory` for the player's own, else vanilla's
    /// (`minecraft:generic_9x3`, `minecraft:anvil`, `minecraft:merchant`...) or a mod's, the same
    /// for a world's own container and a server's; `sunrise:mount_inventory` for a mount's, which
    /// has no menu type.
    fn menu(&self) -> &str;
    /// Every slot drawn, in the screen's order.
    fn slots(&self) -> Vec<Slot>;
    fn stack(&self, slot: SlotId) -> Option<Stack>;
    /// The stack on the cursor.
    fn carried(&self) -> Option<Stack>;
    /// The shown name of the stack in `slot` (its custom name, else its item's).
    fn name(&self, slot: SlotId) -> Option<String>;
    /// Whether `slot` takes one more of `of`, a stack this view returned, by the menu's rules
    /// (`mayPlace`, the slot's limit).
    fn takes_one(&mut self, slot: SlotId, of: &Stack) -> bool;
    /// Where `quickMoveStack` would put `slot`'s items, nothing changed.
    fn landing(&mut self, slot: SlotId) -> Vec<SlotId>;
    /// One ordinary click, as vanilla sends it: the menu engine's on the player's own world, a
    /// `ContainerClick` on a server. False when nothing took it.
    fn click(&mut self, slot: SlotId, click: Click) -> bool;
    /// The slots vanilla's quick craft drag covers now.
    fn spread(&self) -> usize;
    /// The cursor in GUI pixels, from the panel's corner.
    fn cursor(&self) -> (f32, f32);
    fn shift(&self) -> bool;
    fn panel(&self) -> Panel;
}

/// What a crate does on a container screen. Every method does nothing by default, so vanilla's
/// handling runs. They run on the game's thread and return quickly.
///
/// `slot` is the slot under the cursor. `true` means the hook took the input: nothing else
/// reads it, and a press or drag taken ends vanilla's quick craft drag. The first hook that
/// returns `true` wins; every hook hears [`release`](Self::release) and draws.
pub trait ContainerHooks: Sync {
    /// A mouse button went down (`ScreenEvent.MouseButtonPressed.Pre`). When a hook takes it,
    /// the button's release is the hooks' too: vanilla never sees it.
    fn press(&self, _c: &mut dyn Container, _slot: Option<SlotId>, _button: Button) -> bool {
        false
    }

    /// A mouse button went up (`ScreenEvent.MouseButtonReleased.Pre`). Every hook hears it;
    /// `true` from any keeps it from vanilla.
    fn release(&self, _c: &mut dyn Container, _slot: Option<SlotId>, _button: Button) -> bool {
        false
    }

    /// The cursor moved with `button` held (`ScreenEvent.MouseDragged.Pre`).
    fn drag(&self, _c: &mut dyn Container, _slot: Option<SlotId>, _button: Button) -> bool {
        false
    }

    /// The wheel turned (`ScreenEvent.MouseScrolled.Pre`): `steps` is positive up, and
    /// fractional on a trackpad.
    fn wheel(&self, _c: &mut dyn Container, _slot: Option<SlotId>, _steps: f32) -> bool {
        false
    }

    /// A key went down (`ScreenEvent.KeyPressed.Pre`), by its name (`key.keyboard.r`). Not asked
    /// while a text field has the focus (a search box, an anvil's name).
    fn key(&self, _c: &mut dyn Container, _slot: Option<SlotId>, _key: &str) -> bool {
        false
    }

    /// Draws over the panel (`ScreenEvent.Render.Foreground`): after its slots, under the
    /// tooltips and the carried stack. Coordinates are from the panel's corner, and
    /// `canvas.size()` is the screen's.
    fn draw(&self, _c: &dyn Container, _canvas: &mut dyn crate::hud::Canvas) {}
}

/// The engine's side. Only the engine calls these.
pub mod engine {
    use super::*;
    use std::sync::OnceLock;

    static REGISTRANTS: OnceLock<Vec<&'static dyn ContainerHooks>> = OnceLock::new();

    #[cfg(any(test, feature = "test-support"))]
    thread_local! {
        static OVERRIDE: std::cell::RefCell<Option<Vec<&'static dyn ContainerHooks>>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Runs `f` with exactly `hooks` as this thread's registrants, in place of the registered
    /// ones.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_registrants<R>(hooks: &[&'static dyn ContainerHooks], f: impl FnOnce() -> R) -> R {
        let before = OVERRIDE.with(|o| o.replace(Some(hooks.to_vec())));
        let result = f();
        OVERRIDE.with(|o| *o.borrow_mut() = before);
        result
    }

    fn registered() -> &'static [&'static dyn ContainerHooks] {
        REGISTRANTS.get_or_init(|| {
            crate::all()
                .iter()
                .filter_map(|hooks| hooks.containers())
                .collect()
        })
    }

    /// Calls `f` with the registrants.
    fn with<R>(f: impl FnOnce(&[&'static dyn ContainerHooks]) -> R) -> R {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hooks) = OVERRIDE.with(|o| o.borrow().clone()) {
            return f(&hooks);
        }
        f(registered())
    }

    /// Whether any hook offered container hooks: without one the engine builds no [`Container`].
    pub fn any() -> bool {
        with(|hooks| !hooks.is_empty())
    }

    /// [`ContainerHooks::press`]: the first hook that takes it.
    pub fn press(c: &mut dyn Container, slot: Option<SlotId>, button: Button) -> bool {
        with(|hooks| hooks.iter().any(|h| h.press(c, slot, button)))
    }

    /// [`ContainerHooks::release`]: every hook; true when any took it.
    pub fn release(c: &mut dyn Container, slot: Option<SlotId>, button: Button) -> bool {
        with(|hooks| {
            hooks
                .iter()
                .fold(false, |taken, h| h.release(c, slot, button) | taken)
        })
    }

    /// [`ContainerHooks::drag`]: the first hook that takes it.
    pub fn drag(c: &mut dyn Container, slot: Option<SlotId>, button: Button) -> bool {
        with(|hooks| hooks.iter().any(|h| h.drag(c, slot, button)))
    }

    /// [`ContainerHooks::wheel`]: the first hook that takes it.
    pub fn wheel(c: &mut dyn Container, slot: Option<SlotId>, steps: f32) -> bool {
        with(|hooks| hooks.iter().any(|h| h.wheel(c, slot, steps)))
    }

    /// [`ContainerHooks::key`]: the first hook that takes it.
    pub fn key(c: &mut dyn Container, slot: Option<SlotId>, key: &str) -> bool {
        with(|hooks| hooks.iter().any(|h| h.key(c, slot, key)))
    }

    /// [`ContainerHooks::draw`]: every hook.
    pub fn draw(c: &dyn Container, canvas: &mut dyn crate::hud::Canvas) {
        with(|hooks| hooks.iter().for_each(|h| h.draw(c, canvas)));
    }
}

#[cfg(test)]
mod tests {
    use super::engine::{any, press, release, with_registrants};
    use super::*;
    use std::sync::Mutex;

    struct Nothing;
    impl Container for Nothing {
        fn menu(&self) -> &str {
            "inventory"
        }
        fn slots(&self) -> Vec<Slot> {
            Vec::new()
        }
        fn stack(&self, _: SlotId) -> Option<Stack> {
            None
        }
        fn carried(&self) -> Option<Stack> {
            None
        }
        fn name(&self, _: SlotId) -> Option<String> {
            None
        }
        fn takes_one(&mut self, _: SlotId, _: &Stack) -> bool {
            false
        }
        fn landing(&mut self, _: SlotId) -> Vec<SlotId> {
            Vec::new()
        }
        fn click(&mut self, _: SlotId, _: Click) -> bool {
            false
        }
        fn spread(&self) -> usize {
            0
        }
        fn cursor(&self) -> (f32, f32) {
            (0.0, 0.0)
        }
        fn shift(&self) -> bool {
            false
        }
        fn panel(&self) -> Panel {
            Panel::new(176.0, 166.0, 6.0, 72.0)
        }
    }

    /// Takes presses and releases when `take`, and logs every call it hears.
    struct Hook(&'static str, bool, Mutex<Vec<String>>);
    impl ContainerHooks for Hook {
        fn press(&self, _: &mut dyn Container, _: Option<SlotId>, _: Button) -> bool {
            self.2.lock().unwrap().push(format!("{} press", self.0));
            self.1
        }
        fn release(&self, _: &mut dyn Container, _: Option<SlotId>, _: Button) -> bool {
            self.2.lock().unwrap().push(format!("{} release", self.0));
            self.1
        }
    }

    static FIRST: Hook = Hook("first", true, Mutex::new(Vec::new()));
    static SECOND: Hook = Hook("second", false, Mutex::new(Vec::new()));

    #[test]
    fn the_first_taker_wins_and_release_reaches_every_hook() {
        with_registrants(&[], || assert!(!any()));
        with_registrants(&[&SECOND, &FIRST, &SECOND], || {
            assert!(any());
            assert!(press(&mut Nothing, None, Button::Left));
            assert!(release(&mut Nothing, None, Button::Left));
        });
        assert_eq!(*FIRST.2.lock().unwrap(), ["first press", "first release"]);
        assert_eq!(
            *SECOND.2.lock().unwrap(),
            ["second press", "second release", "second release"]
        );
    }
}
