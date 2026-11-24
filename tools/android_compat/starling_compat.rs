//! Starling: Horizon's mod API names, on Sunrise's, for a Horizon mod built unchanged (mod-builds' adoption puts
//! this module in each adopted mod with `use crate::starling_compat::*;` in its files). Each is the old call's
//! shape over the new call, nothing more. Generated code of the patch crates names these too.
#![allow(dead_code, unused_imports)]
use mod_api::{ClientHost, ClientModifiers, Hand, Host, ItemStack, LivingRef, PlayerRef};

/// Horizon's `ClientHost::mod_key` and `consume_click`, over a key mapping's handle (`mod_key_mapping`).
pub trait StarlingModKeys {
    fn mod_key(&self, owner: &str, id: &str) -> Option<(String, ClientModifiers)>;
    fn consume_click(&mut self, owner: &str, id: &str) -> bool;
    fn is_mod_key_down(&self, owner: &str, id: &str) -> bool;
}

impl StarlingModKeys for dyn ClientHost + '_ {
    fn mod_key(&self, owner: &str, id: &str) -> Option<(String, ClientModifiers)> {
        let mapping = self.mod_key_mapping(owner, id).ok()?;
        mapping
            .binding(self)
            .ok()
            .map(|(key, modifiers)| (key.to_owned(), modifiers))
    }

    fn consume_click(&mut self, owner: &str, id: &str) -> bool {
        self.mod_key_mapping(owner, id)
            .ok()
            .and_then(|mapping| mapping.consume_click(&mut *self).ok())
            .unwrap_or(false)
    }

    fn is_mod_key_down(&self, owner: &str, id: &str) -> bool {
        self.mod_key_mapping(owner, id)
            .ok()
            .and_then(|mapping| mapping.is_down(self).ok())
            .unwrap_or(false)
    }
}

/// Horizon's `PlayerRef::inventory_slots`: every slot with an item, numbered as `with_inventory` numbers them.
pub trait StarlingInventory {
    fn inventory_slots(self, host: &dyn Host) -> Option<Vec<(usize, ItemStack)>>;
    fn inventory(self, host: &dyn Host) -> Option<Vec<(usize, ItemStack)>>;
}

impl StarlingInventory for PlayerRef {
    fn inventory_slots(self, host: &dyn Host) -> Option<Vec<(usize, ItemStack)>> {
        self.with_inventory(host, |view| {
            view.iter()
                .filter_map(|(slot, stack)| stack.project().map(|stack| (slot as usize, stack)))
                .collect()
        })
    }

    fn inventory(self, host: &dyn Host) -> Option<Vec<(usize, ItemStack)>> {
        self.inventory_slots(host)
    }
}

/// Horizon's `LivingRef::hand_item`: the stack in `hand`, if any.
pub trait StarlingHands {
    fn hand_item(self, host: &dyn Host, hand: Hand) -> Option<ItemStack>;
}

impl StarlingHands for LivingRef {
    fn hand_item(self, host: &dyn Host, hand: Hand) -> Option<ItemStack> {
        self.with_hand_item(host, hand, |stack| stack.project()).flatten()
    }
}

/// Horizon's ids were `&str` where Sunrise's are `Option<&str>`: the old text methods, an unknown id as "".
pub trait StarlingId {
    fn to_string(&self) -> String;
    fn is_empty(&self) -> bool;
    fn as_str(&self) -> &str;
    fn starts_with(&self, prefix: &str) -> bool;
    fn ends_with(&self, suffix: &str) -> bool;
}

impl StarlingId for Option<&str> {
    fn to_string(&self) -> String {
        self.unwrap_or_default().to_owned()
    }
    fn is_empty(&self) -> bool {
        self.unwrap_or_default().is_empty()
    }
    fn as_str(&self) -> &str {
        self.unwrap_or_default()
    }
    fn starts_with(&self, prefix: &str) -> bool {
        self.unwrap_or_default().starts_with(prefix)
    }
    fn ends_with(&self, suffix: &str) -> bool {
        self.unwrap_or_default().ends_with(suffix)
    }
}

/// Values Horizon handed as `Option` that Sunrise hands plain: the old `unwrap_or` still reads them.
pub trait StarlingPlain: Sized {
    fn unwrap_or(self, _default: Self) -> Self {
        self
    }
    fn unwrap_or_default(self) -> Self {
        self
    }
}
impl StarlingPlain for i32 {}
impl StarlingPlain for u32 {}
impl StarlingPlain for f32 {}
impl StarlingPlain for f64 {}
impl StarlingPlain for bool {}

/// Horizon's `movement_input` event: the old fields over Sunrise's `movement_input_update` input.
/// The adoption renames the hook (`fn movement_input` becomes a `movement_input_update` that builds this, runs the
/// old body and writes it back): the old `forward`/`strafe` are Sunrise's `move_vector` `[left, forward]`.
#[derive(Clone, Copy, Debug, Default)]
pub struct MovementInputEvent {
    pub forward: f32,
    pub strafe: f32,
    pub jump: bool,
    pub sneak: bool,
    pub forward_key: bool,
    pub backward_key: bool,
    pub left_key: bool,
    pub right_key: bool,
    pub sprint: bool,
}

impl MovementInputEvent {
    pub fn of(input: &mod_api::MovementInput) -> Self {
        let keys = input.key_presses;
        Self {
            forward: input.move_vector[1],
            strafe: input.move_vector[0],
            jump: keys.jump,
            sneak: keys.sneak,
            forward_key: keys.forward,
            backward_key: keys.backward,
            left_key: keys.left,
            right_key: keys.right,
            sprint: keys.sprint,
        }
    }

    pub fn write(&self, input: &mut mod_api::MovementInput) {
        input.move_vector = [self.strafe, self.forward];
        let keys = &mut input.key_presses;
        keys.jump = self.jump;
        keys.sneak = self.sneak;
        keys.forward = self.forward_key;
        keys.backward = self.backward_key;
        keys.left = self.left_key;
        keys.right = self.right_key;
        keys.sprint = self.sprint;
    }
}
