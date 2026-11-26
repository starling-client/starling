//! Load-owned type bindings for the game's default providers.
use super::*;

impl PartRegistrar {
    fn default_block_id(&mut self, block: &crate::BlockKey) -> Option<String> {
        let id = self
            .registry_keys
            .borrow()
            .blocks
            .id(mod_vocab::__private::parts_installation::block_key_slot(
                *block,
            ))
            .map(str::to_owned);
        let Some(id) = id else {
            self.refused.push(format!(
                "mod {} declared a default provider for an invalid block key",
                self.mod_id
            ));
            return None;
        };
        if self.has_builtin_entry("minecraft:block", &id) {
            self.refused.push(format!(
                "mod {} declared a mod block default provider for vanilla block {id}; use block_capability instead",
                self.mod_id
            ));
            return None;
        }
        Some(id)
    }

    fn insert_default_block<P>(
        &mut self,
        id: String,
        declaration: P,
        table: fn(&mut Parts) -> &mut HashMap<Box<str>, Vec<(usize, P)>>,
    ) {
        table(&mut self.parts)
            .entry(id.into())
            .or_default()
            .push((self.mod_index, declaration));
    }
}

impl DefaultCapabilitiesRegistrar for PartRegistrar {
    fn block_container(&mut self, block: &crate::BlockKey, container: crate::ModContainer) {
        let Some(id) = self.default_block_id(block) else {
            return;
        };
        // Vanilla's ItemStackWithSlot.CODEC saves each Slot as an unsigned byte.
        const MAX_SAVED_SLOTS: usize = u8::MAX as usize + 1;
        if container.slots > MAX_SAVED_SLOTS {
            self.refused.push(format!(
                "mod {} declared a block container for {id} with {} slots, exceeding the saved limit of {MAX_SAVED_SLOTS}",
                self.mod_id, container.slots
            ));
            return;
        }
        let slots = container.slots;
        if let Some((face, slot)) = container.faces.iter().enumerate().find_map(|(face, list)| {
            list.iter()
                .copied()
                .find(|&slot| slot >= slots)
                .map(|slot| (face, slot))
        }) {
            self.refused.push(format!(
                "mod {} declared a block container for {id} with face {face} slot {slot} outside 0..{slots}",
                self.mod_id
            ));
            return;
        }
        if let Some(&slot) = container.inputs.iter().find(|&&slot| slot >= slots) {
            self.refused.push(format!(
                "mod {} declared a block container for {id} with input slot {slot} outside 0..{slots}",
                self.mod_id
            ));
            return;
        }
        if let Some(&slot) = container.outputs.iter().find(|&&slot| slot >= slots) {
            self.refused.push(format!(
                "mod {} declared a block container for {id} with output slot {slot} outside 0..{slots}",
                self.mod_id
            ));
            return;
        }
        if container.insert_rules.len() > slots {
            self.refused.push(format!(
                "mod {} declared a block container for {id} with {} insert rules for {slots} slots",
                self.mod_id,
                container.insert_rules.len()
            ));
            return;
        }
        self.insert_default_block(id, container, |parts| &mut parts.default_block_containers);
    }
    fn block_fluids(&mut self, block: &crate::BlockKey, fluids: crate::ModFluidTanks) {
        let Some(id) = self.default_block_id(block) else {
            return;
        };
        let tanks = fluids.tanks.len();
        if let Some((face, tank)) = fluids.faces.iter().enumerate().find_map(|(face, list)| {
            list.iter()
                .copied()
                .find(|&tank| tank >= tanks)
                .map(|tank| (face, tank))
        }) {
            self.refused.push(format!(
                "mod {} declared block fluid tanks for {id} with face {face} tank {tank} outside 0..{tanks}",
                self.mod_id
            ));
            return;
        }
        if let Some(&tank) = fluids.inputs.iter().find(|&&tank| tank >= tanks) {
            self.refused.push(format!(
                "mod {} declared block fluid tanks for {id} with input tank {tank} outside 0..{tanks}",
                self.mod_id
            ));
            return;
        }
        if let Some(&tank) = fluids.outputs.iter().find(|&&tank| tank >= tanks) {
            self.refused.push(format!(
                "mod {} declared block fluid tanks for {id} with output tank {tank} outside 0..{tanks}",
                self.mod_id
            ));
            return;
        }
        self.insert_default_block(id, fluids, |parts| &mut parts.default_block_fluids);
    }
    fn block_energy(&mut self, block: &crate::BlockKey, energy: crate::ModEnergyBuffer) {
        let Some(id) = self.default_block_id(block) else {
            return;
        };
        if energy.capacity < 0 {
            self.refused.push(format!(
                "mod {} declared a block energy buffer for {id} with negative capacity {}",
                self.mod_id, energy.capacity
            ));
            return;
        }
        if energy.max_insert < 0 {
            self.refused.push(format!(
                "mod {} declared a block energy buffer for {id} with negative max_insert {}",
                self.mod_id, energy.max_insert
            ));
            return;
        }
        if energy.max_extract < 0 {
            self.refused.push(format!(
                "mod {} declared a block energy buffer for {id} with negative max_extract {}",
                self.mod_id, energy.max_extract
            ));
            return;
        }
        self.insert_default_block(id, energy, |parts| &mut parts.default_block_energy);
    }
    fn register_item_container(
        &mut self,
        item: &crate::ItemKey,
        provider: Arc<dyn crate::ItemContainerProvider>,
    ) {
        let id = self
            .registry_keys
            .borrow()
            .items
            .id(mod_vocab::__private::parts_installation::item_key_slot(
                *item,
            ))
            .map(str::to_owned);
        let Some(id) = id else {
            self.refused.push(format!(
                "mod {} declared a default provider for an invalid item key",
                self.mod_id
            ));
            return;
        };
        self.parts
            .default_item_containers
            .entry(id.into())
            .or_default()
            .push(DefaultItemContainer {
                mod_id: self.mod_id,
                mod_index: self.mod_index,
                provider,
                panicked: Default::default(),
            });
    }
}
