//! Crafting for LAN guests: the inventory's 2x2 grid (window 0) and the
//! crafting table (`minecraft:crafting`) run the host's menu engine
//! (`sunrise_client::menu`, `Kind::Inventory` / `Kind::Crafting`) over the
//! guest's inventory, with vanilla's packets: OpenScreen, container clicks
//! and their corrections, and the grid back into the inventory when the menu
//! closes (`AbstractContainerMenu.clearContainer`).
use super::{
    Game,
    lan_actions::RECIPE_CHECK_COST,
    transactions::{After, decision::MenuOwner},
};
use sunrise_client::lan::recipes;
use sunrise_client::{
    containers::{BlockEntity, ModMenuType},
    furnace::FurnaceCooking,
    items::{ItemStack, Items},
    lan::{
        guest::Guest,
        menu::{ModCraft, Slots, linked_slot_count, read_links, write_links},
    },
    menu::{
        Env, Input, Kind, Menu,
        place::{PostPlace, TransferCheck, TransferError},
    },
};
use sunrise_protocol::packets::play::{self, Clientbound as C};

/// `AbstractContainerMenu.dropOrPlaceInInventory`: killed or disconnected players
/// drop inputs, while a live close or a dimension change returns them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuRemoval {
    Live,
    ChangedDimension,
    Killed,
    Disconnected,
}

impl MenuRemoval {
    pub(super) fn drops_items(self) -> bool {
        matches!(self, Self::Killed | Self::Disconnected)
    }
}

/// The guest's menus the game runs: kept between clicks, so a drag (quick
/// craft) spans them as in vanilla.
#[derive(Default)]
pub(super) struct GuestCrafting {
    /// The inventory menu; its grid lives in the hub's guest.
    pub(super) inventory: MenuOwner<Option<Box<Menu>>>,
    /// The open crafting table: its window, state id, menu and block.
    pub(super) table: MenuOwner<Option<(i32, i32, Box<Menu>, [i32; 3])>>,
    /// The open furnace's or brewing stand's window and menu (`AbstractFurnaceMenu`,
    /// `BrewingStandMenu`); its slots live in the block entity and the hub's
    /// container menu keeps the client in sync.
    pub(super) machine: MenuOwner<Option<(i32, Box<Menu>)>>,
}

/// The most containers `native_recipe_sources` adds to one placement.
const MAX_RECIPE_SOURCES: usize = 64;
/// The most containers `native_menu_links` links one menu's slots to.
const MAX_MENU_LINKS: usize = 64;

/// ServerboundContainerClickPacket's input.
pub(super) fn input(id: i32) -> Input {
    match id {
        0 => Input::Pickup,
        1 => Input::QuickMove,
        2 => Input::Swap,
        3 => Input::Clone,
        4 => Input::Throw,
        5 => Input::QuickCraft,
        _ => Input::PickupAll,
    }
}

/// The armor's player-container slots, head to feet (`Inventory` 39..36).
const ARMOR: [usize; 4] = [39, 38, 37, 36];
const OFFHAND: usize = 40;

/// The guest's inventory, armor, off hand, selection and carried stack into `menu`.
pub(super) fn load_player(menu: &mut Menu, g: &Guest) {
    for i in 0..36 {
        menu.set_player_item(i, g.inventory[i].clone());
    }
    for (k, slot) in ARMOR.into_iter().enumerate() {
        menu.set_player_item(slot, g.armor[k].clone());
    }
    menu.set_player_item(OFFHAND, g.offhand.clone());
    menu.selected = g.selected;
    menu.creative = g.creative();
    menu.levels = g.xp_level;
    menu.carried = g.carried.clone();
}

/// The vanilla menu engine's kind for a guest's menu over block entity `entity`, with the
/// container's slots: a furnace's, a brewing stand's, a crafter's, or a content mod's menu type
/// with its slots' rules. None for the others, whose clicks the hub runs.
pub(super) fn machine_menu(
    entity: &BlockEntity,
    mod_type: Option<&'static sunrise_client::containers::ModMenuType>,
) -> Option<(Kind, Vec<Option<ItemStack>>)> {
    Some(match entity {
        BlockEntity::Furnace(f) => {
            let recipe = match f.kind.recipe_type() {
                "minecraft:blasting" => "blasting",
                "minecraft:smoking" => "smoking",
                _ => "smelting",
            };
            (Kind::Furnace(recipe), f.items.to_vec())
        }
        BlockEntity::Brewing(b) => (Kind::Brewing, b.items.to_vec()),
        BlockEntity::Crafter(c) => (Kind::CrafterBlock, c.items.to_vec()),
        BlockEntity::Mod(m) => {
            let t = mod_type?;
            let mut items = m.items.clone();
            items.resize(t.slots, None);
            (Kind::Mod(t), items)
        }
        _ => return None,
    })
}

/// `load_player` back.
pub(super) fn store_player(menu: &mut Menu, g: &mut Guest) {
    for i in 0..36 {
        g.inventory[i] = menu.player_item(i).cloned();
    }
    for (k, slot) in ARMOR.into_iter().enumerate() {
        g.armor[k] = menu.player_item(slot).cloned();
    }
    g.offhand = menu.player_item(OFFHAND).cloned();
    g.carried = menu.carried.take();
}

/// Inventory.placeItemBackInInventory for each stack: what doesn't fit is
/// left for Player.drop (creative players keep it too).
fn place_back(menu: &mut Menu, items: &Items, stacks: impl IntoIterator<Item = ItemStack>) {
    menu.creative = false;
    for mut stack in stacks {
        menu.give(items, &mut stack);
        if stack.count > 0 {
            menu.dropped.push(stack);
        }
    }
}

/// The non-storage effects from one guest menu action.
#[derive(Default)]
pub(in crate::game) struct GuestMenuEffects {
    dropped: Vec<ItemStack>,
    equipped: Vec<(&'static str, Option<ItemStack>)>,
    brewed: Vec<ItemStack>,
    brewed_potion_takes: Vec<ItemStack>,
    crafted: Vec<(String, Vec<Option<ItemStack>>)>,
    item_crafted: Vec<(String, ItemStack, Vec<Option<ItemStack>>)>,
    crafted_counts: Vec<(std::sync::Arc<str>, i32)>,
}

pub(in crate::game) struct FurnaceMenuEffects {
    uuid: u128,
    entity_id: i32,
    pos: (i32, i32, i32),
    at: [f64; 3],
    smelted: Vec<(Option<ItemStack>, u32)>,
    extracted: Option<(ItemStack, u32)>,
}

impl GuestMenuEffects {
    pub(in crate::game) fn dropped(dropped: Vec<ItemStack>) -> Self {
        Self {
            dropped,
            ..Self::default()
        }
    }

    fn take(menu: &mut Menu) -> Self {
        let dropped = std::mem::take(&mut menu.dropped);
        menu.drops.clear();
        Self {
            dropped,
            equipped: std::mem::take(&mut menu.equipped),
            brewed: std::mem::take(&mut menu.brewed),
            brewed_potion_takes: std::mem::take(&mut menu.brewed_potion_takes),
            crafted: std::mem::take(&mut menu.crafted),
            item_crafted: std::mem::take(&mut menu.item_crafted),
            crafted_counts: std::mem::take(&mut menu.crafted_counts),
        }
    }
}

impl Game<'_> {
    /// `LocalPlayer.doLimitedCrafting`: whether limited crafting is active.
    pub fn limited_crafting(&self) -> bool {
        self.remote.as_ref().map_or_else(
            || {
                self.ui
                    .rules
                    .bool_at(sunrise_client::gamerules::vanilla::LIMITED_CRAFTING)
            },
            |r| r.do_limited_crafting,
        )
    }

