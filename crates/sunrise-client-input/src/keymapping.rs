//! The down/click state of `KeyMapping` and `ToggleKeyMapping` (sneak,
//! sprint, attack and use can be toggled instead of held).
use crate::keybinds::{KeyBinds, MAPPINGS, VanillaMappingSlot};
use std::{
    collections::HashMap,
    num::NonZeroU64,
    sync::{
        LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};

/// One mapping's runtime state. `toggle` is `Some(restore)` for a
/// `ToggleKeyMapping` (with its `shouldRestore` flag), `None` for a plain one.
#[derive(Clone, Debug)]
pub struct KeyState {
    pub is_down: bool,
    click_count: i32,
    released_by_screen_when_down: bool,
    toggle: Option<bool>,
    keyboard: bool,
    /// Held down by a controller's toggle (Sunrise's pad toggle-sneak, which
    /// vanilla has no option for): until it lets go, the mapping toggles as
    /// if its vanilla option were on, so a screen releases and restores it.
    pad_toggled: bool,
}

impl Default for KeyState {
    fn default() -> Self {
        Self::plain()
    }
}

impl KeyState {
    pub fn plain() -> Self {
        Self {
            is_down: false,
            click_count: 0,
            released_by_screen_when_down: false,
            toggle: None,
            keyboard: true,
            pad_toggled: false,
        }
    }
    /// A `ToggleKeyMapping`; `keyboard` is whether the bound key is a keyboard key.
    pub fn toggle(should_restore: bool, keyboard: bool) -> Self {
        Self {
            toggle: Some(should_restore),
            keyboard,
            ..Self::plain()
        }
    }
    /// `KeyMapping.click`: one more pending click.
    pub fn click(&mut self) {
        self.click_count = self.click_count.saturating_add(1);
    }
    /// `consumeClick`.
    pub fn consume_click(&mut self) -> bool {
        if self.click_count == 0 {
            false
        } else {
            self.click_count -= 1;
            true
        }
    }
    /// The mapping toggles: its option is on, or a pad toggled it down.
    pub fn needs_toggle(&self, option: bool) -> bool {
        option || self.pad_toggled
    }
    /// `setDown`; `needs_toggle` is the mapping's option value.
    pub fn set_down(&mut self, down: bool, needs_toggle: bool) {
        if self.toggle.is_some() && self.needs_toggle(needs_toggle) {
            if down {
                self.is_down = !self.is_down;
                // Toggled back up: the pad's toggle is over.
                self.pad_toggled &= self.is_down;
            }
        } else {
            self.is_down = down;
        }
    }
    /// `KeyboardHandler.keyPress` for this mapping's key: a release sets it up
    /// (`KeyMapping.set(key, false)`); a press or a repeat with no screen open
    /// sets it down and clicks it (`KeyMapping.set(key, true)`, `KeyMapping.click`).
    pub fn key(&mut self, action: KeyAction, screen_open: bool, needs_toggle: bool) {
        match action {
            KeyAction::Release => self.set_down(false, needs_toggle),
            KeyAction::Press | KeyAction::Repeat if !screen_open => {
                self.set_down(true, needs_toggle);
                self.click();
            }
            _ => {}
        }
    }
    /// `MouseHandler.onButton` for this mapping's button: only without a
    /// screen or overlay, and a press clicks it.
    pub fn button(&mut self, pressed: bool, screen_open: bool, needs_toggle: bool) {
        if !screen_open {
            self.set_down(pressed, needs_toggle);
            if pressed {
                self.click();
            }
        }
    }
    /// A controller's toggle button: flips the mapping like `setDown(true)`
    /// with its option on, whatever the option says.
    pub fn pad_toggle(&mut self) {
        self.is_down = !self.is_down;
        self.pad_toggled = self.is_down;
    }
    /// `shouldSetOnIngameFocus` for a keyboard-bound, bound key.
    pub fn should_set_on_ingame_focus(&self, needs_toggle: bool) -> bool {
        self.keyboard && (self.toggle.is_none() || !needs_toggle)
    }
    /// `release` (a screen opened): drops pending clicks and the held state.
    pub fn release(&mut self, needs_toggle: bool) {
        if self.toggle.is_some() {
            // ToggleKeyMapping.release does not clear pending clicks.
            if self.needs_toggle(needs_toggle) && self.is_down || self.released_by_screen_when_down
            {
                self.released_by_screen_when_down = true;
            }
            self.reset();
        } else {
            self.click_count = 0;
            self.set_down(false, needs_toggle);
        }
    }
    /// `ToggleKeyMapping.reset`. A pad toggle stays on record, so the screen
    /// that released it restores it.
    pub fn reset(&mut self) {
        self.is_down = false;
    }
    /// Clicks not yet consumed.
    pub fn pending_clicks(&self) -> i32 {
        self.click_count
    }
    /// `shouldRestoreStateOnScreenClosed` (clears the remembered release).
    pub fn should_restore_state_on_screen_closed(&mut self, needs_toggle: bool) -> bool {
        let Some(restore) = self.toggle else {
            return false;
        };
        let result = restore
            && self.needs_toggle(needs_toggle)
            && self.keyboard
            && self.released_by_screen_when_down;
        self.released_by_screen_when_down = false;
        result
    }
}

/// A keyboard event (`KeyEvent.Action`): SDL reports a held key's repeats
/// as their own action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Release,
    Press,
    Repeat,
}

