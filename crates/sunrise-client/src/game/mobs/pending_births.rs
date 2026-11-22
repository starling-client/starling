//! The engine side of pending children (docs/design/mod-api/pending-children.md):
//! it constructs the children of a split or a breeding, lends them to `mob_split` and
//! `baby_entity_spawn` listeners as an engine-owned, ordered list, then freezes, validates and
//! hands back what is left, once. Mod listeners reach the same arena through
//! the same editor (`native_pending_children`), so a call has the same effect from either.
//!
//! Nothing here replays construction or draws randomness: a veto, a stale parent or an invalid
//! list keeps every draw construction already made and spawns no child.

use super::Game;
use super::native_pending_children::{Children, PendingMob, valid_child};
use mod_api::PendingChildrenEdits;
use sunrise_client::mobs::Mob;
use sunrise_mod_runtime::{
    attachments::Attachments,
    pending_children::{Decision, PendingChildren},
};

/// A split's most children, and a breeding's only one (the design's limits).
const SPLIT_CAPACITY: usize = 64;

/// What a breeding's baby was derived from, per parent: compared again before the baby is
/// accepted, so a listener that changed a parent through the host cannot leave a baby built
/// from the old state. Anything the baby's kind, variant, colour, ownership or age came from.
#[derive(PartialEq)]
struct ParentState {
    kind: String,
    position: [f64; 3],
    age: i32,
    in_love: i32,
    variant: u8,
    variant2: u8,
    color: u8,
    tame: bool,
}

fn parent_state(game: &Game<'_>, id: u32) -> Option<ParentState> {
    if game.resident_target_dimension(super::commands::Target::Mob(id)) != Some(game.dim) {
        return None;
    }
    let mob = game.mobs.iter().find(|mob| mob.id == id)?;
    Some(ParentState {
        kind: mob.kind.id.clone(),
        position: [mob.position.x, mob.position.y, mob.position.z],
        age: mob.extra.age,
        in_love: mob.extra.in_love,
        variant: mob.extra.variant,
        variant2: mob.extra.variant2,
        color: mob.extra.color,
        tame: mob.extra.tame,
    })
}

/// Restores the hook the pending arena was serving when a dispatch ends, even by unwinding.
struct HookScope {
    cell: std::rc::Rc<std::cell::Cell<Option<mod_api::HookId>>>,
    previous: Option<mod_api::HookId>,
}
impl Drop for HookScope {
    fn drop(&mut self) {
        self.cell.set(self.previous);
    }
}