    pub(super) fn menu_env(&self) -> Env<'_> {
        Env {
            crafting_remainder: Some(self),
            player: self
                .lan_acting_uuid
                .or_else(|| self.remote.as_ref().map(|r| r.profile())),
            player_entity_id: Some(self.own_entity_id()),
            loop_context: &self.loop_context,
            items: self.items(),
            recipes: self.recipes(),
            brewing: &self.stations().brewing,
            banners: &self.stations().banners,
            station_recipes: self.station_recipes(),
            enchantments: &self.stations().enchantments,
            names: &self.pack.names,
            trade: None,
            maps: None,
            limited_crafting: self.limited_crafting(),
            can_equip: None,
            known_recipes: Some(&self.level.recipe_book),
            slot_rules: Some(self),
            stacked: None,
        }
    }

    /// Runs `op` on the guest's menu with their inventory copied in and back.
    pub(super) fn guest_menu_op<R>(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        op: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        self.guest_menu_op_with(gi, menu, None, op)
    }

    /// `guest_menu_op` with the mods' answer to a decided pickup, when there is one.
    pub(super) fn guest_menu_op_with<R>(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        stacked: Option<&sunrise_client::menu::StackedOutcome>,
        op: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        let result = self.guest_menu_edit(gi, menu, stacked, op)?;
        let conn = self.lan.as_ref()?.hub.guests[gi].conn;
        self.publish_guest_menu_effects(conn, GuestMenuEffects::take(menu));
        Some(result)
    }

    /// Run one edit with the guest's rules and storage, leaving effects in the menu until its
    /// caller accepts the action. No `Env` survives this call.
    fn guest_menu_edit<R>(
        &mut self,
        gi: usize,
        menu: &mut Menu,
        stacked: Option<&sunrise_client::menu::StackedOutcome>,
        op: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        self.lan.as_ref()?;
        // Crafting and cartography read and derive this level's saved maps.
        let maps = matches!(
            menu.kind(),
            Kind::Cartography | Kind::Crafting | Kind::Inventory
        )
        .then(|| {
            let g = &self.lan.as_ref().unwrap().hub.guests[gi];
            (0..menu.slot_count())
                .filter_map(|s| menu.slot(s).cloned())
                .chain(g.inventory.iter().flatten().cloned())
                .chain(g.carried.clone())
                .collect::<Vec<_>>()
        })
        .filter(|held| {
            menu.kind() == Kind::Cartography
                || held
                    .iter()
                    .filter_map(super::maps::map_id)
                    .any(|id| self.map(id).is_some())
        })
        .map(|held| self.level_maps(&held));
        let host = self.lan.as_ref()?;
        // `ArmorSlot.mayPlace` asks the parts with the game as `Host`: the menu is the guest's own.
        let parts = self.menu_can_equip();
        let env = Env {
            maps: maps.as_ref().map(|m| m as &dyn sunrise_client::menu::Maps),
            can_equip: Some(&parts),
            stacked,
            ..self.load_guest_menu(menu, host, gi)
        };
        menu.brewed_potion_hook = !mod_api::__private::mods().is_empty();
        menu.item_smelted_hook = menu.brewed_potion_hook;
        let result = op(menu, &env);
        if let Some(maps) = maps {
            self.register_made_maps(maps);
        }
        self.lan
            .as_mut()?
            .hub
            .guests
            .update(gi, |guest| store_player(menu, guest));
        Some(result)
    }

    pub(super) fn publish_furnace_menu_effects(&mut self, effects: FurnaceMenuEffects) {
        let FurnaceMenuEffects {
            uuid,
            entity_id,
            pos,
            at,
            smelted,
            extracted,
        } = effects;
        for (stack, removed) in smelted {
            self.native_item_smelted(uuid, entity_id, stack.as_ref(), removed);
        }
        if let Some((stack, taken)) = extracted {
            if mod_api::__private::listened(mod_api::HookId::FurnaceExtract)
                && let Some(name) = self.lan.as_ref().and_then(|host| {
                    host.hub
                        .guests
                        .iter()
                        .find(|g| g.uuid == uuid)
                        .map(|g| g.name.clone())
                })
            {
                self.native_furnace_extract(uuid, &name, [pos.0, pos.1, pos.2], &stack.item, taken);
            }
            self.award_furnace_recipes(pos, at);
        }
    }

    /// Load the player and slot context for the menu's live queries, without storing any edits.
    fn load_guest_menu<'a>(
        &'a self,
        menu: &mut Menu,
        host: &'a super::lan::LanHost,
        gi: usize,
    ) -> Env<'a> {
        let guest = &host.hub.guests[gi];
        let open = guest.menu.as_ref();
        let limits = open
            .map(|opened| opened.slots.stack_limits(&self.world, opened.size))
            .unwrap_or_default();
        menu.set_container_limits(limits);
        menu.set_block_pos(
            open.and_then(|opened| opened.pos)
                .map(|(x, y, z)| [x, y, z]),
        );
        load_player(menu, guest);
        Env {
            player: Some(guest.uuid),
            player_entity_id: Some(guest.entity_id),
            limited_crafting: host.hub.limited_crafting,
            known_recipes: Some(&guest.recipe_book),
            ..self.menu_env()
        }
    }

    pub(in crate::game) fn publish_guest_menu_effects(
        &mut self,
        conn: u32,
        effects: GuestMenuEffects,
    ) {
        let Some(gi) = self.guest_index(conn) else {
            return;
        };
        let guest = &self.lan.as_ref().unwrap().hub.guests[gi];
        let (uuid, entity_id) = (guest.uuid, guest.entity_id);
        let GuestMenuEffects {
            dropped,
            equipped,
            brewed,
            brewed_potion_takes,
            crafted,
            item_crafted,
            crafted_counts,
        } = effects;
        // `ItemStack.onCraftedBy`'s `Stats.ITEM_CRAFTED` count, before the
        // recipe the take used.
        if !crafted_counts.is_empty() {
            self.as_guest(conn, |g| {
                for (item, count) in crafted_counts {
                    g.award_stat("crafted", &item, count);
                }
            });
        }
        if !brewed.is_empty() || !brewed_potion_takes.is_empty() {
            self.as_guest(conn, |g| {
                for stack in &brewed_potion_takes {
                    g.native_player_brewed_potion(uuid, entity_id, stack);
                }
                for stack in &brewed {
                    g.trigger_brewed_potion(stack);
                }
            });
        }
        if !crafted.is_empty() {
            self.as_guest(conn, |g| {
                for (recipe, ingredients) in &crafted {
                    g.trigger_recipe_crafted(recipe, ingredients);
                }
            });
        }
        if !item_crafted.is_empty() {
            self.as_guest(conn, |g| {
                let player = g.native_acting_id();
                for (recipe, stack, inputs) in &item_crafted {
                    // ItemStack.onCraftedBy awards Stats.ITEM_CRAFTED.
                    g.award_stat("crafted", &stack.item, stack.count as i32);
                    g.native_item_crafted(player, recipe, stack, inputs);
                }
            });
        }
        if !equipped.is_empty() {
            self.as_guest(conn, |g| {
                for (slot, stack) in equipped {
                    g.player_equipped(slot, stack.as_ref());
                }
            });
        }
        self.drop_guest_items(gi, dropped);
    }

    /// `InventoryMenu.slotsChanged` for guest `uuid`'s crafting grid written outside a click
    /// (a mod's menu slot write, at the root commit): the result made again from the grid, with
    /// `native_prepare_item_craft` as a click has it, kept by the guest and their menu. The
    /// client hears it with the tick's inventory sync.
    pub(super) fn guest_craft_result(&mut self, conn: u32) {
        let Some(g) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn).filter(|g| g.placed))
        else {
            return;
        };
        let uuid = g.uuid;
        let mut result = self
            .recipes()
            .find(&self.loop_context, &g.craft_grid, 2)
            .map(|(_, stack)| stack);
        if mod_api::__private::listened(mod_api::HookId::PrepareItemCraft) {
            let name = g.name.clone();
            self.native_prepare_item_craft(uuid, &name, None, &mut result);
        }
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        if host
            .hub
            .update_guest(conn, |g| g.craft_result.clone_from(&result))
            .is_none()
        {
            return;
        }
        let menu = host
            .menus
            .get_mut(&conn)
            .and_then(|p| p.crafting.inventory.take());
        if let Some(mut menu) = menu {
            menu.set_slot(&self.menu_env(), 0, result);
            if let Some(play) = self.lan.as_mut().and_then(|host| host.menus.get_mut(&conn)) {
                *play.crafting.inventory = Some(menu);
            }
        }
    }

    /// A click on the guest's inventory menu (window 0): `InventoryMenu`,
    /// crafting grid included. False for other windows.
    pub(super) fn lan_inventory_click(&mut self, conn: u32, p: &play::ContainerClick) -> bool {
        if p.container != 0 {
            return false;
        }
        let Some(host) = self.lan.as_mut() else {
            return true;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return true;
        };
        if host.hub.guests[gi].spectator() {
            return true;
        }
        let crafting = host.menus.entry(conn).or_default();
        let mut menu = crafting
            .crafting
            .inventory
            .take()
            .unwrap_or_else(|| Box::new(Menu::new(Kind::Inventory)));
        let grid = host.hub.guests[gi].craft_grid.clone();
        let Some(changed) = self.guest_menu_click(gi, &mut menu, p, move |menu, env| {
            for (k, s) in grid.iter().enumerate() {
                menu.set_slot(env, 1 + k, s.clone());
            }
        }) else {
            return true;
        };
        let Some(host) = self.lan.as_ref() else {
            return true;
        };
        let guest = &host.hub.guests[gi];
        let (uuid, name) = (guest.uuid, guest.name.clone());
        let mut result = menu.slot(0).cloned();
        self.native_prepare_item_craft(uuid, &name, None, &mut result);
        let env = self.menu_env();
        menu.set_slot(&env, 0, result);
        let Some(host) = self.lan.as_mut() else {
            return true;
        };
        host.hub.guests.update(gi, |g| {
            for k in 0..4 {
                g.craft_grid[k] = menu.slot(1 + k).cloned();
            }
            g.craft_result = menu.slot(0).cloned();
        });
        *host.menus.entry(conn).or_default().crafting.inventory = Some(menu);
        host.hub.inventory_clicked(conn, p, &self.world, &changed);
        true
    }

    /// Run a blockless owned menu through the same slot layout and click engine as its client.
    pub(super) fn lan_owned_menu_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
    ) -> Option<bool> {
        use super::lan_menus::ClickReply;
        let host = self.lan.as_mut()?;
        let gi = host.hub.guests.connection_index(conn)?;
        let guest = &host.hub.guests[gi];
        let layout = guest.menu.as_ref().and_then(|m| {
            let layout = m.mod_type.filter(|t| t.owned_slots.is_some())?;
            (m.window == p.container && !guest.spectator()).then_some(layout)
        })?;
        let generation = guest.menu_generation;
        let items = guest.native_menu.clone();
        let mut menu = host
            .menus
            .entry(conn)
            .or_default()
            .owned_menu
            .take()
            .filter(|(g, _)| *g == generation)
            .map_or_else(|| Box::new(Menu::new(Kind::Mod(layout))), |(_, menu)| menu);
        let live = |g: &Guest| {
            g.menu_generation == generation
                && g.menu.as_ref().is_some_and(|m| m.window == p.container)
        };
        let click = super::lan_pickup::click_of(p);
        let prepare = |menu: &mut Menu, env: &Env| menu.load_container(env, &items, None);
        let snapshot = menu.snapshot();
        let changed = self.guest_menu_action(
            gi,
            &mut menu,
            Some(click),
            Some(&prepare),
            |game, menu, stacked| {
                let gi = game.guest_index(conn)?;
                game.lan.as_ref()?.hub.guest(conn).filter(|g| live(g))?;
                let changed: Vec<_> = game.guest_menu_edit(gi, menu, stacked, |menu, env| {
                    prepare(menu, env);
                    let before: Vec<_> = (0..menu.slot_count())
                        .map(|s| menu.slot(s).cloned())
                        .collect();
                    menu.click(env, click.0, click.1, click.2).ok()?;
                    Some(
                        (0..menu.slot_count())
                            .filter(|&s| menu.slot(s) != before[s].as_ref())
                            .collect(),
                    )
                })??;
                game.lan.as_mut()?.hub.guests.update(gi, |g| {
                    g.native_menu = (0..layout.slots).map(|s| menu.slot(s).cloned()).collect();
                });
                game.after_commit(After::GuestMenu(
                    conn,
                    Box::new(GuestMenuEffects::take(menu)),
                ));
                Some(changed)
            },
        );
        let applied = changed.is_some();
        if !applied {
            menu.restore(snapshot);
        }
        if let Some(host) = self.lan.as_mut()
            && host.hub.guest(conn).is_some_and(live)
        {
            host.menus.entry(conn).or_default().owned_menu = Some((generation, menu));
            if let Some(changed) = changed {
                self.reply_predicted_menu_click(conn, p, changed);
            } else {
                self.reply_menu_click(conn, ClickReply::Resend(p.container));
            }
        }
        Some(applied)
    }

    /// A click in a machine or block-backed mod menu. `None` means a different kind of menu.
    pub(in crate::game) fn lan_machine_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
    ) -> Option<bool> {
        use super::lan_menus::ClickReply;
        let host = self.lan.as_mut()?;
        let gi = host.hub.guests.connection_index(conn)?;
        let (pos, mod_type, menu_id) = host.hub.guests[gi]
            .menu
            .as_ref()
            .filter(|m| p.container != 0 && m.window == p.container)
            .and_then(|m| match m.slots {
                Slots::Block(pos) => Some((
                    pos,
                    m.mod_type,
                    host.hub.registry_name("minecraft:menu", m.kind)?.to_owned(),
                )),
                _ => None,
            })?;
        // A mod menu type's linked slots show their containers' slots
        // (`native_menu_links`); those with nothing behind them are left out
        // of the click, as a hidden slot is.
        let linked = mod_type.and_then(|t| t.links).map(|l| {
            let links = host.hub.guests[gi]
                .menu
                .as_ref()
                .map(|m| m.links.clone())
                .unwrap_or_default();
            (l, links)
        });
        let unlinked = linked
            .as_ref()
            .map(|(l, links)| l.slots().skip(linked_slot_count(&self.world, links)));
        let entity = self.world.block_entities.get(&pos)?;
        let (kind, mut items) = machine_menu(entity, mod_type)?;
        let heard = matches!((kind, entity), (Kind::Mod(_), BlockEntity::Mod(entity))
            if mod_api::__private::block_entity_type_listens(mod_api::HookId::MenuSlotTake, entity.id()));
        let click = super::lan_pickup::click_of(p);
        let native_quick_move = click.2 == Input::QuickMove
            && (0..=1).contains(&click.1)
            && matches!((kind, entity), (Kind::Mod(_), BlockEntity::Mod(entity))
                if mod_api::__private::block_entity_type_listens(mod_api::HookId::MenuQuickMove, entity.id()));
        let overlay_links = |world: &sunrise_client::world::World,
                             items: &mut [Option<ItemStack>]| {
            if let Some((l, links)) = &linked {
                for (k, stack) in l.slots().zip(
                    read_links(world, links, l.count)
                        .into_iter()
                        .chain(std::iter::repeat(None)),
                ) {
                    items[k] = stack;
                }
            }
        };
        overlay_links(&self.world, &mut items);
        let furnace = matches!(kind, Kind::Furnace(_));
        // CrafterBlockEntity.containerData: the disabled slots, then triggered.
        let crafter_data = match self.world.block_entities.get(&pos) {
            Some(BlockEntity::Crafter(c)) => Some((c.disabled, c.is_triggered())),
            _ => None,
        };
        if host.hub.guests[gi].spectator() {
            // handleContainerClick: nothing changes, and sendAllDataToRemote
            // takes back what the client predicted.
            self.reply_menu_click(conn, ClickReply::Resend(p.container));
            return Some(true);
        }
        let menu_generation = host.hub.guests[gi].menu_generation;
        let generation = self.world.block_entities.generation(&pos);
        let live = |guest: &Guest| {
            guest.menu_generation == menu_generation
                && guest
                    .menu
                    .as_ref()
                    .is_some_and(|open| open.window == p.container)
        };
        self.journal_menu_click(conn);
        let host = self.lan.as_mut()?;
        let mut menu = host
            .menus
            .entry(conn)
            .or_default()
            .crafting
            .machine
            .take()
            .filter(|(window, m)| *window == p.container && m.kind() == kind)
            .map_or_else(|| Box::new(Menu::new(kind)), |(_, m)| m);
        menu.set_registry_id(Some(&menu_id));
        let mut refreshed_result = None;
        if matches!(kind, Kind::Mod(_)) {
            menu.reset_mod_visibility(unlinked.into_iter().flatten());
            let host = self.lan.as_ref().unwrap();
            let shown = Self::lan_mod_craft_shown(host, gi, &items);
            let env = self.load_guest_menu(&mut menu, host, gi);
            // Use the shown result without assembling again. If the grid is stale,
            // prepare its new result before any rule or move reads that slot.
            let needs_preview = shown.is_none();
            menu.load_container(&env, &items, shown);
            if needs_preview && let Some(craft) = menu.mod_crafting() {
                self.lan_mod_craft_settle(gi, pos, &mut menu);
                refreshed_result = Some(craft.result);
            }
        }
        let refresh = |game: &Game<'_>, menu: &mut Menu| {
            if generation != game.world.block_entities.generation(&pos)
                || !game
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guest(conn))
                    .is_some_and(live)
            {
                return false;
            }
            let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&pos) else {
                return false;
            };
            let mut actual = entity.items.clone();
            actual.resize(items.len(), None);
            overlay_links(&game.world, &mut actual);
            let old = menu.container_items();
            // `load_container` recomputes from the grid. Keep the current (possibly
            // customized) result when the listener only edited slots outside the grid.
            let kept_result = menu
                .mod_crafting()
                .filter(|c| c.grid_slots().all(|k| old[k] == actual[k]))
                .map(|c| menu.slot(c.result).cloned());
            let Some(host) = game.lan.as_ref() else {
                return false;
            };
            let Some(gi) = host.hub.guests.connection_index(conn) else {
                return false;
            };
            let env = game.load_guest_menu(menu, host, gi);
            menu.load_container(&env, &actual, kept_result);
            true
        };
        let prepare = |menu: &mut Menu, env: &Env| {
            if !matches!(kind, Kind::Mod(_)) {
                for (k, s) in items.iter().enumerate() {
                    menu.set_slot(env, k, s.clone());
                }
            }
            // After the slots: setting a disabled slot (CrafterBlockEntity.setItem) enables it.
            if let Some((disabled, triggered)) = crafter_data {
                for (k, d) in disabled.iter().enumerate() {
                    menu.data[k] = i32::from(*d);
                }
                menu.data[9] = i32::from(triggered);
            }
        };
        let pickup_checked = if native_quick_move
            && !menu.quick_crafting()
            && let Ok(slot) = usize::try_from(click.0)
            && slot < menu.slot_count()
        {
            let env = self.load_guest_menu(&mut menu, self.lan.as_ref().unwrap(), gi);
            if !menu.may_pickup(&env, slot) {
                // The source refused before quickMoveStack. Correct the client's prediction.
                if let Some(host) = self.lan.as_mut()
                    && host.hub.guest(conn).is_some_and(live)
                {
                    *host.menus.entry(conn).or_default().crafting.machine =
                        Some((p.container, menu));
                    self.reply_predicted_menu_click(
                        conn,
                        p,
                        refreshed_result.into_iter().collect(),
                    );
                }
                return Some(true);
            }
            if let Some(applied) = self.lan_mod_quick_move(conn, p, Some(&menu)) {
                if !applied
                    && let Some(host) = self.lan.as_mut()
                    && host.hub.guest(conn).is_some_and(live)
                {
                    *host.menus.entry(conn).or_default().crafting.machine =
                        Some((p.container, menu));
                }
                return Some(applied);
            }
            // The listener can leave live edits even when it doesn't handle the click.
            // Refresh the already-admitted menu without asking the pickup rule again.
            if !refresh(self, &mut menu) {
                self.reply_menu_click(conn, ClickReply::Resend(p.container));
                return Some(false);
            }
            true
        } else {
            false
        };
        // The write-back to the block's storage and its effects: inside the click's decision, so
        // an outer abort restores them with the cursor and the slots.
        let write_back = |game: &mut Game<'_>, menu: &mut Menu, changed: bool| {
            game.journal_block_entity(pos);
            if let Some((_, links)) = &linked {
                for &at in links {
                    game.journal_block_entity(at);
                }
            }
            let mut linked_after = Vec::new();
            match game.world.block_entities.get_mut(&pos) {
                Some(BlockEntity::Furnace(f)) => {
                    for k in 0..3 {
                        f.items[k] = menu.slot(k).cloned();
                    }
                    // AbstractFurnaceBlockEntity.setItem: only a new input restarts cooking.
                    f.input_clicked(&game.loop_context, world_recipes!(game), items[0].clone());
                }
                // BrewingStandBlockEntity.serverTick notices a new ingredient itself.
                Some(BlockEntity::Brewing(b)) => {
                    for k in 0..5 {
                        b.items[k] = menu.slot(k).cloned();
                    }
                }
                // The mod's items past its menu's slots stay as they were.
                Some(BlockEntity::Mod(m)) => {
                    let n = items.len();
                    if m.items.len() < n {
                        m.items.resize(n, None);
                    }
                    let mut after = menu.container_items();
                    // Linked slots go back to their containers, never into the
                    // block entity.
                    if let Some((l, _)) = &linked {
                        linked_after = l
                            .slots()
                            .filter_map(|k| after.get_mut(k).map(Option::take))
                            .collect();
                    }
                    for (slot, stack) in m.items.iter_mut().zip(after) {
                        *slot = stack;
                    }
                }
                // CrafterBlockEntity.setItem: filling a disabled slot enables it.
                Some(BlockEntity::Crafter(c)) => {
                    for k in 0..9 {
                        c.items[k] = menu.slot(k).cloned();
                        c.disabled[k] = menu.data[k] == 1;
                    }
                }
                _ => {}
            }
            // FurnaceResultSlot.checkTakeAchievements and its observable effects may run
            // only after the click, including any outer menu decision, is committed, and only for
            // actual takes (`onTake` / `onQuickCraft`), never direct `Slot.set` edits.
            let smelted = std::mem::take(&mut menu.item_smelted);
            let extracted = if furnace {
                let mut takes = std::mem::take(&mut menu.furnace_extracted);
                takes.pop().map(|(stack, last)| {
                    let total = takes.into_iter().map(|(_, n)| n).sum::<u32>() + last;
                    (stack, total)
                })
            } else {
                None
            };
            if (!smelted.is_empty() || extracted.is_some())
                && let Some(guest) = game.lan.as_ref().and_then(|host| host.hub.guest(conn))
            {
                let effects = FurnaceMenuEffects {
                    uuid: guest.uuid,
                    entity_id: guest.entity_id,
                    pos,
                    at: guest.position,
                    smelted,
                    extracted,
                };
                game.after_commit(After::FurnaceMenu(Box::new(effects)));
            }
            // Slot.setChanged on a changed container slot: BlockEntity.setChanged,
            // so the chunk is saved and comparators read the block again.
            if changed {
                game.after_commit(After::Changed(pos));
            }
            if let Some((l, links)) = &linked {
                let bound = linked_slot_count(&game.world, links).min(l.count);
                linked_after.truncate(bound);
                for at in write_links(&mut game.world, links, linked_after) {
                    game.after_commit(After::Changed(at));
                }
            }
        };
        if heard {
            let host = self.lan.as_ref().unwrap();
            let before: Vec<_> = (0..menu.slot_count())
                .map(|s| host.hub.view_slot(gi, &self.world, s))
                .collect();
            let original = menu.snapshot();
            let uuid = host.hub.guests[gi].uuid;
            let offhand_before = host.hub.guests[gi].offhand.clone();
            let applied = mod_api::run_transaction(self, |game, _| {
                game.journal_guest_click(conn);
                game.guest_menu_action(
                    gi,
                    &mut menu,
                    Some(click),
                    Some(&prepare),
                    |game, menu, stacked| {
                        let (mut previous, mut rest) =
                            game.guest_menu_edit(gi, menu, stacked, |menu, env| {
                                if !pickup_checked {
                                    prepare(menu, env);
                                }
                                menu.track_takes(true);
                                let before = menu.container_items();
                                let rest = if pickup_checked {
                                    menu.start_checked_quick_move(env, click.0, click.1)
                                } else {
                                    menu.start_click(env, click.0, click.1, click.2)
                                };
                                (before, rest)
                            })?;
                        if rest.is_err() {
                            return None;
                        }
                        loop {
                            let after = menu.container_items();
                            write_back(game, menu, after != previous);
                            let menu_live = if let Some(take) = menu.take_notice() {
                                if game.as_guest(conn, |game| {
                                    game.mod_menu_slot_take(pos, uuid, &menu_id, &take)
                                }) != Some(true)
                                {
                                    return None;
                                }
                                refresh(game, menu)
                            } else {
                                true
                            };
                            let Some(step) = rest.ok()? else {
                                break;
                            };
                            let step = if menu_live {
                                // Listener edits are now the step's inputs. Don't overwrite them.
                                previous = menu.container_items();
                                step
                            } else {
                                let Some(owed) = step.after_menu_closed() else {
                                    break;
                                };
                                // Losing the block queues a close, so the admitted cursor is
                                // still live here. Never settle this write on a different window.
                                game.lan.as_ref()?.hub.guest(conn).filter(|g| live(g))?;
                                owed
                            };
                            let next = game.guest_menu_edit(gi, menu, stacked, |menu, env| {
                                menu.resume_click(env, step)
                            })?;
                            if !menu_live {
                                // Only the player-side write was owed. Don't write back the menu.
                                break;
                            }
                            rest = Ok(next);
                        }
                        menu.track_takes(false);
                        game.after_commit(After::GuestMenu(
                            conn,
                            Box::new(GuestMenuEffects::take(menu)),
                        ));
                        Some(())
                    },
                )
                .ok_or(())
            })
            .is_ok();
            if applied && refresh(self, &mut menu) {
                self.lan_mod_craft_settle(gi, pos, &mut menu);
            }
            if !applied {
                menu.restore(original);
            }
            if let Some(host) = self.lan.as_mut()
                && let Some(guest) = host.hub.guest(conn)
            {
                let offhand = guest.offhand != offhand_before
                    && !(p.input == sunrise_client::lan::menu::input::SWAP && p.button == 40);
                if live(guest) {
                    let reply = if !applied {
                        ClickReply::Resend(p.container)
                    } else if generation != self.world.block_entities.generation(&pos) {
                        ClickReply::Close(p.container)
                    } else {
                        let changed = before
                            .into_iter()
                            .enumerate()
                            .filter_map(|(s, old)| {
                                (refreshed_result == Some(s)
                                    || old != host.hub.view_slot(gi, &self.world, s))
                                .then_some(s)
                            })
                            .collect();
                        ClickReply::Predicted(p.clone(), changed)
                    };
                    *host.menus.entry(conn).or_default().crafting.machine =
                        Some((p.container, menu));
                    self.reply_menu_click(conn, reply);
                }
                if offhand {
                    self.reply_menu_click(conn, ClickReply::Offhand);
                }
            }
            return Some(applied);
        }
        let original = menu.snapshot();
        let changed = self.guest_menu_action(
            gi,
            &mut menu,
            Some(click),
            Some(&prepare),
            |game, menu, stacked| {
                let changed: Vec<_> = game.guest_menu_edit(gi, menu, stacked, |menu, env| {
                    if !pickup_checked {
                        prepare(menu, env);
                    }
                    let before: Vec<_> = (0..menu.slot_count())
                        .map(|s| menu.slot(s).cloned())
                        .collect();
                    if pickup_checked {
                        menu.start_checked_quick_move(env, click.0, click.1).ok()?;
                    } else {
                        menu.click(env, click.0, click.1, click.2).ok()?;
                    }
                    Some(
                        (0..menu.slot_count())
                            .filter(|&s| menu.slot(s) != before[s].as_ref())
                            .collect(),
                    )
                })??;
                write_back(game, menu, changed.iter().any(|&s| s < items.len()));
                game.after_commit(After::GuestMenu(
                    conn,
                    Box::new(GuestMenuEffects::take(menu)),
                ));
                Some(changed)
            },
        );
        let applied = changed.is_some();
        let changed = if let Some(mut changed) = changed {
            if self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .is_some_and(live)
            {
                for slot in self
                    .lan_mod_craft_settle(gi, pos, &mut menu)
                    .into_iter()
                    .chain(refreshed_result)
                {
                    if !changed.contains(&slot) {
                        changed.push(slot);
                    }
                }
            }
            Some(changed)
        } else {
            menu.restore(original);
            None
        };
        if let Some(host) = self.lan.as_mut()
            && host.hub.guest(conn).is_some_and(live)
        {
            *host.menus.entry(conn).or_default().crafting.machine = Some((p.container, menu));
            if let Some(changed) = changed {
                self.reply_predicted_menu_click(conn, p, changed);
            } else {
                self.reply_menu_click(conn, ClickReply::Resend(p.container));
            }
        }
        Some(applied)
    }

    /// `ServerGamePacketListenerImpl.handleContainerSlotStateChanged`: a
    /// guest's crafter slot toggle (`CrafterBlockEntity.setSlotState`) in
    /// their open crafter window.
    pub(super) fn lan_crafter_slot_state(
        &mut self,
        conn: u32,
        window: i32,
        slot: i32,
        enabled: bool,
    ) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(pos) = host
            .hub
            .guest(conn)
            .filter(|g| !g.spectator())
            .and_then(|g| g.menu.as_ref())
            .filter(|m| window != 0 && m.window == window)
            .and_then(|m| match m.slots {
                Slots::Block(pos) => Some(pos),
                _ => None,
            })
        else {
            return;
        };
        if let (Some(BlockEntity::Crafter(c)), Ok(slot)) = (
            self.world.block_entities.get_mut(&pos),
            usize::try_from(slot),
        ) {
            c.set_slot_state(slot, enabled);
            self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
        }
    }

    /// ServerGamePacketListenerImpl.handlePlaceRecipe: the guest's recipe
    /// book places a known recipe in their inventory grid, crafting table or furnace
    /// (`ServerPlaceRecipe`), and answers with a ghost recipe when the items
    /// are missing.
    pub(super) fn lan_place_recipe(
        &mut self,
        conn: u32,
        p: &sunrise_protocol::packets::recipes::PlaceRecipe,
    ) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &host.hub.guests[gi];
        if g.spectator() {
            return;
        }
        // The server menu uses the world's active data-pack tables, the same
        // tables used by the guest menu below. The base resources can have a
        // different display registry or placement for an overridden recipe.
        let Some(id) = world_stations!(self)
            .recipe_index
            .recipe_of_display(p.recipe)
            .map(str::to_owned)
        else {
            return;
        };
        // An unknown or impossible recipe is refused outright.
        if !g.recipe_book.knows(&id)
            || world_recipes!(self)
                .placement(&id)
                .is_none_or(|placement| placement.impossible())
        {
            return;
        }
        let (player, name, creative) = (g.uuid, g.name.clone(), g.creative());
        self.native_player_recipe_book_click(player, &name, &id, p.use_max);
        // A mod menu type's crafting grid places as a crafting table's.
        if self.lan_mod_crafting_window(gi, p.container).is_some() {
            let placed = self.lan_mod_craft_apply(gi, p.container, |menu, env| {
                menu.place_recipe(env, &id, p.use_max, creative, None)
            });
            if placed == Some(PostPlace::Ghost) {
                self.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .send_ghost_recipe(conn, p.container, &id);
            }
            return;
        }
        // AbstractFurnaceMenu.handlePlacement: the current furnace's slots,
        // rather than the separate crafting-table menu, own cooking inputs.
        let furnace = self.lan.as_ref().unwrap().hub.guests[gi]
            .menu
            .as_ref()
            .filter(|m| p.container != 0 && m.window == p.container)
            .and_then(|m| match m.slots {
                Slots::Block(pos) => Some(pos),
                _ => None,
            })
            .and_then(|pos| match self.world.block_entities.get(&pos) {
                Some(BlockEntity::Furnace(f)) => {
                    let recipe = match f.kind.recipe_type() {
                        "minecraft:blasting" => "blasting",
                        "minecraft:smoking" => "smoking",
                        _ => "smelting",
                    };
                    Some((pos, Kind::Furnace(recipe), f.items.clone()))
                }
                _ => None,
            });
        let placed = if let Some((pos, kind, items)) = furnace {
            // Opening need not have created the click/drag cache yet.
            let mut menu = self
                .lan
                .as_mut()
                .unwrap()
                .menus
                .entry(conn)
                .or_default()
                .crafting
                .machine
                .take()
                .filter(|(window, m)| *window == p.container && m.kind() == kind)
                .map_or_else(|| Box::new(Menu::new(kind)), |(_, m)| m);
            let placed = self.guest_menu_op(gi, &mut menu, |menu, env| {
                for (k, item) in items.iter().enumerate() {
                    menu.set_slot(env, k, item.clone());
                }
                menu.place_recipe(env, &id, p.use_max, creative, None)
            });
            let changed = (0..3).any(|k| menu.slot(k) != items[k].as_ref());
            if let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get_mut(&pos) {
                for k in 0..3 {
                    f.items[k] = menu.slot(k).cloned();
                }
                f.input_clicked(&self.loop_context, world_recipes!(self), items[0].clone());
            }
            if changed {
                self.container_changed(pos);
            }
            *self
                .lan
                .as_mut()
                .unwrap()
                .menus
                .entry(conn)
                .or_default()
                .crafting
                .machine = Some((p.container, menu));
            // Hub::sync_containers broadcasts the actual changed slots.
            placed
        } else {
            self.lan_apply_recipe(gi, p.container, |menu, env| {
                menu.place_recipe(env, &id, p.use_max, creative, None)
            })
        };
        if placed == Some(PostPlace::Ghost)
            && let Some(host) = self.lan.as_mut()
        {
            host.hub.send_ghost_recipe(conn, p.container, &id);
        }
    }

    /// `menu_transfer` for the guest's open mod menu in window `container`: the part of its
    /// type moves `recipe`'s ingredients itself (or, with `dry_run`, only says what that would
    /// do). None when no part answers, the window isn't the guest's mod menu, or no mod listens.
    fn guest_menu_transfer(
        &mut self,
        gi: usize,
        container: i32,
        recipe: &str,
        max: bool,
        dry_run: bool,
    ) -> Option<mod_api::RecipeTransferResult> {
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuTransfer) {
            return None;
        }
        let g = &self.lan.as_ref()?.hub.guests[gi];
        let menu = g
            .menu
            .as_ref()
            .filter(|menu| menu.window == container && menu.mod_type.is_some())?;
        if !mod_api::__private::menu_listens(mod_api::HookId::MenuTransfer, &menu.registry_id) {
            return None;
        }
        let (player, menu_id) = (g.uuid, menu.registry_id.to_string());
        let pos = menu.pos.map(|(x, y, z)| [x, y, z]);
        let mut result = None;
        self.native_menu_transfer(mod_api::__private::MenuTransferCall {
            player,
            menu: &menu_id,
            pos,
            recipe,
            max,
            dry_run,
            result: &mut result,
        });
        result
    }

    pub(super) fn guest_recipe_transfer(&mut self, conn: u32, data: &[u8]) {
        use sunrise_core::vanilla_bytes::Writer;
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &host.hub.guests[gi];
        if g.brand != "sunrise" || g.spectator() {
            return;
        }
        let Ok((container, id, all)) = sunrise_client::lan::recipes::decode_transfer_request(data)
        else {
            return;
        };
        let blocked = container == 0 && (g.menu.is_some() || g.creative());
        // A mod menu's part moves the ingredients itself; without one the grid transfer runs.
        let answered = self.guest_menu_transfer(gi, container, &id, all, false);
        let result = if let Some(answer) = answered {
            recipes::transfer_outcome(answer)
        } else if blocked {
            Err(TransferError::Unavailable)
        } else {
            self.lan_apply_recipe(gi, container, |menu, env| {
                menu.transfer_recipe(env, &id, all)
            })
            .unwrap_or(Err(TransferError::Unavailable))
        };
        let mut reply = Writer::new();
        reply.varint(container);
        reply.u8(recipes::result_byte(recipes::transfer_result(result)));
        if let Some(host) = self.lan.as_ref() {
            host.hub.guests[gi].send(&C::CustomPayload(
                sunrise_protocol::packets::configuration::CustomPayload {
                    channel: recipes::TRANSFER_CHANNEL.into(),
                    data: reply.data,
                },
            ));
        }
    }

    /// A viewer's check of a transfer (`sunrise:recipe_check`): the matching
    /// and placement `guest_recipe_transfer` makes, the mods' recipe sources
    /// counted, on a copy of the guest's menu (`lan_check_recipe`). Nothing is
    /// written back: no slot, block entity or `container_changed`, and no
    /// packet but the reply. Each guest can burst ten checks, then gains one
    /// check per server tick (twenty per second at 20 TPS); more go unanswered.
    pub(super) fn guest_recipe_check(&mut self, conn: u32, data: &[u8]) {
        use sunrise_core::vanilla_bytes::Writer;
        use sunrise_protocol::buf::Reader;
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &host.hub.guests[gi];
        if g.brand != "sunrise" || g.spectator() {
            return;
        }
        let unavailable = g.menu.is_some() || g.creative();
        let mut reader = Reader::new(data);
        let (Ok(container), Ok(tag), Ok(id)) =
            (reader.varint(), reader.varint(), reader.string(512))
        else {
            return;
        };
        if reader.remaining() != 0 {
            return;
        }
        let play = host.play.entry(conn).or_default();
        if !play.recipe_check_available() {
            return;
        }
        play.recipe_checks += RECIPE_CHECK_COST;
        let answered = self.guest_menu_transfer(gi, container, &id, false, true);
        let check = if let Some(answer) = answered {
            Some(TransferCheck {
                result: recipes::transfer_outcome(answer),
                missing: Vec::new(),
            })
        } else if container == 0 && unavailable {
            None
        } else {
            self.lan_check_recipe(gi, container, &id)
        }
        .unwrap_or(TransferCheck {
            result: Err(TransferError::Unavailable),
            missing: Vec::new(),
        });
        let mut reply = Writer::new();
        reply.varint(container);
        reply.varint(tag);
        reply.u8(recipes::result_byte(recipes::transfer_result(check.result)));
        if check.result == Err(TransferError::Missing) {
            let cells: Vec<u8> = (check.missing.iter())
                .filter_map(|&c| u8::try_from(c).ok())
                .collect();
            reply.varint(cells.len() as i32);
            for c in cells {
                reply.u8(c);
            }
        }
        if let Some(host) = self.lan.as_ref() {
            host.hub.guests[gi].send(&C::CustomPayload(
                sunrise_protocol::packets::configuration::CustomPayload {
                    channel: recipes::CHECK_CHANNEL.into(),
                    data: reply.data,
                },
            ));
        }
    }

    /// `Menu::check_transfer` of recipe `id` on a copy of guest `gi`'s window
    /// `container`, built as `lan_apply_recipe` builds it: the inventory's
    /// grid, a crafting table's, or a mod crafting menu's over its block
    /// entity with its recipe sources. None when that window is none of them
    /// (the window must be the one the guest has open, as for a transfer).
    fn lan_check_recipe(&mut self, gi: usize, container: i32, id: &str) -> Option<TransferCheck> {
        let conn = self.lan.as_ref()?.hub.guests[gi].conn;
        let menu = if let Some((pos, t)) = self.lan_mod_crafting_window(gi, container) {
            let Some(BlockEntity::Mod(m)) = self.world.block_entities.get(&pos) else {
                return None;
            };
            let mut items = m.items.clone();
            items.resize(t.slots, None);
            let sources = self.lan_recipe_sources(gi, t, pos);
            let host = self.lan.as_ref()?;
            let shown = Self::lan_mod_craft_shown(host, gi, &items);
            let limits = (host.hub.guests[gi].menu.as_ref())
                .map(|opened| opened.slots.stack_limits(&self.world, opened.size))
                .unwrap_or_default();
            let env = self.menu_env();
            let mut menu = Menu::new(Kind::Mod(t));
            menu.set_container_limits(limits);
            menu.sources = sources.into_iter().map(|(_, items)| items).collect();
            load_player(&mut menu, &host.hub.guests[gi]);
            menu.load_container(&env, &items, shown);
            menu
        } else if container == 0 {
            let host = self.lan.as_ref()?;
            let env = self.menu_env();
            let mut menu = Menu::new(Kind::Inventory);
            load_player(&mut menu, &host.hub.guests[gi]);
            for (k, s) in host.hub.guests[gi].craft_grid.iter().enumerate() {
                menu.set_slot(&env, 1 + k, s.clone());
            }
            menu
        } else {
            let host = self.lan.as_ref()?;
            let (_, _, table, _) = (host.menus.get(&conn)?.crafting.table.as_ref())
                .filter(|(window, ..)| *window == container)?;
            let mut menu = Menu::clone(table);
            load_player(&mut menu, &host.hub.guests[gi]);
            menu
        };
        let host = self.lan.as_ref()?;
        let env = Env {
            player: Some(host.hub.guests[gi].uuid),
            player_entity_id: Some(host.hub.guests[gi].entity_id),
            loop_context: &self.loop_context,
            limited_crafting: host.hub.limited_crafting,
            can_equip: None,
            known_recipes: Some(&host.hub.guests[gi].recipe_book),
            ..self.menu_env()
        };
        Some(menu.check_transfer(&env, id))
    }

    fn lan_apply_recipe<R>(
        &mut self,
        gi: usize,
        container: i32,
        apply: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        if self.lan_mod_crafting_window(gi, container).is_some() {
            return self.lan_mod_craft_apply(gi, container, apply);
        }
        let conn = self.lan.as_ref()?.hub.guests[gi].conn;
        let host = self.lan.as_mut()?;
        let placed = if container == 0 {
            let crafting = host.menus.entry(conn).or_default();
            let mut menu = crafting
                .crafting
                .inventory
                .take()
                .unwrap_or_else(|| Box::new(Menu::new(Kind::Inventory)));
            let grid = host.hub.guests[gi].craft_grid.clone();
            let placed = self.guest_menu_op(gi, &mut menu, |menu, env| {
                for (k, s) in grid.into_iter().enumerate() {
                    menu.set_slot(env, 1 + k, s);
                }
                apply(menu, env)
            })?;
            let host = self.lan.as_mut()?;
            host.hub.guests.update(gi, |g| {
                for k in 0..4 {
                    g.craft_grid[k] = menu.slot(1 + k).cloned();
                }
                g.craft_result = menu.slot(0).cloned();
            });
            *host.menus.entry(conn).or_default().crafting.inventory = Some(menu);
            host.hub.send_inventory(gi);
            placed
        } else {
            let (window, state, mut menu, pos) = host
                .menus
                .get_mut(&conn)
                .and_then(|play| play.crafting.table.take_if(|(w, ..)| *w == container))?;
            let placed = self.guest_menu_op(gi, &mut menu, apply)?;
            let host = self.lan.as_mut()?;
            *host.menus.entry(conn).or_default().crafting.table = Some((window, state, menu, pos));
            self.lan_send_table_content(conn);
            placed
        };
        Some(placed)
    }

    /// The guest opened a crafting table (`CraftingTableBlock.getMenuProvider`).
    pub(super) fn lan_open_crafting(&mut self, conn: u32, pos: [i32; 3]) {
        if self.guest_may_open(conn, "minecraft:crafting", Some(pos)) != Some(true) {
            return;
        }
        // ServerPlayer.openMenu closes the menu that was open.
        self.guest_close_all(conn);
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let title = sunrise_protocol::text::translatable("container.crafting", &[]);
        let Some(window) = host.hub.open_window(conn, "minecraft:crafting", title) else {
            return;
        };
        let mut menu = Box::new(Menu::new(Kind::Crafting));
        menu.set_registry_id(Some("minecraft:crafting"));
        *host.menus.entry(conn).or_default().crafting.table = Some((window, 0, menu, pos));
        self.lan_send_table_content(conn);
        self.player_menu_opening(conn);
    }

    /// A click on the guest's crafting table window; false for other windows.
    pub(super) fn lan_crafting_click(&mut self, conn: u32, p: &play::ContainerClick) -> bool {
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        let Some(play) = host.menus.get_mut(&conn) else {
            return false;
        };
        let Some((window, mut state, mut menu, pos)) = play
            .crafting
            .table
            .take_if(|(window, ..)| *window == p.container)
        else {
            return false;
        };
        let owner = play.crafting.table.stamp();
        // A full snapshot on the current state would undo clicks the client has since predicted.
        let mut full = true;
        // A table that is no longer valid takes no clicks (handleContainerClick).
        let gi = host.hub.guests.iter().position(|g| g.conn == conn);
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        if let Some(gi) = gi
            && !host.hub.guests[gi].spectator()
            && self.lan_table_valid(&host.hub.guests[gi], pos)
        {
            let click = super::lan_pickup::click_of(p);
            let Some(mut changed) =
                self.guest_menu_op_decided(gi, &mut menu, Some(click), None, |menu, env| {
                    let before: Vec<_> = (0..menu.slot_count())
                        .map(|s| menu.slot(s).cloned())
                        .collect();
                    let _ = menu.click(env, click.0, click.1, click.2);
                    (0..menu.slot_count())
                        .filter(|&s| menu.slot(s) != before[s].as_ref())
                        .collect::<Vec<_>>()
                })
            else {
                // A refused click keeps its menu open. Don't put it back if the
                // listener closed or replaced the owner while the table was lifted.
                if let Some(play) = self.lan.as_mut().and_then(|host| host.menus.get_mut(&conn))
                    && play.crafting.table.stamp() == owner
                {
                    play.crafting
                        .table
                        .restore(Some((window, state, menu, pos)));
                    self.lan_send_table_content(conn);
                }
                return true;
            };
            let Some(guest) = self.lan.as_ref().map(|host| &host.hub.guests[gi]) else {
                return true;
            };
            let (uuid, name) = (guest.uuid, guest.name.clone());
            let mut result = menu.slot(0).cloned();
            let vanilla = result.clone();
            self.native_prepare_item_craft(uuid, &name, Some(pos), &mut result);
            if result != vanilla && !changed.contains(&0) {
                changed.push(0);
            }
            let env = self.menu_env();
            menu.set_slot(&env, 0, result);
            let Some(host) = self.lan.as_mut() else {
                return true;
            };
            full = p.state != state;
            if !full {
                let slots: Vec<_> = (0..menu.slot_count())
                    .map(|s| menu.slot(s).cloned())
                    .collect();
                let corrected = host
                    .hub
                    .sync_click_prediction(gi, p, &mut state, &slots, |slot| {
                        changed.contains(&slot)
                    });
                // The inventory menu's broadcast shouldn't repeat what the click settled.
                host.hub.guests.update(gi, |g| {
                    let settled = p
                        .changed
                        .iter()
                        .map(|(s, _)| *s as usize)
                        .chain(changed)
                        .chain(corrected);
                    for s in settled.filter(|s| (10..slots.len()).contains(s)) {
                        g.menu_sent[s - 1] = slots[s].clone();
                    }
                });
            }
        }
        let Some(host) = self.lan.as_mut() else {
            return true;
        };
        *host.menus.entry(conn).or_default().crafting.table = Some((window, state, menu, pos));
        if full {
            self.lan_send_table_content(conn);
        }
        true
    }

    /// `CraftingMenu.stillValid`: the block is still a crafting table and the
    /// guest is within reach of it.
    pub(super) fn lan_table_valid(&self, g: &Guest, pos: [i32; 3]) -> bool {
        let name = &self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name;
        &**name == "minecraft:crafting_table" && g.within_block_range(pos, 4.0)
    }

    /// Guest `gi`'s open window `container` when it is a mod menu type with a
    /// crafting grid: its block and its type.
    fn lan_mod_crafting_window(
        &self,
        gi: usize,
        container: i32,
    ) -> Option<((i32, i32, i32), &'static ModMenuType)> {
        let host = self.lan.as_ref()?;
        let m = host.hub.guests[gi]
            .menu
            .as_ref()
            .filter(|m| container != 0 && m.window == container)?;
        let Slots::Block(pos) = m.slots else {
            return None;
        };
        let t = m.mod_type.filter(|t| t.crafting.is_some())?;
        matches!(
            self.world.block_entities.get(&pos),
            Some(BlockEntity::Mod(_))
        )
        .then_some((pos, t))
    }

    /// `apply` (a recipe book placement or a viewer's transfer) on guest
    /// `gi`'s open mod crafting menu `container`, over its block entity's
    /// slots as a click is; None when that window is no such menu. Changes
    /// reach the guest with the tick's container sync.
    fn lan_mod_craft_apply<R>(
        &mut self,
        gi: usize,
        container: i32,
        apply: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        let (pos, t) = self.lan_mod_crafting_window(gi, container)?;
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get(&pos) else {
            return None;
        };
        let mut items = m.items.clone();
        items.resize(t.slots, None);
        let host = self.lan.as_mut()?;
        let conn = host.hub.guests[gi].conn;
        let kind = Kind::Mod(t);
        let mut menu = host
            .menus
            .entry(conn)
            .or_default()
            .crafting
            .machine
            .take()
            .filter(|(window, m)| *window == container && m.kind() == kind)
            .map_or_else(|| Box::new(Menu::new(kind)), |(_, m)| m);
        let shown = Self::lan_mod_craft_shown(self.lan.as_ref()?, gi, &items);
        let sources = self.lan_recipe_sources(gi, t, pos);
        menu.sources = sources.iter().map(|(_, items)| items.clone()).collect();
        let out = self.guest_menu_op(gi, &mut menu, |menu, env| {
            menu.load_container(env, &items, shown);
            apply(menu, env)
        })?;
        for ((at, before), after) in sources.into_iter().zip(std::mem::take(&mut menu.sources)) {
            if after == before {
                continue;
            }
            match self.world.block_entities.get_mut(&at) {
                Some(BlockEntity::Chest(items)) => *items = after,
                Some(BlockEntity::Mod(m)) => {
                    for (slot, stack) in m.items.iter_mut().zip(after) {
                        *slot = stack;
                    }
                }
                _ => {}
            }
            self.container_changed(at);
        }
        let after = menu.container_items();
        if after != items {
            if let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&pos) {
                if m.items.len() < after.len() {
                    m.items.resize(after.len(), None);
                }
                for (slot, stack) in m.items.iter_mut().zip(after) {
                    *slot = stack;
                }
            }
            self.container_changed(pos);
        }
        self.lan_mod_craft_settle(gi, pos, &mut menu);
        *self
            .lan
            .as_mut()?
            .menus
            .entry(conn)
            .or_default()
            .crafting
            .machine = Some((container, menu));
        Some(out)
    }

    /// The containers recipe placement into guest `gi`'s mod crafting menu
    /// `t` at `pos` may also take from (`native_recipe_sources`): each block
    /// and its slots, chest-like or a mod's container, none twice, not `pos`.
    pub(super) fn lan_recipe_sources(
        &mut self,
        gi: usize,
        t: &ModMenuType,
        pos: (i32, i32, i32),
    ) -> Vec<((i32, i32, i32), Vec<Option<ItemStack>>)> {
        let mut asked = Vec::new();
        let Some(uuid) = self.lan.as_ref().map(|host| host.hub.guests[gi].uuid) else {
            return Vec::new();
        };
        self.native_recipe_sources(uuid, t.id, [pos.0, pos.1, pos.2], &mut asked);
        let mut sources: Vec<((i32, i32, i32), Vec<Option<ItemStack>>)> = Vec::new();
        for at in asked {
            let at = (at[0], at[1], at[2]);
            if at == pos || sources.len() >= MAX_RECIPE_SOURCES || sources.iter().any(|s| s.0 == at)
            {
                continue;
            }
            let Some(items) = self.linkable_container(at) else {
                continue;
            };
            sources.push((at, items));
        }
        sources
    }

    /// The slots of the block container at `at` that a mod may lend a menu
    /// (`native_recipe_sources`, `native_menu_links`): a chest-like block
    /// entity's (chests, barrels, shulker boxes, hoppers, dispensers,
    /// droppers) or a mod container's. None for anything else.
    fn linkable_container(&self, at: (i32, i32, i32)) -> Option<Vec<Option<ItemStack>>> {
        let state = self.world.block(at.0, at.1, at.2).0;
        match self.world.block_entities.get(&at)? {
            BlockEntity::Chest(items) => Some(items.clone()),
            BlockEntity::Mod(m) => {
                let c = self.world.block_entities.mod_container(state)?;
                Some(m.items.iter().take(c.slots).cloned().collect())
            }
            _ => None,
        }
    }

    /// The guests' open mod menus with linked slots (`links`): the mods name
    /// the containers behind them again (`native_menu_links`), before the
    /// hub sends the tick's slot changes, so a tab the player picked or a
    /// chest placed or broken shows in the same tick.
    pub(super) fn lan_menu_links_tick(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::MenuLinks) {
            return;
        }
        let Some(count) = self.lan.as_ref().map(|host| host.hub.guests.len()) else {
            return;
        };
        for gi in 0..count {
            let host = self.lan.as_ref().unwrap();
            let g = &host.hub.guests[gi];
            let Some((t, pos)) = g
                .menu
                .as_ref()
                .filter(|_| host.hub.here(g) && !self.player_tick_canceled(g.conn))
                .and_then(|m| match (m.mod_type, m.slots) {
                    (Some(t), Slots::Block(pos)) if t.links.is_some() => Some((t, pos)),
                    _ => None,
                })
            else {
                continue;
            };
            let mut asked = Vec::new();
            self.native_menu_links(g.uuid, t.id, [pos.0, pos.1, pos.2], &mut asked);
            let mut links: Vec<(i32, i32, i32)> = Vec::new();
            for at in asked {
                let at = (at[0], at[1], at[2]);
                if at == pos || links.len() >= MAX_MENU_LINKS || links.contains(&at) {
                    continue;
                }
                if self.linkable_container(at).is_some() {
                    links.push(at);
                }
            }
            self.lan.as_mut().unwrap().hub.guests.update(gi, |g| {
                if let Some(m) = g.menu.as_mut() {
                    m.links = links;
                }
            });
        }
    }

    /// The result guest `gi`'s mod crafting menu shows (`Some(None)` if empty),
    /// while the block entity's grid (`items`) is the one it was made from.
    fn lan_mod_craft_shown(
        host: &super::lan::LanHost,
        gi: usize,
        items: &[Option<ItemStack>],
    ) -> Option<Option<ItemStack>> {
        let menu = host.hub.guests[gi].menu.as_ref()?;
        let c = menu.mod_type?.crafting?;
        let craft = menu.craft.as_ref()?;
        (items.get(c.grid_slots()) == Some(&craft.grid[..])).then(|| craft.result.clone())
    }

    /// After the menu engine ran on guest `gi`'s mod crafting menu at `pos`:
    /// the mods have their say on the result (`native_prepare_item_craft`,
    /// as on a crafting table's), which goes into `menu` and is what the guest
    /// is shown from now on. The result's slot when the mods changed it.
    fn lan_mod_craft_settle(
        &mut self,
        gi: usize,
        pos: (i32, i32, i32),
        menu: &mut Menu,
    ) -> Option<usize> {
        let c = menu.mod_crafting()?;
        let guest = &self.lan.as_ref()?.hub.guests[gi];
        let open = guest.menu.as_ref()?;
        let grid = c.grid_slots().map(|k| menu.slot(k));
        let shown = menu.slot(c.result);
        let cached = open.craft.as_ref();
        let same_grid =
            cached.is_some_and(|cached| cached.grid.iter().map(Option::as_ref).eq(grid));
        if same_grid && cached.and_then(|cached| cached.result.as_ref()) == shown {
            return None;
        }
        let mut result = shown.cloned();
        let window = open.window;
        // Taking from the shown result is not a grid update. Do not build a new
        // preview over what the click left unless the inputs changed.
        if !same_grid {
            let (uuid, name) = (guest.uuid, guest.name.clone());
            self.native_prepare_item_craft(uuid, &name, Some([pos.0, pos.1, pos.2]), &mut result);
        }
        let changed = result.as_ref() != shown;
        if changed {
            menu.load_slot(c.result, result.clone());
        }
        let grid = c.grid_slots().map(|k| menu.slot(k).cloned()).collect();
        self.lan_mod_craft_store(gi, window, ModCraft { grid, result });
        changed.then_some(c.result)
    }

    /// Publish the result and the grid it belongs to as one value, before the hub syncs it.
    pub(super) fn lan_mod_craft_store(&mut self, gi: usize, window: i32, craft: ModCraft) {
        if let Some(host) = self.lan.as_ref() {
            self.journal_menu_click(host.hub.guests[gi].conn);
        }
        if let Some(host) = self.lan.as_mut() {
            host.hub.guests.update(gi, |guest| {
                if let Some(open) = guest.menu.as_mut().filter(|open| open.window == window) {
                    open.craft = Some(craft);
                }
            });
        }
    }

    /// `CraftingMenu.slotsChanged` for the guests' mod crafting menus: the
    /// result each is shown follows its block entity's grid, which another
    /// player, a hopper or the mod may have changed, made again only when
    /// the grid changed.
    pub(super) fn lan_mod_crafting_tick(&mut self) {
        let Some(count) = self.lan.as_ref().map(|host| host.hub.guests.len()) else {
            return;
        };
        for gi in 0..count {
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            let g = &host.hub.guests[gi];
            let Some((t, pos)) = g
                .menu
                .as_ref()
                .filter(|_| host.hub.here(g) && !self.player_tick_canceled(g.conn))
                .and_then(|m| match (m.mod_type, m.slots) {
                    (Some(t), Slots::Block(pos)) if t.crafting.is_some() => Some((t, pos)),
                    _ => None,
                })
            else {
                continue;
            };
            let Some(BlockEntity::Mod(m)) = self.world.block_entities.get(&pos) else {
                continue;
            };
            let mut items = m.items.clone();
            items.resize(t.slots, None);
            if Self::lan_mod_craft_shown(host, gi, &items).is_some() {
                continue;
            }
            let mut menu = Menu::new(Kind::Mod(t));
            self.guest_menu_op(gi, &mut menu, |menu, env| {
                menu.load_container(env, &items, None);
            });
            self.lan_mod_craft_settle(gi, pos, &mut menu);
        }
    }

    /// `ServerPlayer.tick`'s `stillValid`: a guest's crafting table closes
    /// once it is gone or out of reach, giving its grid back.
    pub(super) fn lan_tables_tick(&mut self) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let invalid: Vec<(u32, i32)> = host
            .menus
            .iter()
            .filter_map(|(c, p)| p.crafting.table.as_ref().map(|t| (*c, t.0, t.3)))
            .filter(|&(c, _, pos)| {
                host.hub
                    .guests
                    .iter()
                    .find(|g| g.conn == c)
                    .is_some_and(|g| {
                        host.hub.here(g)
                            && !self.player_tick_canceled(c)
                            && !self.lan_table_valid(g, pos)
                    })
            })
            .map(|(c, window, _)| (c, window))
            .collect();
        for (conn, window) in invalid {
            self.lan_crafting_closed(conn, Some(window));
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            host.hub.send_to(
                conn,
                &C::ContainerClose(play::ContainerClose { container: window }),
            );
        }
    }

    /// The whole crafting table menu for the client (`sendAllDataToRemote`).
    fn lan_send_table_content(&mut self, conn: u32) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let Some((window, state, menu, _)) = host
            .menus
            .get_mut(&conn)
            .and_then(|p| p.crafting.table.as_mut())
        else {
            return;
        };
        *state = (*state + 1) & 0x7FFF;
        let (window, state) = (*window, *state);
        let mut items: Vec<Option<ItemStack>> = (0..10).map(|s| menu.slot(s).cloned()).collect();
        let g = &host.hub.guests[gi];
        items.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
        host.hub.send_menu_snapshot(gi, window, state, &items);
        host.hub.guests.update(gi, |g| g.inventory_told());
    }

    /// `AbstractContainerMenu.removed` of a crafting menu: the closed window
    /// (`window`; None closes the table whichever it is, and 0 the inventory
    /// menu) gives its grid back to the inventory, what doesn't fit is
    /// dropped. False when no such window is open.
    /// The open crafting table's own slots (result and grid) before the
    /// player's inventory, if the guest has one open.
    pub(super) fn lan_crafting_table_own_slots(&self, conn: u32) -> Option<usize> {
        let (_, _, menu, _) = self
            .lan
            .as_ref()?
            .menus
            .get(&conn)?
            .crafting
            .table
            .as_ref()?;
        Some(menu.slot_count() - 36)
    }

    pub(super) fn lan_crafting_closed(&mut self, conn: u32, window: Option<i32>) -> bool {
        let closed = self.lan_crafting_removed(conn, window, MenuRemoval::Live);
        if closed && window != Some(0) {
            self.player_menu_closing(conn);
        }
        closed
    }

    /// Retire a game-owned menu on the guest record, even when their active
    /// cursor is temporarily in `Game` during a guest turn.
    pub(super) fn close_guest_menu_cursor(&mut self, gi: usize, window: i32) -> Option<ItemStack> {
        self.with_own_record(gi, |host, _| {
            host.hub.guests.update(gi, |g| g.close_menu_cursor(window))
        })
    }

    /// Return the cursor and inputs, or drop them if the player died or disconnected.
    pub(super) fn lan_crafting_removed(
        &mut self,
        conn: u32,
        window: Option<i32>,
        removal: MenuRemoval,
    ) -> bool {
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return false;
        };
        let crafting = &mut host.menus.entry(conn).or_default().crafting;
        let grid: Vec<ItemStack> = if window == Some(0) {
            *crafting.inventory = None;
            self.with_own_record(gi, |host, _| {
                Some(host.hub.guests.update(gi, |g| {
                    g.craft_result = None;
                    g.take_inventory_carried()
                        .into_iter()
                        .chain(g.craft_grid.iter_mut().filter_map(Option::take))
                        .collect()
                }))
            })
            .unwrap_or_default()
        } else {
            match crafting
                .table
                .take_if(|(w, ..)| window.is_none_or(|window| *w == window))
            {
                Some((window, _, menu, _)) => {
                    let carried = self.close_guest_menu_cursor(gi, window);
                    carried
                        .into_iter()
                        .chain((1..10).filter_map(|s| menu.slot(s).cloned()))
                        .collect()
                }
                None => return false,
            }
        };
        if !grid.is_empty() {
            if removal.drops_items() {
                self.drop_guest_items(gi, grid);
            } else {
                self.lan_place_back(gi, grid);
            }
        }
        true
    }

    /// `Inventory.placeItemBackInInventory` for the stacks a closing menu returned.
    /// Drop overflow after restoring the active turn, since spawning an item can call mods.
    pub(super) fn lan_place_back(&mut self, gi: usize, inputs: Vec<ItemStack>) {
        if inputs.is_empty() {
            return;
        }
        let overflow = self.with_own_record(gi, |lan, items| {
            Some(lan.hub.guests.update(gi, |guest| {
                let mut menu = Menu::new(Kind::Inventory);
                load_player(&mut menu, guest);
                place_back(&mut menu, items, inputs);
                store_player(&mut menu, guest);
                std::mem::take(&mut menu.dropped)
            }))
        });
        if let Some(overflow) = overflow.filter(|stacks| !stacks.is_empty()) {
            self.drop_guest_items(gi, overflow);
        }
    }

    /// A guest left: what their crafting grids and workstation held goes back into their
    /// inventory before it is saved (dropped where they stood if full).
    pub(super) fn lan_crafting_left(&mut self, g: &mut Guest) {
        // The guest's play state stays: their statistics are awarded and saved
        // after this (`HostEvent::Left`), which removes it.
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        // If the leaving level could not be opened for normal removal, put
        // its remaining cursors in the same fallback as the remaining inputs.
        let mut grid: Vec<_> = host.hub.take_closed_menu_cursors(g.conn).collect();
        grid.extend(g.take_menu_cursors());
        grid.extend(g.craft_grid.iter_mut().filter_map(Option::take));
        let play = host.menus.get_mut(&g.conn);
        g.craft_result = None;
        if let Some(play) = play {
            if let Some((_, _, menu, _)) = play.crafting.table.take() {
                grid.extend((1..10).filter_map(|s| menu.slot(s).cloned()));
            }
            // A workstation's inputs too.
            if let Some(station) = play.station.take() {
                grid.extend(super::lan_stations::station_inputs(&station));
            }
        }
        if grid.is_empty() {
            return;
        }
        let mut menu = Menu::new(Kind::Inventory);
        load_player(&mut menu, g);
        place_back(&mut menu, self.items(), grid);
        let dropped = std::mem::take(&mut menu.dropped);
        store_player(&mut menu, g);
        for stack in dropped {
            self.spawn_item(stack, g.position, [0.0, 0.1, 0.0], 40);
        }
    }
}

