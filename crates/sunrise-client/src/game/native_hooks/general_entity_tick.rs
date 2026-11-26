use super::*;
use crate::game::{
    commands::{Target, entity_pitch},
    general::GeneralRow,
    transactions::OpenTransactions,
};
use mod_api::{
    EntityRef, EntityServerAccess, EntityServerScope, EntitySubjectView, ResidentGeneralEntity,
    ServerHost,
};
use sunrise_client::lan::playerdata::ints_uuid;
use sunrise_core::geometry::Vec3;

struct ResidentGeneralRow<'a> {
    row: &'a mut GeneralRow,
    journal: &'a mut OpenTransactions,
    kind: &'a str,
}

impl ResidentGeneralEntity for ResidentGeneralRow<'_> {
    fn kind(&self) -> &str {
        self.kind
    }
    fn position(&self) -> [f64; 3] {
        [
            self.row.position.x,
            self.row.position.y,
            self.row.position.z,
        ]
    }
    fn velocity(&self) -> [f64; 3] {
        [
            self.row.velocity.x,
            self.row.velocity.y,
            self.row.velocity.z,
        ]
    }
    fn rotation(&self) -> [f32; 2] {
        self.row.rotation
    }
    fn uuid(&self) -> u128 {
        ints_uuid(self.row.uuid)
    }
    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        if !velocity.iter().all(|v| v.is_finite()) {
            return false;
        }
        if let Some(kept) = self.journal.journal_general_entity(self.row.join) {
            kept.velocity.get_or_insert(self.row.velocity);
        }
        self.row.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        true
    }
    fn set_rotation(&mut self, yaw: f32, pitch: f32) -> bool {
        if !yaw.is_finite() || !pitch.is_finite() {
            return false;
        }
        if let Some(kept) = self.journal.journal_general_entity(self.row.join) {
            kept.rotation.get_or_insert(self.row.rotation);
        }
        self.row.rotation = [yaw, entity_pitch(pitch)];
        true
    }
    fn reset_fall_distance(&mut self) {
        if let Some(kept) = self.journal.journal_general_entity(self.row.join) {
            kept.fall_distance.get_or_insert(self.row.fall_distance);
        }
        self.row.fall_distance = 0.0;
    }
}

struct GeneralTickAccess<'g, 'w, 'k> {
    game: &'g mut Game<'w>,
    subject: EntityRef,
    kind: &'k str,
}

impl EntityServerAccess for GeneralTickAccess<'_, '_, '_> {
    fn with_view(&mut self, call: &mut dyn FnMut(EntitySubjectView<'_>)) {
        if !self.game.is_logical_server()
            || self.game.native_lifecycle.epoch() != Some(self.subject.epoch())
            || self
                .game
                .persist
                .removals
                .contains(&Target::General(self.subject.id()))
        {
            return;
        }
        let Some(row) = self
            .game
            .resident_entities
            .general
            .row_mut(self.subject.id())
        else {
            return;
        };
        let mut data = ResidentGeneralRow {
            row,
            journal: &mut self.game.transactions,
            kind: self.kind,
        };
        call(EntitySubjectView::new(self.subject, &mut data));
    }

    fn host(&mut self) -> &mut dyn ServerHost {
        self.game
    }

    fn with_current_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn mod_api::EntityServerAccess,
            &'loan mod_api::TransactionContext,
        ),
    ) -> Result<(), mod_api::TransactionUnavailable> {
        let frame = self
            .game
            .current_storage_frame()
            .ok_or(mod_api::TransactionUnavailable)?;
        frame.lend(|tx| callback(self, tx));
        Ok(())
    }

    fn with_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn mod_api::EntityServerAccess,
            &'loan mod_api::TransactionContext,
        ) -> bool,
    ) -> Result<bool, mod_api::TransactionScopeError> {
        let (subject, kind) = (self.subject, self.kind);
        self.game
            .scoped_storage(|game, tx| {
                let mut child = GeneralTickAccess {
                    game,
                    subject,
                    kind,
                };
                callback(&mut child, tx).then_some(())
            })
            .map(|settled| settled.is_some())
    }
}

