//! Collect native spawn predicates in registration order.
use super::*;
use mod_vocab::__private::parts_installation::entity_type_key_slot;
use mod_vocab::{
    EntityTypeKey, Heightmap, SpawnPlacementError, SpawnPlacementOperation,
    SpawnPlacementPredicate, SpawnPlacementType,
};

impl SpawnPlacementsRegistrar for PartRegistrar {
    fn register_spawn_placement(
        &mut self,
        kind: EntityTypeKey,
        placement: Option<SpawnPlacementType>,
        heightmap: Option<Heightmap>,
        predicate: Arc<dyn SpawnPlacementPredicate>,
        operation: SpawnPlacementOperation,
    ) -> Result<(), SpawnPlacementError> {
        let keys = self.registry_keys.borrow();
        let slot = entity_type_key_slot(kind);
        let Some(id) = keys.entity_types.id(slot) else {
            self.refused.push(format!(
                "mod {} spawn placement: entity type key slot {slot} is unavailable",
                self.mod_id
            ));
            return Err(SpawnPlacementError::InvalidEntityType);
        };
        self.spawn_placements.push(SpawnPlacementDeclaration {
            id: id.to_owned(),
            kind,
            mod_id: self.mod_id,
            placement,
            heightmap,
            predicate,
            operation,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BlockPos, Hooks, ModEntry, RegistrationContext, SpawnPlacementContext, SpawnReason,
    };

    #[test]
    fn spawn_predicates_for_one_key_are_published_in_registration_order() {
        struct SpawnRules(SpawnPlacementOperation);
        #[mod_api::__subscriptions]
        impl Hooks for SpawnRules {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let kind = r.entity_type_key("minecraft:zombie");
                r.register_spawn_placement(
                    kind,
                    None,
                    None,
                    Arc::new(
                        |_: EntityTypeKey,
                         _: &mut dyn SpawnPlacementContext,
                         _: SpawnReason,
                         _: BlockPos| true,
                    ),
                    self.0,
                )
                .unwrap();
            }
        }
        crate::parts::tests::install_parts_fixture(&[
            ModEntry::new("ruby", Box::new(SpawnRules(SpawnPlacementOperation::Or))),
            ModEntry::new("quartz", Box::new(SpawnRules(SpawnPlacementOperation::And))),
        ])
        .unwrap();
        let declared = crate::__private::spawn_placement_declarations();
        assert_eq!(declared.len(), 2);
        assert_eq!(declared[0].id, "minecraft:zombie");
        assert_eq!(declared[1].id, "minecraft:zombie");
        assert_eq!(declared[0].kind, declared[1].kind);
        assert_eq!(declared[0].mod_id, "ruby");
        assert_eq!(declared[1].mod_id, "quartz");
        assert_eq!(declared[0].operation, SpawnPlacementOperation::Or);
        assert_eq!(declared[1].operation, SpawnPlacementOperation::And);
    }

    #[test]
    fn an_unavailable_spawn_placement_key_refuses_the_registering_mod() {
        let mut registrar = PartRegistrar::new("ruby");
        let kind = mod_vocab::__private::parts_installation::entity_type_key(42);
        assert_eq!(
            registrar.register_spawn_placement(
                kind,
                None,
                None,
                Arc::new(
                    |_: EntityTypeKey,
                     _: &mut dyn SpawnPlacementContext,
                     _: SpawnReason,
                     _: BlockPos| true
                ),
                SpawnPlacementOperation::Replace,
            ),
            Err(SpawnPlacementError::InvalidEntityType)
        );
        assert!(registrar.spawn_placements.is_empty());
        assert_eq!(
            registrar.refused,
            ["mod ruby spawn placement: entity type key slot 42 is unavailable"]
        );
    }
}