#[cfg(test)]
mod machine_recipe_placement_tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use sunrise_core::position::ChunkPos;
    use sunrise_protocol::packets::{
        configuration::ClientInformation,
        play::{Serverbound, UseItemOn},
        recipes::PlaceRecipe,
    };

    #[test]
    #[ignore = "daily"]
    fn h234_registered_place_recipe_reaches_actual_furnace_and_preserves_controls() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            assert!(game.world.loaded(8, 8));
            game.lan_host(
                crate::game::lan::LanSettings {
                    port: 0,
                    guest_commands: true,
                    force_game_mode: false,
                },
                NetConfig {
                    announce: false,
                    ..NetConfig::loopback(0, "machine-place-test".into(), 2)
                },
                HubConfig {
                    default_game_mode: 0,
                    guest_commands: true,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: "minecraft:overworld".into(),
                    allow_flight: false,
                    seed: 0,
                    max_players: 2,
                    integrated: false,
                    levels: Default::default(),
                    level_seeds: Default::default(),
                },
            )
            .unwrap();
            let (tx, _rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 2,
                    uuid: 234,
                    name: "RecipeGuest".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                2,
                [8.5, 200.0, 8.5],
                2,
            );
            guest.dimension = "minecraft:overworld".into();
            guest.placed = true;
            guest.loaded = true;
            guest.selected = 1;
            guest.permission = 2;
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            let clock = game.world.game_time;
            let planks = game
                .stations()
                .recipe_index
                .first_display("minecraft:oak_planks")
                .unwrap();
            let charcoal = game
                .stations()
                .recipe_index
                .first_display("minecraft:charcoal")
                .unwrap();
            let command = |game: &mut Game<'_>, text: &str| {
                assert!(game.as_guest(2, |g| g.run_command(text)).unwrap(), "{text}");
            };
            let place = |game: &mut Game<'_>, container, recipe| {
                game.lan_guest_packet(
                    2,
                    Serverbound::PlaceRecipe(PlaceRecipe {
                        container,
                        recipe,
                        use_max: false,
                    }),
                );
            };
            let open = |game: &mut Game<'_>, pos| {
                game.lan_guest_packet(
                    2,
                    Serverbound::UseItemOn(UseItemOn {
                        hand: 0,
                        pos,
                        face: 1,
                        cursor: [0.5, 1.0, 0.5],
                        inside: false,
                        world_border: false,
                        sequence: 234,
                    }),
                );
                assert!(
                    game.lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(2)
                        .unwrap()
                        .ack_up_to
                        .get()
                        >= 234
                );
                game.lan
                    .as_ref()
                    .unwrap()
                    .menus
                    .get(&2)
                    .and_then(|p| p.crafting.table.as_ref())
                    .map(|(window, ..)| *window)
                    .or_else(|| {
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(2)
                            .unwrap()
                            .menu
                            .as_ref()
                            .map(|m| m.window)
                    })
                    .expect("actual table or machine window opened")
            };
            command(game, "recipe give @s minecraft:oak_planks");
            command(game, "recipe give @s minecraft:charcoal");
            assert!(
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(2)
                    .unwrap()
                    .recipe_book
                    .knows("minecraft:charcoal")
            );
            command(game, "item replace entity @s hotbar.0 with oak_log 3");
            place(game, 0, planks);
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guest(2).unwrap().craft_grid[0],
                Some(ItemStack::new("minecraft:oak_log", 1))
            );
            assert_eq!(
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(2)
                    .unwrap()
                    .craft_result,
                Some(ItemStack::new("minecraft:oak_planks", 4))
            );
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guest(2).unwrap().inventory[0]
                    .as_ref()
                    .unwrap()
                    .count,
                2
            );
            game.lan_crafting_closed(2, Some(0));
            assert!(game.run_command("setblock 10 200 8 crafting_table"));
            let window = open(game, [10, 200, 8]);
            place(game, window, planks);
            let (_, _, table, _) = game.lan.as_ref().unwrap().menus[&2]
                .crafting
                .table
                .as_ref()
                .unwrap();
            assert_eq!(
                table.slot(0),
                Some(&ItemStack::new("minecraft:oak_planks", 4))
            );
            game.guest_close_all(2);
            let mut failures = Vec::new();
            for (label, count, known) in [
                ("place", 3, true),
                ("ghost", 0, true),
                ("forgotten", 3, false),
            ] {
                game.as_guest(2, |g| {
                    g.run_command("clear @s");
                })
                .unwrap();
                if count > 0 {
                    command(game, "item replace entity @s hotbar.0 with oak_log 3");
                }
                command(
                    game,
                    "item replace entity @s hotbar.8 with stick[minecraft:custom_data={h234:1}] 7",
                );
                assert!(game.run_command("setblock 10 200 8 air"));
                assert!(game.run_command("setblock 10 200 8 furnace"));
                assert!(game.run_command("item replace block 10 200 8 container.1 with coal[minecraft:custom_data={h234:2}] 2"));
                let window = open(game, [10, 200, 8]);
                if !known {
                    command(game, "recipe take @s minecraft:charcoal");
                }
                assert_eq!(
                    game.lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(2)
                        .unwrap()
                        .recipe_book
                        .knows("minecraft:charcoal"),
                    known
                );
                let before = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(2)
                    .unwrap()
                    .inventory
                    .clone();
                let pos = (10, 200, 8);
                let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&pos) else {
                    panic!("actual furnace missing")
                };
                assert!(f.items[0].is_none() && f.items[2].is_none());
                let fuel = f.items[1].clone();
                assert_eq!(fuel.as_ref().unwrap().count, 2);
                place(game, window, charcoal);
                let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&pos) else {
                    panic!("furnace removed")
                };
                assert_eq!(f.items[1], fuel);
                assert!(f.items[2].is_none());
                let mut expected = before.clone();
                let input = if label == "place" {
                    expected[0].as_mut().unwrap().count = 2;
                    Some(ItemStack::new("minecraft:oak_log", 1))
                } else {
                    None
                };
                if f.items[0] != input
                    || game.lan.as_ref().unwrap().hub.guest(2).unwrap().inventory != expected
                {
                    failures.push((
                        label,
                        f.items.clone(),
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(2)
                            .unwrap()
                            .inventory
                            .clone(),
                    ));
                }
                assert_eq!(game.lan.as_ref().unwrap().hub.guest(2).unwrap().uuid, 234);
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guest(2).unwrap().health,
                    20.0
                );
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guest(2).unwrap().position,
                    [8.5, 200.0, 8.5]
                );
                assert_eq!(game.world.game_time, clock);
                game.guest_close_all(2);
            }
            assert!(
                failures.is_empty(),
                "actual registered PlaceRecipe furnace failures: {failures:?}"
            );
        });
    }
}

