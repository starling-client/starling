//! The live player and menu witness used across mod callbacks.
use super::*;
use mod_api::__private::game::Fields;
use mod_api::{ItemPlace, MenuSlotBacking, MenuSlotPlace, ServerHost};
use std::collections::BTreeSet;
use std::result::Result;
use sunrise_client::{
    containers::ModMenuType,
    lan::menu::{ModCraft, Slots},
    recipe_book::placement_sync::Refusal,
};

#[derive(Clone, PartialEq)]
pub(super) struct NativeMenuSlot {
    pub(super) place: MenuSlotPlace,
    pub(super) stack: Option<mod_api::ItemStack>,
}

// no wire: server-local state compared before publishing a detached placement.
#[derive(Clone, PartialEq)]
pub(super) struct MenuSnapshot {
    pub(super) conn: u32,
    pub(super) uuid: u128,
    pub(super) entity_id: i32,
    pub(super) placed_at: u64,
    pub(super) dimension: String,
    pub(super) instance: u64,
    pub(super) window: i32,
    pub(super) kind: &'static ModMenuType,
    pub(super) slots: Slots,
    pub(super) pos: Option<(i32, i32, i32)>,
    pub(super) size: usize,

    pub(super) links: Vec<(i32, i32, i32)>,
    pub(super) craft: Option<ModCraft>,
    pub(super) inventory: Vec<Option<ItemStack>>,
    pub(super) armor: [Option<ItemStack>; 4],
    pub(super) offhand: Option<ItemStack>,
    pub(super) carried: Option<ItemStack>,
    pub(super) selected: usize,
    pub(super) game_mode: i32,
    pub(super) health: u32,
    pub(super) xp: (i32, u32, i32, i32),
    pub(super) known: BTreeSet<String>,
    pub(super) native: Option<Vec<Option<NativeMenuSlot>>>,
    block: Option<(u32, Option<u64>, String, usize)>,
}
impl Game<'_> {
    pub(super) fn mod_book_menu_snapshot(
        &mut self,
        conn: u32,
        window: i32,
        menu: &str,
    ) -> Result<MenuSnapshot, Refusal> {
        let (
            uuid,
            entity_id,
            placed_at,
            dimension,
            instance,
            kind,
            slots,
            pos,
            size,
            links,
            craft,
            inventory,
            armor,
            offhand,
            carried,
            selected,
            game_mode,
            health,
            xp,
            known,
        ) = {
            let host = self.lan.as_ref().ok_or(Refusal::Unavailable)?;
            let g = host
                .hub
                .guest(conn)
                .filter(|g| g.placed && host.hub.here(g) && g.health > 0.0 && g.game_mode != 3)
                .ok_or(Refusal::Denied)?;
            let m = g
                .menu
                .as_ref()
                .filter(|m| window != 0 && m.window == window && &*m.registry_id == menu)
                .ok_or(Refusal::Stale)?;
            let kind = m
                .mod_type
                .filter(|t| t.id == menu && t.slots == m.size)
                .ok_or(Refusal::Unavailable)?;
            if !m.still_valid(&host.hub.loop_context, &self.world, g) {
                return Err(Refusal::Stale);
            }
            (
                g.uuid,
                g.entity_id,
                g.placed_at,
                g.dimension.clone(),
                g.menu_generation,
                kind,
                m.slots,
                m.pos,
                m.size,
                m.links.clone(),
                m.craft.clone(),
                g.inventory.clone(),
                g.armor.clone(),
                g.offhand.clone(),
                g.carried.clone(),
                g.selected,
                g.game_mode,
                g.health.to_bits(),
                (g.xp_level, g.xp_progress.to_bits(), g.xp_total, g.xp_seed),
                g.recipe_book.recipes.clone(),
            )
        };
        let (block, native) = match slots {
            Slots::Block(pos) => {
                let Some(BlockEntity::Mod(block)) = self.world.block_entities.get(&pos) else {
                    return Err(Refusal::Unavailable);
                };
                (
                    Some((
                        self.world.block(pos.0, pos.1, pos.2).0,
                        self.world.block_entities.generation(&pos),
                        block.id().to_owned(),
                        block.items.len(),
                    )),
                    None,
                )
            }
            Slots::Native => (
                None,
                Some(self.mod_book_native_slots(conn, uuid, window, size)?),
            ),
            _ => return Err(Refusal::Unavailable),
        };
        Ok(MenuSnapshot {
            conn,
            uuid,
            entity_id,
            placed_at,
            dimension,
            instance,
            window,
            kind,
            slots,
            pos,
            size,

            links,
            craft,
            inventory,
            armor,
            offhand,
            carried,
            selected,
            game_mode,
            health,
            xp,
            known,
            native,
            block,
        })
    }
    pub(super) fn mod_book_menu_unchanged(&self, before: &MenuSnapshot) -> bool {
        let slots = {
            let Some(host) = &self.lan else {
                return false;
            };
            let Some(g) = host.hub.guest(before.conn) else {
                return false;
            };
            let Some(m) = &g.menu else {
                return false;
            };
            if !g.placed
                || !host.hub.here(g)
                || g.uuid != before.uuid
                || g.entity_id != before.entity_id
                || g.placed_at != before.placed_at
                || g.dimension != before.dimension
                || g.menu_generation != before.instance
                || m.window != before.window
                || &*m.registry_id != before.kind.id
                || m.mod_type != Some(before.kind)
                || m.slots != before.slots
                || m.pos != before.pos
                || m.size != before.size
                || m.links != before.links
                || m.craft != before.craft
                || g.inventory != before.inventory
                || g.armor != before.armor
                || g.offhand != before.offhand
                || g.carried != before.carried
                || g.selected != before.selected
                || g.game_mode != before.game_mode
                || g.health.to_bits() != before.health
                || (g.xp_level, g.xp_progress.to_bits(), g.xp_total, g.xp_seed) != before.xp
                || g.recipe_book.recipes != before.known
                || !m.still_valid(&host.hub.loop_context, &self.world, g)
            {
                return false;
            }
            m.slots
        };
        match slots {
            Slots::Block(pos) => {
                let Some(BlockEntity::Mod(block)) = self.world.block_entities.get(&pos) else {
                    return false;
                };
                before.native.is_none()
                    && before.block.as_ref().is_some_and(|state| {
                        self.world.block(pos.0, pos.1, pos.2).0 == state.0
                            && self.world.block_entities.generation(&pos) == state.1
                            && block.id() == state.2.as_str()
                            && block.items.len() == state.3
                    })
            }
            Slots::Native => {
                before.block.is_none()
                    && before.native.as_ref().is_some_and(|native| {
                        native.iter().enumerate().all(|(slot, backing)| {
                            self.native_menu_place(before.conn, before.window, slot)
                                == backing.as_ref().map(|backing| backing.place)
                        })
                    })
            }
            _ => false,
        }
    }

    fn mod_book_native_slots(
        &mut self,
        conn: u32,
        uuid: u128,
        window: i32,
        size: usize,
    ) -> Result<Vec<Option<NativeMenuSlot>>, Refusal> {
        let menu = self.menu(uuid, window).ok_or(Refusal::Changed)?;
        let player = menu.fields().player;
        let places: Vec<_> = (0..size)
            .map(|slot| self.native_menu_place(conn, window, slot))
            .collect();
        let mut slots = Vec::with_capacity(size);
        let host: &mut dyn ServerHost = self;
        for (slot, place) in places.into_iter().enumerate() {
            match place {
                Some(place)
                    if menu.slot_backing(host, slot) == Some(MenuSlotBacking::Place(place))
                        && place.resolves(host, player) =>
                {
                    let stack = ItemPlace::MenuSlot { menu, slot }
                        .with_stack(host, |stack| stack.project())
                        .flatten();
                    slots.push(Some(NativeMenuSlot { place, stack }));
                }
                Some(_) => return Err(Refusal::Changed),
                None if matches!(
                    menu.slot_backing(host, slot),
                    Some(MenuSlotBacking::Native(_))
                ) =>
                {
                    slots.push(None);
                }
                None => return Err(Refusal::Changed),
            }
        }
        Ok(slots)
    }
}
