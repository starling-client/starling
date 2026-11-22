//! The chunk-unload sweep: rows leave with their chunks independently
//! of whether they wrote records to the world snapshot.
use super::*;

impl Game<'_> {
    /// Remove entities with the chunks in `keys`, except for a family's
    /// normal exceptions (such as a boat the player is riding).
    pub(super) fn unload_removed_entities(&mut self, keys: &HashSet<(i32, i32)>) {
        let gone = |p: Vec3| keys.contains(&chunk_of(p));
        // Clear just the opt-out for rows that don't use `forget_entity` here.
        let keep_row = |id, at, transient: &mut HashSet<u32>| {
            if gone(at) {
                transient.remove(&id);
                false
            } else {
                true
            }
        };
        let mut removed: Vec<u32> = Vec::new();
        let riding = self.riding;
        let ridden_roots: HashSet<_> = self
            .lan
            .iter()
            .flat_map(|host| host.hub.guests.iter())
            .filter(|guest| guest.placed && guest.dimension == self.dim.id())
            .filter_map(|guest| guest.vehicle)
            .chain(riding)
            .filter_map(|vehicle| self.riding_root(vehicle))
            .collect();
        // Resolve every passenger before any family is removed, so the real
        // root's chunk controls the whole tree even across storage families.
        let leaves = |id, at, mount: Option<u32>| {
            let Some(root) = mount.and_then(|vehicle| self.riding_root(vehicle)) else {
                return !ridden_roots.contains(&id) && gone(at);
            };
            !ridden_roots.contains(&root) && gone(self.saved_vehicle_position(root).unwrap_or(at))
        };
        let leaving: HashSet<u32> = self
            .mob_objects()
            .filter(|m| leaves(m.id, m.position, m.extra.mount))
            .map(|m| m.id)
            .chain(
                self.resident_entities
                    .general
                    .rows
                    .iter()
                    .filter(|r| leaves(r.id, r.position, r.mount))
                    .map(|r| r.id),
            )
            .collect();
        self.boats.retain(|b| {
            let keep = !gone(b.position) || ridden_roots.contains(&b.id);
            if !keep {
                removed.push(b.id);
            }
            keep
        });
        self.crystals
            .retain(|c| keep_row(c.id, c.position, &mut self.persist.transient));
        self.ominous_spawners.retain(|s| {
            let keep = !gone(Vec3::new(s.position[0], s.position[1], s.position[2]));
            if !keep {
                removed.push(s.id);
            }
            keep
        });
        if self.dragon.as_ref().is_some_and(|d| gone(d.position)) {
            self.discard_resident_dragon();
        }
        self.carts.retain(|c| {
            let keep = !gone(c.position) || ridden_roots.contains(&c.id);
            if !keep {
                removed.push(c.id);
            }
            keep
        });
        self.fangs.list.retain(|f| {
            let keep = !gone(Vec3::new(f.pos[0], f.pos[1], f.pos[2]));
            if !keep {
                removed.push(f.id);
            }
            keep
        });
        self.others.retain(|o| {
            let keep = !gone(o.position) || ridden_roots.contains(&o.id);
            if !keep {
                removed.push(o.id);
            }
            keep
        });
        self.armor_stands.retain(|a| {
            let keep = !gone(a.position) || ridden_roots.contains(&a.id);
            if !keep {
                removed.push(a.id);
            }
            keep
        });
        self.item_frames.retain(|f| {
            let (lo, hi) = f.bounds();
            let keep = !gone(Vec3::new(
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            ));
            if !keep {
                removed.push(f.id);
            }
            keep
        });
        self.paintings.retain(|p| {
            let keep = !gone(painting_center(p));
            if !keep {
                removed.push(p.id);
            }
            keep
        });
        self.cushions.retain(|c| {
            let keep = !gone(c.position);
            if !keep {
                removed.push(c.id);
            }
            keep
        });
        // The potion clouds written above leave with their chunk; a
        // dragon's breath (no potion) was not written and stays.
        self.breath_clouds.retain(|c| {
            let keep = c.potion.is_none() || !gone(Vec3::new(c.at[0], c.at[1], c.at[2]));
            if !keep {
                removed.push(c.id);
            }
            keep
        });
        self.mobs.retain(|m| {
            let keep = !leaving.contains(&m.id);
            if !keep {
                removed.push(m.id);
            }
            keep
        });
        self.persist.off_level.retain(|id, _| {
            let keep = !leaving.contains(id);
            if !keep {
                removed.push(*id);
            }
            keep
        });
        if !self.persist.off_level.is_empty() {
            // A retained boat or cart root is in no section and never saved:
            // it lives only while a passenger or player still rides it.
            let ridden: HashSet<u32> = self
                .mob_objects()
                .filter_map(|m| m.extra.mount)
                .chain(
                    self.resident_entities
                        .general
                        .rows
                        .iter()
                        .filter(|row| !leaving.contains(&row.id))
                        .filter_map(|row| row.mount),
                )
                .chain(ridden_roots.iter().copied())
                .collect();
            self.persist.off_level.retain(|id, object| {
                let keep = object.as_mob().is_some() || ridden.contains(id);
                if !keep {
                    removed.push(*id);
                }
                keep
            });
        }
        self.dropped.retain(|e| {
            let keep = !gone(e.position);
            if !keep {
                removed.push(e.id);
            }
            keep
        });
        self.orbs
            .retain(|o| keep_row(o.id, o.position, &mut self.persist.transient));
        // Unloaded with their chunk: PersistentEntitySectionManager.stopTracking.
        self.native_leaves_flush();
        // the ones saved with the chunk go with it (a trident too, unsaved).
        self.creatures.projectiles.retain(|p| {
            let leaves = entity_nbt::saved_with_chunk(p) || p.arrow.is_some();
            let keep = !leaves || !gone(p.position);
            if !keep {
                removed.push(p.id);
            }
            keep
        });
        self.falling
            .retain(|f| keep_row(f.id, f.position, &mut self.persist.transient));
        self.rockets
            .retain(|r| keep_row(r.id, r.position, &mut self.persist.transient));
        self.tridents
            .retain(|t| keep_row(t.id, t.position, &mut self.persist.transient));
        self.tnts
            .retain(|t| keep_row(t.id, t.position, &mut self.persist.transient));
        self.resident_entities.general.rows.retain(|row| {
            let keep = !leaving.contains(&row.id);
            if !keep {
                removed.push(row.id);
            }
            keep
        });
        self.persist
            .anon
            .retain(|_, (at, _)| !keys.contains(&chunk_of(*at)));
        for id in removed {
            self.forget_entity(id);
        }
    }
}