#[cfg(test)]
mod recipe_check_tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_client::{
        containers::{ModBlockEntity, ModBlockEntityFields, ModCrafting},
        lan::{
            Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig,
            menu::Menu as HubMenu,
        },
    };
    use sunrise_protocol::packets::configuration::ClientInformation;

    const STATION: (i32, i32, i32) = (4, 200, 4);
    const WINDOW: i32 = 7;

    /// What a check must leave alone: the station's block entity, the guest's
    /// inventory, grid and cursor, its menus' slots, and the station's chunk
    /// unmarked as changed (`container_changed` marks it; this takes the mark).
    fn state(game: &mut Game<'_>) -> String {
        let changed = game
            .world
            .modified
            .remove(&(STATION.0 >> 4, STATION.2 >> 4));
        let host = game.lan.as_ref().unwrap();
        let g = &host.hub.guests[0];
        format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {:?}",
            game.world.block_entities.get(&STATION),
            g.inventory,
            g.craft_grid,
            g.carried,
            g.menu.as_ref().map(|m| (&m.sent, m.state, &m.craft)),
            host.menus
                .get(&2)
                .is_some_and(|p| p.crafting.machine.is_some()),
            changed,
        )
    }

    /// The packets sent to the guest since the last call.
    fn sent(rx: &mpsc::Receiver<Out>) -> Vec<C> {
        let mut bodies = Vec::new();
        for out in rx.try_iter() {
            match out {
                Out::Packet(body) => bodies.push(body),
                Out::Packets(more) => bodies.extend(more),
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            }
        }
        bodies
            .iter()
            .map(|body| {
                let mut r = sunrise_protocol::buf::Reader::new(body);
                let id = r.varint().unwrap();
                C::decode(id, r.rest()).unwrap()
            })
            .collect()
    }

    /// A check's reply: the window, the tag, the result byte and the missing cells.
    fn reply(packets: &[C]) -> (i32, i32, u8, Vec<u8>) {
        let [C::CustomPayload(p)] = packets else {
            panic!("expected only the reply, got {packets:?}");
        };
        assert_eq!(p.channel, recipes::CHECK_CHANNEL);
        let mut r = sunrise_protocol::buf::Reader::new(&p.data);
        let (window, tag, result) = (r.varint().unwrap(), r.varint().unwrap(), r.u8().unwrap());
        let mut cells = Vec::new();
        if result == 1 {
            for _ in 0..r.varint().unwrap() {
                cells.push(r.u8().unwrap());
            }
        }
        assert_eq!(r.remaining(), 0);
        (window, tag, result, cells)
    }

    fn request(window: i32, tag: i32, id: &str) -> Vec<u8> {
        let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
        w.varint(window);
        w.varint(tag);
        w.string(id);
        w.data
    }

    /// A check on a guest's mod crafting station answers as the transfer
    /// would and changes nothing: no slot, no block entity, no chunk marked
    /// changed, no packet but its reply. The transfer after it does move.
    #[test]
    #[ignore = "daily"]
    fn a_check_answers_and_leaves_the_world_and_the_guest_alone() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.lan_host(
                crate::game::lan::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                NetConfig {
                    announce: false,
                    ..NetConfig::loopback(0, "recipe-check-test".into(), 2)
                },
                HubConfig {
                    default_game_mode: 0,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: "minecraft:overworld".into(),
                    allow_flight: false,
                    seed: 0,
                    max_players: 2,
                    integrated: false,
                    levels: Default::default(),
                    level_seeds: Default::default(),
                },
            )
            .unwrap();
            let station: &'static ModMenuType = Box::leak(Box::new(ModMenuType {
                id: "test:station",
                owner: "test",
                slots: 10,
                player_inventory: true,
                data_slots: 0,
                owned_slots: None,
                results: vec![],
                inputs: Vec::new(),
                links: None,
                crafting: Some(ModCrafting {
                    grid: 1,
                    size: 3,
                    result: 0,
                }),
            }));
            let mut items = vec![None; 10];
            items[5] = Some(ItemStack::new("minecraft:cobblestone", 1));
            game.world.block_entities.insert(
                STATION,
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "test:station".into(),
                    ModBlockEntityFields {
                        items,
                        ..Default::default()
                    },
                ))),
            );
            let (tx, rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 2,
                    uuid: 2640,
                    name: "CheckGuest".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "sunrise".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                2,
                [4.5, 201.0, 4.5],
                2,
            );
            guest.placed = true;
            guest.loaded = true;
            guest.inventory[0] = Some(ItemStack::new("minecraft:oak_planks", 2));
            guest.menu = Some(HubMenu {
                window: WINDOW,
                kind: 0,
                registry_id: "test:station".into(),
                pos: Some(STATION),
                slots: Slots::Block(STATION),
                size: 10,
                state: 0,
                sent: vec![None; 46],
                drag: None,
                sent_data: Vec::new(),
                reported_unsent_data_len: None,
                blockless_data: Vec::new(),

                mod_type: Some(station),
                craft: None,
                links: Vec::new(),
                asks_still_valid: false,
                opening_item: None,
            });
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            sent(&rx);
            game.world.modified.clear();
            let before = state(game);
            let check = |game: &mut Game<'_>, window, tag, id| {
                game.guest_recipe_check(2, &request(window, tag, id));
                assert_eq!(state(game), before, "a check of {id} changed something");
                reply(&sent(&rx))
            };
            // Two planks make a stick; the grid's cobblestone has room to go.
            assert_eq!(
                check(game, WINDOW, 1, "minecraft:stick"),
                (WINDOW, 1, 0, vec![])
            );
            // A crafting table's last two planks lack.
            assert_eq!(
                check(game, WINDOW, 2, "minecraft:crafting_table"),
                (WINDOW, 2, 1, vec![2, 3])
            );
            // An unknown recipe, and a window the guest doesn't have open:
            // not into this menu.
            assert_eq!(
                check(game, WINDOW, 3, "minecraft:no_such_recipe"),
                (WINDOW, 3, 3, vec![])
            );
            assert_eq!(
                check(game, WINDOW + 1, 4, "minecraft:stick"),
                (WINDOW + 1, 4, 3, vec![])
            );
            // The transfer the first check answered for does move them.
            game.guest_recipe_transfer(2, &{
                let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
                w.varint(WINDOW);
                w.string("minecraft:stick");
                w.data
            });
            assert_ne!(state(game), before, "the transfer moved nothing");
        });
    }

    /// Ten checks at once are answered, the next is dropped, and one more
    /// goes through after one tick.
    #[test]
    #[ignore = "daily"]
    fn checks_past_the_limit_go_unanswered() {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let (tx, rx) = mpsc::channel();
            let host = game.lan.as_mut().unwrap();
            host.hub.guests.update(0, |g| {
                g.brand = "sunrise".into();
                g.sender = PlayerSender::new(tx, Arc::default());
            });
            let conn = host.hub.guests[0].conn;
            for tag in 0..11 {
                game.guest_recipe_check(conn, &request(0, tag, "minecraft:stick"));
            }
            let replies = sent(&rx);
            assert_eq!(replies.len(), 10);
            game.tick_guest_connections(false).unwrap();
            drop(sent(&rx));
            game.guest_recipe_check(conn, &request(0, 11, "minecraft:stick"));
            assert_eq!(reply(&sent(&rx)).1, 11);
        });
    }
}