impl Game<'_> {
    /// Dispatch the authoritative tick of one resident general entity row of type `kind`,
    /// lending its live row sequentially with `ServerHost`.
    pub(in crate::game) fn tick_general_row_scoped(
        &mut self,
        kind: &str,
        subject: mod_api::EntityRef,
    ) {
        let mut access = GeneralTickAccess {
            game: self,
            subject,
            kind,
        };
        Self::native_general_entity_tick(mod_api::__private::GeneralEntityTickCall {
            kind,
            subject: EntityServerScope::new(&mut access),
        });
    }

    /// The authoritative tick of one general entity row of type `kind`
    /// (`mymod:robot`) (#792 stage 4): `ServerLevel.tickNonPassenger`'s
    /// `entity.tick()` for a row the type registry owns, its rider subtree
    /// ticked after it. The scope lends the live row sequentially with `ServerHost`.
    /// Vanilla families never reach this call. Server-side.
    pub(in crate::game) fn native_general_entity_tick(
        call: mod_api::__private::GeneralEntityTickCall<'_>,
    ) {
        Self::fire_general_entity_tick(call);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{api_values::test_snapshot, native_entity_refs::tests::with_game};

    #[test]
    #[ignore = "daily"]
    fn general_subject_and_host_writes_commit_or_abort_together() {
        crate::game::in_own_process(
            "game::native_hooks::general_entity_tick::tests::general_subject_and_host_writes_commit_or_abort_together",
            || {
                with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    crate::game::native_entity_refs::tests::fixture_resident_column(
                        &mut game.world,
                        (0, 0),
                    );
                    game.spawn_item(ItemStack::new("minecraft:stick", 1), [0.0; 3], [0.0; 3], 0);
                    let peer = game
                        .item_entity(game.entity(game.dropped[0].id).unwrap())
                        .unwrap();
                    game.next_entity += 1;
                    let id = game.next_entity;
                    let epoch = game.native_lifecycle.epoch().unwrap();
                    game.resident_entities.general.rows.push(GeneralRow {
                        id,
                        uuid: [1, 2, 3, 4],
                        join: Default::default(),
                        type_slot: 0,
                        position: Vec3::ZERO,
                        previous: Vec3::ZERO,
                        velocity: Vec3::new(1.0, 2.0, 3.0),
                        rotation: [4.0, 5.0],
                        previous_rotation: [4.0, 5.0],
                        owner: None,
                        fall_distance: 6.0,
                        custom_name: None,
                        custom_name_visible: false,
                        base: Default::default(),
                        on_ground: false,
                        tick_count: 0,
                        no_physics: false,
                        horizontal_collision: false,
                        vertical_collision: false,
                        vertical_collision_below: false,
                        fluids: Default::default(),
                        under_water: false,
                        payload_version: 1,
                        payload_bytes: Vec::new(),
                        mount: None,
                        vehicle_order: 0,
                        boarding_cooldown: 0,
                        attachment_lifetime: Default::default(),
                    });
                    let stack = test_snapshot(&ItemStack::new("minecraft:stick", 2));
                    for commit in [false, true] {
                        let mut access = GeneralTickAccess {
                            game: &mut *game,
                            subject: EntityRef::new(epoch, id),
                            kind: "test:robot",
                        };
                        let mut scope = EntityServerScope::new(&mut access);
                        let result = scope.transaction(|scope, _| {
                            assert_eq!(
                                scope.with_view(|mut view| view.set_velocity([7.0; 3])),
                                Some(true)
                            );
                            scope
                                .transaction(|scope, _| {
                                    scope
                                        .with_view(|mut view| {
                                            assert!(view.set_velocity([8.0; 3]));
                                            assert!(view.set_rotation(9.0, 10.0));
                                            view.reset_fall_distance();
                                        })
                                        .unwrap();
                                    assert_eq!(
                                        peer.set_stack(scope.host(), Some(stack.clone())),
                                        Ok(true)
                                    );
                                    Ok::<_, ()>(())
                                })
                                .unwrap();
                            commit.then_some(()).ok_or(())
                        });
                        assert_eq!(
                            result,
                            commit
                                .then_some(())
                                .ok_or(mod_api::TransactionError::Aborted(()))
                        );
                        let row = game.resident_entities.general.row(id).unwrap();
                        assert_eq!(
                            (
                                row.velocity.components(),
                                row.rotation,
                                row.fall_distance,
                                game.dropped[0].stack.count
                            ),
                            if commit {
                                ([8.0; 3], [9.0, 10.0], 0.0, 2)
                            } else {
                                ([1.0, 2.0, 3.0], [4.0, 5.0], 6.0, 1)
                            }
                        );
                    }
                })
            },
        );
    }
}
