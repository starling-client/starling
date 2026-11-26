use super::*;

impl Game<'_> {
    /// Whether the mods let the guest open the menu `kind` (`native_container_opened`),
    /// heard with the guest's state swapped in. Asked before the menu that
    /// was open closes. None if the guest's turn cannot be entered.
    pub(in crate::game) fn guest_may_open(
        &mut self,
        conn: u32,
        kind: &str,
        pos: Option<[i32; 3]>,
    ) -> Option<bool> {
        let guest = self.lan.as_ref()?.hub.guest(conn)?;
        if !guest.participates_in_player_simulation() || guest.disconnect_pending {
            return None;
        }
        let (uuid, name) = (guest.uuid, guest.name.clone());
        let hook = |game: &mut Self| {
            let mut cancel = false;
            game.native_container_opened(uuid, &name, kind, pos, &mut cancel);
            !cancel
        };
        // The guest whose turn it is is in place already.
        if self.lan_acting == Some(conn) {
            Some(hook(self))
        } else {
            self.as_guest(conn, hook)
        }
    }

    /// Finish the menu handoff after the guest's use. As in
    /// `ChestBlock.useWithoutItem`, the block has already decided its use
    /// result independently of what `ServerPlayer.openMenu` does.
    pub(in crate::game) fn lan_open_container(
        &mut self,
        conn: u32,
        pos: (i32, i32, i32),
    ) -> MenuOpen {
        if pos.1 == crate::game::minecarts::CART_KEY_Y {
            return self.lan_open_vehicle_container(conn, pos);
        }
        self.open_block_menu(conn, pos)
    }

    /// Install the block menu for a guest's use or a direct BlockEntityRef open.
    pub(in crate::game) fn open_block_menu(&mut self, conn: u32, pos: (i32, i32, i32)) -> MenuOpen {
        let generation = self.world.block_entities.generation(&pos);
        let name = &self
            .res
            .facts()
            .get(self.world.block(pos.0, pos.1, pos.2))
            .name;
        let block = name.to_string();
        let (menu, slots) = if &**name == "minecraft:ender_chest" {
            // EnderChestBlock.useWithoutItem: the player's own inventory.
            ("minecraft:generic_9x3", Slots::Ender)
        } else if let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get(&pos) {
            // AbstractFurnaceBlock.openContainer: FurnaceMenu, BlastFurnaceMenu, SmokerMenu.
            let menu = match f.kind.recipe_type() {
                "minecraft:blasting" => "minecraft:blast_furnace",
                "minecraft:smoking" => "minecraft:smoker",
                _ => "minecraft:furnace",
            };
            (menu, Slots::Block(pos))
        } else if let Some(BlockEntity::Crafter(_)) = self.world.block_entities.get(&pos) {
            // CrafterBlock.useWithoutItem: CrafterMenu over the block entity.
            ("minecraft:crafter_3x3", Slots::Block(pos))
        } else if let Some(BlockEntity::Brewing(_)) = self.world.block_entities.get(&pos) {
            // BrewingStandBlock.useWithoutItem: BrewingStandMenu.
            ("minecraft:brewing_stand", Slots::Block(pos))
        } else if let Some((other, right)) = self.chest_partner(pos) {
            // ChestBlock.combine: the right half's slots come first.
            let halves = if right {
                Slots::Double(pos, other)
            } else {
                Slots::Double(other, pos)
            };
            ("minecraft:generic_9x6", halves)
        } else if let Some(menu) = self.mod_menu_at(pos) {
            (menu, Slots::Block(pos))
        } else {
            let Some(BlockEntity::Chest(items)) = self.world.block_entities.get(&pos) else {
                return MenuOpen::NoProvider;
            };
            let menu = match items.len() {
                27 => "minecraft:generic_9x3",
                9 => "minecraft:generic_3x3",
                5 => "minecraft:hopper",
                _ => return MenuOpen::NoProvider,
            };
            (menu, Slots::Block(pos))
        };
        // The menu provider's getDisplayName: a container's custom name
        // (BaseContainerBlockEntity.getName) or its default name.
        let custom = |k: (i32, i32, i32)| self.block_name_plain(k);
        let title = match slots {
            Slots::Ender => sunrise_protocol::text::translatable("container.enderchest", &[]),
            Slots::Double(first, second) => sunrise_client::containers::double_chest_name(
                custom(first).as_deref(),
                custom(second).as_deref(),
            ),
            Slots::Block(k) => {
                // A mod block's container is named after its block
                // (`Block.getDescriptionId`).
                let key = match sunrise_client::containers::default_name_key(name) {
                    Some(key) => key.to_owned(),
                    None if self.mod_menu_at(k).is_some() => {
                        format!("block.{}", name.replace(':', "."))
                    }
                    None => return MenuOpen::NoProvider,
                };
                sunrise_client::containers::display_name(custom(k).as_deref(), &key)
            }
            // Opened by a mod (`open_menu`), never here.
            Slots::Native => return MenuOpen::NoProvider,
        };
        let Some(host) = self.lan.as_ref() else {
            return MenuOpen::Refused;
        };
        if host
            .hub
            .guest(conn)
            .and_then(|g| slots.read(&self.world, g))
            .is_none()
        {
            return MenuOpen::Refused;
        }
        // The block entity's createMenu: a shulker box opens a ShulkerBoxMenu
        // (not a 9x3 ChestMenu), whose slots refuse shulker boxes.
        let menu = match slots {
            Slots::Block(_) => sunrise_client::containers::block_menu_type(name).unwrap_or(menu),
            _ => menu,
        };
        // BaseContainerBlockEntity.createMenu (and a double chest's own menu
        // provider, which asks both halves) refuses a locked container with
        // `sendChestLockedNotifications`: the action bar names it and everyone
        // near hears the locked chest; no menu opens.
        let (spectator, hand) = host.hub.guest(conn).map_or((false, None), |g| {
            (
                g.spectator(),
                g.inventory.get(g.selected).cloned().flatten(),
            )
        });
        let halves = match &slots {
            Slots::Double(first, second) => [Some(*first), Some(*second)],
            Slots::Block(k) => [Some(*k), None],
            _ => [None, None],
        };
        let Some((uuid, name)) = host.hub.guest(conn).map(|g| (g.uuid, g.name.clone())) else {
            return MenuOpen::Refused;
        };
        // A mod container's table waits for `RandomizableContainerBlockEntity.createMenu`;
        // a vanilla container's rolled in its block's use.
        let loot = matches!(slots, Slots::Block(k) if self.mod_menu_at(k).is_some()
            && self.block_loot_pending(k));
        // RandomizableContainerBlockEntity.canOpen: no spectator while a table waits, before
        // the lock is asked; ServerPlayer.openMenu tells them in red on the action bar.
        if loot && spectator {
            let Some(host) = self.lan.as_ref() else {
                return MenuOpen::Refused;
            };
            let message = sunrise_client::text::Text::translate("container.spectatorCantOpen", &[])
                .with_color(sunrise_client::text::RED);
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: sunrise_client::lan::data::component_nbt(&message.component()),
                    overlay: true,
                }),
            );
            return MenuOpen::Refused;
        }
        let mut locked = false;
        for key in halves.iter().flatten().copied() {
            let mut check = self.container_locked_for(key, spectator, hand.as_ref());
            self.native_block_lock_check(uuid, &name, [key.0, key.1, key.2], &mut check);
            locked |= check;
        }
        if locked {
            let Some(host) = self.lan.as_mut() else {
                return MenuOpen::Refused;
            };
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: sunrise_client::containers::locked_message(&title),
                    overlay: true,
                }),
            );
            // `ChestBlock`: the double chest sounds between its halves, from
            // the first one's centre plus half a block toward the other.
            let at = match &slots {
                Slots::Double(first, second) => [
                    f64::from(first.0) + 0.5 + f64::from((second.0 - first.0).signum()) / 2.0,
                    f64::from(first.1) + 0.5,
                    f64::from(first.2) + 0.5 + f64::from((second.2 - first.2).signum()) / 2.0,
                ],
                Slots::Block(k) => [
                    f64::from(k.0) + 0.5,
                    f64::from(k.1) + 0.5,
                    f64::from(k.2) + 0.5,
                ],
                _ => [0.0; 3],
            };
            let name = "minecraft:block.chest.locked";
            self.send_level_sound(LevelSound::new(name, at, 1.0, 1.0), None);
            return MenuOpen::Refused;
        }
        // A mod menu type only for a client that has it and negotiated the
        // `sunrise:menu_extra` opening layout: as `ServerPlayer.openMenu` with
        // no menu, nothing opens.
        let Some(host) = self.lan.as_ref() else {
            return MenuOpen::Refused;
        };
        let Some(guest) = host.hub.guest(conn) else {
            return MenuOpen::Refused;
        };
        if !sunrise_client::containers::menu_reaches(&guest.brand, menu) {
            return MenuOpen::Refused;
        }
        let sends_menu_extra = matches!(slots, Slots::Block(k) if self.mod_menu_at(k).is_some());
        let required_rev = if let Some(kind) = sunrise_client::containers::mod_menu_type(menu) {
            if !kind.player_inventory || kind.slots == 0 {
                sunrise_protocol::menu_extra::LIVE_DATA_REVISION
            } else {
                1
            }
        } else if sends_menu_extra || !menu.starts_with("minecraft:") {
            1
        } else {
            0
        };
        if guest.sender.menu_extra_rev < required_rev {
            return MenuOpen::Refused;
        }
        // The mods may stop the menu; the click is spent.
        match self.guest_may_open(conn, menu, Some([pos.0, pos.1, pos.2])) {
            Some(true) => {}
            Some(false) => return MenuOpen::Cancelled,
            None => return MenuOpen::Refused,
        }
        let mut cancel = false;
        self.native_inventory_block_start(uuid, &name, [pos.0, pos.1, pos.2], &mut cancel);
        if cancel {
            return MenuOpen::Refused;
        }
        // ServerPlayer.openMenu closes the menu that was open.
        let previous_window = self.lan.as_ref().and_then(|host| {
            super::super::native_menus::open_menu_of(host, conn).map(|(window, ..)| window)
        });
        self.guest_replace_menu_then(conn, false, |game| {
            // `createMenu` of an admitted open: `unpackLootTable(player)` with the opener's own
            // state (their luck, `THIS_ENTITY` and advancements), then the menu reads the slots.
            if loot {
                let roll = |game: &mut Self| {
                    let player = game.native_acting_player();
                    game.unpack_loot(crate::game::container_loot::LootTarget::Block(pos), player);
                };
                if game.lan_acting == Some(conn) {
                    roll(game);
                } else if game.as_guest(conn, roll).is_none() {
                    return Err(MenuOpen::Refused);
                }
            }
            let Some(items) = game
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .and_then(|g| slots.read(&game.world, g))
            else {
                return Err(MenuOpen::Refused);
            };
            // A mod menu type's block tells the client where it is, with what its
            // mod writes (`openMenu` with an extra-data writer); a stock menu
            // sends nothing more.
            let extra = match slots {
                Slots::Block(k) if game.mod_menu_at(k).is_some() => {
                    let mut data = None;
                    game.native_menu_extra_data(uuid, menu, &block, [k.0, k.1, k.2], &mut data);
                    let data = data.unwrap_or_default();
                    if data.len() > sunrise_protocol::menu_extra::MAX_DATA {
                        log::error!(
                            "Sunrise: {block}'s menu ({menu}) extra data ({} bytes) exceeds {} bytes; refusing open",
                            data.len(),
                            sunrise_protocol::menu_extra::MAX_DATA
                        );
                        if let Some(container) = previous_window
                            && let Some(host) = game.lan.as_mut()
                            && super::super::native_menus::open_menu_of(host, conn).is_none()
                        {
                            host.hub
                                .send_to(conn, &C::ContainerClose(play::ContainerClose { container }));
                        }
                        return Err(MenuOpen::Refused);
                    }
                    Some(data)
                }
                _ => None,
            };
            if game.world.block_entities.generation(&pos) != generation {
                if let Some(container) = previous_window
                    && let Some(host) = game.lan.as_mut()
                    && super::super::native_menus::open_menu_of(host, conn).is_none()
                {
                    host.hub
                        .send_to(conn, &C::ContainerClose(play::ContainerClose { container }));
                }
                return Err(MenuOpen::Stale);
            }
            let Some(gi) = game.guest_index(conn) else {
                return Err(MenuOpen::Refused);
            };
            // Park the opener's window-0 cursor and build the first contents from
            // their inventory, even when an open through BlockEntityRef runs in their turn.
            let Some(menu_generation) = game.with_own_record(gi, |host, _, _| {
                if host
                    .hub
                    .open_menu_window(conn, menu, title, Some(pos), slots, items, extra)
                    .is_none()
                {
                    if let Some(container) = previous_window
                        && super::super::native_menus::open_menu_of(host, conn).is_none()
                    {
                        host.hub
                            .send_to(conn, &C::ContainerClose(play::ContainerClose { container }));
                    }
                    return None;
                }
                host.hub.guests.update(gi, |g| {
                    let open = g.menu.as_mut().expect("the window was just installed");
                    open.opener_block = Some(block.into_boxed_str());
                });
                Some(host.hub.guest(conn)?.menu_generation)
            }) else {
                return Err(MenuOpen::Refused);
            };
            Ok(menu_generation)
        }, |game, result| {
            let menu_generation = match result {
                Ok(generation) => generation,
                Err(refused) => return refused,
            };
            // ChestMenu and ShulkerBoxMenu own startOpen, including an active ender chest.
            game.guest_container_opener(conn, pos, true);
            // The game-event listener can close or replace the installed menu.
            if game.lan.as_ref().and_then(|host| host.hub.guest(conn))
                .is_some_and(|guest| guest.menu.is_some() && guest.menu_generation == menu_generation)
            {
                game.player_menu_opening(conn);
            }
            MenuOpen::Opened
        })
    }

    /// `ContainerEntity.interactWithContainerVehicle` for a guest: the menu
    /// the vehicle's own `MenuProvider.createMenu` builds - three rows of a
    /// `ChestMenu` for a chest boat or a chest minecart, a `HopperMenu` for a
    /// hopper minecart - titled with the vehicle's name.
    fn lan_open_vehicle_container(&mut self, conn: u32, pos: (i32, i32, i32)) -> MenuOpen {
        let entity_type = if let Some(cart) = self.carts.iter().find(|c| c.id as i32 == pos.2) {
            cart.kind.id().to_owned()
        } else if let Some(boat) = self.boats.iter().find(|b| b.id as i32 == pos.2) {
            if !boat.chest {
                return MenuOpen::NoProvider;
            }
            boat.entity_type()
        } else {
            return MenuOpen::NoProvider;
        };
        // `createMenu` (ChestMenu.threeRows or a HopperMenu), and
        // `MenuProvider.getDisplayName`, the vehicle's name key.
        let Some((menu, _)) = sunrise_client::containers::vehicle_menu(&entity_type) else {
            return MenuOpen::NoProvider;
        };
        let title = sunrise_protocol::text::translatable(
            &sunrise_client::containers::vehicle_menu_name(&entity_type),
            &[],
        );
        let slots = Slots::Block(pos);
        let Some(host) = self.lan.as_ref() else {
            return MenuOpen::Refused;
        };
        // The mods may stop the menu; the click is spent. A fake player is
        // stopped here too: the stand-in the open staged leaves the level
        // through the close path, its items back on the vehicle, where a
        // later transfer finds them. A stand-in another guest's menu still
        // backs is not this use's to remove: the close is only for the one
        // nobody is looking at.
        let boat = self
            .boats
            .iter()
            .position(|b| b.id as i32 == pos.2 && b.chest);
        let spectator_pending = boat.is_some_and(|i| {
            host.hub.guest(conn).is_some_and(|g| g.spectator())
                && self.boat_loot_pending(self.boats[i].id)
        });
        let denied = if spectator_pending {
            // ServerPlayer.openMenu: the refused spectator's action bar, red.
            let message = sunrise_client::text::Text::translate("container.spectatorCantOpen", &[])
                .with_color(sunrise_client::text::RED);
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: sunrise_client::lan::data::component_nbt(&message.component()),
                    overlay: true,
                }),
            );
            Some(MenuOpen::Refused)
        } else {
            match self.guest_may_open(conn, menu, None) {
                Some(true) => None,
                Some(false) => Some(MenuOpen::Cancelled),
                None => Some(MenuOpen::Refused),
            }
        };
        if let Some(denied) = denied {
            let viewed = self.lan.as_ref().is_some_and(|host| {
                host.hub
                    .guests
                    .iter()
                    .filter(|g| host.hub.here(g))
                    .any(|g| {
                        g.menu
                            .as_ref()
                            .is_some_and(|m| m.slots == Slots::Block(pos))
                    })
            });
            if !viewed {
                self.close_cart_container(pos);
            }
            return denied;
        }
        if let Some(i) = boat
            && self.boat_loot_pending(self.boats[i].id)
        {
            let id = self.boats[i].id;
            self.unpack_boat_loot(i, true);
            // Advancement rewards can remove the boat while generating its loot.
            if !self.boats.iter().any(|boat| boat.id == id) {
                return MenuOpen::Stale;
            }
        }
        let Some(items) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .and_then(|g| slots.read(&self.world, g))
        else {
            return MenuOpen::Refused;
        };
        self.guest_replace_menu(conn, false, |game| {
            let Some(gi) = game.guest_index(conn) else {
                return MenuOpen::Refused;
            };
            if game
                .with_own_record(gi, |host, _, _| {
                    host.hub
                        .open_menu_window(conn, menu, title, Some(pos), slots, items, None)
                })
                .is_none()
            {
                return MenuOpen::Refused;
            }
            game.player_menu_opening(conn);
            MenuOpen::Opened
        })
    }

    /// The guest's open container is closing (`AbstractContainerMenu.removed`):
    /// it no longer counts among the openers of the lid, and the lid's close
    /// sound is played for everyone around (`ChestBlockEntity.onClose`, a
    /// `playSound` with no player), the guest included, once the last opener
    /// has left.
    pub(in crate::game) fn guest_container_closed(&mut self, conn: u32) {
        if let Some(pos) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .and_then(|guest| guest.menu.as_ref()?.pos)
        {
            self.guest_container_opener(conn, pos, false);
        }
    }

    /// `ContainerOpenersCounter.incrementOpeners` / `ContainerOpenersCounter.decrementOpeners`:
    /// the first opener and the last closer each trigger a sound and game event.
    fn guest_container_opener(&mut self, conn: u32, pos: (i32, i32, i32), open: bool) {
        let acting = self.lan_acting == Some(conn);
        let spectator = if acting {
            self.spectator()
        } else {
            let Some(guest) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) else {
                return;
            };
            guest.spectator()
        };
        if spectator || !self.container_openers_changed(pos, open) {
            return;
        }
        let state = self.world.block(pos.0, pos.1, pos.2);
        let name = &self.res.facts().get(state).name;
        let shulker = super::super::storage::is_shulker_box(name);
        // Resolve the sound before the game event; its listener can replace the block.
        let sound = self.resolve_container_sound(pos, name, open);
        // ContainerOpenersCounter calls onOpen/onClose before the game event;
        // ShulkerBoxBlockEntity emits its game event before playing its sound.
        if !shulker && let Some((sound, at)) = sound {
            self.play_container_sound(sound, at);
        }
        let event = if open {
            "minecraft:container_open"
        } else {
            "minecraft:container_close"
        };
        let post = |game: &mut Self| {
            game.post_block_game_event(
                event,
                [pos.0, pos.1, pos.2],
                Some(sunrise_client::mobs::Who::Player),
                None,
            );
        };
        if acting {
            post(self);
        } else {
            self.as_guest(conn, post);
        }
        if shulker && let Some((sound, at)) = sound {
            self.play_container_sound(sound, at);
        }
    }

    /// Closes whatever menu the guest has open, the way each `ContainerClose`
    /// does (`ServerPlayer.doCloseContainer`, `AbstractContainerMenu.removed`):
    /// the container's lid, a vehicle's stand-in (its items go back to the
    /// vehicle), the trade (written back to the villager), the crafting table,
    /// station and mount menus, and the cursor stack back to the inventory.
    /// Does not send a close packet or remove window 0: ordinary menu
    /// replacement keeps its cursor and crafting inputs. The guest's level
    /// must be the one in place.
    #[cfg(test)]
    pub(in crate::game) fn guest_close_all(&mut self, conn: u32) {
        self.guest_replace_menu(conn, false, |_| ());
    }

    /// Replace a menu after the old owner and Close listeners have run. Opens requested
    /// reentrantly by those callbacks follow the replacement that started this close.
    pub(in crate::game) fn guest_replace_menu<R>(
        &mut self,
        conn: u32,
        tell: bool,
        open: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.guest_replace_menu_then(conn, tell, open, |_, result| result)
    }

    /// Finish installing the new menu before its opening notifications. If teardown queued a
    /// replacement, that request must still follow these notifications in publication order.
    fn guest_replace_menu_then<T, R>(
        &mut self,
        conn: u32,
        tell: bool,
        install: impl FnOnce(&mut Self) -> T,
        notify: impl FnOnce(&mut Self, T) -> R,
    ) -> R {
        self.journal_merchant_replacement(conn);
        let hold = self.hold_menu_opens(conn);
        self.guest_remove_menus(conn, MenuRemoval::Live, tell);
        let installed = install(self);
        let hold = self.release_empty_menu_open_hold(hold);
        let result = notify(self, installed);
        if let Some(hold) = hold {
            self.resume_menu_opens(hold);
        }
        result
    }

    pub(in crate::game) fn close_guest_window(
        &mut self,
        conn: u32,
        window: i32,
        generation: u64,
        tell: bool,
    ) {
        self.in_menu_owner_level(conn, |game| {
            if let Some(host) = game.lan.as_ref()
                && host.hub.guest(conn).is_some_and(|guest| {
                    guest.menu_generation == generation && !guest.disconnect_pending
                })
            {
                if window == 0 {
                    game.lan_crafting_removed(conn, Some(0), MenuRemoval::Live);
                } else if super::super::native_menus::open_menu_of(host, conn)
                    .is_some_and(|(active, ..)| active == window)
                {
                    game.guest_replace_menu(conn, tell, |_| ());
                }
            }
        });
    }

    /// Finish removal before saving or exporting a live guest. Neither menu's
    /// cursor nor the inventory crafting grid is part of the saved player. Opens that the
    /// closing listeners request stay unpublished behind the returned hold, so nothing real can
    /// enter a transient slot the save omits: the caller drops them when the player is leaving
    /// ([`Self::discard_menu_opens`]) and resumes them otherwise
    /// ([`Self::resume_menu_opens`]).
    pub(in crate::game) fn guest_close_menus_for_save(
        &mut self,
        conn: u32,
    ) -> super::super::lan_menus::MenuOpenHold {
        self.journal_merchant_replacement(conn);
        let hold = self.hold_menu_opens(conn);
        self.guest_remove_menus(conn, MenuRemoval::Live, false);
        self.lan_crafting_removed(conn, Some(0), MenuRemoval::Live);
        hold
    }

    /// `Player.remove(CHANGED_DIMENSION)` removes both menus without dropping
    /// a cursor that still fits. Overflow belongs in the level the guest left.
    /// Hold reentrant opens until travel has sent Respawn and finished changing level.
    /// The retired window is needed to send an explicit close only on a failed trip.
    pub(in crate::game) fn guest_close_menus_for_travel(
        &mut self,
        conn: u32,
    ) -> (Option<i32>, super::super::lan_menus::MenuOpenHold) {
        let hold = self.hold_menu_opens(conn);
        let retired = self.guest_remove_menus(conn, MenuRemoval::ChangedDimension, false);
        (retired, hold)
    }

    pub(in crate::game) fn guest_disconnected(&mut self, conn: u32) {
        let hold = self.hold_menu_opens(conn);
        self.guest_remove_menus(conn, MenuRemoval::Disconnected, false);
        self.resume_menu_opens(hold);
    }

    fn guest_remove_menus(&mut self, conn: u32, removal: MenuRemoval, tell: bool) -> Option<i32> {
        let host = self.lan.as_mut()?;
        let gi = host.hub.guests.connection_index(conn)?;
        if host.menus.get(&conn).is_some_and(|menus| menus.closing) {
            // Nested closes of the same instance belong to the outer close.
            return None;
        }
        let guest = &host.hub.guests[gi];
        let generation = guest.menu_generation;
        let hub_menu = guest.menu.as_ref().map(|menu| (menu.window, menu.slots));
        let window = super::super::native_menus::open_menu_of(host, conn)
            .map(|(window, ..)| window)
            .or_else(|| hub_menu.map(|(window, _)| window));
        if let Some(container) = window {
            host.menus.entry(conn).or_default().closing = true;
            if tell {
                host.hub
                    .send_to(conn, &C::ContainerClose(play::ContainerClose { container }));
            }
        }

        // Player.remove removes the inventory menu before the open window.
        if removal != MenuRemoval::Live {
            self.lan_crafting_removed(conn, Some(0), removal);
        }
        if let Some(gi) = self.guest_index(conn) {
            if removal.drops_items() {
                self.guest_drop_carried(gi);
            } else if let Some(host) = self.lan.as_mut() {
                let returned = host.hub.take_closed_menu_cursors(conn).collect();
                // Inputs of a processing menu the hub closed itself keep the drop decision made
                // when it closed (a dead player's drop), even though this removal is live.
                let (to_return, to_drop) = host.hub.take_closed_processing_inputs(conn);
                self.lan_place_back(gi, returned);
                if let Some(gi) = self.guest_index(conn) {
                    self.lan_place_back(gi, to_return);
                }
                if !to_drop.is_empty()
                    && let Some(gi) = self.guest_index(conn)
                {
                    self.drop_guest_items(gi, to_drop);
                }
            }
        }
        if let Some((window, slots)) = hub_menu {
            self.guest_container_closed(conn);
            if let Slots::Block(key) = slots
                && key.1 == crate::game::minecarts::CART_KEY_Y
            {
                self.close_guest_vehicle_menu(conn, key);
            }
            // Returning a cursor and closing a container can call listeners that
            // remove another player, shifting the guest list.
            if let Some(gi) = self.guest_index(conn)
                && let Some((closed, removed)) = self.with_own_record(gi, |host, _, _| {
                    host.hub.remove_container_menu(conn, window)
                })
            {
                self.settle_closed_hub_menu(conn, closed, removal);
                if let Some(removed) = removed {
                    self.deliver_menu_removed(removed);
                }
            }
        }
        self.lan_merchant_removed(conn, window, removal);
        self.lan_crafting_removed(conn, window, removal);
        self.lan_station_removed(conn, window, removal);
        self.lan_mount_removed(conn, window);
        // `PlayerContainerEvent.Close`, after the menu's `removed` and before
        // `containerMenu = inventoryMenu`.
        if let Some(window) = window {
            self.player_menu_closing(conn, window, generation);
        }
        if hub_menu.is_some()
            && let Some(host) = self.lan.as_mut()
        {
            host.hub.finish_close_container(conn, generation);
        }
        if let Some(menus) = self.lan.as_mut().and_then(|host| host.menus.get_mut(&conn)) {
            menus.closing = false;
        }
        window
    }

    /// `AbstractContainerMenu.removed` for a window the hub detached: the cursor, then a
    /// processing menu's non-result inputs (`clearContainer`), each through
    /// `dropOrPlaceInInventory`. A killed or disconnected player drops them; otherwise they go
    /// back to the inventory, dropping what does not fit. A processing menu's results are
    /// already discarded.
    pub(super) fn settle_closed_hub_menu(
        &mut self,
        conn: u32,
        closed: sunrise_client::lan::containers::ClosedHubMenu,
        removal: MenuRemoval,
    ) {
        let carried: Vec<_> = closed.carried.into_iter().collect();
        for stacks in [carried, closed.processing_inputs] {
            // Returning stacks can call listeners that remove another player, shifting the
            // guest list.
            let Some(gi) = self.guest_index(conn).filter(|_| !stacks.is_empty()) else {
                continue;
            };
            if removal.drops_items() {
                self.drop_guest_items(gi, stacks);
            } else {
                self.lan_place_back(gi, stacks);
            }
        }
    }

    /// `ContainerEntity`'s `Container.stopOpen` when a vehicle menu closes
    /// (`AbstractChestBoat.stopOpen` / `MinecartChest.stopOpen`): the menu's
    /// slots go back to the vehicle, the shared placeholder leaves
    /// `world.block_entities`, and CONTAINER_CLOSE fires at the vehicle with
    /// the closing player as the source. A hopper minecart keeps
    /// `Container.stopOpen`'s no-op. Called by the close packet and by the
    /// `ServerPlayer.tick` reach/destroyed close.
    pub(super) fn close_guest_vehicle_menu(&mut self, conn: u32, key: (i32, i32, i32)) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let other_viewers =
            Self::vehicle_has_other_viewers(self.container, &host.hub.guests, Some(conn), key);
        self.store_cart_items(key);
        if !other_viewers {
            self.world.block_entities.remove(&key);
        }
        let at = if let Some(b) = self.boats.iter().find(|b| b.id as i32 == key.2) {
            Some([b.position.x, b.position.y, b.position.z])
        } else if let Some(c) = self.carts.iter().find(|c| c.id as i32 == key.2) {
            (c.kind == crate::game::minecarts::CartKind::Chest).then_some([
                c.position.x,
                c.position.y,
                c.position.z,
            ])
        } else {
            None
        };
        if let Some(at) = at {
            let spect = self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .is_some_and(|g| g.spectator());
            let ctx = sunrise_client::sculk::Context {
                source: Some(sunrise_client::mobs::Who::Player.into()),
                spectator: spect,
                targetable: !spect,
                living: true,
                player: true,
                ..Default::default()
            };
            self.post_game_event("minecraft:container_close", at, ctx);
        }
    }

    /// `ServerPlayer.tick`'s `stillValid` for one guest: closes its block container once its
    /// block is removed or out of reach, or closes every open menu on death.
    pub(in crate::game) fn lan_container_tick_one(&mut self, conn: u32) {
        let Some(dead) = self.lan.as_ref().and_then(|host| {
            let g = host.hub.guest(conn).filter(|g| host.hub.here(g))?;
            let close = (g.dead && super::super::native_menus::open_menu_of(host, conn).is_some())
                || !self.container_menu_valid(g);
            close.then_some(g.dead)
        }) else {
            return;
        };
        self.guest_close_menus(
            conn,
            if dead {
                MenuRemoval::Killed
            } else {
                MenuRemoval::Live
            },
        );
    }

    #[cfg(test)]
    pub(in crate::game) fn lan_containers_tick(&mut self) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && !self.player_tick_canceled(g.conn))
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            self.lan_container_tick_one(conn);
        }
    }

    /// `stillValid` of the guest's open container: its blocks are still there
    /// and in reach, or its vehicle exists and is in reach. True with none open.
    pub(super) fn container_menu_valid(&self, g: &sunrise_client::lan::guest::Guest) -> bool {
        let Some(host) = self.lan.as_ref() else {
            return true;
        };
        g.menu.as_ref().is_none_or(|m| {
            m.slots.valid(&self.world)
                && m.still_valid(&host.hub.loop_context, &self.world, g)
                && !matches!(m.slots, Slots::Block((_, y, id))
                    if y == crate::game::minecarts::CART_KEY_Y
                        && !self.chest_vehicle_still_valid(id, g))
        })
    }

    /// `AbstractContainerMenu.stillValid` of whatever menu the guest has open
    /// (a container or vehicle, a trade, the crafting table, a station, a
    /// lectern, a mount's inventory). True when none is open.
    pub(in crate::game) fn guest_menu_still_valid(&self, conn: u32) -> bool {
        let Some(host) = self.lan.as_ref() else {
            return true;
        };
        let Some(g) = host.hub.guest(conn) else {
            return true;
        };
        if !self.container_menu_valid(g) {
            return false;
        }
        let p = host.play.get(&conn);
        let menus = host.menus.get(&conn);
        menus
            .and_then(|m| m.crafting.table.as_ref())
            .is_none_or(|t| self.lan_table_valid(g, t.3))
            && menus
                .and_then(|m| m.station.as_ref())
                .is_none_or(|s| self.lan_station_valid(g, s))
            && p.and_then(|p| p.lectern.as_ref())
                .is_none_or(|l| self.lan_lectern_valid(g, l.key))
            && menus
                .and_then(|m| m.merchant.as_ref())
                .and_then(|m| m.merchant.mob)
                .is_none_or(|id| self.lan_merchant_valid(g, id))
            && p.and_then(|p| p.mount_window.as_ref())
                .is_none_or(|m| self.lan_mount_valid(g, m.mob))
    }

    /// The check before a menu click: a menu that is no longer valid closes
    /// (returning what it held, as `ContainerClose` does) and the client's
    /// window with it, so the click lands on nothing
    /// (`ServerGamePacketListenerImpl.handleContainerClick` and the others
    /// ask `stillValid` first).
    pub(super) fn guest_validate_menu(&mut self, conn: u32) -> bool {
        if self.guest_menu_still_valid(conn) {
            return true;
        }
        self.guest_close_menus_and_tell(conn);
        false
    }

    /// `ServerPlayer.closeContainer`: the client is told to close each window first, and
    /// then every menu the guest has open closes through `guest_close_all` (the lid,
    /// stand-ins, the cursor stack back to the inventory, `PlayerContainerEvent.Close`), once.
    pub(in crate::game) fn guest_close_menus_and_tell(&mut self, conn: u32) {
        if let Some(window) = self.lan.as_ref().and_then(|host| {
            super::super::native_menus::open_menu_of(host, conn).map(|(window, ..)| window)
        }) {
            self.reply_menu_click(conn, super::super::lan_menus::ClickReply::Close(window));
        }
    }

    pub(super) fn guest_close_menus(&mut self, conn: u32, removal: MenuRemoval) {
        let hold = self.hold_menu_opens(conn);
        self.guest_remove_menus(conn, removal, true);
        self.resume_menu_opens(hold);
    }

    /// `ContainerEntity.isChestVehicleStillValid`: the chest minecart or chest
    /// boat is still there and within the guest's entity interaction range
    /// plus 4 of its box.
    pub(super) fn chest_vehicle_still_valid(
        &self,
        id: i32,
        g: &sunrise_client::lan::guest::Guest,
    ) -> bool {
        let bounds = self
            .carts
            .iter()
            .find(|c| c.id as i32 == id)
            .map(|c| c.bounds())
            .or_else(|| {
                self.boats
                    .iter()
                    .find(|b| b.id as i32 == id)
                    .map(|b| b.bounds())
            });
        bounds.is_some_and(|b| guest_within_entity_range(g, &b, 4.0))
    }
}
