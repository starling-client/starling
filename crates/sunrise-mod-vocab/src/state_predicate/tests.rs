use super::*;
use crate::{BlockMetadata, BlockState};
use std::collections::BTreeMap;
use sunrise_mod_runtime::block_definitions::BlockDefinition;

fn register_block() -> BlockKey {
    crate::parts::install_input(Default::default());
    let id = "test:connected";
    let off = BTreeMap::from([
        ("left".into(), "false".into()),
        ("right".into(), "false".into()),
    ]);
    let on = BTreeMap::from([
        ("left".into(), "true".into()),
        ("right".into(), "true".into()),
    ]);
    registry_keys::resolve_block_keys([id], |_| Ok(BlockDefinition::new(&[&off, &on], 0).unwrap()))
        .unwrap();
    registry_keys::block_key(id).unwrap()
}

#[test]
fn strict_block_predicates_validate_schema_without_default_bias() {
    let key = register_block();
    // The default is false/false, so setting just left=true would not be a complete state.
    // It is nevertheless a legal partial predicate matching the independent true/true row.
    assert!(StatePredicate::block(key, &[StateConstraint::new("left", "true")]).is_ok());
    assert_eq!(
        StatePredicate::block(key, &[StateConstraint::new("missing", "true")]).unwrap_err(),
        BlockStateError::UnknownProperty(0)
    );
    assert_eq!(
        StatePredicate::block(key, &[StateConstraint::new("left", "not_a_value")]).unwrap_err(),
        BlockStateError::InvalidValue(0)
    );
    assert_eq!(
        StatePredicate::block(
            key,
            &[
                StateConstraint::new("left", "true"),
                StateConstraint::new("left", "false")
            ]
        )
        .unwrap_err(),
        BlockStateError::DuplicateProperty(1)
    );
    for (name, value, error) in [
        ("missing", "true", BlockStateError::UnknownProperty(1)),
        ("right", "invalid", BlockStateError::InvalidValue(1)),
    ] {
        assert_eq!(
            StatePredicate::block(
                key,
                &[
                    StateConstraint::new("left", "true"),
                    StateConstraint::new(name, value)
                ]
            )
            .unwrap_err(),
            error
        );
    }
}

#[test]
fn strict_block_predicates_refuse_unknown_properties_even_if_custom_metadata_accepts_them() {
    struct AcceptAll(BlockState);

    impl BlockMetadata for AcceptAll {
        fn name(&self, _: BlockKey) -> Result<&str, BlockStateError> {
            Ok("test:connected")
        }

        fn property(&self, _: BlockState, _: &str) -> Result<Option<&str>, BlockStateError> {
            Ok(None)
        }

        fn with(&self, _: BlockState, _: &str, _: &str) -> Result<BlockState, BlockStateError> {
            Ok(self.0)
        }

        fn of(&self, _: BlockKey, _: &[StateConstraint]) -> Result<BlockState, BlockStateError> {
            Ok(self.0)
        }

        fn visit_properties(
            &self,
            _: BlockState,
            _: &mut dyn FnMut(&str, &str),
        ) -> Result<(), BlockStateError> {
            Ok(())
        }

        fn key(&self, _: &str) -> Option<BlockKey> {
            Some(self.0.block())
        }

        fn default_state(&self, _: BlockKey) -> Result<BlockState, BlockStateError> {
            Ok(self.0)
        }

        fn state_definition(
            &self,
            _: BlockKey,
        ) -> Result<crate::StateProperties<'_>, BlockStateError> {
            Ok(crate::StateProperties::of(&[]))
        }
    }

    let key = register_block();
    let state = BlockState::of(registry_keys::block_definitions().unwrap(), key, &[]).unwrap();
    let custom = AcceptAll(state);
    let constraint = StateConstraint::new("invented", "true");
    assert_eq!(BlockState::of(&custom, key, &[]), Ok(state));
    assert_eq!(
        state.with(&custom, &constraint.name, &constraint.value),
        Ok(state)
    );
    assert_eq!(
        StatePredicate::block(key, &[constraint]),
        Err(BlockStateError::UnknownProperty(0))
    );
}

#[test]
fn tag_predicates_keep_vague_properties_and_refuse_duplicates() {
    let vague = StatePredicate::tag(
        TagKey::slot_unchecked(0),
        &[StateConstraint::new(
            "property_no_member_has",
            "arbitrary_value",
        )],
    )
    .unwrap();
    assert_eq!(vague.constraints()[0].name, "property_no_member_has");
    assert_eq!(vague.constraints()[0].value, "arbitrary_value");
    assert_eq!(vague.tag_key(), Some(TagKey::slot_unchecked(0)));
    assert_eq!(
        StatePredicate::tag(
            TagKey::slot_unchecked(0),
            &[
                StateConstraint::new("unknown", "a"),
                StateConstraint::new("unknown", "b"),
            ],
        )
        .unwrap_err(),
        BlockStateError::DuplicateProperty(1)
    );
}
