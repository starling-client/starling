//! The world border's server setters, guest synchronization, tick, collision and damage.
use super::{Dim, Game, Result};
use mod_api::LevelRef;
use sunrise_client::{lan::hub::Hub, worldborder::WorldBorder};
use sunrise_protocol::packets::play::{self, Clientbound as C};

/// A command's level in place or an unvalidated handle supplied by a mod.
#[derive(Clone, Copy)]
pub(in crate::game) enum BorderTarget {
    InPlace,
    Handle(LevelRef),
}

/// A resolved border and the player-list route for its level. Kept only for the write.
pub(in crate::game) struct LevelBorderMut<'a> {
    border: &'a mut WorldBorder,
    hub: Option<&'a Hub>,
    dimension: &'static str,
}

impl LevelBorderMut<'_> {
    /// `PlayerList.addWorldborderListener`: only watchers of the border's level get the packet.
    fn broadcast(&self, packet: &C) {
        if let Some(hub) = self.hub {
            hub.broadcast_in_level(self.dimension, packet);
        }
    }
}

impl Game<'_> {
    pub(in crate::game) fn border_mut(
        &mut self,
        target: BorderTarget,
    ) -> Result<LevelBorderMut<'_>> {
        let (dimension, border) = match target {
            BorderTarget::InPlace => (self.dim.id(), &mut self.border),
            BorderTarget::Handle(level) => {
                if !self.is_logical_server() {
                    return Err("not on the logical server".into());
                }
                if !self.level_handle_current(level) {
                    return Err("stale level handle".into());
                }
                let dimension = level.dimension_id();
                let border = if dimension == self.dim.id() {
                    &mut self.border
                } else {
                    &mut self
                        .levels
                        .parked_mut_of(dimension)
                        .ok_or("level is not resident")?
                        .border
                };
                (dimension, border)
            }
        };
        Ok(LevelBorderMut {
            border,
            hub: self.lan.as_ref().map(|host| &host.hub),
            dimension,
        })
    }

    fn border_level_ref(&self, target: BorderTarget) -> Option<LevelRef> {
        match target {
            BorderTarget::Handle(level) => Some(level),
            BorderTarget::InPlace if self.is_logical_server() => self
                .native_lifecycle
                .epoch()
                .map(|epoch| self.level_ref(epoch, self.dim.key())),
            BorderTarget::InPlace => None,
        }
    }

    /// `WorldBorder.setCenter`: notify even if the centre did not move.
    pub(in crate::game) fn set_border_center(
        &mut self,
        target: BorderTarget,
        center: [f64; 2],
    ) -> Result<()> {
        let border = self.border_mut(target)?;
        border.border.set_center(center)?;
        let [x, z] = border.border.center();
        border.broadcast(&C::SetBorderCenter(play::SetBorderCenter { x, z }));
        if let Some(level) = self.border_level_ref(target) {
            self.native_world_border_center_changed(level);
        }
        Ok(())
    }

    /// `WorldBorder.setSize` / `lerpSizeBetween`: read the packet back from the new extent.
    pub(in crate::game) fn set_border_size(
        &mut self,
        target: BorderTarget,
        size: f64,
        ticks: i64,
    ) -> Result<()> {
        let border = self.border_mut(target)?;
        let lerp_ticks = if ticks > 0 {
            border
                .border
                .lerp_between(border.border.size(), size, ticks)?;
            border.broadcast(&C::SetBorderLerpSize(play::SetBorderLerpSize {
                old_size: border.border.size(),
                new_size: border.border.target_size(),
                lerp_time: border.border.remaining_ticks(),
            }));
            Some(ticks)
        } else {
            border.border.set_size(size)?;
            border.broadcast(&C::SetBorderSize(play::SetBorderSize {
                size: border.border.size(),
            }));
            None
        };
        if let Some(level) = self.border_level_ref(target) {
            self.native_world_border_bounds_changed(level, lerp_ticks);
        }
        Ok(())
    }

    /// `WorldBorder.setDamagePerBlock`: a server setting, not broadcast.
    pub(in crate::game) fn set_border_damage_per_block(
        target: &mut LevelBorderMut<'_>,
        per_block: f64,
    ) {
        target.border.damage_per_block = per_block;
    }

    /// `WorldBorder.setSafeZone`: a server setting, not broadcast.
    pub(in crate::game) fn set_border_safe_zone(target: &mut LevelBorderMut<'_>, safe_zone: f64) {
        target.border.safe_zone = safe_zone;
    }

    /// `WorldBorder.setWarningBlocks`.
    pub(in crate::game) fn set_border_warning_blocks(target: &mut LevelBorderMut<'_>, blocks: i32) {
        target.border.warning_distance = blocks;
        target.broadcast(&C::SetBorderWarningDistance(
            play::SetBorderWarningDistance { blocks },
        ));
    }

    /// `WorldBorder.setWarningTime`.
    pub(in crate::game) fn set_border_warning_time(target: &mut LevelBorderMut<'_>, delay: i32) {
        target.border.warning_time = delay;
        target.broadcast(&C::SetBorderWarningDelay(play::SetBorderWarningDelay {
            delay,
        }));
    }

    /// `ServerLevel.tick` advances its border before weather, including on an idle level.
    /// Publish a finish only for a moving extent that reached its target.
    pub(in crate::game) fn tick_worldborder(&mut self, dim: Dim) {
        let border = if dim == self.dim {
            &mut self.border
        } else if let Some(parked) = self.levels.parked_mut(dim) {
            &mut parked.border
        } else {
            return;
        };
        let remaining = border.remaining_ticks();
        if let Err(error) = border.tick() {
            log::warn!("world border tick refused in {}: {error}", dim.id());
        } else if remaining > 0
            && border.remaining_ticks() == 0
            && self.is_logical_server()
            && let Some(epoch) = self.native_lifecycle.epoch()
        {
            let level = self.level_ref(epoch, dim.key());
            self.native_world_border_bounds_change_finish(level);
        }
    }
    /// The player's movement collides with the border as it is now
    /// (`CollisionGetter.borderCollision`): from inside, its walls stop it.
    pub(in crate::game) fn player_border(&mut self) {
        if let Some(p) = self.player.as_mut() {
            p.border = Some(self.border.geometry().clone());
        }
    }
    /// `LivingEntity.baseTick`: a player outside the border, further than
    /// its safe zone, is hurt by the blocks beyond it. The game's player, or
    /// a guest's in their turn (`as_guest`).
    pub(in crate::game) fn border_damage(&mut self) {
        let Some((px, pz, half)) = self.player.as_ref().map(|p| {
            (
                p.position.x,
                p.position.z,
                f64::from(p.dimensions.width) / 2.0,
            )
        }) else {
            return;
        };
        let inside = self.border.contains(px - half, pz - half)
            && self.border.contains(px + half, pz + half);
        if inside {
            return;
        }
        let dist = self.border.distance(px, pz) + self.border.safe_zone;
        if dist < 0.0 && self.border.damage_per_block > 0.0 && self.ticks.is_multiple_of(10) {
            let amount = (-dist * self.border.damage_per_block).floor().max(1.0) as f32;
            self.hurt_player_by("minecraft:outside_border", amount);
        }
    }
}
