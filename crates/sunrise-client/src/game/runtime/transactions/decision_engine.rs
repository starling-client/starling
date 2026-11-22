//! Adapter to actual game-owned menu values. No synthetic slot translation and
//! no rules, recipe lookup or callbacks during validation or frozen write-back.
//!
//! The owners live in `LanHost::menus` (`lan_menus.rs`), outside the `GuestPlay` that
//! `as_guest` lifts, so they stay reachable during a guest's turn. This adapter is not
//! yet a complete LAN read set: guest/backing-storage and registry versions still need
//! to be bound before any subscribed production route may use it.
use super::{storage::GameHost, *};
use crate::game::Game;
use sunrise_client::menu::{Menu as EngineMenu, MutationStamp, Snapshot};

/// Names the guest's inventory owner, never a borrowed menu moved out for dispatch.
#[derive(Clone, Copy)]
pub(in crate::game) struct Target {
    conn: u32,
    id: MenuId,
}

pub(in crate::game) struct EngineReads {
    owner: OwnerStamp,
    value: MutationStamp,
    menu: EngineMenu,
}

/// The lower menu planner supplies already frozen values. None is no action,
/// which consumes the decision without dispatch. Effects remain in After.
pub(in crate::game) struct EnginePlan(pub Option<Snapshot>);

pub(super) struct KeptOwner {
    target: Target,
    value: Option<Box<EngineMenu>>,
}

impl Target {
    /// Check the active action before looking in a slot that an outer action
    /// might have lifted. A missing menu is never the reentrancy mechanism.
    pub(in crate::game) fn admit(game: &Game<'_>, conn: u32) -> Result<Self, Admission> {
        if game.transactions.iter().any(|level| {
            level
                .action
                .as_ref()
                .and_then(|action| level.actions.owners.get(&action.menu))
                .is_some_and(|kept| kept.target.conn == conn)
        }) {
            return Err(Admission::Busy);
        }
        let menus = game
            .lan
            .as_ref()
            .and_then(|host| host.menus.get(&conn))
            .ok_or(Admission::MissingHolder)?;
        Ok(Self {
            conn,
            id: menus.crafting.inventory.id(),
        })
    }

    /// The owner's stable identity, which no take, restore or lift changes.
    pub(in crate::game) fn owner_id(&self) -> MenuId {
        self.id
    }

    fn read<'a>(&self, game: &'a Game<'_>) -> Option<(OwnerStamp, &'a EngineMenu)> {
        let menus = game.lan.as_ref()?.menus.get(&self.conn)?;
        let inventory = &menus.crafting.inventory;
        Some((inventory.stamp(), inventory.as_deref()?))
    }

    /// Every route must use this boundary before taking or mutating its owner.
    /// First-write snapshots merge into the existing Host transaction journal.
    pub(in crate::game) fn edit<R>(
        &self,
        game: &mut Game<'_>,
        edit: impl FnOnce(&mut EngineMenu) -> R,
    ) -> Option<R> {
        if self.read(game)?.0.id != self.id {
            return None;
        }
        self.keep_in(game);
        let menus = game.lan.as_mut()?.menus.get_mut(&self.conn)?;
        let menu = menus.crafting.inventory.as_deref_mut()?;
        Some(edit(menu))
    }

    fn keep_in(&self, game: &mut Game<'_>) {
        let Some(level) = game.transactions.last() else {
            return;
        };
        if level.actions.owners.contains_key(&self.id) {
            return;
        }
        let Some(menus) = game.lan.as_ref().and_then(|h| h.menus.get(&self.conn)) else {
            return;
        };
        let value = (*menus.crafting.inventory).clone();
        game.transactions.last_mut().unwrap().actions.owners.insert(
            self.id,
            KeptOwner {
                target: *self,
                value,
            },
        );
    }
}

#[cfg(test)]
#[path = "decision_engine_tests.rs"]
mod tests;

impl KeptOwner {
    pub(super) fn rollback(self, game: &mut Game<'_>) {
        let Some(menus) = game
            .lan
            .as_mut()
            .and_then(|h| h.menus.get_mut(&self.target.conn))
        else {
            return;
        };
        menus.crafting.inventory.restore(self.value);
    }
}

impl ActionTarget<GameHost<'_, '_>> for Target {
    type Reads = EngineReads;
    type View = EngineMenu;
    type Plan = EnginePlan;
    type Frozen = EnginePlan;

    fn id(&self) -> MenuId {
        self.id
    }

    fn keep(&self, host: &mut GameHost<'_, '_>) {
        self.keep_in(host.game);
    }

    fn capture(&self, host: &GameHost<'_, '_>) -> Result<EngineReads, Outcome> {
        let (owner, menu) = self
            .read(host.game)
            .ok_or(Outcome::Stale(StaleReason::MenuInstance))?;
        if owner.id != self.id {
            return Err(Outcome::Stale(StaleReason::MenuInstance));
        }
        Ok(EngineReads {
            owner,
            value: menu.mutation_stamp(),
            menu: menu.clone(),
        })
    }

    fn view(reads: &EngineReads) -> &EngineMenu {
        &reads.menu
    }

    fn freeze(&self, _reads: &EngineReads, plan: EnginePlan) -> Result<EnginePlan, InvalidReason> {
        Ok(plan)
    }

    fn empty(plan: &EnginePlan) -> bool {
        plan.0.is_none()
    }

    fn validate(&self, host: &GameHost<'_, '_>, reads: &EngineReads) -> Result<(), Outcome> {
        let (owner, menu) = self
            .read(host.game)
            .ok_or(Outcome::Stale(StaleReason::MenuInstance))?;
        if owner != reads.owner {
            return Err(Outcome::Stale(StaleReason::MenuInstance));
        }
        if menu.mutation_stamp() != reads.value {
            return Err(Outcome::Stale(StaleReason::Mutation));
        }
        Ok(())
    }

    fn apply(&self, host: &mut GameHost<'_, '_>, plan: EnginePlan) {
        if let Some(snapshot) = plan.0 {
            self.edit(host.game, |menu| menu.restore(snapshot))
                .expect("validated live menu owner");
        }
    }
}
