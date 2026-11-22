//! Item/enchantment answers resolved when parts install, read from registry snapshots.
use super::*;

impl Game<'_> {
    /// Resolve support on both sides before the server resolves primary eligibility.
    /// Both hooks see default stacks and the installed tag view.
    pub(in crate::game) fn resolve_primary_items(&self) {
        let mut support = sunrise_world::enchantment_answers::Answers::new();
        let enchantments = self.item_enchantments();
        let items = self.items();
        if mod_api::__private::has_part_hook(mod_api::HookId::SupportsEnchantment) {
            for item in items.items.keys() {
                if !mod_api::__private::item_listens(mod_api::HookId::SupportsEnchantment, item) {
                    continue;
                }
                let stack = ItemStack::new(item, 1);
                for enchantment in &enchantments.list {
                    let mut answer = None;
                    self.native_supports_enchantment(&stack, &enchantment.id, &mut answer);
                    // An explicit answer can equal one consumer's fallback but replace another's.
                    if let Some(supported) = answer {
                        support
                            .entry(item.clone())
                            .or_default()
                            .insert(enchantment.id.clone(), supported);
                    }
                }
            }
        }
        for table in self.enchantment_snapshots() {
            table.support_answers().install(support.clone());
        }
        if !self.is_logical_server() {
            // A client's tag install never asks, writes or clears the server's primary answers.
            return;
        }
        if let Some(loot) = self.server_loot.get() {
            loot.support_answers().install(support);
        }
        let mut answers = sunrise_world::enchantment_answers::Answers::new();
        if mod_api::__private::has_part_hook(mod_api::HookId::IsPrimaryItemFor) {
            for item in items.items.keys() {
                if !mod_api::__private::item_listens(mod_api::HookId::IsPrimaryItemFor, item) {
                    continue;
                }
                let stack = ItemStack::new(item, 1);
                for enchantment in &enchantments.list {
                    // Support is already installed: primary listeners see the support-layered default.
                    let vanilla = enchantments.is_primary(items, enchantment, item);
                    let mut primary = vanilla;
                    self.native_is_primary_item_for(&stack, &enchantment.id, &mut primary);
                    if primary != vanilla {
                        answers
                            .entry(item.clone())
                            .or_default()
                            .insert(enchantment.id.clone(), primary);
                    }
                }
            }
        }
        for table in self.enchantment_snapshots() {
            table.primary_answers().install(answers.clone());
        }
        if let Some(loot) = self.server_loot.get() {
            loot.primary_answers().install(answers);
        }
    }
    /// Retire the world's answers with its parts. A client holds no primary answers.
    pub(in crate::game) fn clear_world_parts(&self) {
        for table in self.enchantment_snapshots() {
            table.support_answers().clear();
            if self.is_logical_server() {
                table.primary_answers().clear();
            }
        }
        if self.is_logical_server()
            && let Some(loot) = self.server_loot.get()
        {
            loot.primary_answers().clear();
            loot.support_answers().clear();
        }
        mod_api::__private::clear_game_parts();
    }
    /// Preserve installed answers when publishing a fresh world-loot snapshot,
    /// including loads after parts install. This never asks a listener twice.
    pub(in crate::game) fn copy_loot_enchantment_answers(
        &self,
        loot: &sunrise_world::loot::LootData,
    ) {
        let enchantments = &self.stations().enchantments;
        loot.primary_answers()
            .install(enchantments.primary_answers().snapshot());
        loot.support_answers()
            .install(enchantments.support_answers().snapshot());
    }
    /// The registry snapshots the engines read, once when stations and items share one.
    fn enchantment_snapshots(&self) -> impl Iterator<Item = &sunrise_items::enchant::Enchantments> {
        let stations = &self.stations().enchantments;
        let items = &self.items().enchantments;
        let active = self.item_enchantments();
        let other = (!std::sync::Arc::ptr_eq(stations, items)).then_some(items);
        let synced = (!std::sync::Arc::ptr_eq(active, stations)
            && !std::sync::Arc::ptr_eq(active, items))
        .then_some(active);
        std::iter::once(stations)
            .chain(other)
            .chain(synced)
            .map(|table| table.as_ref())
    }
}
