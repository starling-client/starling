//! A subscribed guest's merchant result click is one savepoint. Each take has its own
//! frozen menu plan, collect and apply; a later refusal aborts every earlier iteration.
use super::{
    Game,
    decided_action::{DecidedTarget, Epoch},
    lan_trading::{MerchantRun, opaque_villager},
    stacked_on_other::Pickup,
    trading::MadeMaps,
    transactions::{
        After,
        decision::{Identity, OwnerStamp, Reply, Side},
    },
};
use std::{cell::RefCell, collections::HashMap};
use sunrise_client::{
    advancements::Advancement,
    items::ItemStack,
    lan::conn::transaction_packets::Scope,
    menu::{Input, Menu, StackedOutcome},
    merchant::Event,
};
use sunrise_protocol::packets::play;

/// The live owner's generation covers its slots, offer list, selection, and packet snapshot.
/// The action's player epoch separately covers the cursor and player-owned destinations.
struct TakeReads {
    conn: u32,
    window: i32,
    identity: Identity,
    stamp: OwnerStamp,
    hub_generation: u64,
    creative: bool,
    map_id: Option<i32>,
}

impl TakeReads {
    fn fresh(&self, game: &Game<'_>) -> bool {
        let Some(host) = game.lan.as_ref() else {
            return false;
        };
        let Some(guest) = host.hub.guest(self.conn) else {
            return false;
        };
        let mob = self.identity.target.id();
        game.guest_actor(self.conn) == Some(self.identity.actor)
            && host
                .menus
                .get(&self.conn)
                .is_some_and(|m| m.merchant.stamp() == self.stamp)
            && host
                .play
                .get(&self.conn)
                .and_then(|p| p.merchant.as_ref())
                .is_some_and(|gm| gm.window == self.window && gm.merchant.mob == Some(mob))
            && game
                .mobs
                .iter()
                .any(|m| m.id == mob && m.alive() && !m.removed)
            && guest.menu_generation == self.hub_generation
            && guest.creative() == self.creative
            && self.map_id.is_none_or(|id| game.peek_map_id() == id)
    }
}

struct TakePlan {
    menu: Menu,
    made: MadeMaps,
    player_slots: Vec<(usize, Option<ItemStack>)>,
    offhand: Option<Option<ItemStack>>,
}

