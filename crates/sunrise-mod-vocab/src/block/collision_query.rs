//! What a collision shape query knows of the entity asking: vanilla's `EntityCollisionContext`.
use std::sync::Arc;

/// The class of the entity a collision query was made for, as the context captures it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: the four classes the game's context separates, as `ShapeEntityKind` has.
#[derive(sunrise_mod_api_macros::Wire)]
pub enum CollisionEntityKind {
    /// An item entity.
    Item,
    /// A player.
    Player,
    /// Any other living entity.
    Living,
    /// Anything else, and a query made for no entity.
    #[default]
    Other,
}

/// What a block part's `collision_shape` is told about one collision query: vanilla's
/// `EntityCollisionContext`, captured when the query was made, and nothing else.
///
/// The reads are vanilla's: [`Self::entity`] (`getEntity`, as the entity's id: `None` for the
/// empty, position and cache contexts), [`Self::is_descending`], [`Self::is_placement`],
/// [`Self::always_collide_with_fluid`], [`Self::is_above`], [`Self::is_holding_item`] and
/// [`Self::can_stand_on_fluid`], with [`Self::dimension`] for the level the query is in (the
/// `level` vanilla's `getCollisionShape` is given). They are the entity's state from before the move that asks:
/// the query holds no `Host`, so nothing about the entity can be read while the move runs. A rule
/// that needs more (a belt's set of carried entities, say) keeps its own state keyed by
/// [`Self::entity`] and syncs it to the client, which asks the same question for its own
/// prediction.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: a query is all its reads; the game builds one per question and a mod's test may
// build one by hand through `new`.
#[derive(sunrise_mod_api_macros::Wire)]
pub struct CollisionQuery {
    entity: Option<u32>,
    kind: CollisionEntityKind,
    /// The entity's y when it is known, for `is_above`; none answers `is_above` with its default.
    bottom: Option<f64>,
    descending: bool,
    placement: bool,
    always_collide_with_fluid: bool,
    /// The main hand's item id for a living entity.
    held_item: Option<Arc<str>>,
    /// The fluids the entity can stand on, when it is a living one (`Entity.canStandOnFluid`).
    standable_fluids: Arc<[Arc<str>]>,
    /// The dimension id of the level the query is in.
    dimension: Option<Arc<str>>,
}

impl CollisionQuery {
    /// A query for the entity `entity` of class `kind`, standing at `bottom` (`Entity.getY()`);
    /// no entity and no `bottom` for the contexts that carry none.
    pub fn new(entity: Option<u32>, kind: CollisionEntityKind, bottom: Option<f64>) -> Self {
        Self {
            entity,
            kind,
            bottom,
            ..Self::default()
        }
    }

    /// The same query, for an entity that is (or is not) descending (`Entity.isDescending`).
    #[must_use]
    pub fn with_descending(mut self, descending: bool) -> Self {
        self.descending = descending;
        self
    }

    /// The same query, made to place a block (`CollisionContext.placementContext`).
    #[must_use]
    pub fn with_placement(mut self, placement: bool) -> Self {
        self.placement = placement;
        self
    }

    /// The same query, for a mover that collides with every fluid.
    #[must_use]
    pub fn with_always_collide_with_fluid(mut self, always: bool) -> Self {
        self.always_collide_with_fluid = always;
        self
    }

    /// The same query, for a living entity holding `item` (an item id) in its main hand. The game
    /// shares the id between queries rather than copying it.
    #[must_use]
    pub fn with_held_item(mut self, item: Arc<str>) -> Self {
        self.held_item = Some(item);
        self
    }

    /// The same query, for a living entity that can stand on `fluids` (fluid ids).
    #[must_use]
    pub fn with_standable_fluids(mut self, fluids: Arc<[Arc<str>]>) -> Self {
        self.standable_fluids = fluids;
        self
    }

    /// The same query, made in the level whose dimension id is `dimension`. The game shares the
    /// id between queries rather than copying it.
    #[must_use]
    pub fn with_dimension(mut self, dimension: Arc<str>) -> Self {
        self.dimension = Some(dimension);
        self
    }

