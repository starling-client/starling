//! The mouse-key portion of Minecraft.handleKeybinds. Events only count clicks;
//! the client tick consumes them before player movement and CLIENT_TICK_END.

#[derive(Clone, Copy)]
pub enum Button {
    Attack,
    Use,
    Pick,
}

impl Button {
    pub const ALL: [Self; 3] = [Self::Attack, Self::Use, Self::Pick];

    pub fn mapping(self) -> &'static str {
        match self {
            Self::Attack => "key.attack",
            Self::Use => "key.use",
            Self::Pick => "key.pickItem",
        }
    }
}

pub trait InteractionWorld {
    fn pending_clicks(&self, button: Button) -> u32;
    fn consume_click(&mut self, button: Button) -> bool;
    fn is_down(&self, button: Button) -> bool;
    fn can_continue_attack(&self) -> bool {
        true
    }
    fn spectator_hotbar(&mut self) {}
    /// Minecraft.pick before handleKeybinds: the tick's `hitResult`, which
    /// every click of the tick reads, even after one of them changed the
    /// block it names (a creative break, then a use that replaces it).
    fn pick_hit_result(&mut self);
    /// handleKeybinds' keys between the number keys and the clicks (the inventory key, swap
    /// hands, drop), which act before any attack, use or pick of the tick.
    fn handle_keys(&mut self) {}
    /// The tick's clicks are done: later picks follow the crosshair again.
    fn release_hit_result(&mut self);
    fn using_item(&self) -> bool;
    fn release_using(&mut self);
    fn start_attack(&mut self) -> bool;
    fn start_use(&mut self, held: bool);
    fn pick(&mut self);
    fn use_delay(&self) -> u32;
    fn continue_attack(&mut self, down: bool);
}

/// Minecraft.startUseItem dispatches one hit result per hand. A passing
/// entity interaction falls back to the item, without trying a block behind it.
pub trait TargetUseWorld {
    fn use_entity(&mut self) -> bool;
    fn use_block(&mut self) -> bool;
    fn use_item(&mut self);
}

pub fn use_on_target(world: &mut impl TargetUseWorld, entity: bool) {
    let consumed = if entity {
        world.use_entity()
    } else {
        world.use_block()
    };
    if !consumed {
        world.use_item();
    }
}

/// The click phase consumes live mappings, so a screen opened in `handle_keys`
/// can release them even partway through a tick. Clicks enqueued by interaction
/// callbacks are left for the next tick, not processed recursively.
pub fn tick(world: &mut impl InteractionWorld) {
    let pending = Button::ALL.map(|button| world.pending_clicks(button));
    world.pick_hit_result();
    world.handle_keys();
    let using = world.using_item();
    if using && !world.is_down(Button::Use) {
        world.release_using();
    }
    let mut instant = false;
    for (button, count) in Button::ALL.into_iter().zip(pending) {
        for _ in 0..count {
            if !world.consume_click(button) {
                break;
            }
            if !using {
                match button {
                    Button::Attack => instant |= world.start_attack(),
                    Button::Use => world.start_use(false),
                    Button::Pick => world.pick(),
                }
            }
        }
    }
    if !using {
        world.spectator_hotbar();
    }
    if world.is_down(Button::Use) && world.use_delay() == 0 && !world.using_item() {
        world.start_use(true);
    }
    world.continue_attack(!instant && world.is_down(Button::Attack) && world.can_continue_attack());
    world.release_hit_result();
}

