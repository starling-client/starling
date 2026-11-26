//! A player's blow at a ghast's fireball or a wind charge
//! (`#minecraft:redirectable_projectile`): `Player.attack` turns it with
//! `ProjectileDeflection.AIM_DEFLECT` instead of hurting anything. It flies on
//! along the player's view and is the player's now (`Projectile.deflect`
//! sets the owner), so a ghast it meets dies of it (`Ghast.isReflectedFireball`).
use super::Game;
use sunrise_client::{
    mobs::Who,
    projectiles::{Projectile, ProjectileKind, Turn},
};
use sunrise_core::geometry::Vec3;

/// `AbstractHurtingProjectile.getPickRadius`: the crosshair finds it a block
/// round its box.
const PICK_RADIUS: f64 = 1.0;

/// `#minecraft:redirectable_projectile`: a ghast's fireball and both wind charges.
pub(super) fn redirectable(p: &Projectile) -> bool {
    !p.stuck
        && matches!(
            p.kind,
            ProjectileKind::LargeFireball { .. }
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        )
}

/// `Projectile.onRedirectProjectile`: another projectile hit `p`, which turns
/// with `deflect(AIM_DEFLECT, hitter's owner, hitter's owner, true, 1.0)`.
/// It flies along the owner's look (`Entity.getLookAngle`; unchanged when the
/// owner is gone), is that owner's now, and its acceleration power is back
/// to 0.1 (`AbstractHurtingProjectile.onDeflection` by an attack). A wind
/// charge in its first five ticks refuses (`WindCharge.deflect`): false, and
/// nothing changes.
pub(super) fn aim_deflect(
    p: &mut Projectile,
    look: Option<Vec3>,
    owner: Option<Who>,
    owner_uuid: Option<[i32; 4]>,
    guest: Option<u32>,
) -> bool {
    p.deflect(
        Turn::Direction(look),
        [1.0; 3],
        (owner, owner_uuid, guest),
        true,
    )
}

/// A projectile a player's blow reaches under the vanilla default tag: a redirectable
/// projectile or a shulker bullet (`ShulkerBullet.isPickable` is `true`).
pub(super) fn attackable(p: &Projectile) -> bool {
    redirectable(p) || matches!(p.kind, ProjectileKind::ShulkerBullet { .. })
}

