//! The game's inputs to `EntityGetter.getEntityCollisions`: the entity lists, the spatial
//! queries and the stand's root-vehicle grid, offered to the one rule in
//! `sunrise_client::entity_solids` (which a mob's own move reads too).
use super::{
    Game, boats::Boat, commands::Target, minecarts::Minecart, remote_vehicles::passenger_root,
};
use sunrise_client::entity_solids::{Candidate, Mover as Rule, Side, SolidKind, Solids};
use sunrise_core::geometry::{Aabb, Vec3};

/// The mover asking, as the game's callers name it.
#[derive(Clone, Copy)]
pub(in crate::game) enum Mover<'a> {
    /// The window's player.
    Player,
    /// Any player that is not riding, by the height of its feet: a guest the server holds.
    PlayerAt { feet_y: f64 },
    /// A mover that is neither a player nor a vehicle: an item, a stand, an orb, TNT, a
    /// falling block, a body a piston pushes.
    Passive,
    /// A minecart, within two blocks of itself.
    Cart(&'a Minecart),
    /// Boat `id` within `area`.
    Boat { id: u32, area: Aabb },
}

fn boat(b: &Boat) -> Candidate {
    Candidate::fixed(SolidKind::Boat, b.id, b.bounds())
}

fn cart(c: &Minecart) -> Candidate {
    Candidate::fixed(SolidKind::Cart, c.id, c.bounds())
}

/// A player the rule may offer; the rule never reads its id.
fn player(bounds: Aabb) -> Candidate {
    Candidate::fixed(SolidKind::Player, 0, bounds)
}

/// What a moving cart asks about: two blocks around itself.
fn cart_reach(c: &Minecart) -> Aabb {
    c.bounds().inflate(Vec3::new(2.0, 2.0, 2.0))
}

