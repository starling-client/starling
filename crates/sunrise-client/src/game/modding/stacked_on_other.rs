//! `item_stacked_on_other` on the game's side: a pickup click, decided, fired and applied as one
//! action (the design's decide/fire/apply).
//!
//! The click reads what the hook is told (the cursor and the clicked slot's stack) and freezes
//! it. With no listener the plain click runs at once, with nothing allocated. With one, the game
//! opens an action savepoint, fires the hook with `HostSide` and the frozen stacks, validates the
//! typed answer once, checks that nothing the click read was written meanwhile, and only then
//! runs the click with the answer, inside the same savepoint. A stale read, an invalid answer, a
//! panicking listener or a refusal to admit leaves the click undone and every write the listener
//! made rolled back; the click then does nothing (the existing no-change result) and the reason
//! goes to the log.
use super::*;
use crate::game::api_values::{TryIntoApi, TryIntoEngine};
use crate::game::transactions::decision::{
    self, ActionTarget, GameHost, Identity, InvalidReason, MenuId, Outcome, Reply, Side,
    StaleReason,
};
use mod_api::item::menu::{ClickAction, StackedEdits, StackedOnOther, StackedOnOtherDirection};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use sunrise_client::menu::{
    Env, Input, StackedClick, StackedEdit, StackedEditPolicy, StackedOutcome, StackedResume,
};

/// What a pickup click did: it ran, or the decision refused it and nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Pickup {
    Ran,
    Refused,
}

/// The click, run with the mods' answer (`None`: no listener, the plain click).
type Perform<'p> = Box<dyn FnOnce(&mut Game<'_>, Option<&StackedOutcome>) + 'p>;
/// What a click reads now, to compare with what it froze.
type Current<'p> = Box<dyn Fn(&Game<'_>) -> Option<StackedClick> + 'p>;

struct Shared<'p> {
    frozen: StackedClick,
    current: Current<'p>,
    perform: RefCell<Option<Perform<'p>>>,
    answer: RefCell<Option<StackedOutcome>>,
    /// A slot rule or a stale input refused the mods' edit (`StackedOutcome::refused`).
    refused: Cell<bool>,
}

/// The decision's target: an id that names the route's menu (a second click on it while this one
/// is active is refused as busy), and what the click froze.
#[derive(Clone)]
struct PickupTarget<'p> {
    id: MenuId,
    shared: Rc<Shared<'p>>,
}

/// What the click read: the journaled-write epoch before the listeners ran.
struct PickupReads {
    writes: u64,
}

/// The one thing a pickup decision plans: that the click runs. The write set is the click's own.
struct PickupPlan;

impl<'a, 'g> ActionTarget<GameHost<'a, 'g>> for PickupTarget<'_> {
    type Reads = PickupReads;
    type View = PickupReads;
    type Plan = PickupPlan;
    type Frozen = PickupPlan;

    fn id(&self) -> MenuId {
        self.id
    }
    fn capture(&self, host: &GameHost<'a, 'g>) -> std::result::Result<PickupReads, Outcome> {
        Ok(PickupReads {
            writes: host.game.transactions.writes(),
        })
    }
    fn view(reads: &PickupReads) -> &PickupReads {
        reads
    }
    fn freeze(
        &self,
        _: &PickupReads,
        plan: PickupPlan,
    ) -> std::result::Result<PickupPlan, InvalidReason> {
        Ok(plan)
    }
    fn empty(_: &PickupPlan) -> bool {
        false
    }
    /// The conservative read set: any write the journals were asked to keep since the click
    /// froze its stacks, even one that restored what was there, makes it stale; and what the
    /// click would read now must still be what it froze.
    fn validate(
        &self,
        host: &GameHost<'a, 'g>,
        reads: &PickupReads,
    ) -> std::result::Result<(), Outcome> {
        if host.game.transactions.writes() != reads.writes {
            return Err(Outcome::Stale(StaleReason::Mutation));
        }
        match (self.shared.current)(host.game) {
            Some(now) if now == self.shared.frozen => Ok(()),
            _ => Err(Outcome::Stale(StaleReason::Snapshot)),
        }
    }
    fn apply(&self, host: &mut GameHost<'a, 'g>, _: PickupPlan) {
        let perform = self.shared.perform.take().expect("a pickup runs once");
        let answer = self.shared.answer.take();
        perform(host.game, answer.as_ref());
        self.shared
            .refused
            .set(answer.is_some_and(|a| a.refused.get()));
    }
}