impl Game<'_> {
    /// Whether `kind` belongs to `#minecraft:redirectable_projectile`, reading the server's synced
    /// entity-type tags on a remote client when present, else the loaded entity tags.
    pub(in crate::game) fn is_redirectable_projectile_type(&self, kind: &str) -> bool {
        if let Some(remote) = &self.remote
            && let Some(tags) = remote.registries.tags.get("minecraft:entity_type")
            && let Some(ids) = remote.builtin.get("minecraft:entity_type")
        {
            return tags
                .get("minecraft:redirectable_projectile")
                .is_some_and(|members| ids.id(kind).is_some_and(|id| members.contains(&id)));
        }
        if let Some(members) = self
            .items()
            .entity_type_tags
            .members()
            .get("minecraft:redirectable_projectile")
        {
            return members.contains(kind);
        }
        matches!(
            kind,
            "minecraft:fireball" | "minecraft:wind_charge" | "minecraft:breeze_wind_charge"
        )
    }

    /// `Projectile.isPickable` (with `ShulkerBullet` and `AbstractArrow` overrides).
    pub(in crate::game) fn projectile_is_pickable(&self, p: &Projectile) -> bool {
        if matches!(p.kind, ProjectileKind::ShulkerBullet { .. }) {
            return true;
        }
        if p.is_arrow() && p.stuck {
            return false;
        }
        self.is_redirectable_projectile_type(p.entity_type())
    }

    /// `Projectile.getPickRadius`: `this.isPickable() ? 1.0F : 0.0F`.
    pub(in crate::game) fn projectile_pick_radius(&self, p: &Projectile) -> f64 {
        if self.projectile_is_pickable(p) {
            PICK_RADIUS
        } else {
            0.0
        }
    }

    /// `AbstractArrow.isPickable` for `ThrownTrident`.
    pub(in crate::game) fn trident_is_pickable(&self, t: &super::trident::ThrownTrident) -> bool {
        !t.in_ground && self.is_redirectable_projectile_type("minecraft:trident")
    }

    /// `Projectile.isPickable` for `FireworkRocketEntity`.
    pub(in crate::game) fn rocket_is_pickable(&self) -> bool {
        self.is_redirectable_projectile_type("minecraft:firework_rocket")
    }
    /// The pickable projectile under the crosshair within entity reach.
    pub(super) fn redirectable_hit(&self) -> Option<(usize, f64)> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, _) = hit?;
            let index = self
                .creatures
                .projectiles
                .iter()
                .position(|f| f.id == entity.id() && self.projectile_is_pickable(f))?;
            return Some((index, distance));
        }
        let super::pick_entity::CrosshairDecision::Entity(hit) = self.compute_crosshair_decision()
        else {
            return None;
        };
        let index = self
            .creatures
            .projectiles
            .iter()
            .position(|f| f.id == hit.id && self.projectile_is_pickable(f))?;
        Some((index, hit.distance_sq.sqrt()))
    }

    /// `Player.attack` on a redirectable projectile nearer than any mob or
    /// block in view: `AIM_DEFLECT` (its movement the unit view vector), the
    /// player its owner, its acceleration power back to 0.1
    /// (`onDeflection` by an attack), and the no-damage sound. True when a
    /// projectile was turned.
    pub(super) fn attack_redirectable(&mut self) -> bool {
        let Some((i, t)) = self.redirectable_hit() else {
            return false;
        };
        if self.mob_hit().is_some_and(|(_, d, _)| d < t) {
            return false;
        }
        if !self.uses_stored_pick()
            && let (Some(hit), Some(eye)) = (self.pick_unfiltered(), self.eye_position())
        {
            let d = (0..3)
                .map(|a| (hit.point[a] - eye[a]).powi(2))
                .sum::<f64>()
                .sqrt();
            if d < t {
                return false;
            }
        }
        if matches!(
            self.creatures.projectiles[i].kind,
            ProjectileKind::ShulkerBullet { .. }
        ) {
            self.hit_shulker_bullet(i);
        } else {
            self.deflect(i);
        }
        true
    }

    /// `ShulkerBullet.hurtServer` from a player's blow: the hurt sound, fifteen crit
    /// particles (`sendParticles`, spread 0.2) and `destroy`. The blow is
    /// `Player.attack` on a target that is not a mob: its sounds, then the swing.
    pub(super) fn hit_shulker_bullet(&mut self, i: usize) {
        let ((_, _, _, magic), admitted) = self.melee_damage_with_admission(
            "minecraft:shulker_bullet",
            false,
            Some(self.creatures.projectiles[i].id),
        );
        let sweep = self.sweep_before_hit("minecraft:shulker_bullet", false);
        let p = self.destroy_shulker_bullet(i);
        self.vehicle_attack_sounds(p.aabb(), sweep, magic.then_some(p.id as i32));
        if admitted {
            self.remember_player_target(None);
        }
        self.reset_attack_strength();
        self.swing_arm();
    }

    /// `ShulkerBullet.hurtServer`, whatever hurt it: the hurt sound, fifteen
    /// crit particles (`sendParticles`, spread 0.2), then `destroy`
    /// (`discard` and `gameEvent(ENTITY_DAMAGE)` from the bullet).
    pub(in crate::game) fn destroy_shulker_bullet(&mut self, i: usize) -> Projectile {
        let (id, at, owner, owner_uuid, silent) = {
            let p = &self.creatures.projectiles[i];
            (
                p.id,
                [p.position.x, p.position.y, p.position.z],
                p.owner,
                p.owner_uuid,
                self.entity_silent(p.id),
            )
        };
        let ctx = sunrise_client::sculk::Context {
            owner,
            owner_uuid,
            ..self.entity_event_context(id, None)
        };
        self.shulker_bullet_hurt(at, silent);
        let p = self.remove_projectile_at(i);
        self.post_game_event("minecraft:entity_damage", at, ctx);
        p
    }

    /// `ShulkerBullet.hurtServer`'s sound (unless silent) and fifteen crit particles
    /// (`sendParticles`, spread 0.2) at the bullet, before it is discarded.
    pub(super) fn shulker_bullet_hurt(&mut self, at: [f64; 3], silent: bool) {
        if !silent {
            self.play_audio("minecraft:entity.shulker_bullet.hurt", Some(at), 1.0, 1.0);
        }
        if let Some(ty) = sunrise_client::particles::ParticleType::from_name("crit") {
            let options = sunrise_client::particles::ParticleOptions::simple(ty);
            self.particles
                .send_particles(&options, at, [0.2, 0.2, 0.2], 0.0, 15, None);
            self.lan_level_particles(&options, at, [0.2, 0.2, 0.2], 0.0, 15, None);
        }
    }
    /// `Player.canInteractWithEntity(box, 3.0)` as the server checks an
    /// attack packet: the projectile's box within the reach plus 3 of the eyes.
    pub(super) fn within_interaction_range(&self, i: usize) -> bool {
        let (Some(eye), Some(p)) = (self.eye_position(), self.creatures.projectiles.get(i)) else {
            return false;
        };
        // A client's copy takes the box of its `entity_size` answer.
        let b = self.client_copy_box(p.id, p.position, p.bounds());
        let (lo, hi) = (b.min_position(), b.max_position());
        let (lo, hi) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
        let d2: f64 = (0..3)
            .map(|a| (eye[a] - eye[a].clamp(lo[a], hi[a])).powi(2))
            .sum();
        let range = self.entity_reach + 3.0;
        d2 < range * range
    }

    /// `Entity.getLookAngle` of a projectile's owner: the host, the guest who
    /// threw it, or a mob; none when the owner is not here.
    pub(super) fn owner_look(&self, owner: Option<Who>, guest: Option<u32>) -> Option<Vec3> {
        let (pitch, yaw) = match (owner?, guest) {
            (Who::Player, Some(conn)) => {
                let lan = self.lan.as_ref()?;
                let g = lan.hub.guests.iter().find(|g| g.conn == conn)?;
                (g.pitch, g.yaw)
            }
            (Who::Player, None) => {
                self.player.as_ref()?;
                (self.pitch, self.yaw)
            }
            (Who::Mob(id), _) => {
                let m = self.mobs.iter().find(|m| m.id == id)?;
                (m.head_pitch, m.yaw)
            }
        };
        Some(sunrise_world::view_direction::view_vector(pitch, yaw))
    }

    /// `Player.attack` on a redirectable projectile: `onAttack` resets the
    /// attack strength, then `Projectile.deflect(AIM_DEFLECT, player, player,
    /// true)`. When the projectile refuses (a wind charge in its first five
    /// ticks) the blow goes on as an ordinary attack, which a projectile does
    /// not take: `hurtOrSimulate` fails and `PLAYER_ATTACK_NODAMAGE` plays,
    /// the same sound. Either way the swing follows. True when it turned.
    pub(super) fn deflect(&mut self, i: usize) -> bool {
        self.reset_attack_strength();
        let turned = self.turn_projectile(i);
        self.swing_arm();
        let from = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        self.play_audio("minecraft:entity.player.attack.nodamage", from, 1.0, 1.0);
        turned
    }

    /// The deflection itself: the view as the new movement, the player as
    /// owner. False, and nothing changes, when the projectile refuses.
    fn turn_projectile(&mut self, i: usize) -> bool {
        let (pitch, yaw) = (self.pitch, self.yaw);
        let (guest, owner_uuid) = (self.lan_acting, Some(self.acting_player_uuid()));
        self.creatures
            .projectiles
            .update(i, |p| attack_deflect(p, pitch, yaw, owner_uuid, guest))
    }
}