/// Sneak.
pub const SNEAK: usize = 0;
/// Sprint.
pub const SPRINT: usize = 1;
/// Attack (mouse).
pub const ATTACK: usize = 2;
/// Use (mouse).
pub const USE: usize = 3;

/// Toggle slot index in [`ToggleKeys`] for the four vanilla `ToggleKeyMapping`s.
pub fn toggle_index(slot: VanillaMappingSlot) -> Option<usize> {
    match slot.definition().0 {
        "key.sneak" => Some(SNEAK),
        "key.sprint" => Some(SPRINT),
        "key.attack" => Some(ATTACK),
        "key.use" => Some(USE),
        _ => None,
    }
}

fn toggle_vanilla_slot(toggle_slot: usize) -> Option<VanillaMappingSlot> {
    static SLOTS: LazyLock<[Option<VanillaMappingSlot>; 4]> = LazyLock::new(|| {
        [
            VanillaMappingSlot::by_name("key.sneak"),
            VanillaMappingSlot::by_name("key.sprint"),
            VanillaMappingSlot::by_name("key.attack"),
            VanillaMappingSlot::by_name("key.use"),
        ]
    });
    SLOTS.get(toggle_slot).copied().flatten()
}

/// Uniform runtime `KeyState` (`isDown`, `clickCount`) for every vanilla mapping in [`MAPPINGS`].
#[derive(Clone, Debug)]
pub struct VanillaMappings {
    states: [KeyState; MAPPINGS.len()],
}

impl Default for VanillaMappings {
    /// Built once and copied: a server takes a fresh set for every player turn.
    fn default() -> Self {
        static DEFAULT: LazyLock<VanillaMappings> = LazyLock::new(|| VanillaMappings {
            states: std::array::from_fn(|index| match MAPPINGS[index].0 {
                "key.sneak" | "key.sprint" => KeyState::toggle(true, true),
                "key.attack" => KeyState::toggle(true, false),
                "key.use" => KeyState::toggle(false, false),
                _ => KeyState::plain(),
            }),
        });
        DEFAULT.clone()
    }
}

impl VanillaMappings {
    pub fn state(&self, slot: VanillaMappingSlot) -> &KeyState {
        &self.states[slot.index()]
    }

    pub fn state_mut(&mut self, slot: VanillaMappingSlot) -> &mut KeyState {
        &mut self.states[slot.index()]
    }

    pub fn is_down(&self, slot: VanillaMappingSlot) -> bool {
        self.states[slot.index()].is_down
    }

    pub fn set_down(&mut self, slot: VanillaMappingSlot, down: bool, toggle_enabled: bool) {
        self.states[slot.index()].set_down(down, toggle_enabled);
    }

    pub fn click(&mut self, slot: VanillaMappingSlot) {
        self.states[slot.index()].click();
    }

    pub fn consume_click(&mut self, slot: VanillaMappingSlot) -> bool {
        self.states[slot.index()].consume_click()
    }

    pub fn pending_clicks(&self, slot: VanillaMappingSlot) -> i32 {
        self.states[slot.index()].pending_clicks()
    }

    /// Update the mappings not already delivered through the shared device edges.
    pub fn key_event(
        &mut self,
        binds: &KeyBinds,
        input: &str,
        action: KeyAction,
        screen_open: bool,
    ) {
        for slot in binds
            .slots_for(input)
            .filter(|slot| slot.definition().2 != "movement" && toggle_index(*slot).is_none())
        {
            self.state_mut(slot).key(action, screen_open, false);
        }
    }

    pub fn mouse_event(&mut self, binds: &KeyBinds, input: &str, pressed: bool, screen_open: bool) {
        for slot in binds
            .slots_for(input)
            .filter(|slot| slot.definition().2 != "movement" && toggle_index(*slot).is_none())
        {
            self.state_mut(slot).button(pressed, screen_open, false);
        }
    }

    /// `KeyMapping.releaseAll` when a screen or overlay opens.
    pub fn release_all(&mut self, needs_toggle: [bool; 4]) {
        for (index, state) in self.states.iter_mut().enumerate() {
            let option = VanillaMappingSlot::from_index(index)
                .and_then(toggle_index)
                .is_some_and(|i| needs_toggle[i]);
            state.release(option);
        }
    }

    /// `KeyMapping.restoreToggleStatesOnScreenClosed`.
    pub fn restore_toggles_on_screen_closed(&mut self, needs_toggle: [bool; 4]) {
        for (index, state) in self.states.iter_mut().enumerate() {
            if let Some(i) = VanillaMappingSlot::from_index(index).and_then(toggle_index)
                && state.should_restore_state_on_screen_closed(needs_toggle[i])
            {
                state.set_down(true, needs_toggle[i]);
            }
        }
    }

