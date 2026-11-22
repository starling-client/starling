//! A hub container click the game owns. `Hub::container_click` used to run inside
//! `self.lan.as_mut()`, so nothing it called could reach the game: a hook fired in the middle
//! of a click had no `&Game` to be given. Here the game lifts the host out of itself for the
//! click, so the click code and the game are separate values and the game is whole while the
//! hub runs.
use super::*;
use crate::game::{lan_menus::ClickReply, stacked_on_other::Pickup};
use std::cell::RefCell;
use sunrise_client::menu::StackedOutcome;

impl Game<'_> {
    /// Run `f` with the LAN host taken out of `self.lan`, then put it back. `f` is given the whole
    /// game (`self.lan` is `None` while it runs) and the host. The host is restored on every
    /// path, including a panic in `f`, which is then propagated: no borrow of the host outlives
    /// the call and the game is not left without it. `None` when there is no host to lift,
    /// which includes a call made while the host is already lifted.
    pub(in crate::game) fn with_lan_lifted<R>(
        &mut self,
        f: impl FnOnce(&mut Self, &mut LanHost) -> R,
    ) -> Option<R> {
        let mut host = self.lan.take()?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self, &mut host)));
        debug_assert!(self.lan.is_none(), "a lifted LAN host was replaced");
        self.lan = Some(host);
        match result {
            Ok(result) => Some(result),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// `ServerGamePacketListenerImpl.handleContainerClick` for the guest `conn`. The hub reads
    /// what the click may do and the container it clicks; a pickup the mods listen to is then
    /// decided with the game whole (`stacked_pickup`: they are fired with `HostSide`, the LAN host
    /// still in place); and the click itself runs with the host lifted out of the game. Returns
    /// the stacks the click threw out, or `None` for a refused click.
    pub(super) fn lan_hub_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
    ) -> Option<Vec<ItemStack>> {
        let host = self.lan.as_mut()?;
        let mut prepared = host.hub.prepare_click(conn, p, &self.world, false)?;
        let frozen = if Self::stacked_listened() {
            host.hub
                .stacked_click(&mut prepared, p)
                .filter(Self::stacked_listens_for)
        } else {
            None
        };
        let route = host.menus.entry(conn).or_default().hub.id();
        // The block entities behind the container the click writes into.
        let blocks: Vec<_> = host
            .hub
            .guest(conn)
            .and_then(|g| g.menu.as_ref())
            .map(|m| match m.slots {
                Slots::Block(key) => vec![key],
                Slots::Double(first, second) => vec![first, second],
                _ => Vec::new(),
            })
            .unwrap_or_default();
        let prepared = RefCell::new(Some(prepared));
        let outcome = RefCell::new(None);
        if let Some(frozen) = frozen {
            let shared = &prepared;
            let outcome_ref = &outcome;
            let (stacks, slot, primary) = (frozen.clone(), frozen.slot, frozen.primary);
            let picked = self.stacked_pickup(
                route,
                frozen,
                move |game| {
                    let (carried, clicked) =
                        game.lan
                            .as_ref()?
                            .hub
                            .click_stacks(conn, slot, &game.world)?;
                    (carried == stacks.carried && clicked == stacks.clicked).then(|| stacks.clone())
                },
                |game, carried, clicked| {
                    game.with_lan_lifted(|game, host| {
                        let mut prepared = prepared.borrow_mut();
                        let Some(prepared) = prepared.as_mut() else {
                            return false;
                        };
                        let rules = ItemRules(
                            world_items!(game),
                            &world_stations!(game).enchantments,
                            Some(&*game),
                            None,
                            prepared.container_insert_rules().to_vec(),
                        );
                        host.hub.probe_carried_vanilla_override(
                            prepared, slot, primary, carried, clicked, &rules,
                        )
                    })
                    .unwrap_or(false)
                },
                move |game, answer| {
                    // What the click writes belongs to the action: the guest's inventory and
                    // cursor, and the container's block entities.
                    game.journal_guest_click(conn);
                    for key in blocks {
                        game.journal_block_entity(key);
                    }
                    *outcome_ref.borrow_mut() = game.hub_click_run(p, shared, answer);
                },
            );
            if matches!(picked, Pickup::Refused) {
                self.reply_menu_click(conn, ClickReply::Resend(p.container));
                return None;
            }
        } else {
            // Plain hub routing writes the parked guest's inventory and cursor directly.
            // A surrounding transaction must keep that storage before the hub takes it.
            if !self.transactions.is_empty() {
                self.journal_parked_player(self.guest_index(conn)?);
            }
            for key in blocks {
                self.journal_block_entity(key);
            }
            *outcome.borrow_mut() = self.hub_click_run(p, &prepared, None);
        }
        let (drops, changed) = outcome.into_inner()?;
        self.reply_predicted_menu_click(conn, p, changed);
        Some(drops)
    }

    /// Run the prepared click with the LAN host lifted out of the game, the game lent to it as
    /// the mods' `menu_slot_rules` (their read-only `Host`; what it reads of the LAN host answers
    /// as if none were open) and the mods' answer to the pickup, if they gave one.
    fn hub_click_run(
        &mut self,
        p: &play::ContainerClick,
        prepared: &RefCell<Option<sunrise_client::lan::containers::PreparedClick>>,
        answer: Option<&StackedOutcome>,
    ) -> Option<(Vec<ItemStack>, Vec<usize>)> {
        let mut click = prepared.take()?;
        self.with_lan_lifted(|game, host| {
            let rules = ItemRules(
                world_items!(game),
                &world_stations!(game).enchantments,
                Some(&*game),
                answer,
                click.container_insert_rules().to_vec(),
            );
            let outcome = host.hub.run_click(&mut click, p, &rules);
            host.hub.write_click(click, &mut game.world);
            outcome
        })
    }
}