impl Game<'_> {
    /// Lends `initial` to the listeners `fire` calls, then issues and applies the accepted list
    /// inside the action savepoint. Commit callbacks cannot intervene between validation and
    /// applying the list. Returns `None` for a cancel (including a contained listener panic), a
    /// list a listener emptied, a stale parent or level, or an invalid final child. A refusal rolls
    /// back journaled listener writes in the action's level, not construction. Nothing is recomputed.
    ///
    /// `hook` is the hook `fire` dispatches: its listener count ends each listener's leases.
    /// `breeding` selects the one-baby limit and requires every child to be a baby. A child that
    /// already has an entity number keeps its UUID and constructor attachments here, outside the
    /// game's tables, until it joins; children a listener removed or replaced are forgotten, also
    /// when a listener panics, so none leaves state behind.
    pub(super) fn dispatch_pending_children<T>(
        &mut self,
        hook: mod_api::HookId,
        breeding: bool,
        initial: Vec<Mob>,
        parents: &[u32],
        fire: impl FnOnce(&mut Self, &dyn PendingChildrenEdits, &mut bool),
        apply: impl FnOnce(&mut Self, Vec<Mob>) -> T,
    ) -> Option<T> {
        use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
        let capacity = if breeding { 1 } else { SPLIT_CAPACITY };
        let (dim, epoch, resources) = (self.dim, self.lend_epoch(), self.res.clone());
        let before: Vec<(u32, Option<ParentState>)> = parents
            .iter()
            .map(|&id| (id, parent_state(self, id)))
            .collect();
        // A child with a number already published state: take its UUID and, where nothing is
        // bound to it, its constructor attachments back out of the game's tables, so none of
        // it is visible before the child joins.
        let mut reserved: Vec<u32> = initial.iter().map(|m| m.id).filter(|&id| id != 0).collect();
        let initial = initial
            .into_iter()
            .map(|mob| {
                let id = mob.id;
                let uuid = (id != 0).then(|| self.persist.uuids.remove(&id)).flatten();
                let attachments = match self.entity_attachments.get(&id) {
                    Some(held)
                        if id != 0 && !held.has_index_entries() && !held.has_index_binding() =>
                    {
                        self.entity_attachments.remove(&id).unwrap_or_default()
                    }
                    _ => Attachments::default(),
                };
                PendingMob {
                    mob,
                    uuid,
                    attachments,
                    dimension: dim,
                }
            })
            .collect();
        let (arena, hook_cell) = {
            let lent = self.lent_mobs.borrow();
            (lent.pending.clone(), lent.hook.clone())
        };
        let run = catch_unwind(AssertUnwindSafe(|| {
            mod_api::run_transaction(self, |game, _| {
                let mut cancel = false;
                let mut frozen = {
                    let _scope = HookScope {
                        previous: hook_cell.replace(Some(hook)),
                        cell: hook_cell.clone(),
                    };
                    PendingChildren::run_scoped(
                        &arena,
                        epoch.get(),
                        capacity,
                        initial,
                        |action| {
                            let editor = Children::new(game, action, breeding);
                            // The baseline: listeners entered from here on end the previous lease.
                            super::native_pending_children::sync_listener(&arena, &hook_cell);
                            let failures = mod_api::runtime::isolated_failures();
                            fire(game, &editor, &mut cancel);
                            if mod_api::runtime::isolated_failures() != failures {
                                cancel = true;
                            }
                            if cancel || arena.borrow().is_empty(action) == Ok(true) {
                                cancel = true;
                                return Decision::Cancel;
                            }
                            if game.lend_epoch() != epoch
                                || game.dim != dim
                                || before.iter().any(|(id, state)| {
                                    let now = parent_state(game, *id);
                                    now.is_none() || now.as_ref() != state.as_ref()
                                })
                            {
                                return Decision::Stale;
                            }
                            Decision::Accept
                        },
                        |child| valid_child(&resources, dim, breeding, child),
                    )
                    .map_err(|_| ())?
                };
                // A cancelled arena finishes successfully with an empty batch; the journal aborts.
                if cancel {
                    return Err(());
                }
                let mut kept = Vec::new();
                while frozen
                    .consume_next::<()>(|child| {
                        kept.push(game.issue_pending_child(child));
                        Ok(())
                    })
                    .unwrap_or(false)
                {}
                // Keep track until apply returns: if it unwinds, the journal aborts and all the
                // reservations are retired. A later unwind in commit publication is not a rollback.
                let published = reserved
                    .iter()
                    .copied()
                    .filter(|id| kept.iter().any(|child| child.id == *id))
                    .collect::<Vec<_>>();
                let result = apply(game, kept);
                reserved.retain(|id| !published.contains(id));
                Ok(result)
            })
        }));
        for id in reserved {
            self.forget_entity(id);
        }
        match run {
            Ok(result) => result.ok(),
            Err(panic) => resume_unwind(panic),
        }
    }

    /// The child as an ordinary unjoined mob. A child a mod constructed gets its entity number
    /// now, so breeding's finalization and its advancement see the real one. Its UUID and
    /// attachments, and those an engine-made child kept out of the tables while it waited, are
    /// published here: the first moment it can join.
    fn issue_pending_child(&mut self, child: PendingMob) -> Mob {
        let PendingMob {
            mut mob,
            uuid,
            attachments,
            ..
        } = child;
        let fresh = mob.id == 0;
        if fresh {
            self.next_entity += 1;
            mob.id = self.next_entity;
        }
        if let Some(uuid) = uuid {
            self.persist.uuids.insert(mob.id, uuid);
        }
        if !attachments.to_nbt().is_empty() {
            self.entity_attachments
                .entry(mob.id)
                .or_default()
                .replace_contents(attachments);
        }
        if fresh {
            let kind = super::native_hooks::mob_kind(&mob);
            self.initialize_entity(mod_api::Holder::Entity(mob.id), &kind);
        }
        mob
    }

    /// `AbstractCubeMob.remove`'s children once the parent `dying` is dead: the count is drawn,
    /// then each half-size child is constructed in order with its yaw draw from the parent. With a
    /// `mob_split` listener the whole list is constructed first and lent; otherwise each
    /// child joins as it is made, exactly as vanilla does.
    pub(super) fn split_cube_children(&mut self, dying: &mut Mob) {
        let drawn = sunrise_client::mobs::cube::split_count(dying);
        if drawn == 0 {
            return;
        }
        if !mod_api::__private::listened(mod_api::HookId::MobSplit) {
            for i in 0..drawn {
                self.next_entity += 1;
                let mut child = Mob::new(self.next_entity, dying.kind.clone(), dying.position, 0.0);
                sunrise_client::mobs::cube::split_child(dying, &mut child, i);
                self.mobs.push(child);
                self.native_mobs_joined(self.mobs.len() - 1, false);
            }
            return;
        }
        // The numbers are issued when the children join, so a listener's own spawns cannot
        // collide; the constructor still seeds from the number the child would have.
        let first = self.next_entity + 1;
        let children = (0..drawn)
            .map(|i| {
                let mut child = Mob::new(first + i, dying.kind.clone(), dying.position, 0.0);
                sunrise_client::mobs::cube::split_child(dying, &mut child, i);
                child.id = 0;
                child
            })
            .collect();
        let dimension = self.dim.id();
        let (parent, kind, size) = (
            dying.id,
            super::native_hooks::mob_kind(dying),
            dying.extra.size,
        );
        let _ = self.dispatch_pending_children(
            mod_api::HookId::MobSplit,
            false,
            children,
            &[],
            |game, edits, cancel| {
                game.native_mob_split(mod_api::__private::MobSplitCall {
                    dimension,
                    parent,
                    kind: &kind,
                    size,
                    child_size: size / 2,
                    edits,
                    cancel,
                });
            },
            |game, kept| {
                for child in kept {
                    game.mobs.push(child);
                    game.native_mobs_joined(game.mobs.len() - 1, false);
                }
            },
        );
    }

    /// `Animal.spawnChildFromBreeding` once `baby` is constructed: lends it to the listeners and
    /// applies the remaining baby (or its replacement) before publishing the action's effects.
    pub(super) fn breed_with_listeners<T>(
        &mut self,
        parent: u32,
        partner: u32,
        player: (Option<u128>, Option<i32>),
        baby: Mob,
        apply: impl FnOnce(&mut Self, Mob) -> T,
    ) -> Option<T> {
        let dimension = self.dim.id();
        self.dispatch_pending_children(
            mod_api::HookId::BabyEntitySpawn,
            true,
            vec![baby],
            &[parent, partner],
            |game, edits, cancel| {
                game.native_baby_entity_spawn(mod_api::__private::BabyEntitySpawnCall {
                    dimension,
                    parent,
                    partner,
                    player: player.0,
                    player_entity_id: player.1,
                    edits,
                    cancel,
                });
            },
            |game, mut kept| apply(game, kept.pop().expect("accepted breeding has one child")),
        )
    }
}
