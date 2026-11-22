//! Generation-scoped access to the published baked model store. Mods borrow geometry and sprite
//! names without owning the renderer's models or affecting what the mesher draws.
use crate::{assets::RenderAssets, models::catalog::BakedModel};
use mod_api::{
    __private::{
        built,
        game::{BakedQuadSource, Fields},
    },
    BakedModelKey, BakedModelKeys, BakedModelView, BakedQuad, ModelError,
};
use sunrise_assets::model::Direction as FaceDirection;
use sunrise_core::geometry::Direction;
use sunrise_world::registry::BlockStateId;

impl RenderAssets {
    /// Keys of every baked model the resolved `state` can select, in bake order without repeats.
    /// Empty for a state with no binding in this store.
    pub fn block_state_models(&self, state: BlockStateId) -> BakedModelKeys<'_> {
        let slots = self.model_catalog.state_slots(state).unwrap_or_default();
        built::baked_model_keys(self.generation, slots)
    }

    /// Borrow the model `key` names, validating the key against this store's generation first.
    pub fn baked_model(&self, key: BakedModelKey) -> Result<BakedModelView<'_>, ModelError> {
        let fields = key.fields();
        if fields.generation != self.generation {
            return Err(ModelError::StaleModel);
        }
        let model = self
            .model_catalog
            .model_by_slot(fields.slot)
            .ok_or(ModelError::UnknownModel)?;
        let particle = model
            .particle
            .and_then(|layer| self.sprite_names.get(layer as usize)?.as_deref());
        Ok(built::baked_model_view(
            model.ambient_occlusion,
            particle,
            model,
        ))
    }
}

impl BakedQuadSource for BakedModel {
    fn len(&self) -> usize {
        self.faces.len()
    }

    fn quad(&self, index: usize) -> Option<BakedQuad<'_>> {
        let quad = &self.faces.get(index)?.quad;
        Some(built::baked_quad(
            &quad.positions,
            &quad.uv,
            &quad.material.sprite,
            direction(quad.facing),
            quad.cull.map(direction),
            quad.tint_index,
            quad.color.map(|argb| argb as u32),
            quad.shade,
            quad.light_emission,
        ))
    }
}

fn direction(face: FaceDirection) -> Direction {
    match face {
        FaceDirection::Down => Direction::Down,
        FaceDirection::Up => Direction::Up,
        FaceDirection::North => Direction::North,
        FaceDirection::South => Direction::South,
        FaceDirection::West => Direction::West,
        FaceDirection::East => Direction::East,
    }
}

#[cfg(test)]
#[path = "baked_model_views_tests.rs"]
mod tests;
