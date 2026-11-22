//! What a projectile can strike: the mobs, the player, the dragon's parts and
//! End crystals, and the non-living entities that are pickable (boats,
//! minecarts, item frames).
use super::super::Game;
use sunrise_client::join_seq::ListStamp;
use sunrise_client::mobs::{Species, Who};
use sunrise_core::geometry::{Aabb, Vec3};

/// What a projectile's hit tests read of the level, kept to see whether a hook told during the
/// projectiles' loop changed it: the write stamps of the entity lists the targets come from, and
/// the boxes of the dragon's parts and of the player, which no list stamps. The guests are
/// compared with their targets as they were (`Game::lan_projectile_targets_are`).
#[derive(PartialEq)]
pub(in crate::game) struct TargetsRead {
    lists: [ListStamp; 6],
    dragon: Option<[Aabb; 8]>,
    player: Option<Aabb>,
}

/// What the server's projectiles' ticks read of the other entities: built for the family's loop,
/// and again when a hook told in it changed what they read.
pub(in crate::game) struct ProjectileTargets {
    /// What a projectile can hit.
    pub targets: Vec<(Who, Aabb)>,
    /// The guests a mob's projectile can hit too.
    pub guests: Vec<(Who, Aabb)>,
    /// `targets` and `guests`, when there are guests.
    pub targets_guests: Vec<(Who, Aabb)>,
    pub has_guests: bool,
    /// The breezes, which turn a projectile away.
    pub breezes: Vec<u32>,
    /// Every guest, by connection and entity id: a dead or spectating guest's pearl or arrow
    /// still resolves to them.
    pub guest_entities: Vec<(u32, u32)>,
    /// Whether the host's player is dead, and the dead guests' connections
    /// (`ThrownEnderpearl.tick`'s ender_pearls_vanish_on_death).
    pub host_dead: bool,
    pub dead_guests: Vec<u32>,
}

impl Game<'_> {
    /// What the server's projectiles' ticks read of the other entities.
    pub(in crate::game) fn projectile_tick_targets(&self) -> ProjectileTargets {
        let targets = self.projectile_targets();
        let breezes = self
            .mobs
            .iter()
            .filter(|m| m.alive() && m.species() == Species::Breeze)
            .map(|m| m.id)
            .collect();
        // Mobs' projectiles also hit the guests (game/network/lan_mobs.rs).
        let guests = self.lan_projectile_targets();
        let has_guests = !guests.is_empty();
        let targets_guests = if has_guests {
            let mut targets_guests = targets.clone();
            targets_guests.extend_from_slice(&guests);
            targets_guests
        } else {
            Vec::new()
        };
        let dead_guests = self.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| g.dead)
                .map(|g| g.conn)
                .collect()
        });
        ProjectileTargets {
            targets,
            guests,
            targets_guests,
            has_guests,
            breezes,
            guest_entities: self.lan_guest_entities(),
            host_dead: self.player_view().is_some_and(|p| !p.alive),
            dead_guests,
        }
    }

    /// What a projectile's hit tests read now ([`TargetsRead`]): asked around a hook told in the
    /// projectiles' loop, it costs the same whatever the number of entities.
    pub(in crate::game) fn projectile_targets_read(&self) -> TargetsRead {
        TargetsRead {
            lists: [
                self.mobs.stamp(),
                self.boats.stamp(),
                self.carts.stamp(),
                self.tnts.stamp(),
                self.item_frames.stamp(),
                self.crystals.stamp(),
            ],
            dragon: self.dragon_part_boxes(),
            player: self.player_view().filter(|p| p.alive).map(|p| p.bounds),
        }
    }

    /// Whether a hook told in the projectiles' loop changed what their hit tests read since
    /// `read` was taken and `seen` built: then the next projectile sees its writes.
    pub(in crate::game) fn projectile_targets_changed(
        &self,
        read: &TargetsRead,
        seen: &ProjectileTargets,
    ) -> bool {
        self.projectile_targets_read() != *read || !self.lan_projectile_targets_are(&seen.guests)
    }

    /// The boats, minecarts, primed TNT and item frames a projectile can strike
    /// (`VehicleEntity.isPickable`, `PrimedTnt.isPickable`,
    /// `BlockAttachedEntity.isPickable`).
    pub(in crate::game) fn pickable_object_targets(&self) -> Vec<(Who, Aabb)> {
        let mut targets: Vec<(Who, Aabb)> = self
            .boats
            .iter()
            .map(|b| (Who::Mob(b.id), b.bounds()))
            .collect();
        targets.extend(self.carts.iter().map(|c| (Who::Mob(c.id), c.bounds())));
        targets.extend(self.tnts.iter().map(|t| (Who::Mob(t.id), t.bounds())));
        targets.extend(self.item_frames.iter().map(|f| {
            let (lo, hi) = f.bounds();
            (
                Who::Mob(f.id),
                Aabb::new(
                    Vec3::new(lo[0], lo[1], lo[2]),
                    Vec3::new(hi[0], hi[1], hi[2]),
                ),
            )
        }));
        targets
    }
    /// What a projectile can hit: the mobs, the player, the dragon's parts and
    /// the End crystals (under reserved ids, game/dragon.rs), and the vehicles
    /// and item frames.
    pub(in crate::game) fn projectile_targets(&self) -> Vec<(Who, Aabb)> {
        let mut targets: Vec<(Who, Aabb)> = self
            .mobs
            .iter()
            .filter(|m| m.alive())
            .map(|m| (Who::Mob(m.id), m.bounds()))
            .collect();
        if let Some(p) = self.player_view()
            && p.alive
        {
            targets.push((Who::Player, p.bounds));
        }
        targets.extend(
            self.special_targets()
                .into_iter()
                .map(|(id, b)| (Who::Mob(id), b)),
        );
        // Boats, minecarts, primed TNT and item frames are pickable for every
        // projectile, so every kind shares this one list.
        targets.extend(self.pickable_object_targets());
        targets
    }
}
