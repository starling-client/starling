//! One decision for a hook that is not a pickup click (`stacked_on_other.rs` is the pickup's): the
//! mod menu's shift-click (`menu_quick_move`) and the click on a menu a mod opened
//! (`menu_clicked`). The same coordinator, the same savepoint, the same refusal.
//!
//! With no listener the caller makes the ordinary call and nothing here runs. With one, the game
//! opens an action savepoint, fires the hook, checks that nothing the action read was written
//! while the listener ran (`Epoch`), and only then performs the action, inside the same
//! savepoint. A stale read, an invalid answer, a panicking listener and a busy menu leave it
//! undone with every write the listener made rolled back.
use super::*;
use crate::game::transactions::decision::{
    self, ActionTarget, GameHost, Identity, InvalidReason, MenuId, Outcome, Reply, Side,
    StaleReason,
};
use mod_api::EntityRef;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// What a decision reads of the journals.
pub(in crate::game) enum Epoch<'p> {
    /// Players' state (inventory, cursor, menu storage). The epoch is one count for all players,
    /// so a write to any player's state makes the decision stale, not only the acting player's: a
    /// cautious refusal, never a partial move. A write to another block, which a mod's move
    /// legitimately makes, does not make the decision stale.
    Player,
    /// A continued click must keep its player read set; a replacement owns those writes.
    Click(&'p Cell<bool>),
}

/// The decision's menu owner and the lifecycle identity of its actor and target.
pub(in crate::game) struct DecidedTarget {
    pub(in crate::game) route: MenuId,
    pub(in crate::game) identity: Identity,
}

#[cfg(test)]
thread_local! {
    /// How many times `decided_action` was entered: the no-listener tests prove it stays zero.
    pub(in crate::game) static DECIDED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

type Fresh<'p> = Box<dyn Fn(&Game<'_>) -> bool + 'p>;
type Perform<'p> = Box<dyn FnOnce(&mut Game<'_>) + 'p>;

struct Shared<'p> {
    epoch: Epoch<'p>,
    fresh: Fresh<'p>,
    perform: RefCell<Option<Perform<'p>>>,
}

#[derive(Clone)]
struct Action<'p> {
    id: MenuId,
    shared: Rc<Shared<'p>>,
}

struct Reads {
    player_writes: u64,
}

struct Plan;

impl<'a, 'g> ActionTarget<GameHost<'a, 'g>> for Action<'_> {
    type Reads = Reads;
    type View = Reads;
    type Plan = Plan;
    type Frozen = Plan;

    fn id(&self) -> MenuId {
        self.id
    }
    fn capture(&self, host: &GameHost<'a, 'g>) -> std::result::Result<Reads, Outcome> {
        Ok(Reads {
            player_writes: host.game.transactions.player_writes(),
        })
    }
    fn view(reads: &Reads) -> &Reads {
        reads
    }
    fn freeze(&self, _: &Reads, plan: Plan) -> std::result::Result<Plan, InvalidReason> {
        Ok(plan)
    }
    fn empty(_: &Plan) -> bool {
        false
    }
    fn validate(&self, host: &GameHost<'a, 'g>, reads: &Reads) -> std::result::Result<(), Outcome> {
        let check_player_writes = match self.shared.epoch {
            Epoch::Player => true,
            Epoch::Click(handled) => !handled.get(),
        };
        if check_player_writes && host.game.transactions.player_writes() != reads.player_writes {
            return Err(Outcome::Stale(StaleReason::Mutation));
        }
        if (self.shared.fresh)(host.game) {
            Ok(())
        } else {
            Err(Outcome::Stale(StaleReason::Snapshot))
        }
    }
    fn apply(&self, host: &mut GameHost<'a, 'g>, _: Plan) {
        let perform = self.shared.perform.take().expect("an action runs once");
        perform(host.game);
    }
}

