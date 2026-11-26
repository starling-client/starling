//! The previous tick's position and rotation of an entity (`Entity.xo`, `yo`, `zo`, `yRotO`,
//! `xRotO`) and whether it is alive (`Entity.isAlive`), through an `EntityRef`, on both sides.
//! They read the body a handle names as `native_entity_refs.rs` finds it, by its resident row
//! rather than a pass over its list.
use super::{Game, commands::Target, entity::Entity};
use mod_api::EntityRef;
use sunrise_client::join_seq::{JoinList, JoinSeq, Joined, residency::table::EntityLookup};

/// What `Entity.setOldPosAndRot` kept when the entity's tick began: `xo`, `yo`, `zo` and
/// `yRotO`, `xRotO`.
#[derive(Clone, Copy)]
struct Old {
    position: [f64; 3],
    rotation: [f32; 2],
}

/// The old values of the body at `join` in `list`.
fn list_old<T: Entity + Joined, const DIRECT: bool>(
    list: &JoinList<T, DIRECT>,
    join: JoinSeq,
) -> Option<Old> {
    let body = list.get(list.locate(join)?)?;
    Some(Old {
        position: body.previous_position().components(),
        rotation: body.previous_rotation(),
    })
}

/// `Entity.isAlive` of the body at `join` in `list`: not removed, and for a living kind
/// (a mob, an armor stand) its health above 0.
fn list_alive<T: Entity + Joined, const DIRECT: bool>(
    list: &JoinList<T, DIRECT>,
    join: JoinSeq,
) -> Option<bool> {
    let body = list.get(list.locate(join)?)?;
    Some(!body.is_removed() && body.is_alive())
}

