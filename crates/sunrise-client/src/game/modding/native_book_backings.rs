//! Canonical physical storage behind a detached custom placement.
use super::api_values::TryIntoEngine;
use super::{
    Game,
    native_book_scope::{MenuSnapshot, NativeMenuSlot},
};
use mod_api::__private::game::Fields;
use mod_api::{ItemPlace, MenuSlotBacking, MenuSlotPlace, ServerHost};
use std::collections::{BTreeMap, BTreeSet};
use sunrise_client::{
    containers::BlockEntity, items::ItemStack, lan::menu::SlotBacking,
    menu::place::custom_plan::MenuPlacement, recipe_book::placement_sync::Refusal,
};
use sunrise_core::nbt::Compound;
type Pos = (i32, i32, i32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key {
    pub dimension: &'static str,
    pub pos: Pos,
    pub slot: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum MainKey {
    Block(Key),
    Native(usize),
}
struct Block {
    state: u32,
    generation: Option<u64>,
    kind: std::mem::Discriminant<BlockEntity>,
    id: Option<String>,
    items: Vec<Option<ItemStack>>,
    limit: Option<u32>,
    data: Option<(u64, Compound, Vec<i32>)>,
}
pub(super) struct NativeChange {
    pub(super) slot: usize,
    pub(super) place: MenuSlotPlace,
    pub(super) before: Option<mod_api::ItemStack>,
    pub(super) after: Option<ItemStack>,
}
pub(super) struct Changes {
    pub(super) blocks: BTreeMap<Key, Option<ItemStack>>,
    pub(super) native: Vec<NativeChange>,
}
pub(super) struct Backings {
    dimension: &'static str,
    blocks: BTreeMap<Pos, Block>,
    pub main: Vec<Option<ItemStack>>,
    main_keys: Vec<Option<MainKey>>,
    native: Option<Vec<Option<NativeMenuSlot>>>,
    native_menu: Option<(u32, u128, i32)>,

    clear: BTreeSet<MainKey>,
    pub source_keys: Vec<Vec<Option<Key>>>,
}
impl Backings {
    pub fn capture(
        game: &mut Game<'_>,
        before: &MenuSnapshot,
        plan: &MenuPlacement,
    ) -> Result<Self, Refusal> {
        // A `Mod` layout names its grid later, so any visible stored slot may be cleared.
        let any_slot = plan.awaits_grid();
        let (main, native, native_menu) = match before.slots {
            sunrise_client::lan::menu::Slots::Native => {
                let native = before
                    .native
                    .as_ref()
                    .filter(|slots| slots.len() == before.size)
                    .ok_or(Refusal::Unavailable)?
                    .clone();
                let main = native
                    .iter()
                    .map(|slot| {
                        slot.as_ref()
                            .map(|slot| {
                                slot.stack
                                    .as_ref()
                                    .map(|stack| {
                                        stack
                                            .try_into_engine(game.items())
                                            .map_err(|_| Refusal::Unavailable)
                                    })
                                    .transpose()
                            })
                            .transpose()
                            .map(Option::flatten)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                (
                    main,
                    Some(native),
                    Some((before.conn, before.uuid, before.window)),
                )
            }
            sunrise_client::lan::menu::Slots::Block(_) => {
                let guest = game
                    .lan
                    .as_ref()
                    .and_then(|h| h.hub.guest(before.conn))
                    .ok_or(Refusal::Changed)?;
                let main = before
                    .slots
                    .read(&game.world, guest)
                    .filter(|items| items.len() == before.size)
                    .ok_or(Refusal::Unavailable)?;
                (main, None, None)
            }
            _ => return Err(Refusal::Unavailable),
        };
        let mut value = Self {
            dimension: game.dim.id(),
            blocks: BTreeMap::new(),
            main,
            main_keys: Vec::with_capacity(before.size),
            native,
            native_menu,

            clear: BTreeSet::new(),
            source_keys: Vec::new(),
        };
        let mut seen = BTreeSet::new();
        if let Some(native) = &value.native {
            let mut places = Vec::<MenuSlotPlace>::new();
            for (slot, backing) in native.iter().enumerate() {
                match backing {
                    Some(backing) => {
                        if places.contains(&backing.place) {
                            return Err(Refusal::Unavailable);
                        }
                        places.push(backing.place);
                        value.main_keys.push(Some(MainKey::Native(slot)));
                    }
                    None => value.main_keys.push(None),
                }
            }
        } else {
            let guest = game
                .lan
                .as_ref()
                .and_then(|h| h.hub.guest(before.conn))
                .ok_or(Refusal::Changed)?;
            for slot in 0..before.size {
                let key = match before.slots.logical_backing(&game.world, guest, slot) {
                    Some(SlotBacking::Block(pos, index)) => {
                        let key = Key {
                            dimension: value.dimension,
                            pos,
                            slot: index,
                        };
                        if !seen.insert(key) {
                            return Err(Refusal::Unavailable);
                        }
                        value.keep(game, pos)?;
                        Some(MainKey::Block(key))
                    }
                    None => None,
                    _ => return Err(Refusal::Unavailable),
                };
                value.main_keys.push(key);
            }
        }
        let cleared: Vec<usize> = if any_slot {
            (0..before.size)
                .filter(|slot| value.main_keys[*slot].is_some())
                .collect()
        } else {
            plan.cleared_slots().to_vec()
        };
        for slot in cleared {
            let key = value
                .main_keys
                .get(slot)
                .copied()
                .flatten()
                .ok_or(Refusal::Unavailable)?;
            value.clear.insert(key);
        }
        Ok(value)
    }
    fn keep(&mut self, game: &Game<'_>, pos: Pos) -> Result<(), Refusal> {
        if self.blocks.contains_key(&pos) {
            return Ok(());
        }
        let entity = game
            .world
            .block_entities
            .get(&pos)
            .ok_or(Refusal::Changed)?;
        let (id, data) = match entity {
            BlockEntity::Mod(m) => (
                Some(m.id().to_owned()),
                Some((m.revision().identity(), m.nbt.clone(), m.data.clone())),
            ),
            _ => (None, None),
        };
        self.blocks.insert(
            pos,
            Block {
                state: game.world.block(pos.0, pos.1, pos.2).0,
                generation: game.world.block_entities.generation(&pos),
                kind: std::mem::discriminant(entity),
                id,
                items: entity.slots().to_vec(),
                limit: game.world.container_stack_limit(pos),
                data,
            },
        );
        Ok(())
    }
    /// The source hook grants containers, not duplicate copies of a physical
    /// slot. Cleared slots already return their contents through the player.
    pub fn add_sources(
        &mut self,
        game: &Game<'_>,
        sources: &[(Pos, Vec<Option<ItemStack>>)],
    ) -> Result<Vec<Vec<Option<ItemStack>>>, Refusal> {
        let mut seen: BTreeSet<Key> = self
            .clear
            .iter()
            .filter_map(|key| match key {
                MainKey::Block(key) => Some(*key),
                MainKey::Native(_) => None,
            })
            .collect();
        let mut stock = Vec::with_capacity(sources.len());
        for (pos, items) in sources {
            self.keep(game, *pos)?;
            let mut keys = Vec::with_capacity(items.len());
            let mut values = Vec::with_capacity(items.len());
            for (slot, item) in items.iter().enumerate() {
                let key = Key {
                    dimension: self.dimension,
                    pos: *pos,
                    slot,
                };
                let owned = seen.insert(key);
                keys.push(owned.then_some(key));
                values.push(if owned { item.clone() } else { None });
            }
            self.source_keys.push(keys);
            stock.push(values);
        }
        Ok(stock)
    }
    pub fn unchanged_blocks(&self, game: &Game<'_>) -> bool {
        game.dim.id() == self.dimension
            && self.blocks.iter().all(|(pos, before)| {
                let Some(entity) = game.world.block_entities.get(pos) else {
                    return false;
                };
                if game.world.block(pos.0, pos.1, pos.2).0 != before.state
                    || game.world.block_entities.generation(pos) != before.generation
                    || std::mem::discriminant(entity) != before.kind
                    || entity.slots() != before.items
                    || game.world.container_stack_limit(*pos) != before.limit
                {
                    return false;
                }
                match entity {
                    BlockEntity::Mod(m) => {
                        before.id.as_deref() == Some(m.id())
                            && before.data.as_ref().is_some_and(|(identity, nbt, data)| {
                                m.revision().identity() == *identity
                                    && &m.nbt == nbt
                                    && &m.data == data
                            })
                    }
                    _ => before.id.is_none() && before.data.is_none(),
                }
            })
    }
    pub fn unchanged(&self, game: &mut Game<'_>) -> bool {
        self.unchanged_blocks(game) && self.native_is_unchanged(game)
    }
    fn native_is_unchanged(&self, game: &mut Game<'_>) -> bool {
        let (Some(native), Some((conn, uuid, window))) = (&self.native, self.native_menu) else {
            return self.native.is_none() && self.native_menu.is_none();
        };
        if game.menu(uuid, window).is_none()
            || native.iter().enumerate().any(|(slot, before)| {
                game.native_menu_place(conn, window, slot) != before.as_ref().map(|slot| slot.place)
            })
        {
            return false;
        }
        // Live handler values are checked through the same menu slot place recipe placement
        // publishes, not the guest's cached native-menu mirror.
        let Some(menu) = game.menu(uuid, window) else {
            return false;
        };
        let player = menu.fields().player;
        let host: &mut dyn ServerHost = game;
        native
            .iter()
            .enumerate()
            .all(|(slot, before)| match before {
                Some(before) => {
                    before.place.resolves(host, player)
                        && menu.slot_backing(host, slot)
                            == Some(MenuSlotBacking::Place(before.place))
                        && ItemPlace::MenuSlot { menu, slot }
                            .with_stack(host, |stack| stack.project())
                            .flatten()
                            .as_ref()
                            == before.stack.as_ref()
                }
                None => matches!(
                    menu.slot_backing(host, slot),
                    Some(MenuSlotBacking::Native(_))
                ),
            })
    }
    /// Whether `slot` is stored in a block, so a placement there can be published.
    pub fn stores(&self, slot: usize) -> bool {
        self.main_keys.get(slot).is_some_and(Option::is_some)
    }
    pub fn limits(&self) -> Vec<Option<u32>> {
        self.main_keys
            .iter()
            .map(|key| match key {
                Some(MainKey::Block(key)) => {
                    self.blocks.get(&key.pos).and_then(|block| block.limit)
                }
                _ => None,
            })
            .collect()
    }
    /// Only operation-owned cells are published. Rewriting untouched displayed
    /// slots would restore consumed items when a linked view aliases a source.
    pub fn changes(&self, menu: &mut sunrise_client::menu::Menu) -> Changes {
        let mut blocks = BTreeMap::new();
        let mut native = BTreeMap::new();
        for (slot, key) in self.main_keys.iter().enumerate() {
            if let Some(key) = key
                && self.clear.contains(key)
            {
                match key {
                    MainKey::Block(key) => {
                        blocks.insert(*key, menu.slot(slot).cloned());
                    }
                    MainKey::Native(index) => {
                        let before = self.native.as_ref().unwrap()[*index]
                            .as_ref()
                            .unwrap()
                            .stack
                            .clone();
                        native.insert(
                            *index,
                            NativeChange {
                                slot: *index,
                                place: self.native.as_ref().unwrap()[*index]
                                    .as_ref()
                                    .unwrap()
                                    .place,
                                before,
                                after: menu.slot(slot).cloned(),
                            },
                        );
                    }
                }
            }
        }
        for (keys, items) in self.source_keys.iter().zip(&menu.sources) {
            for (key, item) in keys.iter().zip(items) {
                if let Some(key) = key {
                    blocks.insert(*key, item.clone());
                }
            }
        }
        for (slot, key) in self.main_keys.iter().enumerate() {
            if let Some(MainKey::Block(key)) = key
                && let Some(item) = blocks.get(key)
            {
                menu.load_slot(slot, item.clone());
            }
        }
        blocks.retain(|key, item| {
            self.blocks
                .get(&key.pos)
                .and_then(|block| block.items.get(key.slot))
                != Some(item)
        });
        let native = native
            .into_values()
            .filter(|change| self.main.get(change.slot) != Some(&change.after))
            .collect();
        Changes { blocks, native }
    }
}