impl Game<'_> {
    /// The boxes of the solid entities for `mover`, in the order its inputs are read.
    pub(in crate::game) fn entity_collisions(&self, mover: Mover<'_>) -> Vec<Aabb> {
        let mut boxes = Vec::new();
        self.each_solid(mover, |c, _| boxes.push(c.bounds));
        boxes
    }

    /// Fills `out` with the entities that can be solid to a player as `Entity.canBeCollidedWith`
    /// allows (a boat, a living shulker, a happy ghast), as candidates the hub asks its rule of
    /// for each reported move. Built once per tick into the hub's own buffer, and left empty when
    /// no player is connected (`guests`): one test per mob and boat, no allocation once the buffer has grown.
    pub(in crate::game) fn fill_move_solids(&self, out: &mut Vec<Candidate>, guests: bool) {
        out.clear();
        if !guests {
            return;
        }
        let rule = Solids::new(
            Rule::Player {
                feet_y: 0.0,
                riding: None,
                side: Side::Server,
            },
            None,
        );
        out.extend(
            self.mobs
                .iter()
                .filter(|m| rule.takes(m.solid_kind()))
                .map(|m| m.solid_candidate()),
        );
        out.extend(self.boats.iter().map(boat));
    }

    /// [`Self::entity_collisions`] for an armor stand, which mobs can ride (`/ride`): each mob's
    /// box comes with the vehicle at the bottom of what it rides, so the stand leaves its own
    /// riders out (`isPassengerOfSameVehicle`, through the grid's `near_except`).
    pub(in crate::game) fn stand_entity_collisions(
        &self,
    ) -> sunrise_client::entities::ObstacleGrid {
        let mut boxes = Vec::new();
        let mut roots = Vec::new();
        self.each_solid(Mover::Passive, |c, mob| {
            boxes.push(c.bounds);
            roots.push(mob.and_then(|j| c.mount.and_then(|_| self.mob_root_vehicle(j))));
        });
        sunrise_client::entities::ObstacleGrid::with_roots(boxes, roots)
    }

    /// Asks the rule about every entity `mover` may meet, and hands the solid ones to `each`
    /// with the index of its mob.
    fn each_solid(&self, mover: Mover<'_>, mut each: impl FnMut(&Candidate, Option<usize>)) {
        let (rule, area) = match mover {
            Mover::Player => {
                let Some(p) = &self.player else {
                    return;
                };
                (
                    Rule::Player {
                        feet_y: p.position.y,
                        riding: self.riding,
                        side: if self.remote.is_some() {
                            Side::Client
                        } else {
                            Side::Server
                        },
                    },
                    None,
                )
            }
            Mover::PlayerAt { feet_y } => (
                Rule::Player {
                    feet_y,
                    riding: None,
                    side: Side::Server,
                },
                None,
            ),
            Mover::Passive => (Rule::Passive, None),
            Mover::Cart(c) => (Rule::Cart { id: c.id }, Some(cart_reach(c))),
            Mover::Boat { id, area } => (
                Rule::Boat {
                    id,
                    root: self.boat_collision_root(id, Target::Boat(id)),
                },
                Some(area),
            ),
        };
        let solids = Solids::new(rule, area);
        match mover {
            Mover::Player | Mover::PlayerAt { .. } | Mover::Passive => {
                for (j, m) in self.mobs.iter().enumerate() {
                    if solids.takes(m.solid_kind()) {
                        let c = m.solid_candidate();
                        if solids.accepts(&c, || None) {
                            each(&c, Some(j));
                        }
                    }
                }
                for b in self.boats.iter().map(boat) {
                    if solids.accepts(&b, || None) {
                        each(&b, None);
                    }
                }
            }
            Mover::Cart(c) => {
                for o in self
                    .carts
                    .iter()
                    .filter(|o| o.id != c.id)
                    .map(cart)
                    .chain(self.boats.iter().map(boat))
                {
                    if solids.accepts(&o, || None) {
                        each(&o, None);
                    }
                }
                self.mobs_meeting(cart_reach(c), |j, m, _| {
                    let c = m.solid_candidate();
                    if solids.accepts(&c, || None) {
                        each(&c, Some(j));
                    }
                });
                if self.riding != Some(c.id)
                    && let Some(p) = self.player.as_ref()
                    && !self.spectator()
                {
                    let p = player(p.bounds());
                    if solids.accepts(&p, || None) {
                        each(&p, None);
                    }
                }
                if let Some(host) = self.lan.as_ref() {
                    for g in host.hub.guests.iter().filter(|g| {
                        host.hub.here(g)
                            && g.loaded
                            && !g.dead
                            && !g.spectator()
                            && g.vehicle != Some(c.id)
                    }) {
                        let p = player(g.bounds());
                        if solids.accepts(&p, || None) {
                            each(&p, None);
                        }
                    }
                }
            }
            Mover::Boat { id, area } => {
                self.mobs_meeting(area, |j, m, _| {
                    let c = m.solid_candidate();
                    let target = Target::Mob(m.id);
                    if solids.accepts(&c, || self.boat_collision_root(m.id, target))
                        && !self.target_removed(&target)
                    {
                        each(&c, Some(j));
                    }
                });
                for (c, target) in self
                    .boats
                    .iter()
                    .map(|b| (boat(b), Target::Boat(b.id)))
                    .chain(self.carts.iter().map(|c| (cart(c), Target::Cart(c.id))))
                    .chain(
                        self.players_outside(id)
                            .into_iter()
                            .map(|(bb, target, id)| {
                                (Candidate::fixed(SolidKind::Player, id, bb), target)
                            }),
                    )
                {
                    if solids.accepts(&c, || self.boat_collision_root(c.id, target))
                        && !self.target_removed(&target)
                    {
                        each(&c, None);
                    }
                }
            }
        }
    }

    fn boat_collision_root(&self, id: u32, target: Target) -> Option<u32> {
        match self.remote.as_ref() {
            Some(r) => passenger_root(&r.passenger_vehicles, id).ok(),
            None => self.native_root_vehicle_id(id, target),
        }
    }
}
