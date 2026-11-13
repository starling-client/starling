use super::*;

impl HitLevel<'_> {
    /// NeoForge's `LivingDamageEvent.Post`: the health of entity `entity` of type `kind` has
    /// just lost the hit `container` describes, with its final values (health damage, the
    /// reductions, the post-attack invulnerability ticks, and `should_cause_side_effects`,
    /// `hurtServer`'s fresh-hit decision). It only listens, and fires where
    /// `native_living_damage_pre` does once the health took more than 0. It can't write the
    /// entity or the world's blocks: the level host gives the hit's random and level attachments.
    pub(in crate::game) fn native_living_damage_post(
        &mut self,
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        container: &mod_api::DamageContainer,
    ) {
        self.fire_living_damage_post(entity, kind, source, container);
    }
}