    /// The dimension id of the level the query is in (`minecraft:overworld`, a mod's own): the
    /// `level` vanilla's `getCollisionShape(state, level, pos, context)` is given. One position
    /// exists in every dimension, so a part that keeps its own state by position keys it by this
    /// too. Every query the game makes names its level; `None` only for a query built without
    /// [`Self::with_dimension`].
    pub fn dimension(&self) -> Option<&str> {
        self.dimension.as_deref()
    }

    /// `EntityCollisionContext.getEntity`: the id of the entity the query is for, `None` for a
    /// query that has none (the empty, position and cache contexts).
    pub fn entity(&self) -> Option<u32> {
        self.entity
    }

    /// The class of the entity the query is for; [`CollisionEntityKind::Other`] without one.
    pub fn kind(&self) -> CollisionEntityKind {
        self.kind
    }

    /// `CollisionContext.isDescending`.
    pub fn is_descending(&self) -> bool {
        self.descending
    }

    /// `CollisionContext.isPlacement`.
    pub fn is_placement(&self) -> bool {
        self.placement
    }

    /// `CollisionContext.alwaysCollideWithFluid`.
    pub fn always_collide_with_fluid(&self) -> bool {
        self.always_collide_with_fluid
    }

    /// `CollisionContext.isAbove(shape, pos, defaultValue)`: whether the entity is above `shape`,
    /// whose boxes (`[min_x, min_y, min_z, max_x, max_y, max_z]`, block-local) sit at `pos`: its
    /// y is over the shape's top by more than 1e-5. `default` when the query has no y.
    pub fn is_above(&self, shape: &[[f64; 6]], pos: [i32; 3], default: bool) -> bool {
        let Some(bottom) = self.bottom else {
            return default;
        };
        let top = shape.iter().map(|b| b[4]).fold(f64::NEG_INFINITY, f64::max);
        bottom > f64::from(pos[1]) + top - f64::from(1.0e-5f32)
    }

    /// `CollisionContext.isHoldingItem`: whether the entity's main hand holds the item `item`
    /// (an item id, `minecraft:air` for an empty hand). A query that captured no held item (no
    /// living entity) holds `ItemStack.EMPTY`, so only `minecraft:air` is held.
    pub fn is_holding_item(&self, item: &str) -> bool {
        self.held_item
            .as_deref()
            .map_or(item == "minecraft:air", |held| held == item)
    }

    /// `CollisionContext.canStandOnFluid(above, fluid)`: whether the living entity can stand on
    /// `fluid` (a fluid id; a flowing fluid counts as its source, as `FluidTags` do) under
    /// `above`, which is not the same fluid. False for an entity that is not a living one.
    pub fn can_stand_on_fluid(&self, above: &str, fluid: &str) -> bool {
        // The id with its namespace and `flowing_` prefix removed, so a flowing fluid and its
        // source compare equal; another namespace keeps its whole id.
        fn family(id: &str) -> &str {
            id.strip_prefix("minecraft:flowing_")
                .or_else(|| id.strip_prefix("minecraft:"))
                .unwrap_or(id)
        }
        let fluid = family(fluid);
        self.standable_fluids.iter().any(|f| family(f) == fluid) && family(above) != fluid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strider_stands_on_flowing_lava_but_not_under_it() {
        let query = CollisionQuery::default()
            .with_standable_fluids(Arc::from([Arc::<str>::from("minecraft:lava")]));
        assert!(query.can_stand_on_fluid("minecraft:water", "minecraft:lava"));
        assert!(query.can_stand_on_fluid("minecraft:water", "minecraft:flowing_lava"));
        assert!(!query.can_stand_on_fluid("minecraft:flowing_lava", "minecraft:lava"));
        assert!(!query.can_stand_on_fluid("minecraft:lava", "minecraft:flowing_lava"));
        assert!(!query.can_stand_on_fluid("minecraft:lava", "minecraft:water"));
    }

    #[test]
    fn an_entity_without_a_captured_hand_holds_air() {
        let query = CollisionQuery::default();
        assert!(query.is_holding_item("minecraft:air"));
        assert!(!query.is_holding_item("minecraft:stick"));
        let holding = query.with_held_item("minecraft:stick".into());
        assert!(
            holding.is_holding_item("minecraft:stick") && !holding.is_holding_item("minecraft:air")
        );
    }
}