impl Game<'_> {
    /// The body a handle names on the side the game runs, with its resident row's join when
    /// the resident table keeps one. None for a stale handle or, on a client, an entity whose
    /// body the client doesn't keep.
    fn native_entity_read_row(&self, entity: EntityRef) -> Option<(Target, Option<JoinSeq>)> {
        if self.is_logical_server() {
            let (target, location) = self.native_entity_target_with_location(entity)?;
            return Some((target, location.map(|location| location.join)));
        }
        let target = self.native_client_entity_target(entity)?;
        let join = match self.server_resident_entity(entity.id()) {
            EntityLookup::Found { location, value } if value == target => Some(location.join),
            _ => None,
        };
        Some((target, join))
    }

    /// The old values of the body `target` names (`join`: its resident row's).
    fn target_old(&self, target: Target, join: Option<JoinSeq>) -> Option<Old> {
        let current = || {
            Some(Old {
                position: self.target_position(target)?,
                rotation: self.target_rotation(target),
            })
        };
        match target {
            // The client's own player: where the camera and the model ease from (`LocalPlayer`'s
            // xo, yo, zo), and `yRotO`, `xRotO`, which `Entity.turn` moves with the view.
            Target::Player if !self.is_logical_server() => Some(Old {
                position: self.previous_position.components(),
                rotation: [self.rope_rot_old.0, self.rope_rot_old.1],
            }),
            // A server's player: `ServerGamePacketListenerImpl` snaps every move it accepts
            // (`absSnapTo`, which sets xo and yRotO too), `tickPlayer` snaps it again after
            // `doTick`, and `commonTick` and a teleport set the old values, so at every moment a
            // hook runs they are its position and rotation.
            Target::Player | Target::Guest(_) => current(),
            Target::Mob(_) => list_old(&self.mobs, join?),
            Target::Item(_) => list_old(&self.dropped, join?),
            Target::Orb(_) => list_old(&self.orbs, join?),
            Target::Projectile(_) => list_old(&self.creatures.projectiles, join?),
            Target::Stand(_) => list_old(&self.armor_stands, join?),
            Target::Boat(_) => list_old(&self.boats, join?),
            Target::Cart(_) => list_old(&self.carts, join?),
            Target::Tnt(_) => list_old(&self.tnts, join?),
            Target::Falling(_) => list_old(&self.falling, join?),
            Target::Rocket(_) => list_old(&self.rockets, join?),
            Target::Trident(_) => list_old(&self.tridents, join?),
            Target::General(id) => join
                .and_then(|join| list_old(&self.resident_entities.general.rows, join))
                .or_else(|| {
                    // A client's copy of a row its server shows it.
                    let row = self.remote.as_ref()?.general.rows.get(&(id as i32))?;
                    Some(Old {
                        position: row.previous,
                        rotation: row.previous_rotation,
                    })
                }),
            Target::Dragon(_) => {
                let dragon = self.dragon.as_ref()?;
                // Its pitch changes only with a load, which sets the old pitch too.
                Some(Old {
                    position: dragon.previous.components(),
                    rotation: [dragon.previous_yaw, dragon.pitch],
                })
            }
            // `EnderDragon.aiStep` keeps each part's old position before it places it again.
            Target::DragonPart(id) => {
                let (dragon, part) = self.resident_dragon_part(id)?;
                Some(Old {
                    position: dragon.parts[part].1.components(),
                    rotation: self.target_rotation(target),
                })
            }
            Target::Crystal(_) => {
                let crystal = self.crystals.get(self.crystals.locate(join?)?)?;
                Some(Old {
                    position: crystal.previous.components(),
                    rotation: crystal.rotation,
                })
            }
            Target::Hook(id) => {
                let (hook, _) = self.fishing_hook_by_id(id)?;
                Some(Old {
                    position: hook.previous.components(),
                    rotation: [hook.previous_yaw, hook.previous_pitch],
                })
            }
            // Kinds that never move or turn on their own: what moves or turns them (a load, a
            // teleport, `/rotate`) sets the old position and rotation too.
            Target::Frame(_)
            | Target::Painting(_)
            | Target::Other(_)
            | Target::Cushion(_)
            | Target::Cloud(_)
            | Target::Bolt(_)
            | Target::Fangs(_)
            | Target::Spawner(_)
            | Target::Knot(_) => current(),
        }
    }

    /// `Entity.xo`, `yo`, `zo` of the entity a handle names (see the vocabulary's
    /// `entity_previous_position`).
    pub(super) fn entity_previous_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        match self.native_entity_read_row(entity) {
            Some((target, join)) => Some(self.target_old(target, join)?.position),
            None if self.is_logical_server() => None,
            // An entity a client tracks without a body of its own, as `entity_position` reads it.
            None => {
                let (remote, id) = self.native_client_entity_context(entity).ok()?;
                if let Some(row) = remote.general.rows.get(&id) {
                    return Some(row.previous);
                }
                self.native_client_tracked(entity)?;
                if let Some(display) = self.native_client_display(entity) {
                    return Some(display.previous_position().to_array());
                }
                remote.entities.get(&id).map(|tracked| tracked.previous)
            }
        }
    }

    /// `Entity.yRotO`, `xRotO` of the entity a handle names (see the vocabulary's
    /// `entity_previous_rotation`).
    pub(super) fn entity_previous_rotation(&self, entity: EntityRef) -> Option<[f32; 2]> {
        let (target, join) = self.native_entity_read_row(entity)?;
        Some(self.target_old(target, join)?.rotation)
    }

    /// `Entity.isAlive` of the entity a handle names (see the vocabulary's `entity_is_alive`).
    pub(super) fn entity_is_alive(&self, entity: EntityRef) -> Option<bool> {
        let (target, join) = self.native_entity_read_row(entity)?;
        match target {
            // `LivingEntity.isAlive`: a player's health, synced (`DATA_HEALTH_ID`) on a client.
            Target::Player | Target::Guest(_) => {
                let body = self.native_living_body(target)?;
                Some(super::native_living_body::body_health(body)? > 0.0)
            }
            Target::Dragon(_) => Some(self.dragon.as_ref()?.health > 0.0),
            Target::Mob(_) => list_alive(&self.mobs, join?),
            Target::Item(_) => list_alive(&self.dropped, join?),
            Target::Orb(_) => list_alive(&self.orbs, join?),
            Target::Projectile(_) => list_alive(&self.creatures.projectiles, join?),
            Target::Stand(_) => list_alive(&self.armor_stands, join?),
            Target::Boat(_) => list_alive(&self.boats, join?),
            Target::Cart(_) => list_alive(&self.carts, join?),
            Target::Tnt(_) => list_alive(&self.tnts, join?),
            Target::Falling(_) => list_alive(&self.falling, join?),
            Target::Rocket(_) => list_alive(&self.rockets, join?),
            Target::Trident(_) => list_alive(&self.tridents, join?),
            // A client keeps a row it shows until the server removes it.
            Target::General(_) => join
                .and_then(|join| list_alive(&self.resident_entities.general.rows, join))
                .or(Some(true)),
            // Not living, and a handle names them only while they are in their level.
            Target::Frame(_)
            | Target::Painting(_)
            | Target::Other(_)
            | Target::Crystal(_)
            | Target::DragonPart(_)
            | Target::Cushion(_)
            | Target::Cloud(_)
            | Target::Bolt(_)
            | Target::Hook(_)
            | Target::Fangs(_)
            | Target::Spawner(_)
            | Target::Knot(_) => Some(true),
        }
    }
}
