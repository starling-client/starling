//! `ClientRenderClientHost::set_block_extra_models`: the standalone models a client mod draws as
//! part of a block (NeoForge's `ModelData` use), kept in the client's columns for the chunk mesher
//! (`World::set_extra_models`), which reads them on its workers and calls no mod code.
use super::Game;
use mod_api::{BlockExtraModelsError, ModelRef};

impl Game<'_> {
    /// `models` (resolved standalone models) replace those drawn as part of the block at `pos`,
    /// its section meshed again when they change. A stale or unknown handle refuses the list.
    pub(in crate::game) fn set_block_extra_models(
        &mut self,
        pos: [i32; 3],
        models: &[ModelRef],
    ) -> Result<(), BlockExtraModelsError> {
        for &model in models {
            self.pack
                .render
                .validate_standalone_model(model)
                .map_err(BlockExtraModelsError::Model)?;
        }
        let [x, y, z] = pos;
        self.world.set_extra_models(x, y, z, models)
    }
}

#[cfg(test)]
mod tests {
    use mod_api::BlockExtraModelsError;
    use mod_api::prelude::ClientRenderClientHost;

    /// The live Host refuses an unloaded position and only accepts registered standalone models.
    #[test]
    #[ignore = "daily"]
    fn host_sets_only_loaded_cells_with_registered_standalone_models() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_client_game};

        if run_fresh_cases(
            "game::native_block_extra_models::tests::host_sets_only_loaded_cells_with_registered_standalone_models",
            "SUNRISE_TEST_BLOCK_EXTRA_MODELS_CASE",
            &["host"],
        ) {
            return;
        }
        let _standalone =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/gold_block".into()]);
        with_client_game(|game| {
            assert_eq!(
                ClientRenderClientHost::set_block_extra_models(game, [0, 9999, 0], &[]),
                Err(BlockExtraModelsError::NotLoaded),
            );

            let render = &game.pack.render;
            let generation = render.generation;
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let slot = render.model_catalog.state_slots(state).unwrap()[0];
            let model = mod_api::__private::built::model_ref(generation, slot);
            assert!(
                render.resolve_model(model).is_ok(),
                "the state slot is baked"
            );
            assert!(
                !render
                    .model_catalog
                    .standalone_slots()
                    .any(|registered| registered == slot)
            );
            let held = render
                .model(&mod_api::Identifier::parse("minecraft:block/gold_block").unwrap())
                .unwrap();
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            ClientRenderClientHost::set_block_extra_models(game, [0, 64, 0], &[held]).unwrap();
            let stale = mod_api::__private::built::model_ref(generation + 1, slot);
            for (bad, reason) in [
                (model, mod_api::ModelError::UnknownModel),
                (stale, mod_api::ModelError::StaleModel),
            ] {
                assert_eq!(
                    ClientRenderClientHost::set_block_extra_models(game, [0, 64, 0], &[bad]),
                    Err(BlockExtraModelsError::Model(reason)),
                );
                assert_eq!(game.world.extra_models(0, 64, 0), [held]);
            }
        });
    }
}
