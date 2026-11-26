//! Whole-aggregate block-state boundaries. No partial draw is admitted.
use super::block_state;
use crate::game::native_scene::SceneBlock;

/// Prepare borrowed scene blocks, with a refusal naming the invalid block.
/// Conversion allocates one engine vector per scene draw, but never clones entity
/// identifiers or saved fields, and never resolves names.
pub(in crate::game) fn scene_blocks(
    blocks: &[mod_api::SceneBlock],
) -> Result<Vec<SceneBlock<'_>>, mod_api::SceneError> {
    if blocks.len() > mod_api::SCENE_MAX_BLOCKS {
        return Err(mod_api::SceneError::TooManyBlocks);
    }
    let index = block_state::current().map_err(|_| mod_api::SceneError::Unavailable)?;
    (0u32..)
        .zip(blocks)
        .map(|(at, block)| {
            Ok(SceneBlock {
                pos: block.pos,
                state: index
                    .to_engine(block.state)
                    .map_err(|_| mod_api::SceneError::BadBlock { index: at })?,
                entity: block.entity.as_ref(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::TryIntoApi;
    use mod_api::{Identifier, SceneBlockEntity};
    use sunrise_world::registry::{BlockRegistry, BlockStateId};

    #[test]
    fn aggregates_preserve_properties_reorder_and_refuse_whole_values() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":2},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north","waterlogged":"false"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east","waterlogged":"true"}}]]}"#;
        let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        super::super::resolve_block_definitions(&registry).unwrap();
        let north = BlockStateId(1).try_into_api().unwrap();
        let east = BlockStateId(2).try_into_api().unwrap();
        let metadata = mod_api::__private::block_definitions().unwrap();
        assert_eq!((north.ordinal(), east.ordinal()), (0, 1));
        assert_eq!(east.property(metadata, "facing"), Ok(Some("east")));
        assert_eq!(east.property(metadata, "waterlogged"), Ok(Some("true")));
        let reordered = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east","waterlogged":"true"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north","waterlogged":"false"}}]]}"#;
        let registry = BlockRegistry::from_facts_json(reordered.as_bytes()).unwrap();
        super::super::resolve_block_definitions(&registry).unwrap();

        let entity = SceneBlockEntity::new(
            Identifier::parse("test:scene").unwrap(),
            east,
            Default::default(),
        );
        let mut scene = vec![
            mod_api::SceneBlock::new([0, 0, 0], north, None),
            mod_api::SceneBlock::new([1, 0, 0], east, Some(entity)),
            mod_api::SceneBlock::new([2, 0, 0], north, None),
        ];
        let original = scene.clone();
        let prepared = scene_blocks(&scene).unwrap();
        assert_eq!(
            prepared.iter().map(|b| b.state).collect::<Vec<_>>(),
            [BlockStateId(2), BlockStateId(1), BlockStateId(2)]
        );
        assert_eq!(prepared[1].pos, [1, 0, 0]);
        assert!(std::ptr::eq(
            prepared[1].entity.unwrap(),
            scene[1].entity.as_ref().unwrap()
        ));
        assert_eq!(scene, original);
        scene[1].state = mod_api::__private::block_state_from_ordinal(east.block(), u16::MAX);
        assert!(matches!(
            scene_blocks(&scene),
            Err(mod_api::SceneError::BadBlock { index: 1 })
        ));
        assert_eq!(scene[1].state.ordinal(), u16::MAX);

        // A valid API key absent from this world's index refuses, rather than drawing air.
        let only_air = BlockRegistry::from_facts_json(
            br#"{"defaults":{"minecraft:air":0},"states":[["a",true,false,"","minecraft:air"]]}"#
                .as_slice(),
        )
        .unwrap();
        block_state::bind(&only_air).unwrap();
        scene[1].state = east;
        assert!(matches!(
            scene_blocks(&scene),
            Err(mod_api::SceneError::BadBlock { index: 0 })
        ));
        block_state::clear();
        assert!(matches!(
            scene_blocks(&scene),
            Err(mod_api::SceneError::Unavailable)
        ));
        assert_eq!(scene, original);
    }
}