/// Where a pickup decision's menu is named: a route's own id, stable for as long as the route's
/// menu lives, so that a click made from inside the action finds it busy.
pub(in crate::game) fn local_pickup_id() -> MenuId {
    static ID: std::sync::OnceLock<MenuId> = std::sync::OnceLock::new();
    *ID.get_or_init(MenuId::fresh)
}

impl Game<'_> {
    /// Whether any server-side listener (global `Hooks::item_stacked_on_other` or `ItemPart`)
    /// answers `item_stacked_on_other`.
    pub(in crate::game) fn stacked_listened() -> bool {
        Self::stacked_listened_for_side(true)
    }

    /// Whether any listener active on `server` side answers `item_stacked_on_other`.
    ///
    /// Global `Hooks::item_stacked_on_other` runs on the authoritative server side only;
    /// `ItemPart::item_stacked_on_other` runs on both sides (including client prediction and
    /// creative inventory-tab/hotbar pickups).
    pub(in crate::game) fn stacked_listened_for_side(server: bool) -> bool {
        (server && mod_api::__private::listened(mod_api::HookId::ItemStackedOnOther))
            || mod_api::__private::has_part_hook(mod_api::HookId::ItemStackedOnOther)
    }

    /// Whether a server-side global listener or an `ItemPart` on either frozen stack subscribes to
    /// `item_stacked_on_other`.
    pub(in crate::game) fn stacked_listens_for(frozen: &StackedClick) -> bool {
        Self::stacked_listens_for_side(true, frozen)
    }

    /// Whether any listener active on `server` side subscribes to `frozen`: lets clicks on
    /// unrelated items skip the decision transaction.
    pub(in crate::game) fn stacked_listens_for_side(server: bool, frozen: &StackedClick) -> bool {
        (server && mod_api::__private::listened(mod_api::HookId::ItemStackedOnOther))
            || frozen.carried.as_ref().is_some_and(|stack| {
                mod_api::__private::item_listens(mod_api::HookId::ItemStackedOnOther, &stack.item)
            })
            || frozen.clicked.as_ref().is_some_and(|stack| {
                mod_api::__private::item_listens(mod_api::HookId::ItemStackedOnOther, &stack.item)
            })
    }

    /// The pickup click whose frozen stacks are `frozen`, decided by the mods and then run by
    /// `perform`, which is given their answer. `current` reads the same stacks again, for the
    /// check that nothing changed them. `probe_carried` runs the shared vanilla carried override
    /// (`vanilla_carried_override`) on a throwaway copy with the working stacks after any carried
    /// `ItemPart` declines, so `OtherStackedOnMe` only runs when the real carried override
    /// returned `false`. `route` names the menu clicked.
    ///
    /// With no listener on this side this is `perform(self, None)` and nothing else: no
    /// transaction, no decision, no allocation.
    pub(in crate::game) fn stacked_pickup<'p>(
        &mut self,
        route: MenuId,
        frozen: StackedClick,
        current: impl Fn(&Game<'_>) -> Option<StackedClick> + 'p,
        probe_carried: impl FnOnce(&mut Game<'_>, Option<ItemStack>, Option<ItemStack>) -> bool,
        perform: impl FnOnce(&mut Game<'_>, Option<&StackedOutcome>) + 'p,
    ) -> Pickup {
        let server = self.is_logical_server();
        if !Self::stacked_listened_for_side(server) {
            perform(self, None);
            return Pickup::Ran;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return Pickup::Refused;
        };
        let Some(entity) = frozen
            .player_entity_id
            .and_then(|id| u32::try_from(id).ok())
        else {
            return Pickup::Refused;
        };
        let actor = mod_api::EntityRef::new(epoch, entity);
        let identity = Identity {
            side: if server { Side::Server } else { Side::Client },
            world: epoch,
            actor,
            target: actor,
        };
        let shared = Rc::new(Shared {
            frozen,
            current: Box::new(current),
            perform: RefCell::new(Some(Box::new(perform))),
            answer: RefCell::new(None),
            refused: Cell::new(false),
        });
        let target = PickupTarget {
            id: route,
            shared: shared.clone(),
        };
        // The action's savepoint needs a transaction to nest in: the open one, or this one.
        let (outcome, ran) = self.storage_scope(|game, _| {
            let mut host = GameHost { game, identity };
            let fired = shared.clone();
            let outcome = decision::run(
                &mut host,
                &target,
                |_, _| PickupPlan,
                move |host| fire(host.game, &fired, probe_carried),
            );
            // A replacement the slot's rules refused is undone with everything the listeners wrote.
            let ran = matches!(outcome, Ok(Outcome::Applied)) && !shared.refused.get();
            ((outcome, ran), ran)
        });
        match outcome {
            Ok(Outcome::Applied) if ran => Pickup::Ran,
            Ok(Outcome::Applied) => {
                log::warn!("item_stacked_on_other: the menu refused the edit; nothing changed");
                Pickup::Refused
            }
            Ok(other) => {
                log::warn!(
                    "item_stacked_on_other: the click was refused ({other:?}); nothing changed"
                );
                Pickup::Refused
            }
            Err(admission) => {
                log::warn!(
                    "item_stacked_on_other: the click was not admitted ({admission:?}); nothing changed"
                );
                Pickup::Refused
            }
        }
    }
}

/// The click as phase `direction` is asked about it, lending `edits`.
fn click_for<'e>(
    frozen: &StackedClick,
    direction: StackedOnOtherDirection,
    edits: &'e mut StackedEdits,
) -> StackedOnOther<'e> {
    let action = if frozen.primary {
        ClickAction::Primary
    } else {
        ClickAction::Secondary
    };
    StackedOnOther::new(
        frozen.player,
        frozen.player_entity_id,
        frozen.slot,
        frozen.container,
        frozen.container_slot,
        action,
        direction,
        edits,
    )
}