    /// `KeyMapping.resetToggleKeys` (`LocalPlayer` respawn / new player).
    pub fn reset_toggle_keys(&mut self) {
        for (index, state) in self.states.iter_mut().enumerate() {
            if VanillaMappingSlot::from_index(index)
                .and_then(toggle_index)
                .is_some()
            {
                state.reset();
            }
        }
    }

    /// Clear held state across all vanilla mappings without dropping pending clicks.
    pub fn set_all_up(&mut self) {
        for state in self.states.iter_mut() {
            state.is_down = false;
        }
    }

    /// Unfocused input no longer holds a key. A toggle the player set stays latched.
    pub fn set_keys_up_on_focus_loss(&mut self, options: [bool; 4]) {
        for (index, state) in self.states.iter_mut().enumerate() {
            let option = VanillaMappingSlot::from_index(index)
                .and_then(toggle_index)
                .is_some_and(|i| options[i]);
            state.set_down(false, option);
        }
    }

    pub fn toggle_state(&self, toggle: usize) -> &KeyState {
        self.state(toggle_vanilla_slot(toggle).expect("vanilla toggle index"))
    }

    /// The four fixed ToggleKeyMapping slots.
    pub fn toggle_state_mut(&mut self, toggle: usize) -> &mut KeyState {
        self.state_mut(toggle_vanilla_slot(toggle).expect("vanilla toggle index"))
    }

    /// Clear device-derived movement when the logical keyboard is reset.
    pub fn set_keyboard_movement_up(&mut self) {
        for name in [
            "key.forward",
            "key.back",
            "key.left",
            "key.right",
            "key.jump",
        ] {
            let slot = VanillaMappingSlot::by_name(name).expect("vanilla movement mapping");
            self.state_mut(slot).is_down = false;
        }
    }
}

/// The `ToggleKeyMapping`s `Options` builds: sneak and sprint (keyboard,
/// restored after a screen), attack (mouse, restored) and use (mouse, not
/// restored), and the handlers that drive them. `needs_toggle` is each
/// mapping's toggle option, read at every event as vanilla does.
#[derive(Clone, Debug)]
pub struct ToggleKeys(pub [KeyState; 4]);
impl Default for ToggleKeys {
    fn default() -> Self {
        Self([
            KeyState::toggle(true, true),
            KeyState::toggle(true, true),
            KeyState::toggle(true, false),
            KeyState::toggle(false, false),
        ])
    }
}
impl ToggleKeys {
    pub fn is_down(&self, slot: usize) -> bool {
        self.0[slot].is_down
    }
    /// `KeyboardHandler.keyPress` for a key bound to `slot`: a release sets
    /// the mapping up (`KeyMapping.set(key, false)`); a press or a repeat
    /// with no screen open sets it down and clicks it.
    pub fn key(&mut self, slot: usize, action: KeyAction, screen_open: bool, needs_toggle: bool) {
        self.0[slot].key(action, screen_open, needs_toggle);
    }
    /// A controller's toggle button bound to `slot` (the pad's toggle-sneak):
    /// a press with no screen open flips the mapping and clicks it, as a
    /// key press does with the mapping's toggle option on.
    pub fn pad_toggle(&mut self, slot: usize, screen_open: bool) {
        if !screen_open {
            self.0[slot].pad_toggle();
            self.0[slot].click();
        }
    }
    /// `MouseHandler.onButton` for a button bound to `slot`: only without a
    /// screen or overlay.
    pub fn button(&mut self, slot: usize, pressed: bool, screen_open: bool, needs_toggle: bool) {
        self.0[slot].button(pressed, screen_open, needs_toggle);
    }
    /// `Gui.setScreen` with a screen: `KeyMapping.releaseAll`.
    pub fn screen_opened(&mut self, needs_toggle: [bool; 4]) {
        for (key, option) in self.0.iter_mut().zip(needs_toggle) {
            key.release(option);
        }
    }
    /// `Gui.setScreen(null)` in a level:
    /// `KeyMapping.restoreToggleStatesOnScreenClosed`.
    pub fn screen_closed(&mut self, needs_toggle: [bool; 4]) {
        for (key, option) in self.0.iter_mut().zip(needs_toggle) {
            if key.should_restore_state_on_screen_closed(option) {
                key.set_down(true, option);
            }
        }
    }
    /// `LocalPlayer.respawn`: `KeyMapping.resetToggleKeys`.
    pub fn respawn(&mut self) {
        for key in &mut self.0 {
            key.reset();
            // As with the vanilla option, the death screen closing after
            // this still restores a toggle it released.
            key.pad_toggled &= key.released_by_screen_when_down;
        }
    }
}

/// The screens a mod's mapping acts in (`KeyConflictContext`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingContext {
    Universal,
    InGame,
    Gui,
}

/// A mod's mapping as the player bound it, for [`ModKeys::bind`].
pub struct ModBinding<'a> {
    pub owner: &'a str,
    pub id: &'a str,
    /// The key's or mouse button's name (`key.keyboard.z`, `key.mouse.left`).
    pub input: String,
    /// The modifiers it holds, in GLFW's encoding (shift 1, control 2, alt 4).
    pub modifiers: u32,
    pub context: MappingContext,
}