/// `Player.attack`'s `deflect(AIM_DEFLECT, player, player, true)`: [`aim_deflect`] along the
/// player's look at `pitch` and `yaw` (`Entity.getLookAngle`, vanilla's sine table), with `p`
/// turned to it. False, and nothing changes, when the projectile refuses.
fn attack_deflect(
    p: &mut Projectile,
    pitch: f32,
    yaw: f32,
    owner_uuid: Option<[i32; 4]>,
    guest: Option<u32>,
) -> bool {
    let look = sunrise_world::view_direction::view_vector(pitch, yaw);
    if !aim_deflect(p, Some(look), Some(Who::Player), owner_uuid, guest) {
        return false;
    }
    // As Projectile.updateRotation turns it: Mth.atan2 times the folded 180F / (float)PI.
    p.yaw = sunrise_core::geometry::trig::atan2_degrees(look.x, look.z);
    p.pitch = sunrise_core::geometry::trig::atan2_degrees(look.y, look.horizontal_distance());
    // Not into the one who turned it on its way out.
    p.left_owner = false;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::geometry::Aabb;

    /// `Projectile.getPickRadius`: `isPickable() ? 1.0F : 0.0F` (`ShulkerBullet` inherits this).
    fn pick_radius(p: &Projectile) -> f64 {
        if attackable(p) { PICK_RADIUS } else { 0.0 }
    }

    /// Its box for the crosshair: the entity's box inflated by `Projectile.getPickRadius` (1.0 when pickable).
    fn pick_box(p: &Projectile) -> Aabb {
        let radius = pick_radius(p);
        p.bounds().inflate(Vec3::new(radius, radius, radius))
    }

    #[test]
    fn only_ghast_fireballs_and_wind_charges_are_redirectable() {
        let at = Vec3::new(0.0, 64.0, 0.0);
        let large = Projectile::fireball(
            ProjectileKind::LargeFireball { power: 1 },
            at,
            None,
            [1.0, 0.0, 0.0],
        );
        let small = Projectile::fireball(ProjectileKind::SmallFireball, at, None, [1.0, 0.0, 0.0]);
        assert!(redirectable(&large));
        assert!(!redirectable(&small));
        // A shulker bullet is pickable (`ShulkerBullet.isPickable`) and inherits `Projectile.getPickRadius` (1.0).
        let bullet = Projectile::new(
            ProjectileKind::ShulkerBullet {
                target: Some(Who::Player),
                dir: None,
                steps: 0,
                delta: [0.0; 3],
                seed: 0,
            },
            at,
            None,
        );
        assert!(attackable(&bullet) && !redirectable(&bullet));
        assert!(!attackable(&small));
        let b = pick_box(&bullet);
        assert_eq!(b.min_position().y, 63.0);
        assert_eq!(b.max_position().y, 65.3125);
        // The pick box: a block round the fireball's 1 x 1 box.
        let b = pick_box(&large);
        assert_eq!(b.min_position().x, -1.5);
        assert_eq!(b.max_position().y, 66.0);
    }

    /// A wind charge past its first five ticks (`noDeflectTicks` run out).
    fn wind_charge() -> Projectile {
        let mut p = Projectile::new(
            ProjectileKind::WindCharge,
            Vec3::new(0.0, 70.0, 0.0),
            Some(Who::Mob(7)),
        );
        p.velocity = Vec3::new(0.0, 1.5, 0.0);
        // Past `WindCharge.noDeflectTicks`.
        p.no_deflect_ticks = 0;
        p
    }

    #[test]
    fn a_new_wind_charge_refuses_a_redirect_for_five_ticks() {
        let mut p = wind_charge();
        p.no_deflect_ticks = 5;
        let look = Vec3::new(0.0, -1.0, 0.0);
        assert!(!aim_deflect(
            &mut p,
            Some(look),
            Some(Who::Player),
            None,
            None
        ));
        assert_eq!(p.velocity, Vec3::new(0.0, 1.5, 0.0));
        assert_eq!(p.owner, Some(Who::Mob(7)));
        assert!(!p.needs_sync);
        p.no_deflect_ticks = 0;
        assert!(aim_deflect(
            &mut p,
            Some(look),
            Some(Who::Player),
            None,
            None
        ));
        assert_eq!(p.velocity, look);
    }

    /// A player's blow sends it along `Entity.calculateViewVector(0, 0.001)`, on HotSpot 25.0.4.1
    /// (0.0, -0.0, 1.0) as double bits: `Mth.sin`'s table leaves no x at that yaw.
    #[test]
    fn a_blow_turns_it_along_vanillas_look() {
        let mut p = wind_charge();
        assert!(attack_deflect(&mut p, 0.0, 0.001, None, None));
        assert_eq!(
            [p.velocity.x, p.velocity.y, p.velocity.z].map(f64::to_bits),
            [0, 0x8000_0000_0000_0000, 0x3ff0_0000_0000_0000]
        );
    }

    #[test]
    fn a_redirected_wind_charge_flies_along_the_new_owners_look() {
        let mut p = wind_charge();
        let look = Vec3::new(0.0, -1.0, 0.0);
        aim_deflect(&mut p, Some(look), Some(Who::Player), Some([4; 4]), Some(3));
        assert_eq!(p.velocity, look);
        assert_eq!((p.owner, p.owner_uuid), (Some(Who::Player), Some([4; 4])));
        assert_eq!(p.guest, Some(3));
        assert_eq!(p.acceleration, 0.1);
        assert!(p.needs_sync);
    }

    #[test]
    fn with_no_owner_here_it_keeps_its_movement_but_changes_hands() {
        let mut p = wind_charge();
        p.owner_uuid = Some([7; 4]);
        aim_deflect(&mut p, None, None, None, None);
        assert_eq!(p.velocity, Vec3::new(0.0, 1.5, 0.0));
        assert_eq!((p.owner, p.owner_uuid), (None, None));
        assert_eq!(p.acceleration, 0.1);
        assert!(!p.needs_sync);
    }
}