impl Game<'_> {
    /// Decide an action `name` on the menu `route` for the player entity `actor`: `fire` calls the
    /// hook and validates what it answered (`Reply::Allow`, or `Invalid` to refuse), `fresh` says
    /// the menu is still the one the action froze, and `perform` is the action, run once inside the
    /// savepoint when the decision stands. The result says whether it did.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn decided_action<'p>(
        &mut self,
        name: &'static str,
        route: MenuId,
        actor: EntityRef,
        epoch: Epoch<'p>,
        fresh: impl Fn(&Game<'_>) -> bool + 'p,
        fire: impl FnOnce(&mut Game<'_>) -> Reply,
        perform: impl FnOnce(&mut Game<'_>) + 'p,
    ) -> bool {
        self.decided_action_on(
            name,
            DecidedTarget {
                route,
                identity: Identity {
                    side: Side::Server,
                    world: actor.epoch(),
                    actor,
                    target: actor,
                },
            },
            epoch,
            fresh,
            fire,
            perform,
        )
    }

    /// Run a decision whose action target is distinct from its acting player.
    pub(in crate::game) fn decided_action_on<'p>(
        &mut self,
        name: &'static str,
        target: DecidedTarget,
        epoch: Epoch<'p>,
        fresh: impl Fn(&Game<'_>) -> bool + 'p,
        fire: impl FnOnce(&mut Game<'_>) -> Reply,
        perform: impl FnOnce(&mut Game<'_>) + 'p,
    ) -> bool {
        let DecidedTarget { route, identity } = target;
        #[cfg(test)]
        DECIDED.set(DECIDED.get() + 1);
        let target = Action {
            id: route,
            shared: Rc::new(Shared {
                epoch,
                fresh: Box::new(fresh),
                perform: RefCell::new(Some(Box::new(perform))),
            }),
        };
        // The action's savepoint needs a transaction to nest in: the open one, or this one.
        let seal = self.transaction_open();
        let outcome = {
            let mut host = GameHost {
                game: &mut *self,
                identity,
            };
            decision::run(&mut host, &target, |_, _| Plan, |host| fire(host.game))
        };
        self.transaction_close(&seal, matches!(outcome, Ok(Outcome::Applied)));
        match outcome {
            Ok(Outcome::Applied) => true,
            // A cancelled action is the listener's own answer, not a refusal: the caller goes on.
            Ok(Outcome::Canceled) => false,
            Ok(other) => {
                log::warn!("{name}: the action was refused ({other:?}); nothing changed");
                false
            }
            Err(admission) => {
                log::warn!("{name}: the action was not admitted ({admission:?}); nothing changed");
                false
            }
        }
    }

    /// The player entity of the guest `conn`, as the decision names it.
    pub(in crate::game) fn guest_actor(&self, conn: u32) -> Option<EntityRef> {
        let epoch = self.native_lifecycle.epoch()?;
        let entity = self.lan.as_ref()?.hub.guest(conn)?.entity_id;
        Some(EntityRef::new(epoch, u32::try_from(entity).ok()?))
    }
}

impl Game<'_> {
    /// A scoped click decision fully consumes the packet. Unsubscribed layouts use the same
    /// default router without opening an action or allocating its read set.
    pub(in crate::game) fn menu_clicked_decided(
        &mut self,
        conn: u32,
        click: &sunrise_protocol::packets::play::ContainerClick,
    ) -> bool {
        use super::lan_menus::ClickReply;
        use sunrise_client::lan::menu::Slots;
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuClicked) {
            return false;
        }
        let Some(guest) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) else {
            return false;
        };
        let Some(open) = guest.menu.as_ref().filter(|m| {
            m.window == click.container && (m.mod_type.is_some() || m.slots == Slots::Native)
        }) else {
            return false;
        };
        if guest.spectator() {
            return false;
        }
        if usize::try_from(click.slot).is_ok_and(|slot| slot >= open.slot_count()) {
            return true;
        }
        if !mod_api::__private::menu_listens(mod_api::HookId::MenuClicked, &open.registry_id) {
            return false;
        }
        let (uuid, generation, state, slots) =
            (guest.uuid, guest.menu_generation, open.state, open.slots);
        let menu = open.registry_id.clone();
        let items = slots.read(&self.world, guest);
        let drag = open.drag.clone();
        let craft = open.craft.clone();
        let (selected, mode, creative, pos) = (
            guest.selected,
            guest.game_mode,
            guest.creative(),
            guest.position,
        );
        let armor = guest.armor.clone();
        let inventory = guest.inventory.clone();
        let offhand = guest.offhand.clone();
        let carried = guest.carried.clone();
        let capture_block =
            |game: &Game<'_>, guest: &sunrise_client::lan::guest::Guest, pos: (i32, i32, i32)| {
                (
                    game.world.block_entities.generation(&pos),
                    game.world.block(pos.0, pos.1, pos.2),
                    Slots::Block(pos).read(&game.world, guest),
                )
            };
        let blocks: Vec<_> = slots
            .halves()
            .chain(open.links.iter().copied())
            .map(|pos| (pos, capture_block(self, guest, pos)))
            .collect();
        let redo = ClickReply::Resend(click.container);
        let Some(actor) = self.guest_actor(conn) else {
            log::warn!("menu_clicked: the clicking player's action identity is unavailable");
            self.reply_menu_click(conn, redo);
            return true;
        };
        let route = self
            .lan
            .as_mut()
            .unwrap()
            .menus
            .entry(conn)
            .or_default()
            .click
            .id();
        let handled = Cell::new(false);
        let default_ok = Cell::new(false);
        // `decided_action` gives the hook and router one savepoint. Its outer transaction
        // also has the router's outcome, so a nested refused default rolls the hook back.
        let seal = self.transaction_open();
        let decided = self.decided_action(
            "menu_clicked",
            route,
            actor,
            Epoch::Click(&handled),
            |game| {
                let Some(guest) = game.lan.as_ref().and_then(|host| host.hub.guest(conn)) else {
                    return false;
                };
                let live = !guest.dead
                    && !guest.disconnect_pending
                    && guest.menu_generation == generation
                    && guest.menu.as_ref().is_some_and(|m| {
                        m.window == click.container
                            && m.state == state
                            && m.drag == drag
                            && m.craft == craft
                    });
                live && (handled.get()
                    || ((
                        guest.selected,
                        guest.game_mode,
                        guest.creative(),
                        guest.position,
                    ) == (selected, mode, creative, pos)
                        && guest.inventory == inventory
                        && guest.armor == armor
                        && guest.offhand == offhand
                        && guest.carried == carried
                        && slots.read(&game.world, guest) == items
                        && blocks
                            .iter()
                            .all(|(pos, old)| capture_block(game, guest, *pos) == *old)))
            },
            |game| {
                game.transactions.last_mut().unwrap().menu_click_active = Some(conn);
                let mut skip_default = false;
                game.native_menu_clicked(mod_api::__private::MenuClickedCall {
                    menu: &menu,
                    uuid,
                    window: click.container,
                    slot: i32::from(click.slot),
                    button: i32::from(click.button),
                    mode: click.input,
                    handled: &mut skip_default,
                });
                handled.set(skip_default);
                Reply::Allow
            },
            |game| {
                if handled.get() {
                    default_ok.set(true);
                } else {
                    game.journal_guest_click(conn);
                    game.journal_menu_click(conn);
                    default_ok.set(game.lan_default_container_click(conn, click));
                }
            },
        );
        let applied = decided && default_ok.get();
        self.transaction_close(&seal, applied);
        if !applied || handled.get() {
            // The client predicted the unmodified click. Compare neither IDs nor component
            // hashes here: a replacement may leave the same counts with different components.
            self.reply_menu_click(conn, redo);
        }
        true
    }
}
