#[cfg(test)]
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, BackendRef, Host, HostBlockStates};
    struct MetadataHost(&'static sunrise_mod_runtime::block_definitions::Blocks);
    impl Backend for MetadataHost {}
    crate::impl_backend_areas!(MetadataHost, except(block_states));
    impl crate::prelude::BlockStatesBackend for MetadataHost {
        fn block_metadata_name(&self, block: BlockKey) -> Result<&str, BlockStateError> {
            self.0.name(block)
        }
        fn block_metadata_property(
            &self,
            state: BlockState,
            name: &str,
        ) -> Result<Option<&str>, BlockStateError> {
            self.0.property(state, name)
        }
        fn block_metadata_with(
            &self,
            state: BlockState,
            name: &str,
            value: &str,
        ) -> Result<BlockState, BlockStateError> {
            self.0.with(state, name, value)
        }
        fn block_metadata_visit_properties(
            &self,
            state: BlockState,
            visit: &mut dyn FnMut(&str, &str),
        ) -> Result<(), BlockStateError> {
            self.0.visit_properties(state, visit)
        }
        fn block_metadata_of(
            &self,
            block: BlockKey,
            properties: &[StateConstraint],
        ) -> Result<BlockState, BlockStateError> {
            self.0.of(block, properties)
        }
    }
    impl Host for MetadataHost {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(mod_vocab::__private::parts_installation::backend_ref(self))
        }
    }
    crate::impl_host_areas!(MetadataHost);
    struct Empty;
    impl Host for Empty {}
    crate::impl_host_areas!(Empty);

    #[test]
    fn metadata_enumerates_a_states_properties_in_name_order() {
        crate::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let state = |facing: &str, lit: &str| {
            std::collections::BTreeMap::from([
                ("facing".to_string(), facing.to_string()),
                ("lit".to_string(), lit.to_string()),
            ])
        };
        let states = [
            state("north", "false"),
            state("north", "true"),
            state("south", "false"),
            state("south", "true"),
        ];
        let plain = std::collections::BTreeMap::new();
        crate::__private::resolve_block_keys(["probe:furnace", "probe:plain"], |name| {
            let states: Vec<_> = if name == "probe:plain" {
                vec![&plain]
            } else {
                states.iter().collect()
            };
            sunrise_mod_runtime::block_definitions::BlockDefinition::new(&states, 0)
                .map_err(|e| format!("{e:?}"))
        })
        .unwrap();
        let host = MetadataHost(crate::__private::block_definitions().unwrap());
        let metadata = host.block_states();
        let furnace = crate::__private::block_key("probe:furnace").unwrap();
        let lit_south = metadata
            .of(
                furnace,
                &[
                    StateConstraint::new("facing", "south"),
                    StateConstraint::new("lit", "true"),
                ],
            )
            .unwrap();
        let mut seen = Vec::new();
        lit_south
            .visit_properties(&metadata, &mut |name, value| {
                seen.push((name.to_string(), value.to_string()))
            })
            .unwrap();
        assert_eq!(
            seen,
            [
                ("facing".to_string(), "south".to_string()),
                ("lit".to_string(), "true".to_string())
            ]
        );
        let plain = metadata
            .of(crate::__private::block_key("probe:plain").unwrap(), &[])
            .unwrap();
        let mut visited = 0;
        plain
            .visit_properties(&metadata, &mut |_, _| visited += 1)
            .unwrap();
        assert_eq!(visited, 0);
        assert_eq!(
            lit_south.visit_properties(&Empty.block_states(), &mut |_, _| {}),
            Err(BlockStateError::Unavailable)
        );
        assert_eq!(
            metadata.visit_properties(
                crate::__private::block_state_from_ordinal(furnace, 9),
                &mut |_, _| {}
            ),
            Err(BlockStateError::UnknownState)
        );
    }

    #[test]
    fn metadata_service_reads_edits_and_refuses_invalid_states() {
        crate::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let states = [
            std::collections::BTreeMap::from([("lit".into(), "false".into())]),
            std::collections::BTreeMap::from([("lit".into(), "true".into())]),
        ];
        crate::__private::resolve_block_keys(["probe:lamp"], |_| {
            sunrise_mod_runtime::block_definitions::BlockDefinition::new(
                &[&states[0], &states[1]],
                0,
            )
            .map_err(|e| format!("{e:?}"))
        })
        .unwrap();
        let host = MetadataHost(crate::__private::block_definitions().unwrap());
        let metadata = host.block_states();
        let block = crate::__private::block_key("probe:lamp").unwrap();
        assert_eq!(metadata.name(block), Ok("probe:lamp"));
        assert_eq!(
            metadata.name(mod_vocab::__private::parts_installation::block_key(999)),
            Err(BlockStateError::UnknownBlock)
        );
        assert_eq!(
            Empty.block_states().name(block),
            Err(BlockStateError::Unavailable)
        );
        let off = metadata.of(block, &[]).unwrap();
        assert_eq!(off.ordinal(), 0);
        let on = metadata.with(off, "lit", "true").unwrap();
        assert_eq!(on.ordinal(), 1);
        assert_eq!(metadata.property(on, "lit"), Ok(Some("true")));
        assert_eq!(metadata.property(off, "lit"), Ok(Some("false")));
        let borrowed = crate::__private::backend_read(&host, |backend| {
            backend.block_metadata_property(off, "lit")
        })
        .unwrap()
        .unwrap();
        assert_eq!(borrowed, Some("false"));
        let value = crate::BlockState::of(&metadata, block, &[]).unwrap();
        let changed = value.with(&metadata, "lit", "true").unwrap();
        assert_eq!(value.property(&metadata, "lit"), Ok(Some("false")));
        assert_eq!(changed.property(&metadata, "lit"), Ok(Some("true")));
        assert_eq!(changed.ordinal(), 1);
        let states = crate::__private::block_state_range(block).unwrap();
        assert_eq!(
            states
                .iter()
                .map(|state| state.property(&metadata, "lit").unwrap().unwrap())
                .collect::<Vec<_>>(),
            ["false", "true"]
        );
        assert_eq!(
            crate::__private::block_state_range(
                mod_vocab::__private::parts_installation::block_key(999)
            ),
            Err(BlockStateError::UnknownBlock)
        );
        assert_eq!(
            changed.with(&metadata, "lit", "invalid"),
            Err(BlockStateError::InvalidValue(0))
        );
        assert_eq!(
            value.property(&Empty.block_states(), "lit"),
            Err(BlockStateError::Unavailable)
        );
        assert_eq!(metadata.property(off, "missing"), Ok(None));
        assert_eq!(
            metadata.with(off, "lit", "bad"),
            Err(BlockStateError::InvalidValue(0))
        );
        assert_eq!(
            metadata.property(crate::__private::block_state_from_ordinal(block, 2), "lit"),
            Err(BlockStateError::UnknownState)
        );
        assert_eq!(
            metadata.property(
                crate::__private::block_state_from_ordinal(
                    mod_vocab::__private::parts_installation::block_key(999),
                    0
                ),
                "lit"
            ),
            Err(BlockStateError::UnknownBlock)
        );
        assert_eq!(
            Empty.block_states().property(off, "lit"),
            Err(BlockStateError::Unavailable)
        );
    }
}