// Keep typed snapshots in closures so no storage's private representation escapes its owner.
type Restore = Box<dyn FnOnce(&mut Game<'_>)>;
#[derive(Default)]
pub(in crate::game) struct Journal {
    guests: HashMap<u32, Restore>,
    mobs: HashMap<u32, Restore>,
    sequences: Option<sunrise_client::random_sequences::RandomSequences>,
    active: Option<u32>,
    pub(in crate::game) correct_on_abort: std::collections::BTreeSet<(u32, i32)>,
    pub(in crate::game) refused_clicks: std::collections::BTreeSet<(u32, i32)>,
}
impl Journal {
    pub(in crate::game) fn merge(&mut self, child: Self) {
        for (conn, kept) in child.guests {
            self.guests.entry(conn).or_insert(kept);
        }
        for (id, kept) in child.mobs {
            self.mobs.entry(id).or_insert(kept);
        }
        if self.sequences.is_none() {
            self.sequences = child.sequences;
        }
        self.correct_on_abort.extend(child.correct_on_abort);
    }
    pub(in crate::game) fn rollback(self, game: &mut Game<'_>) {
        for restore in self.guests.into_values() {
            restore(game);
        }
        for restore in self.mobs.into_values() {
            restore(game);
        }
        if let Some(sequences) = self.sequences {
            game.rand_sequences = sequences;
        }
    }
}

pub(in crate::game) enum Effect {
    Orb([f64; 3], u32),
    Drops(u32, Vec<sunrise_client::items::ItemStack>),
    Reward(u128, Advancement),
    Notice(u128, String, u32),
}

impl Game<'_> {
    pub(super) fn lan_merchant_click(&mut self, conn: u32, p: &play::ContainerClick) -> bool {
        let input = match (p.slot, p.input) {
            (2, 0) => Input::Pickup,
            (2, 1) => Input::QuickMove,
            (2, 2) => Input::Swap,
            (2, 4) => Input::Throw,
            _ => return self.lan_merchant_click_plain(conn, p),
        };
        if self.menu_click_in_progress(conn) {
            return true;
        }
        if !mod_api::__private::listened(mod_api::HookId::TradeWithVillager) {
            return self.lan_merchant_click_plain(conn, p);
        }
        let Some(mob) = self.lan.as_ref().and_then(|host| {
            let gm = host.play.get(&conn)?.merchant.as_ref()?;
            // A non-quick-craft click while a drag is active only resets the drag (`doClick`).
            (gm.window == p.container && !gm.drag.active())
                .then_some(gm.merchant.mob)
                .flatten()
        }) else {
            return self.lan_merchant_click_plain(conn, p);
        };
        if self.mobs.iter().find(|m| m.id == mob).is_some_and(|m| {
            opaque_villager(
                &self.stations().professions,
                m.species(),
                &m.extra.profession,
            )
        }) {
            return self.lan_merchant_click_plain(conn, p);
        }
        let Some(actor) = self.guest_actor(conn) else {
            self.resend_merchant_take(conn, p.container);
            return true;
        };
        let identity = Identity {
            side: Side::Server,
            world: actor.epoch(),
            actor,
            target: mod_api::EntityRef::new(actor.epoch(), mob),
        };
        let route = self
            .lan
            .as_mut()
            .unwrap()
            .menus
            .entry(conn)
            .or_default()
            .merchant
            .id();
        let seal = self.transaction_open();
        self.journal_guest_merchant(conn);
        let level = self.transactions.last_mut().unwrap();
        level.trades.active = Some(conn);
        level.trades.correct_on_abort.insert((conn, p.container));
        if level.packets.is_none() {
            level.packets = Some(Scope::begin());
        }
        // The coordinator contains listener unwinds; this guard also closes the click if a
        // lower menu rule panics during planning or a provisional guest turn unwinds.
        let failures = mod_api::runtime::isolated_failures();
        let accepted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.run_merchant_take(conn, p, input, identity, route)
        }))
        .unwrap_or(false)
            && mod_api::runtime::isolated_failures() == failures;
        if Scope::refused() {
            log::warn!(
                "trade_with_villager: merchant packet batch exceeded its retained-byte budget"
            );
        }
        self.transaction_close(&seal, accepted && !Scope::refused());
        true
    }

    /// Screen replacement cannot publish while a transaction owns a click. The older
    /// merchant guard protects its own guest; a scoped mod click can reach any guest's menu.
    pub(super) fn menu_click_in_progress(&self, conn: u32) -> bool {
        self.transactions
            .iter()
            .any(|level| level.menu_click_active.is_some() || level.trades.active == Some(conn))
    }

    fn run_merchant_take(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
        input: Input,
        identity: Identity,
        route: super::transactions::decision::MenuId,
    ) -> bool {
        let mob = identity.target.id();
        let button = i32::from(p.button);
        let mut stacked = None::<StackedOutcome>;
        if input == Input::Pickup && Self::stacked_listened() {
            let mut seen = None;
            self.lan_merchant_op_with(
                conn,
                Some(p.container),
                false,
                MerchantRun::Probe,
                |menu, env| {
                    seen = menu.stacked_click(env, 2, button, input);
                },
            );
            if let Some(frozen) = seen.filter(Self::stacked_listens_for) {
                let host = self.lan.as_ref().unwrap();
                let stamp = host.menus[&conn].merchant.stamp();
                let hub_generation = host.hub.guest(conn).expect("planned guest").menu_generation;
                // The shared pickup decision checks its journal epoch as well as these stacks.
                let (same, resolved_slot, primary) = (frozen.clone(), frozen.slot, frozen.primary);
                if self.stacked_pickup(
                    route,
                    frozen,
                    |game| {
                        let host = game.lan.as_ref()?;
                        let guest = host.hub.guest(conn)?;
                        let gm = host.play.get(&conn)?.merchant.as_ref()?;
                        (game.guest_actor(conn) == Some(identity.actor)
                            && host.menus.get(&conn)?.merchant.stamp() == stamp
                            && gm.window == p.container
                            && gm.merchant.mob == Some(mob)
                            && guest.menu_generation == hub_generation
                            && gm.slots[2] == same.clicked
                            && guest.carried == same.carried)
                            .then(|| same.clone())
                    },
                    |game, carried, clicked| {
                        let mut took = false;
                        let _ = game.lan_merchant_op_with(
                            conn,
                            Some(p.container),
                            false,
                            MerchantRun::Probe,
                            |menu, env| {
                                took = menu.probe_carried_vanilla_override(
                                    env,
                                    resolved_slot,
                                    primary,
                                    carried,
                                    clicked,
                                );
                            },
                        );
                        took
                    },
                    |_, answer| stacked = answer.cloned(),
                ) == Pickup::Refused
                {
                    return false;
                }
            }
        }

        let mut previous = None;
        let mut throw_amount = 1;
        let mut edited = false;
        let mut traded = false;
        let mut refresh = false;
        loop {
            let Some(host) = self.lan.as_ref() else {
                return false;
            };
            let Some(guest) = host.hub.guest(conn) else {
                return false;
            };
            let Some(gm) = host.play.get(&conn).and_then(|p| p.merchant.as_ref()) else {
                return false;
            };
            let Some(owner) = host.menus.get(&conn) else {
                return false;
            };
            let mut reads = TakeReads {
                conn,
                window: p.container,
                identity,
                stamp: owner.merchant.stamp(),
                hub_generation: guest.menu_generation,
                creative: guest.creative(),
                map_id: None,
            };
            if input == Input::Throw && button != 0 && previous.is_none() {
                throw_amount = gm.slots[2].as_ref().map_or(0, |stack| stack.count);
            }
            let run = stacked
                .as_ref()
                .map_or(MerchantRun::Plain, |s| MerchantRun::Decided(Some(s)));
            let mut moved = None;
            let map_id = self.peek_map_id();
            let Some((menu, made)) = self.read_merchant_menu(conn, gm, run, |menu, env| {
                moved =
                    menu.merchant_result_step(env, button, input, previous.as_ref(), throw_amount);
            }) else {
                return false;
            };
            if stacked.as_ref().is_some_and(|s| s.refused.get()) {
                log::warn!(
                    "item_stacked_on_other: the slot refused the replacement; nothing changed"
                );
                return false;
            }
            if !made.is_empty() {
                reads.map_id = Some(map_id);
            }
            let session = menu.merchant.as_ref().expect("planned merchant");
            refresh |= session
                .events
                .iter()
                .any(|event| matches!(event, Event::OffersRefreshed));
            let trade = session.events.iter().find_map(|event| match event {
                Event::Traded(index, offer) => Some((*index, offer.clone())),
                _ => None,
            });
            let player_slots = (0..36)
                .filter(|&slot| menu.player_item(slot) != guest.inventory[slot].as_ref())
                .map(|slot| (slot, menu.player_item(slot).cloned()))
                .collect();
            let offhand = (menu.player_item(40) != guest.offhand.as_ref())
                .then(|| menu.player_item(40).cloned());
            let plan = TakePlan {
                menu,
                made,
                player_slots,
                offhand,
            };
            if !reads.fresh(self) {
                return false;
            }
            let Some((index, offer)) = trade else {
                // Untaken/handled cases can still have a cursor change, but no trade hook.
                self.apply_merchant_take(&reads, plan);
                break;
            };
            let plan = RefCell::new(Some(plan));
            let mut changed = false;
            let accepted = self.decided_action_on(
                "trade_with_villager",
                DecidedTarget { route, identity },
                Epoch::Player,
                |game| reads.fresh(game),
                |game| {
                    let mut slot = plan.borrow_mut();
                    let plan = slot.as_mut().expect("frozen trade");
                    let crafted = std::mem::take(&mut plan.menu.crafted_counts);
                    if !crafted.is_empty() {
                        game.as_guest(conn, |g| {
                            for (item, count) in crafted {
                                g.award_stat("crafted", &item, count);
                            }
                        });
                    }
                    let kind = game
                        .mobs
                        .iter()
                        .find(|m| m.id == mob)
                        .map(super::native_hooks::mob_kind);
                    let Some(kind) = kind else {
                        return Reply::Failed;
                    };
                    let mut answer = (*offer).clone();
                    let Some(outcome) = game.as_guest(conn, |g| {
                        g.record_merchant_trade(Some(mob), &offer.result);
                        g.native_trade_with_villager(mob, &kind, &mut answer)
                    }) else {
                        return Reply::Failed;
                    };
                    match outcome {
                        Ok(true) => {
                            let session = plan.menu.merchant.as_mut().expect("planned session");
                            // A level-up may recalculate all prices after the Traded snapshot.
                            // Preserve it when the listener did not edit this special price.
                            if answer.special_price_diff == offer.special_price_diff {
                                answer.special_price_diff =
                                    session.offers[index].special_price_diff;
                            }
                            session.offers[index] = answer;
                            changed = true;
                            plan.menu.merchant_update_result(game.items());
                            Reply::Allow
                        }
                        Ok(false) => Reply::Allow,
                        Err(reason) => Reply::Invalid(reason),
                    }
                },
                |game| game.apply_merchant_take(&reads, plan.take().expect("one frozen take")),
            );
            if !accepted || Scope::refused() {
                return false;
            }
            traded = true;
            edited |= changed;
            if moved.is_none() {
                break;
            }
            previous = moved;
        }
        if traded && let Some(guest) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) {
            self.after_commit(After::Trade(Box::new(Effect::Notice(
                guest.uuid,
                guest.name.clone(),
                mob,
            ))));
        }
        self.send_merchant_take(conn, p.container, refresh || edited);
        true
    }

    /// Install only the writes calculated by the plan.
    fn apply_merchant_take(&mut self, reads: &TakeReads, mut plan: TakePlan) {
        let conn = reads.conn;
        let mob = reads.identity.target.id();
        self.journal_guest_merchant(conn);
        // The crafted stats run in fire for a take and are already consumed in that case.
        let crafted = std::mem::take(&mut plan.menu.crafted_counts);
        if !crafted.is_empty() {
            self.as_guest(conn, |g| {
                for (item, count) in crafted {
                    g.award_stat("crafted", &item, count);
                }
            });
        }
        self.register_exploration_maps(plan.made);
        let (session, _) = self.merchant_apply(
            plan.menu.merchant.take().expect("planned merchant"),
            Some(mob),
        );
        let host = self.lan.as_mut().expect("validated merchant host");
        let gi = host
            .hub
            .guests
            .connection_index(conn)
            .expect("validated guest");
        host.hub.guests.update(gi, |guest| {
            for (slot, item) in plan.player_slots {
                guest.inventory[slot] = item;
            }
            if let Some(offhand) = plan.offhand {
                guest.offhand = offhand;
            }
            guest.carried = plan.menu.carried.take();
        });
        host.menus.get_mut(&conn).unwrap().merchant.restore(());
        let gm = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.merchant.as_mut())
            .expect("validated merchant window");
        gm.slots = std::array::from_fn(|slot| plan.menu.slot(slot).cloned());
        gm.drag = plan.menu.quick_craft_state();
        gm.merchant.session = session;
        self.merchant_drops(conn, std::mem::take(&mut plan.menu.dropped));
    }

    /// Any write to an open merchant menu, including a MerchantRef edit to another guest,
    /// belongs to the transaction that first changed it.
    pub(super) fn journal_guest_merchant(&mut self, conn: u32) {
        let Some(level) = self.transactions.last() else {
            return;
        };
        if level.trades.guests.contains_key(&conn) {
            return;
        }
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let Some(play) = host.play.get(&conn) else {
            return;
        };
        let Some(merchant) = play.merchant.clone() else {
            return;
        };
        let stats = play.stats.clone();
        let progress = g.advancements.clone();
        let changed = g.advancements_changed.clone();
        let unlocked = g.recipes_unlocked.clone();
        let book = g.recipe_book.clone();
        self.journal_guest_click(conn);
        let level = self.transactions.last_mut().expect("the action is open");
        level.trades.guests.insert(
            conn,
            Box::new(move |game| {
                if let Some(host) = game.lan.as_mut() {
                    host.menus.entry(conn).or_default().merchant.restore(());
                    let play = host.play.entry(conn).or_default();
                    play.merchant = Some(merchant);
                    play.stats = stats;
                    if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
                        host.hub.guests.update(gi, |g| {
                            g.advancements = progress;
                            g.advancements_changed = changed;
                            g.recipes_unlocked = unlocked;
                            g.recipe_book = book;
                        });
                    }
                }
            }),
        );
    }

    /// Keep the fields merchant operations own, not the mob's health, equipment or effects,
    /// which belong to their owning journals.
    pub(super) fn journal_merchant_mob(&mut self, id: u32) {
        let Some(level) = self.transactions.last_mut() else {
            return;
        };
        let std::collections::hash_map::Entry::Vacant(entry) = level.trades.mobs.entry(id) else {
            return;
        };
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let random = m.random.clone();
        let timer = m.ambient_timer;
        let trade = m.extra.trade.clone();
        let profession = m.extra.profession.clone();
        let level = m.extra.villager_level;
        let variant = m.extra.variant;
        let villager = m.extra.villager.clone();
        entry.insert(Box::new(move |game| {
            if let Some(m) = game.mobs.iter_mut().find(|m| m.id == id) {
                m.random = random;
                m.ambient_timer = timer;
                m.extra.trade = trade;
                m.extra.profession = profession;
                m.extra.villager_level = level;
                m.extra.variant = variant;
                m.extra.villager = villager;
            }
        }));
    }

    /// Keep trade-set sequence loans and draws at their first transactional write.
    pub(super) fn journal_trade_sequences(&mut self) {
        if let Some(level) = self.transactions.last_mut() {
            level
                .trades
                .sequences
                .get_or_insert_with(|| self.rand_sequences.clone());
        }
    }

    pub(in crate::game) fn resend_merchant_take(&mut self, conn: u32, window: i32) {
        self.send_merchant_take(conn, window, true);
    }

    fn send_merchant_take(&mut self, conn: u32, window: i32, offers: bool) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some(open) = host.play.get_mut(&conn).map(|p| &mut p.merchant) else {
            return;
        };
        if open.as_ref().is_none_or(|gm| gm.window != window) {
            return;
        }
        let mut gm = open.take().expect("matching window");
        host.menus.entry(conn).or_default().merchant.restore(());
        if offers {
            self.lan_send_offers(conn, &mut gm, true);
        }
        self.lan_send_merchant_content(conn, &mut gm);
        if let Some(host) = self.lan.as_mut() {
            host.play.entry(conn).or_default().merchant = Some(gm);
        }
    }

    pub(super) fn merchant_take_active(&self) -> bool {
        self.transactions.iter().any(|l| l.packets.is_some())
    }

    pub(super) fn merchant_drops(
        &mut self,
        conn: u32,
        stacks: Vec<sunrise_client::items::ItemStack>,
    ) {
        if self.merchant_take_active() {
            if !stacks.is_empty() {
                self.after_commit(After::Trade(Box::new(Effect::Drops(conn, stacks))));
            }
        } else if let Some(gi) = self
            .lan
            .as_ref()
            .and_then(|h| h.hub.guests.iter().position(|g| g.conn == conn))
        {
            self.drop_guest_items(gi, stacks);
        }
    }

    pub(super) fn merchant_orb(&mut self, at: [f64; 3], value: u32) {
        if self.merchant_take_active() {
            self.after_commit(After::Trade(Box::new(Effect::Orb(at, value))));
        } else {
            self.add_orb(at, value);
        }
    }
    pub(super) fn merchant_sound(&mut self, sound: &'static str, at: [f64; 3], pitch: f32) {
        if self.merchant_take_active() {
            self.defer_effect(super::transactions::DeferredEffect::Sound {
                sound,
                at,
                volume: 1.0,
                pitch,
            });
        } else {
            self.audio.play(sound, Some(at), 1.0, pitch);
        }
    }

    // The criterion and its listener run tentatively. Rewards can spawn entities and run
    // arbitrary functions, so their publication belongs to the successful root commit.
    pub(super) fn defer_merchant_reward(&mut self, advancement: &Advancement) -> bool {
        if !self.merchant_take_active() {
            return false;
        }
        let uuid = self.native_acting_id();
        self.after_commit(After::Trade(Box::new(Effect::Reward(
            uuid,
            advancement.clone(),
        ))));
        true
    }
    pub(in crate::game) fn commit_merchant_effect(&mut self, effect: Effect) {
        match effect {
            Effect::Notice(uuid, name, mob) => {
                // A completed trade cannot be undone if the notification fails.
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.native_player_trade(uuid, &name, mob);
                }))
                .is_err()
                {
                    log::warn!("player_trade: a completion listener failed after commit");
                }
            }
            Effect::Orb(at, value) => self.add_orb(at, value),
            Effect::Drops(conn, stacks) => {
                if let Some(gi) = self
                    .lan
                    .as_ref()
                    .and_then(|h| h.hub.guests.iter().position(|g| g.conn == conn))
                {
                    self.drop_guest_items(gi, stacks);
                }
            }
            Effect::Reward(uuid, advancement) => {
                self.native_as_player(uuid, |game| game.commit_merchant_reward(&advancement));
            }
        }
    }
}