/// The working cursor and clicked slot as the edits so far leave them, and the edits converted
/// for the menu.
struct Working {
    carried: Option<ItemStack>,
    clicked: Option<ItemStack>,
    edits: Vec<StackedEdit>,
}

impl Working {
    /// Validate and convert the edits a phase made, from edit `from` on: each stack it set must
    /// be a valid stack (a known item, a count within its maximum).
    fn take_phase(
        &mut self,
        game: &Game<'_>,
        lent: &StackedEdits,
        from: usize,
    ) -> std::result::Result<(), InvalidReason> {
        let items = game.items();
        for edit in lent.since(from) {
            let edit = match edit {
                mod_api::item::menu::StackedEdit::Carried(stack) => {
                    let stack = stack
                        .try_into_engine(items)
                        .map_err(|_| InvalidReason::Count)?;
                    self.carried = stack.clone();
                    StackedEdit::Carried(stack)
                }
                mod_api::item::menu::StackedEdit::Clicked(stack) => {
                    let stack = stack
                        .try_into_engine(items)
                        .map_err(|_| InvalidReason::Count)?;
                    self.clicked = stack.clone();
                    StackedEdit::Clicked(stack)
                }
                mod_api::item::menu::StackedEdit::Take(count) => {
                    if let Some(current) = self.clicked.as_mut() {
                        current.count = current
                            .count
                            .checked_sub(*count)
                            .ok_or(InvalidReason::Count)?;
                    }
                    if self.clicked.as_ref().is_some_and(|stack| stack.count == 0) {
                        self.clicked = None;
                    }
                    StackedEdit::Take(*count)
                }
            };
            self.edits.push(edit);
        }
        Ok(())
    }
}