/// A resolved slot in one ordered roster of registered mappings. Engine-only; the game
/// carries its parts across the mod API boundary and validates them against `ModKeys` on use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModKeySlot {
    generation: NonZeroU64,
    index: usize,
}
impl ModKeySlot {
    /// Reconstructs a slot for validation by the current `ModKeys`.
    pub const fn from_parts(generation: NonZeroU64, index: usize) -> Self {
        Self { generation, index }
    }

    /// The generation and index, for the game's conversion to the opaque API handle.
    pub const fn parts(self) -> (NonZeroU64, usize) {
        (self.generation, self.index)
    }
}

/// One registered mapping and its current binding and logical state.
#[derive(Debug)]
pub struct ModKey {
    owner: Box<str>,
    id: Box<str>,
    input: Box<str>,
    state: KeyState,
    modifiers: u32,
    context: MappingContext,
}
impl ModKey {
    /// The registered owner and local id, unchanged by a binding edit.
    pub fn identity(&self) -> (&str, &str) {
        (&self.owner, &self.id)
    }

    /// The current input name and GLFW modifier word, without copying the name.
    pub fn binding(&self) -> (&str, u32) {
        (&self.input, self.modifiers)
    }

    /// `isDown`: logically held while the conflict context and modifiers are active.
    pub fn is_down(&self, held: u32, screen_open: bool) -> bool {
        self.state.is_down && self.active(held, screen_open)
    }

    /// `isConflictContextAndModifierActive`: its context's screens are showing
    /// (in game with none open, a GUI with one) and every modifier its binding
    /// holds is down (`KeyModifier.isActive`).
    fn active(&self, held: u32, screen_open: bool) -> bool {
        let shown = match self.context {
            MappingContext::Universal => true,
            MappingContext::InGame => !screen_open,
            MappingContext::Gui => screen_open,
        };
        shown && self.modifiers & held == self.modifiers
    }
}

/// The plain `KeyMapping`s mods register (`Registrar::register_key_mapping`):
/// bound once at registration and again when a binding changes, with an index
/// from each key to its mappings, so an input touches only the mappings bound
/// to it and allocates nothing.
#[derive(Debug)]
pub struct ModKeys {
    client_generation: Option<NonZeroU64>,
    // Separate from client identity: only a change to the ordered registered roster rotates this.
    registry_generation: Option<NonZeroU64>,
    keys: Vec<ModKey>,
    /// Each bound key's mappings, by its name.
    by_input: HashMap<Box<str>, Vec<usize>>,
    by_mouse: HashMap<winit::event::MouseButton, Vec<usize>>,
    /// Each mod's mappings, by id.
    by_id: HashMap<Box<str>, HashMap<Box<str>, usize>>,
    /// Every mapping is up with no clicks, so `release_all` has nothing to do.
    idle: bool,
}

/// Issue an identity from one process-wide sequence for mapping generations.
fn issue_mapping_generation() -> Option<NonZeroU64> {
    // The old value is the issued identity; u64::MAX is the exhausted sentinel.
    static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
    NEXT_GENERATION
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(1)
        })
        .ok()
        .and_then(NonZeroU64::new)
}

impl Default for ModKeys {
    fn default() -> Self {
        Self {
            client_generation: issue_mapping_generation(),
            registry_generation: None,
            keys: Vec::new(),
            by_input: HashMap::new(),
            by_mouse: HashMap::new(),
            by_id: HashMap::new(),
            idle: false,
        }
    }
}
impl ModKeys {
    /// The client input state's identity, unchanged by rebinding; `None` if the issuer is exhausted.
    pub fn client_generation(&self) -> Option<NonZeroU64> {
        self.client_generation
    }

