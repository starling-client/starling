use super::*;

impl Game<'_> {
    /// `LivingEntity.knockback` on the player: pushed `power` away from the
    /// direction (`xd`, `zd`), less its knockback resistance, and up only from
    /// the ground.
    /// Delegates to `knock_back_player_full` with `comes_from_effect = false`, matching
    /// vanilla `LivingEntity.knockback(power, xd, zd, source, damage)`.
    /// Both entries are native internals points. API access refuses `HurtSource` and
    /// `Attack`, which have no point mirrors.
    pub(in crate::game) fn knock_back_player(
        &mut self,
        power: f64,
        xd: f64,
        zd: f64,
        source: HurtSource<'_>,
        attack: &Attack,
        damage: f32,
    ) {
        self.knock_back_player_full(power, xd, zd, source, attack, damage, false);
    }

    /// Full living knockback entry. Effect pushes enter here directly.
    /// Damage and the effect flag are carried even though vanilla players do not use them.
    pub(in crate::game) fn knock_back_player_full(
        &mut self,
        power: f64,
        xd: f64,
        zd: f64,
        source: HurtSource<'_>,
        attack: &Attack,
        _damage: f32,
        _comes_from_effect: bool,
    ) {
        // No attacker named: the mob the player was last hurt by.
        let credited;
        let attack = if attack.attacker == 0
            && let Some((Who::Mob(id), _)) = self.player_hurt_by()
        {
            credited = self.attack_of(id as i32);
            &credited
        } else {
            attack
        };
        let plan = self.prepare_knockback_credit(
            self.player.as_ref().map_or(0, |p| p.entity_id),
            None,
            attack,
        );
        let mut credit = plan.lend(&mut self.last_hurt_by_player);
        if let Some(p) = self.player.as_mut() {
            let level = crate::game::entity::damage_level_of!(self, attack, &mut credit, None);
            apply_player_knockback(p, level, power, (xd, zd), source);
        }
    }
}

/// Apply the full entry's physical push.
pub(super) fn apply_player_knockback(
    p: &mut sunrise_client::motion::PlayerMotion,
    level: &mut sunrise_client::health::DamageLevel<'_>,
    power: f64,
    (xd, zd): (f64, f64),
    source: HurtSource<'_>,
) {
    let knock = Knock {
        strength: power,
        x: xd,
        z: zd,
    };
    // A clone is the owner's state, so it goes back whether or not the push lands.
    let mut random = p.random.clone();
    let pushed = player_knockback_velocity(p, &mut random, level, knock, source);
    p.random = random;
    if let Some(velocity) = pushed {
        p.velocity = velocity;
    }
}

/// The full player path, shared with calls that borrow a guest's body in its own level.
/// `random` is the player's own (`Entity.random`), which a short direction's retries draw from;
/// the caller writes it back to the player.
pub(in crate::game) fn player_knockback_velocity(
    player: &sunrise_client::motion::PlayerMotion,
    random: &mut sunrise_core::random::EntityRandom,
    level: &mut sunrise_client::health::DamageLevel<'_>,
    mut knock: Knock,
    source: HurtSource<'_>,
) -> Option<Vec3> {
    let pushed = Hurting::Player(player.entity_id, source);
    if level.knockback_canceled(pushed, &mut knock) {
        return None;
    }
    let resistance = player
        .attributes
        .value(
            player.attributes.key("minecraft:knockback_resistance"),
            sunrise_world::attribute_map::ValueKind::Computed,
        )
        .unwrap_or(0.0);
    let mut velocity = player.velocity;
    knock
        .apply(&mut velocity, resistance, player.ground.on_ground, random)
        .then_some(velocity)
}