/// Fire the global hook (server side only) and item-part overrides in vanilla
/// `AbstractContainerMenu.tryItemClickBehaviourOverride` order, lending the click's cursor and
/// slot to each and validating what each phase did. The accepted edits are kept for the click;
/// any bad answer refuses it.
fn fire(
    game: &mut Game<'_>,
    shared: &Shared<'_>,
    probe_carried: impl FnOnce(&mut Game<'_>, Option<ItemStack>, Option<ItemStack>) -> bool,
) -> Reply {
    let frozen = &shared.frozen;
    let (Ok(carried), Ok(clicked)) = (frozen.carried.try_into_api(), frozen.clicked.try_into_api())
    else {
        return Reply::Invalid(InvalidReason::Plan);
    };
    let mut lent = StackedEdits::new(carried, clicked);
    let mut working = Working {
        carried: frozen.carried.clone(),
        clicked: frozen.clicked.clone(),
        edits: Vec::new(),
    };
    let finish = |outcome: StackedOutcome| -> Reply {
        *shared.answer.borrow_mut() = Some(outcome);
        Reply::Allow
    };

    // 1. Global NeoForge `ItemStackedOnOtherEvent`, authoritative server side only, before either
    // item override. Its edits stand only on a cancelled, handled click, and it may not take.
    if game.is_logical_server() && mod_api::__private::listened(mod_api::HookId::ItemStackedOnOther)
    {
        let mut canceled = false;
        let mut click = click_for(frozen, StackedOnOtherDirection::Event, &mut lent);
        game.native_item_stacked_on_other(&mut click, &mut canceled);
        let handled = click.handled;
        let edited = !lent.since(0).is_empty();
        let took = lent
            .since(0)
            .iter()
            .any(|edit| matches!(edit, mod_api::item::menu::StackedEdit::Take(_)));
        if took || (edited && !(canceled && handled)) {
            return Reply::Invalid(InvalidReason::Plan);
        }
        if let Err(reason) = working.take_phase(game, &lent, 0) {
            return Reply::Invalid(reason);
        }
        if canceled {
            return finish(StackedOutcome {
                canceled,
                handled,
                policy: StackedEditPolicy::Global,
                resume: if handled {
                    StackedResume::Done
                } else {
                    StackedResume::Plain
                },
                edits: working.edits,
                ..StackedOutcome::default()
            });
        }
    }

    // An `ItemPart` phase: `Flow::Cancel` stops the click, `Flow::Continue` goes on with its edits.
    let part = |game: &mut Game<'_>,
                lent: &mut StackedEdits,
                working: &mut Working,
                item: &str,
                direction: StackedOnOtherDirection|
     -> std::result::Result<bool, InvalidReason> {
        let from = lent.since(0).len();
        let mut canceled = false;
        let mut click = click_for(frozen, direction, lent);
        game.item_stacked_on_other_part(item, &mut click, &mut canceled);
        working.take_phase(game, lent, from)?;
        Ok(canceled)
    };
    let mut any_part_ran = false;
    let part_policy = |any_part_ran: bool| {
        if any_part_ran {
            StackedEditPolicy::Part
        } else {
            StackedEditPolicy::Global
        }
    };

    // 2. Carried stack's `Item.overrideStackedOnOther`.
    if let Some(carried) = working.carried.clone()
        && mod_api::__private::item_listens(mod_api::HookId::ItemStackedOnOther, &carried.item)
    {
        any_part_ran = true;
        match part(
            game,
            &mut lent,
            &mut working,
            &carried.item,
            StackedOnOtherDirection::CarriedOnOther,
        ) {
            Ok(true) => {
                return finish(StackedOutcome {
                    canceled: true,
                    handled: true,
                    policy: StackedEditPolicy::Part,
                    resume: StackedResume::Done,
                    edits: working.edits,
                    ..StackedOutcome::default()
                });
            }
            Ok(false) => {}
            Err(reason) => return Reply::Invalid(reason),
        }
    }

    // 3. Vanilla's carried override (`BundleItem.overrideStackedOnOther`), on the stacks the
    // carried phase left, decides whether the clicked phase is asked.
    if probe_carried(game, working.carried.clone(), working.clicked.clone()) {
        return finish(StackedOutcome {
            policy: part_policy(any_part_ran),
            resume: StackedResume::VanillaCarried,
            edits: working.edits,
            ..StackedOutcome::default()
        });
    }

    // 4. Clicked stack's `Item.overrideOtherStackedOnMe`.
    if let Some(clicked) = working.clicked.clone()
        && mod_api::__private::item_listens(mod_api::HookId::ItemStackedOnOther, &clicked.item)
    {
        any_part_ran = true;
        match part(
            game,
            &mut lent,
            &mut working,
            &clicked.item,
            StackedOnOtherDirection::OtherStackedOnMe,
        ) {
            Ok(true) => {
                return finish(StackedOutcome {
                    canceled: true,
                    handled: true,
                    policy: StackedEditPolicy::Part,
                    resume: StackedResume::Done,
                    edits: working.edits,
                    ..StackedOutcome::default()
                });
            }
            Ok(false) => {}
            Err(reason) => return Reply::Invalid(reason),
        }
    }

    finish(StackedOutcome {
        policy: part_policy(any_part_ran),
        resume: StackedResume::VanillaClicked,
        edits: working.edits,
        ..StackedOutcome::default()
    })
}

/// What an applied click writes belongs to the action: an outer abort puts it back.
impl Game<'_> {
    /// Keep guest `conn`'s inventory, armor, off hand, equipment and cursor (and their crafting
    /// grid, ender chest and mod menu storage), as they are, in the open transaction.
    pub(in crate::game) fn journal_guest_click(&mut self, conn: u32) {
        let Some(uuid) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| g.uuid)
        else {
            return;
        };
        self.native_as_player(uuid, |game| game.journal_player(uuid));
        if let Some(player) = self.player(uuid) {
            self.journal_menus(player);
        }
    }
}