    /// Binds the mappings in registration order. A surviving owner/id keeps its held state
    /// and clicks, even if it moves. Resolved slots survive only while the ordered roster stays
    /// the same. An unbound mapping hears no input.
    pub fn bind<'a>(&mut self, bindings: impl IntoIterator<Item = ModBinding<'a>>) {
        self.by_input.clear();
        self.by_mouse.clear();
        let previous_ids = std::mem::take(&mut self.by_id);
        let previous_keys = std::mem::take(&mut self.keys);
        let bindings = bindings.into_iter();
        self.keys.reserve(bindings.size_hint().0);
        let mut same_roster = true;
        for (index, binding) in bindings.enumerate() {
            let at_same_slot = previous_keys
                .get(index)
                .filter(|old| old.identity() == (binding.owner, binding.id));
            same_roster &= at_same_slot.is_some();
            let previous = at_same_slot.or_else(|| {
                let &index = previous_ids.get(binding.owner)?.get(binding.id)?;
                Some(&previous_keys[index])
            });
            let input = binding.input.into_boxed_str();
            if let Some(button) = crate::keybinds::mouse_button(&input) {
                self.by_mouse.entry(button).or_default().push(index);
            }
            if !input.is_empty() && input.as_ref() != "key.keyboard.unknown" {
                self.by_input.entry(input.clone()).or_default().push(index);
            }
            self.by_id
                .entry(binding.owner.into())
                .or_default()
                .insert(binding.id.into(), index);
            self.keys.push(ModKey {
                owner: binding.owner.into(),
                id: binding.id.into(),
                input,
                state: previous.map_or_else(KeyState::plain, |old| old.state.clone()),
                modifiers: binding.modifiers,
                context: binding.context,
            });
        }
        if !same_roster || self.keys.len() != previous_keys.len() {
            self.registry_generation = issue_mapping_generation();
        }
    }
    /// `KeyboardHandler.keyPress` (`mouse` false) or `MouseHandler.onButton`
    /// (`mouse` true) for the mappings bound to `input`: a press sets and
    /// clicks each one whose conflict context and modifiers are active
    /// (NeoForge's `isActiveAndMatches`), a release lets go of every one.
    /// `held` is the modifier word now.
    pub fn input(
        &mut self,
        input: &str,
        action: KeyAction,
        mouse: bool,
        screen_open: bool,
        held: u32,
    ) {
        let Some(indices) = self.by_input.get(input) else {
            return;
        };
        Self::dispatch(
            &mut self.keys,
            &mut self.idle,
            indices,
            action,
            mouse,
            screen_open,
            held,
        );
    }
    /// Dispatch a mouse event by the identity resolved at binding time.
    pub fn button(
        &mut self,
        button: winit::event::MouseButton,
        action: KeyAction,
        screen_open: bool,
        held: u32,
    ) {
        if let Some(indices) = self.by_mouse.get(&button) {
            Self::dispatch(
                &mut self.keys,
                &mut self.idle,
                indices,
                action,
                true,
                screen_open,
                held,
            );
        }
    }
    fn dispatch(
        keys: &mut [ModKey],
        idle: &mut bool,
        indices: &[usize],
        action: KeyAction,
        mouse: bool,
        screen_open: bool,
        held: u32,
    ) {
        for &index in indices {
            let key = &mut keys[index];
            if action != KeyAction::Release && !key.active(held, screen_open) {
                continue;
            }
            if mouse {
                key.state
                    .button(action != KeyAction::Release, screen_open, false);
            } else {
                key.state.key(action, screen_open, false);
            }
            *idle = false;
        }
    }
    /// Resolves a registered owner/id once for subsequent indexed reads and consumes.
    pub fn resolve(&self, owner: &str, id: &str) -> Option<ModKeySlot> {
        let generation = self.registry_generation?;
        let &index = self.by_id.get(owner)?.get(id)?;
        Some(ModKeySlot::from_parts(generation, index))
    }
    fn slot_index(&self, slot: ModKeySlot) -> Option<usize> {
        (self.registry_generation == Some(slot.generation)).then_some(slot.index)
    }
    /// Borrows the current row. Foreign generations and out-of-range slots are stale.
    pub fn get(&self, slot: ModKeySlot) -> Option<&ModKey> {
        self.keys.get(self.slot_index(slot)?)
    }
    /// `isDown` of a resolved mapping, or `None` for a stale slot.
    pub fn is_down(&self, slot: ModKeySlot, held: u32, screen_open: bool) -> Option<bool> {
        Some(self.get(slot)?.is_down(held, screen_open))
    }
    /// Takes one pending click (`KeyMapping.consumeClick`), or returns `None` without changing
    /// state for a stale slot.
    pub fn consume_click(&mut self, slot: ModKeySlot) -> Option<bool> {
        let index = self.slot_index(slot)?;
        let key = self.keys.get_mut(index)?;
        Some(key.state.consume_click())
    }
    /// `KeyMapping.releaseAll` (a screen opened): every mapping is up and its
    /// pending clicks are gone.
    pub fn release_all(&mut self) {
        if self.idle {
            return;
        }
        for key in &mut self.keys {
            key.state.release(false);
        }
        self.idle = true;
    }
    /// Every mapping is up (a script let go of all its keys); pending clicks stay.
    pub fn set_all_up(&mut self) {
        for key in &mut self.keys {
            key.state.set_down(false, false);
        }
    }
}