#[cfg(test)]
#[path = "lan_crafting/lending_tests.rs"]
mod lending_tests;

#[cfg(test)]
#[path = "lan_crafting/stonecutter_stat_tests.rs"]
mod stonecutter_stat_tests;

#[cfg(test)]
mod empty_ingredient_placement_tests {
    struct Predicate;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Predicate {
        fn ingredient(
            &self,
            _: &dyn mod_api::DataLoadContext,
            event: &mut mod_api::IngredientEvent<'_>,
        ) -> Option<mod_api::IngredientDefinition> {
            let kind = event.kind;
            (kind == "empty_fixture:predicate")
                .then(|| mod_api::IngredientDefinition::new(vec![], |_, _| true))
        }
    }

    use super::*;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use sunrise_protocol::packets::{configuration::ClientInformation, recipes::PlaceRecipe};

    #[test]
    #[ignore = "daily"]
    fn server_packet_refuses_retained_empty_ingredient_recipes() {
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("empty_fixture", Box::new(Predicate))].into(),
            || {
                crate::game::native_entity_refs::tests::with_game(|game| {
                    let dir = std::env::temp_dir()
                        .join(format!("sunrise-empty-ingredients-{}", std::process::id()));
                    let pack = dir.join("datapacks/empty");
                    std::fs::create_dir_all(pack.join("data/empty_fixture/recipe")).unwrap();
                    std::fs::create_dir_all(pack.join("data/empty_fixture/tags/item")).unwrap();
                    std::fs::write(
                        pack.join("pack.mcmeta"),
                        r#"{"pack":{"description":"fixture"}}"#,
                    )
                    .unwrap();
                    std::fs::write(
                        pack.join("data/empty_fixture/tags/item/empty.json"),
                        r#"{"values":[]}"#,
                    )
                    .unwrap();
                    std::fs::write(
                        dir.join("datapacks.json"),
                        r#"{"enabled":["file/empty"],"disabled":[]}"#,
                    )
                    .unwrap();
                    for (id, ingredient) in [
                        ("direct", serde_json::json!([])),
                        ("missing", serde_json::json!("#empty_fixture:missing")),
                        ("tag", serde_json::json!("#empty_fixture:empty")),
                        (
                            "custom",
                            serde_json::json!({"type":"empty_fixture:predicate"}),
                        ),
                    ] {
                        std::fs::write(
                            pack.join(format!("data/empty_fixture/recipe/{id}.json")),
                            serde_json::json!({"type":"minecraft:crafting_shaped","pattern":["A"],
                        "key":{"A":ingredient},"result":{"id":"minecraft:stick"}})
                            .to_string(),
                        )
                        .unwrap();
                    }
                    std::fs::write(
                        pack.join("data/empty_fixture/recipe/a_invalid.json"),
                        serde_json::json!({"type":"minecraft:crafting_transmute",
                    "input":"minecraft:stone","material":"#empty_fixture:missing",
                    "result":{"id":"minecraft:stick"}})
                        .to_string(),
                    )
                    .unwrap();
                    std::fs::write(
                        pack.join("data/empty_fixture/recipe/b_neighbor.json"),
                        serde_json::json!({"type":"minecraft:crafting_shaped","pattern":["A"],
                    "key":{"A":"minecraft:stone"},"result":{"id":"minecraft:stick"}})
                        .to_string(),
                    )
                    .unwrap();
                    game.world_root = dir.clone();
                    let resources = game.options.resources.clone();
                    let stack = crate::game::functions::pack_stack(
                        &dir,
                        &resources,
                        &sunrise_assets::resource_source::ResourceSource::directory(
                            resources.clone(),
                        ),
                        &Default::default(),
                        false,
                        &[],
                        &[],
                    );
                    // Malformed recipes abort registry loading; keep each refusal isolated
                    // from the valid recipes whose empty ingredients we test over the wire.
                    let invalid = ["direct", "missing", "a_invalid"].map(|id| {
                        let path = pack.join(format!("data/empty_fixture/recipe/{id}.json"));
                        let json = std::fs::read(&path).unwrap();
                        std::fs::remove_file(&path).unwrap();
                        (id, path, json)
                    });
                    for (id, path, json) in invalid {
                        std::fs::write(&path, json).unwrap();
                        let error = game
                            .stage_world_tables(&stack, false, &resources)
                            .err()
                            .expect("malformed recipe must refuse the load")
                            .to_string();
                        assert!(
                            error.contains(&format!("data/empty_fixture/recipe/{id}.json")),
                            "{error}"
                        );
                        std::fs::remove_file(path).unwrap();
                    }
                    game.world_tables = game
                        .stage_world_tables(&stack, false, &resources)
                        .unwrap()
                        .tables;
                    game.build_registries();
                    {
                        // The generated Host entrypoint serves mod callers;
                        // check its real answer directly (Ingredient holds in-process
                        // predicates and has no Wire form).
                        let host: &dyn mod_api::Host = game;
                        let empty = host
                            .recipe_ingredient(r##""#empty_fixture:empty""##)
                            .unwrap()
                            .unwrap();
                        assert!(empty.items.is_empty());
                        assert!(empty.order.is_empty());
                        assert!(
                            host.recipe_ingredient(r##""#empty_fixture:missing""##)
                                .unwrap()
                                .is_none()
                        );
                        assert!(host.recipe_ingredient("[]").unwrap().is_none());
                        let stone = mod_api::ItemStack::of(
                            mod_api::__private::item_key("minecraft:stone").unwrap(),
                        );
                        assert!(!empty.test(host, &stone));
                    }
                    game.lan_host(
                        crate::game::lan::LanSettings {
                            port: 0,
                            guest_commands: true,
                            force_game_mode: false,
                        },
                        NetConfig {
                            announce: false,
                            ..NetConfig::loopback(0, "empty-ingredient-test".into(), 2)
                        },
                        HubConfig {
                            default_game_mode: 0,
                            guest_commands: true,
                            force_game_mode: false,
                            hardcore: false,
                            view_distance: 2,
                            simulation_distance: 2,
                            dimension: "minecraft:overworld".into(),
                            allow_flight: false,
                            seed: 0,
                            max_players: 2,
                            integrated: false,
                            levels: Default::default(),
                            level_seeds: Default::default(),
                        },
                    )
                    .unwrap();
                    let (tx, _rx) = mpsc::channel();
                    let mut guest = Guest::new(
                        Joined {
                            conn: 2,
                            uuid: 3739,
                            name: "EmptyIngredientGuest".into(),
                            properties: vec![],
                            info: ClientInformation::new(2),
                            brand: "test".into(),
                            sender: PlayerSender::new(tx, Arc::default()),
                            latency: Arc::new(AtomicI32::new(0)),
                            address: "127.0.0.1:25565".parse().unwrap(),
                            expected: false,
                        },
                        2,
                        [8.5, 200.0, 8.5],
                        2,
                    );
                    guest.dimension = "minecraft:overworld".into();
                    guest.placed = true;
                    guest.loaded = true;
                    guest.inventory[0] = Some(ItemStack::new("minecraft:stone", 16));
                    guest.craft_grid[0] = Some(ItemStack::new("minecraft:dirt", 3));
                    let ids = ["empty_fixture:tag", "empty_fixture:custom"].map(str::to_owned);
                    let index = &world_stations!(game).recipe_index;
                    for id in [
                        "empty_fixture:direct",
                        "empty_fixture:missing",
                        "empty_fixture:a_invalid",
                        "empty_fixture:tag",
                        "empty_fixture:custom",
                    ] {
                        assert!(index.first_display(id).is_none(), "{id}");
                    }
                    assert_eq!(guest.recipe_book.add_recipes(index, &ids), (0, vec![]));
                    guest
                        .recipe_book
                        .add_recipes(index, &["empty_fixture:b_neighbor".to_owned()]);
                    let sync = game.lan.as_ref().unwrap().hub.sync_data();
                    assert!(sync.advancements.recipes_added(&ids, sync).is_none());
                    let display = index.first_display("empty_fixture:b_neighbor").unwrap();
                    assert_eq!(
                        sync.advancements.recipe_of_display(display),
                        Some("empty_fixture:b_neighbor")
                    );
                    {
                        use sunrise_protocol::{
                            buf::{Codec, Reader},
                            packets::recipes::RecipeBookAdd,
                        };
                        let requested = [
                            ids[0].clone(),
                            ids[1].clone(),
                            "empty_fixture:b_neighbor".to_owned(),
                        ];
                        let bytes = sync.advancements.recipes_added(&requested, sync).unwrap();
                        let packet = RecipeBookAdd::read(&mut Reader::new(&bytes)).unwrap();
                        assert_eq!(packet.entries.len(), 1);
                        assert_eq!(packet.entries[0].id, display);
                    }
                    for id in &ids {
                        assert!(!guest.recipe_book.knows(id), "{id}");
                        assert!(world_recipes!(game).placement(id).unwrap().impossible());
                        assert!(
                            world_recipes!(game).by_type["minecraft:crafting"]
                                .iter()
                                .any(|recipe| &recipe.id == id)
                        );
                    }
                    game.lan.as_mut().unwrap().hub.guests.push(guest);
                    for display in [-1, i32::MAX] {
                        game.lan_guest_packet(
                            2,
                            play::Serverbound::PlaceRecipe(PlaceRecipe {
                                container: 0,
                                recipe: display,
                                use_max: true,
                            }),
                        );
                        let guest = game.lan.as_ref().unwrap().hub.guest(2).unwrap();
                        assert_eq!(
                            guest.inventory[0],
                            Some(ItemStack::new("minecraft:stone", 16))
                        );
                        assert_eq!(
                            guest.craft_grid[0],
                            Some(ItemStack::new("minecraft:dirt", 3))
                        );
                        assert!(guest.craft_grid[1..].iter().all(Option::is_none));
                        assert!(guest.craft_result.is_none());
                    }
                    let display = world_stations!(game)
                        .recipe_index
                        .first_display("empty_fixture:b_neighbor")
                        .unwrap();
                    game.lan_guest_packet(
                        2,
                        play::Serverbound::PlaceRecipe(PlaceRecipe {
                            container: 0,
                            recipe: display,
                            use_max: false,
                        }),
                    );
                    let guest = game.lan.as_ref().unwrap().hub.guest(2).unwrap();
                    assert_eq!(guest.inventory[0].as_ref().unwrap().count, 15);
                    assert_eq!(
                        guest.craft_grid[0],
                        Some(ItemStack::new("minecraft:stone", 1))
                    );
                    assert_eq!(
                        guest.craft_result,
                        Some(ItemStack::new("minecraft:stick", 1))
                    );
                    std::fs::remove_dir_all(dir).unwrap();
                });
            },
        );
    }
}