/// The sentence of `Minecraft.handleKeybinds` that follows a key which opened
/// a screen: `Gui.setScreen` has already run `KeyMapping.releaseAll()`, so
/// `key.use` reads up and the same call releases an active use
/// (`player.isUsingItem() && !keyUse.isDown()` → `MultiPlayerGameMode.releaseUsingItem`).
/// Only the screens opened from the keybinds do this (the inventory, chat and
/// advancements); the pause menu, server containers, mount inventories and
/// dialogs release on the first tick after closing instead, as vanilla does.
pub fn release_on_screen_open(world: &mut impl InteractionWorld) {
    if world.using_item() {
        world.release_using();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymapping::KeyState;

    macro_rules! input_methods {
        () => {
            fn pending_clicks(&self, button: Button) -> u32 {
                self.input[button as usize].pending_clicks() as u32
            }
            fn consume_click(&mut self, button: Button) -> bool {
                self.input[button as usize].consume_click()
            }
            fn is_down(&self, button: Button) -> bool {
                self.input[button as usize].is_down
            }
        };
    }

    fn step(world: &mut World, attack: bool, use_item: bool) {
        world.input[Button::Attack as usize].is_down = attack;
        world.input[Button::Use as usize].is_down = use_item;
        tick(world);
    }

    #[derive(Default)]
    struct World {
        using: bool,
        destroying: bool,
        delay: u32,
        begin_using: bool,
        events: Vec<&'static str>,
        use_attempts: Vec<bool>,
        callback: bool,
        input: [KeyState; 3],
    }
    impl InteractionWorld for World {
        input_methods!();
        fn pick_hit_result(&mut self) {
            if self.callback {
                for _ in 0..1025 {
                    self.input[Button::Attack as usize].click();
                }
            }
        }
        fn release_hit_result(&mut self) {}
        fn using_item(&self) -> bool {
            self.using
        }
        fn release_using(&mut self) {
            self.events.push("release");
            self.using = false;
        }
        fn start_attack(&mut self) -> bool {
            self.events.push("attack");
            false
        }
        fn start_use(&mut self, held: bool) {
            self.use_attempts.push(held);
            if !self.destroying {
                self.events.push("use");
                self.delay = 4;
                self.using = self.begin_using;
            }
        }
        fn pick(&mut self) {
            self.events.push("pick");
        }
        fn use_delay(&self) -> u32 {
            self.delay
        }
        fn continue_attack(&mut self, _: bool) {
            self.events.push("continue");
        }
    }

    /// The click phase consumes the inputs pending when it began.
    /// callbacks that enqueue more than 1024 clicks are neither capped nor split across ticks.
    #[test]
    fn callback_burst_survives_to_next_tick_without_cap() {
        let mut world = World {
            callback: true,
            ..Default::default()
        };
        for invocation in 0..2 {
            world.events.clear();
            tick(&mut world);
            assert_eq!(
                world
                    .events
                    .iter()
                    .filter(|&&event| event == "attack")
                    .count(),
                if invocation == 0 { 0 } else { 1025 }
            );
        }
    }

    #[test]
    fn quick_clicks_wait_for_a_tick_and_follow_keybind_order() {
        let mut world = World::default();
        world.input[Button::Use as usize].click();
        world.input[Button::Pick as usize].click();
        world.input[Button::Attack as usize].click();
        assert!(world.events.is_empty());
        // Both edges can arrive between ticks; the press still acts once.
        step(&mut world, false, false);
        assert_eq!(world.events, ["attack", "use", "pick", "continue"]);
        world.events.clear();
        step(&mut world, false, false);
        assert_eq!(
            world.events,
            ["continue"],
            "no spurious release or repeated click"
        );
    }

    #[test]
    fn releasing_actual_item_use_discards_clicks_for_that_tick() {
        let mut world = World {
            using: true,
            ..Default::default()
        };
        for button in [Button::Attack, Button::Use, Button::Pick] {
            world.input[button as usize].click();
        }
        step(&mut world, true, false);
        assert_eq!(world.events, ["release", "continue"]);
    }

    #[test]
    fn holding_use_respects_the_delay_and_active_digging() {
        let mut world = World {
            destroying: true,
            ..Default::default()
        };
        world.input[Button::Use as usize].click();
        step(&mut world, true, true);
        assert_eq!(world.events, ["continue"]);
        world.destroying = false;
        world.events.clear();
        step(&mut world, false, true);
        assert_eq!(world.events, ["use", "continue"]);
        world.events.clear();
        step(&mut world, false, true);
        assert_eq!(world.events, ["continue"]);
        assert_eq!(world.use_attempts, [false, true, true]);
    }

    #[test]
    fn a_quick_use_click_releases_on_the_following_tick() {
        let mut world = World {
            begin_using: true,
            ..Default::default()
        };
        world.input[Button::Use as usize].click();
        step(&mut world, false, false);
        assert_eq!(world.events, ["use", "continue"]);
        assert!(world.using);
        world.events.clear();
        step(&mut world, false, false);
        assert_eq!(world.events, ["release", "continue"]);
    }

    /// A floating block in front of the crosshair, nothing behind it, and a
    /// block item in the hand of a creative player.
    #[derive(Default)]
    struct Column {
        block: bool,
        hit: Option<Option<()>>,
        placed: bool,
        input: [KeyState; 3],
    }
    impl Column {
        /// Minecraft.hitResult: the tick's pick, or a fresh one outside it.
        fn target(&self) -> Option<()> {
            self.hit.unwrap_or(self.block.then_some(()))
        }
    }
    impl InteractionWorld for Column {
        input_methods!();
        fn pick_hit_result(&mut self) {
            self.hit = Some(self.target());
        }
        fn release_hit_result(&mut self) {
            self.hit = None;
        }
        fn using_item(&self) -> bool {
            false
        }
        fn release_using(&mut self) {}
        fn start_attack(&mut self) -> bool {
            // MultiPlayerGameMode.startDestroyBlock in creative: the
            // predicted break leaves air.
            if self.target().is_some() {
                self.block = false;
            }
            !self.block
        }
        fn start_use(&mut self, _held: bool) {
            // BlockItem.place: the clicked block, now air, can be replaced.
            if self.target().is_some() && !self.block {
                self.block = true;
                self.placed = true;
            }
        }
        fn pick(&mut self) {}
        fn use_delay(&self) -> u32 {
            0
        }
        fn continue_attack(&mut self, _: bool) {}
    }

    #[test]
    fn attack_and_use_in_one_tick_share_the_hit_result() {
        // #3006: both buttons at once replace the block the crosshair is on.
        let mut world = Column {
            block: true,
            ..Default::default()
        };
        world.input[Button::Attack as usize].click();
        world.input[Button::Use as usize].click();
        tick(&mut world);
        assert!(world.placed, "the use acts on the block the attack broke");
        assert!(world.block);
        assert!(world.hit.is_none(), "the next pick follows the crosshair");
    }

    /// Every queued click drains in one tick in vanilla `while (key.consumeClick())` order.
    #[test]
    fn a_long_burst_drains_all_clicks_in_one_tick() {
        let mut world = World::default();
        use Button::{Attack, Pick, Use};
        for button in [Attack, Attack, Attack, Use, Use, Pick] {
            world.input[button as usize].click();
        }
        step(&mut world, false, false);
        assert_eq!(
            world.events,
            [
                "attack", "attack", "attack", "use", "use", "pick", "continue"
            ]
        );
        for _ in 0..150 {
            world.input[Button::Attack as usize].click();
        }
        for _ in 0..70 {
            world.input[Button::Use as usize].click();
        }
        world.events.clear();
        step(&mut world, false, false);
        let count = |event| world.events.iter().filter(|e| **e == event).count();
        assert_eq!([count("attack"), count("use")], [150, 70]);
        world.events.clear();
        step(&mut world, false, false);
        assert_eq!(world.events, ["continue"]);
    }
}