#[cfg(test)]
mod tests {
    fn binding(
        id: &'static str,
        input: &str,
        modifiers: u32,
        context: MappingContext,
    ) -> ModBinding<'static> {
        ModBinding {
            owner: "m",
            id,
            input: input.to_owned(),
            modifiers,
            context,
        }
    }
    /// `zoom` on Z, `boost` on Shift + Z in game, `grab` on the left button.
    fn keys() -> ModKeys {
        let mut keys = ModKeys::default();
        keys.bind([
            binding("zoom", "key.keyboard.z", 0, MappingContext::Universal),
            binding("boost", "key.keyboard.z", 1, MappingContext::InGame),
            binding("grab", "key.mouse.left", 0, MappingContext::Universal),
        ]);
        keys
    }
    /// #1646: a mapping bound to a mouse button past the fifth hears it.
    #[test]
    fn a_mapping_on_the_sixth_mouse_button_hears_it() {
        let mut keys = ModKeys::default();
        keys.bind([binding("dash", "key.mouse.6", 0, MappingContext::Universal)]);
        let dash = keys.resolve("m", "dash").unwrap();
        let sixth = winit::event::MouseButton::Other(5);
        keys.button(sixth, KeyAction::Press, false, 0);
        assert!(keys.is_down(dash, 0, false).unwrap());
        keys.button(sixth, KeyAction::Release, false, 0);
        assert!(!keys.is_down(dash, 0, false).unwrap());
        assert!(keys.consume_click(dash).unwrap());
    }
    /// A tap is one click, consumed once.
    #[test]
    fn a_tap_adds_one_click() {
        let mut keys = keys();
        let zoom = keys.resolve("m", "zoom").unwrap();
        let boost = keys.resolve("m", "boost").unwrap();
        let grab = keys.resolve("m", "grab").unwrap();
        keys.input("key.keyboard.z", KeyAction::Press, false, false, 0);
        assert!(keys.is_down(zoom, 0, false).unwrap());
        keys.input("key.keyboard.z", KeyAction::Release, false, false, 0);
        assert!(!keys.is_down(zoom, 0, false).unwrap());
        assert!(keys.consume_click(zoom).unwrap());
        assert!(!keys.consume_click(zoom).unwrap());
        assert!(!keys.consume_click(boost).unwrap(), "shift was up");
        assert!(!keys.consume_click(grab).unwrap());
        assert!(keys.resolve("other", "zoom").is_none());
    }
    /// Each auto-repeat of a held key is one more click, as its press.
    #[test]
    fn a_held_key_adds_one_click_per_repeat() {
        let mut keys = keys();
        let boost = keys.resolve("m", "boost").unwrap();
        keys.input("key.keyboard.z", KeyAction::Press, false, false, 1);
        keys.input("key.keyboard.z", KeyAction::Repeat, false, false, 1);
        keys.input("key.keyboard.z", KeyAction::Repeat, false, false, 1);
        assert!(keys.is_down(boost, 1, false).unwrap());
        assert!(!keys.is_down(boost, 0, false).unwrap(), "shift let go");
        assert!(!keys.is_down(boost, 1, true).unwrap(), "in-game only");
        assert_eq!(
            (0..4)
                .map(|_| keys.consume_click(boost).unwrap())
                .collect::<Vec<_>>(),
            [true, true, true, false]
        );
    }
    /// A screen opening drops every pending click and lets go; presses behind
    /// it count nothing, while `consume_click` itself reads the raw click count
    /// without side-effecting `release_all`.
    #[test]
    fn a_screen_opening_zeroes_the_clicks() {
        let mut keys = keys();
        let zoom = keys.resolve("m", "zoom").unwrap();
        let grab = keys.resolve("m", "grab").unwrap();
        keys.input("key.keyboard.z", KeyAction::Press, false, false, 0);
        keys.input("key.keyboard.z", KeyAction::Repeat, false, false, 0);
        keys.input("key.mouse.left", KeyAction::Press, true, false, 0);
        keys.release_all();
        assert!(!keys.is_down(zoom, 0, true).unwrap() && !keys.is_down(grab, 0, true).unwrap());
        assert!(!keys.consume_click(zoom).unwrap() && !keys.consume_click(grab).unwrap());
        keys.input("key.keyboard.z", KeyAction::Press, false, true, 0);
        keys.input("key.mouse.left", KeyAction::Press, true, true, 0);
        assert!(!keys.consume_click(zoom).unwrap() && !keys.consume_click(grab).unwrap());
        keys.input("key.keyboard.z", KeyAction::Press, false, false, 0);
        keys.input("key.mouse.left", KeyAction::Press, true, false, 0);
        assert_eq!(keys.is_down(zoom, 0, true), Some(true));
        assert_eq!(keys.consume_click(zoom), Some(true));
        assert_eq!(keys.consume_click(zoom), Some(false));
        assert_eq!(keys.is_down(zoom, 0, true), Some(true));
        assert_eq!(keys.consume_click(grab), Some(true));
        assert_eq!(keys.consume_click(grab), Some(false));
    }
    /// A rebinding moves a mapping to its new key and keeps its held state and clicks.
    #[test]
    fn a_rebinding_moves_the_mapping() {
        let mut keys = keys();
        let zoom = keys.resolve("m", "zoom").unwrap();
        keys.input("key.keyboard.z", KeyAction::Press, false, false, 0);
        keys.bind([
            binding("zoom", "key.keyboard.x", 0, MappingContext::Universal),
            binding("boost", "key.keyboard.z", 1, MappingContext::InGame),
            binding("grab", "key.mouse.left", 0, MappingContext::Universal),
        ]);
        assert_eq!(keys.get(zoom).unwrap().binding(), ("key.keyboard.x", 0));
        assert_eq!(keys.is_down(zoom, 0, false), Some(true));
        assert_eq!(keys.consume_click(zoom), Some(true));
        keys.input("key.keyboard.z", KeyAction::Press, false, false, 0);
        assert_eq!(keys.consume_click(zoom), Some(false));
        keys.input("key.keyboard.x", KeyAction::Press, false, false, 0);
        assert_eq!(keys.consume_click(zoom), Some(true));
        assert_eq!(keys.consume_click(zoom), Some(false));
    }

    #[test]
    fn stale_registered_mapping_cannot_touch_a_reused_slot() {
        let mut keys = ModKeys::default();
        keys.bind([
            binding("removed", "key.keyboard.a", 0, MappingContext::Universal),
            binding("survivor", "key.keyboard.b", 0, MappingContext::Universal),
        ]);
        let removed = keys.resolve("m", "removed").unwrap();
        keys.input("key.keyboard.a", KeyAction::Press, false, false, 0);
        keys.input("key.keyboard.a", KeyAction::Repeat, false, false, 0);
        keys.input("key.keyboard.b", KeyAction::Press, false, false, 0);

        keys.bind([
            binding("survivor", "key.keyboard.b", 0, MappingContext::Universal),
            ModBinding {
                owner: "other",
                ..binding("survivor", "key.keyboard.c", 0, MappingContext::Universal)
            },
        ]);
        let survivor = keys.resolve("m", "survivor").unwrap();
        assert_eq!(survivor.parts().1, removed.parts().1);
        assert_eq!(keys.get(survivor).unwrap().identity(), ("m", "survivor"));
        assert!(keys.resolve("m", "removed").is_none());
        assert_eq!(keys.is_down(survivor, 0, false), Some(true));
        assert_eq!(keys.consume_click(survivor), Some(true));
        assert_eq!(keys.consume_click(survivor), Some(false));
        let added = keys.resolve("other", "survivor").unwrap();
        assert_eq!(keys.is_down(added, 0, false), Some(false));
        assert_eq!(keys.consume_click(added), Some(false));

        keys.input("key.keyboard.b", KeyAction::Repeat, false, false, 0);
        assert!(keys.get(removed).is_none());
        assert_eq!(keys.is_down(removed, 0, false), None);
        assert_eq!(keys.consume_click(removed), None);
        let out_of_range = ModKeySlot::from_parts(survivor.parts().0, usize::MAX);
        assert!(keys.get(out_of_range).is_none());
        assert_eq!(keys.consume_click(out_of_range), None);
        assert_eq!(keys.consume_click(survivor), Some(true));
        assert_eq!(keys.consume_click(survivor), Some(false));

        let mut other = ModKeys::default();
        other.bind([
            binding("removed", "key.keyboard.a", 0, MappingContext::Universal),
            binding("survivor", "key.keyboard.b", 0, MappingContext::Universal),
        ]);
        let other_removed = other.resolve("m", "removed").unwrap();
        other.input("key.keyboard.a", KeyAction::Press, false, false, 0);
        assert_eq!(other.is_down(removed, 0, false), None);
        assert_eq!(other.consume_click(removed), None);
        assert_eq!(other.consume_click(other_removed), Some(true));
    }
    /// `KeyMapping.setDown` holds without a click; `KeyMapping.click` is a press's increment.
    #[test]
    fn set_changes_only_the_held_state() {
        let mut key = KeyState::plain();
        key.set_down(true, false);
        assert!(key.is_down);
        assert!(!key.consume_click());
        key.click();
        assert!(key.is_down);
        assert!(key.consume_click());
        assert!(!key.consume_click());
    }

    /// `KeyMapping.resetToggleKeys` (a new `LocalPlayer`): a sneak and a sprint toggled on in one
    /// world are off in the next.
    #[test]
    fn resetting_the_toggle_keys_puts_every_mapping_up() {
        let mut keys = ToggleKeys::default();
        keys.key(0, KeyAction::Press, false, true);
        keys.key(1, KeyAction::Press, false, true);
        assert!(keys.is_down(0) && keys.is_down(1));
        for key in &mut keys.0 {
            key.reset();
        }
        assert!(!keys.is_down(0) && !keys.is_down(1));
    }

    use super::*;
    #[test]
    fn toggle_flips_on_press_and_ignores_release() {
        let mut k = KeyState::toggle(true, true);
        k.set_down(true, true);
        assert!(k.is_down);
        k.set_down(false, true);
        assert!(k.is_down);
        k.set_down(true, true);
        assert!(!k.is_down);
    }
    #[test]
    fn screens_block_presses_but_not_releases() {
        let mut keys = ToggleKeys::default();
        keys.key(SNEAK, KeyAction::Press, true, false);
        assert!(!keys.is_down(SNEAK));
        keys.key(SNEAK, KeyAction::Press, false, false);
        assert!(keys.is_down(SNEAK));
        keys.key(SNEAK, KeyAction::Release, true, false);
        assert!(!keys.is_down(SNEAK));
        keys.key(SPRINT, KeyAction::Press, false, true);
        keys.respawn();
        assert!(!keys.is_down(SPRINT));
    }
    /// #3061: the pad's toggle-sneak (vanilla's toggle crouch off) comes
    /// back after a screen, as the vanilla option's toggle does; a
    /// keyboard shift-click in the inventory does not drop it.
    #[test]
    fn pad_toggle_is_restored_after_a_screen() {
        let off = [false; 4];
        let mut keys = ToggleKeys::default();
        keys.pad_toggle(SNEAK, false);
        assert!(keys.is_down(SNEAK));
        keys.screen_opened(off);
        assert!(!keys.is_down(SNEAK));
        keys.key(SNEAK, KeyAction::Press, true, false);
        keys.key(SNEAK, KeyAction::Release, true, false);
        keys.screen_closed(off);
        assert!(keys.is_down(SNEAK), "the pad's toggle is restored");
        // Toggled back up, it stays up through the next screen.
        keys.pad_toggle(SNEAK, false);
        assert!(!keys.is_down(SNEAK));
        keys.screen_opened(off);
        keys.screen_closed(off);
        assert!(!keys.is_down(SNEAK));
        // A held key is not restored, as before.
        keys.key(SNEAK, KeyAction::Press, false, false);
        keys.screen_opened(off);
        keys.key(SNEAK, KeyAction::Release, true, false);
        keys.screen_closed(off);
        assert!(!keys.is_down(SNEAK));
    }
    /// After the pad's toggle is over (toggled up, or reset by a respawn
    /// with no screen to restore it), the shift key holds again.
    #[test]
    fn pad_toggle_ends_with_its_release() {
        let off = [false; 4];
        let mut keys = ToggleKeys::default();
        keys.pad_toggle(SNEAK, false);
        keys.respawn();
        assert!(!keys.is_down(SNEAK));
        keys.key(SNEAK, KeyAction::Press, false, false);
        keys.key(SNEAK, KeyAction::Release, false, false);
        assert!(!keys.is_down(SNEAK), "held, not toggled");
        // The death screen releases it and closes after the respawn: the
        // toggle comes back, as with the vanilla option.
        keys.pad_toggle(SNEAK, false);
        keys.screen_opened(off);
        keys.respawn();
        keys.screen_closed(off);
        assert!(keys.is_down(SNEAK));
        // A press with a screen open does nothing.
        keys.pad_toggle(SNEAK, true);
        assert!(keys.is_down(SNEAK));
    }
    #[test]
    fn screen_release_restores_toggled_state() {
        let mut k = KeyState::toggle(true, true);
        k.set_down(true, true);
        k.release(true);
        assert!(!k.is_down);
        assert!(k.should_restore_state_on_screen_closed(true));
        assert!(!k.should_restore_state_on_screen_closed(true));
    }

    /// A mapping's pending clicks stop at the largest count instead of wrapping.
    #[test]
    fn pending_clicks_stop_at_the_largest_count() {
        let mut key = KeyState::plain();
        key.click_count = i32::MAX - 1;
        key.click();
        key.click();
        assert_eq!(key.pending_clicks(), i32::MAX);
        assert!(key.consume_click());
        assert_eq!(key.pending_clicks(), i32::MAX - 1);
    }

    /// Every `VanillaMappingSlot` (including non-gameplay and click-only ones) tracks uniform
    /// `is_down` and `click_count`, with toggle behavior on the four vanilla toggle mappings and
    /// `release_all` clearing held state and plain clicks on screen open.
    #[test]
    fn vanilla_mappings_track_all_slots_uniformly() {
        let binds = KeyBinds::default();
        let mut mappings = VanillaMappings::default();
        let chat = VanillaMappingSlot::by_name("key.chat").unwrap();
        let pick = VanillaMappingSlot::by_name("key.pickItem").unwrap();
        let sneak = VanillaMappingSlot::by_name("key.sneak").unwrap();
        let screenshot = VanillaMappingSlot::by_name("key.screenshot").unwrap();

        mappings.key_event(&binds, "key.keyboard.t", KeyAction::Press, false);
        assert!(mappings.is_down(chat));
        assert_eq!(mappings.pending_clicks(chat), 1);
        assert!(mappings.consume_click(chat));
        assert!(!mappings.consume_click(chat));

        mappings.set_down(pick, true, false);
        mappings.click(pick);
        assert!(mappings.is_down(pick));
        assert_eq!(mappings.pending_clicks(pick), 1);

        mappings.set_down(screenshot, true, false);
        mappings.click(screenshot);
        assert!(mappings.is_down(screenshot));

        // Sneak toggles when toggle_crouch is on and restores after a screen closes.
        let toggle_crouch = [true, false, false, false];
        mappings.set_down(sneak, true, true);
        mappings.set_down(sneak, false, true);
        assert!(mappings.is_down(sneak));

        mappings.release_all(toggle_crouch);
        assert!(!mappings.is_down(chat));
        assert!(!mappings.is_down(pick));
        assert_eq!(mappings.pending_clicks(pick), 0);
        assert!(!mappings.is_down(screenshot));
        assert_eq!(mappings.pending_clicks(screenshot), 0);
        assert!(!mappings.is_down(sneak));

        mappings.restore_toggles_on_screen_closed(toggle_crouch);
        assert!(mappings.is_down(sneak));
        assert!(!mappings.is_down(chat));
    }
}
