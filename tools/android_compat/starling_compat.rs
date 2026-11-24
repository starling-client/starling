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
}