/// The local and creative menu's route (`menu_ops`).
impl Game<'_> {
    /// The clicks `menu_ops` was asked for, when they resolve to a pickup of a slot of the local
    /// or creative menu (a direct `Pickup` or a one-slot left/right `QuickCraft` drag) and a
    /// listener active on this side answers: `Some(whether it was handled)`. `None`: the plain
    /// clicks.
    pub(in crate::game) fn stacked_menu_ops(
        &mut self,
        ops: &[(Option<usize>, i32, Input)],
    ) -> Option<bool> {
        if self.remote.is_some() && self.screen != Screen::Creative {
            return None;
        }
        let server = self.is_logical_server();
        if !Self::stacked_listened_for_side(server) || ops.is_empty() {
            return None;
        }
        let is_single_pickup = matches!(ops, [(Some(slot), _, Input::Pickup)] if *slot != 200);
        let is_quick_craft = ops.iter().all(|&(_, _, input)| input == Input::QuickCraft)
            && !ops.iter().any(|&(slot, _, _)| slot == Some(200));
        if !is_single_pickup && !is_quick_craft {
            return None;
        }
        self.stacked_menu_gesture(ops)
    }

    /// Decide and execute a single pickup or quick-craft gesture on the open local/creative menu.
    fn stacked_menu_gesture(&mut self, ops: &[(Option<usize>, i32, Input)]) -> Option<bool> {
        let server = self.is_logical_server();
        let mut found = None;
        let mut mapped = Vec::new();
        let modeled = self.menu_session(|menu, env, to_menu| {
            let mut clicks = Vec::with_capacity(ops.len());
            for &(slot, button, input) in ops {
                let menu_slot = match slot {
                    Some(g) => {
                        if g == 200 && input != Input::QuickMove {
                            return None;
                        }
                        to_menu(g)? as i32
                    }
                    None => -999,
                };
                clicks.push((menu_slot, button, input));
            }
            let (last, prefix) = clicks.split_last()?;
            for &(slot, button, input) in prefix {
                if menu.click(env, slot, button, input).is_err() {
                    return Some(false);
                }
            }
            found = menu.stacked_click(env, last.0, last.1, last.2);
            mapped = clicks;
            Some(false)
        });
        let frozen = found
            .filter(|_| modeled)
            .filter(|frozen| Self::stacked_listens_for_side(server, frozen))?;
        let handled = Rc::new(std::cell::Cell::new(false));
        let result = handled.clone();
        let (carried, resolved_slot, primary) =
            (frozen.carried.clone(), frozen.slot, frozen.primary);
        let clicks = mapped;
        let pickup = self.stacked_pickup(
            local_pickup_id(),
            frozen.clone(),
            // What the click reads of the game itself: the cursor. The rest is the journals'.
            move |game| (game.carried == carried).then(|| frozen.clone()),
            |game, carried, clicked| {
                let mut took = false;
                let _ = game.menu_session(|menu, env, _| {
                    if let Some((_, prefix)) = clicks.split_last() {
                        for &(slot, button, input) in prefix {
                            if menu.click(env, slot, button, input).is_err() {
                                return Some(false);
                            }
                        }
                    }
                    took = menu.probe_carried_vanilla_override(
                        env,
                        resolved_slot,
                        primary,
                        carried,
                        clicked,
                    );
                    Some(false)
                });
                took
            },
            |game, answer| {
                // The click's own writes belong to the action: an outer abort puts them back.
                let player = game.native_acting_id();
                game.journal_player(player);
                if let Some(key) = game.container {
                    game.journal_block_entity(key);
                }
                result.set(game.menu_session(|menu, env, _| {
                    let Some((last, prefix)) = clicks.split_last() else {
                        return Some(false);
                    };
                    for &(slot, button, input) in prefix {
                        if menu.click(env, slot, button, input).is_err() {
                            return Some(false);
                        }
                    }
                    let env = Env {
                        stacked: answer,
                        ..*env
                    };
                    menu.click(&env, last.0, last.1, last.2)
                        .map(|()| true)
                        .ok()
                        .or(Some(false))
                }));
            },
        );
        // A refused click was handled: nothing of it is kept, and no other click is made.
        Some(pickup == Pickup::Refused || handled.get())
    }
}
