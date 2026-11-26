use super::*;
use sunrise_core::geometry::Vec3;

impl Game<'_> {
    /// `BlockPlaceContext.getPlayer`: the active server player or the client doing prediction.
    pub(in crate::game) fn placement_placer(&self) -> Option<mod_api::PlacementPlacer> {
        let player = if self.remote.is_some() {
            self.local_player()?
        } else {
            self.native_acting_player()?
        };
        Some(mod_api::PlacementPlacer::new(player, self.eye_position()?))
    }
}

impl GameLevel<'_> {
    /// `BlockItem.updatePlacementContext`, keyed by the placing stack.
    pub(in crate::game) fn native_placement_context(
        &self,
        stack: &ItemStack,
        context: &PlacementContext,
        placer: Option<mod_api::PlacementPlacer>,
        answer: &mut Option<Option<mod_api::PlacementContext>>,
    ) {
        self.fire_placement_context(stack, context, placer, answer);
    }

    /// Convert the part's answer at the mod boundary, before the world checks the destination.
    pub(in crate::game) fn rewrite_placement_context(
        &self,
        stack: &ItemStack,
        context: &PlacementContext,
        placer: Option<mod_api::PlacementPlacer>,
    ) -> Option<Option<PlacementContext>> {
        let mut answer = None;
        self.native_placement_context(stack, context, placer, &mut answer);
        answer.map(|answer| {
            answer.and_then(|snapshot| {
                let item = stack.item.as_ref();
                let Some(source) =
                    crate::game::api_values::placement_source_into_engine(snapshot.source)
                else {
                    log::warn!(
                        "placement_context: refused {item}: invalid or nonfinite placement source"
                    );
                    return None;
                };
                if !snapshot.hit.iter().all(|n| n.is_finite()) {
                    log::warn!("placement_context: refused {item}: nonfinite hit location");
                    return None;
                }
                if snapshot.item.and_then(mod_api::__private::item_key_id) != Some(item) {
                    log::warn!("placement_context: refused {item}: replacement changed the item");
                    return None;
                }
                Some(PlacementContext {
                    pos: snapshot.pos,
                    hit_pos: snapshot.hit_pos,
                    face: snapshot.face,
                    hit: Vec3::new(snapshot.hit[0], snapshot.hit[1], snapshot.hit[2]),
                    inside: snapshot.inside,
                    source,
                    item: context.item.clone(),
                    replace_clicked: snapshot.replace_clicked,
                })
            })
        })
    }
}
