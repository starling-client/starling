//! PURPOSE: ticks experience orbs, primed TNT and falling blocks in the order
//! they joined the level, between the other kinds' batches.
//! WHY: `ServerLevel.tick` ticks every entity in join order, one kind among
//! the rest; these three used to tick as three fixed batches (orbs after the
//! mobs, falling blocks and TNT after the gateways), so a TNT that joined
//! before a boat blew up after the boat moved (#6509, tick order slice 2).
//! REFS: vanilla `ServerLevel.tick` (`entityTickList.forEach`),
//! `EntityTickList`; `join_seq.rs`; `tools/test_tick_order_parity.py`.
//! LOGS: none. READS-BY: `Game::tick_world`, `tick_mobs`.
//! ENDS-WHEN: every kind ticks from the join-order walk.

use super::{Game, Result};
use crate::game::collision_source::Mover;
use sunrise_client::join_seq::{JoinSeq, Joined};
use sunrise_client::random_source;
use sunrise_core::geometry::Vec3;
use sunrise_core::random::RandomSource as _;

/// What this tick has ticked of the three kinds.
#[derive(Default, Clone)]
pub(super) struct OrderedTicks {
    /// Joins at or above this started after the tick began; they tick next tick.
    /// The general rows' pass (`general/tick.rs`) shares the same cutoff.
    pub(in crate::game) born: JoinSeq,
    /// The last join number ticked, if any: later ones are still to come.
    pub(super) done: Option<JoinSeq>,
    /// Joins below this have had their spawn packet; the ones above joined
    /// since (a summoned falling block, a lit TNT) and tick after it goes out.
    pub(super) announced: JoinSeq,
    /// For each batch of another kind, in tick order (see [`Batch`]): the
    /// lowest join number among it and every batch after it.
    suffix_min: [JoinSeq; Batch::COUNT],
    /// The orbs' lava random, made on first use.
    lava_random: Option<sunrise_core::random::LegacyRandom>,
    /// Scratch schedule reused by each ordered pass in a level tick. The
    /// entity lists can change between passes, so this only retains capacity;
    /// entries are rebuilt from the live lists every time.
    pending_buf: Vec<(JoinSeq, Pending)>,
    /// The falling blocks (id, join) that ended a tick still falling with a part listening to
    /// `falling_tick`, delivered after the pass.
    pub(super) falling_ticked: Vec<(u32, JoinSeq)>,
    /// Whether the entity tick hooks are heard this tick, and which types are told alone: the
    /// server's unsimulated entities of those types are told at their place in this order
    /// (game/modding/native_ticks.rs).
    pub(super) told: super::native_ticks::Told,
}

/// The batches of the other kinds, in the order they tick.
#[derive(Clone, Copy)]
pub(super) enum Batch {
    ArmorStands,
    Boats,
    Carts,
    Tridents,
    Rockets,
    Items,
    Mobs,
    Projectiles,
    /// The general rows tick last of the kinds (`general/tick.rs`): a slot
    /// of its own keeps every earlier batch's `ordered_before` draining the
    /// orbs, TNT and falling blocks that joined before the earliest general
    /// row, so the mobile kinds keep their join order against them.
    General,
}

impl Batch {
    const COUNT: usize = 9;
}

#[derive(Clone, Copy)]
enum Pending {
    Orb,
    Tnt,
    Falling,
    /// An entity the server keeps without simulating it (a display, a marker, an interaction),
    /// of a type a mod named: only told its tick.
    Other,
}

/// The lowest join number in a batch, or none when it is empty.
pub(super) fn first_join<T: Joined>(list: &[T]) -> Option<JoinSeq> {
    list.iter().map(Joined::join_seq).min()
}

impl Game<'_> {
    /// A level tick starts: what has joined so far ticks this tick.
    pub(super) fn begin_ordered_ticks(&mut self) {
        let _span = sunrise_client::spans::guard("ordered_ticks");
        let pending_buf = std::mem::take(&mut self.ordered.pending_buf);
        let mut falling_ticked = std::mem::take(&mut self.ordered.falling_ticked);
        falling_ticked.clear();
        let told = self.native_tick_told();
        let firsts = [
            first_join(&self.armor_stands),
            first_join(&self.boats),
            first_join(&self.carts),
            first_join(&self.tridents),
            first_join(&self.rockets),
            first_join(&self.dropped),
            first_join(&self.mobs),
            first_join(&self.creatures.projectiles),
            first_join(&self.resident_entities.general.rows),
        ];
        // A batch ticks whole, so an entity of the three kinds goes after the
        // last batch that holds anything that joined before it.
        let mut suffix_min = [JoinSeq::MAX; Batch::COUNT];
        let mut lowest = JoinSeq::MAX;
        for (slot, first) in suffix_min.iter_mut().zip(firsts).rev() {
            lowest = lowest.min(first.unwrap_or(JoinSeq::MAX));
            *slot = lowest;
        }
        self.ordered = OrderedTicks {
            announced: self.ordered.announced,
            born: JoinSeq::upcoming(),
            done: None,
            suffix_min,
            lava_random: None,
            pending_buf,
            falling_ticked,
            told,
        };
    }

    /// Ticks, one at a time in join order, every orb, primed TNT and falling
    /// block that joined before `cut` and has not ticked this tick.
    pub(super) fn run_ordered_ticks(&mut self, cut: JoinSeq, late: bool) -> Result<()> {
        let _span = sunrise_client::spans::guard("ordered_ticks");
        let mut first_error = None;
        let mut followed = None;
        let mut events = false;
        let entity_ticking = self.entity_ticking_area();
        // Who ticks in this run, sorted once: entities that join meanwhile are
        // at or above `born`, so the set cannot grow, only lose the removed.
        let (done, born, announced) =
            (self.ordered.done, self.ordered.born, self.ordered.announced);
        // The earlier passes never tick a TNT or falling block that joined since
        // the spawn packets (`right_pass`), so the late pass offers none that
        // ticked this tick: an orb that joined after one and ticked in an
        // earlier pass must not move `done` past it.
        let wanted = |j: JoinSeq| (late || done.is_none_or(|d| j > d)) && j < cut && j < born;
        let mut pending = std::mem::take(&mut self.ordered.pending_buf);
        pending.clear();
        let mut offer = |j: JoinSeq, kind: Pending| {
            // Orbs are announced after their tick, always; TNT and falling
            // blocks that joined since the last packets wait for them.
            let right_pass = match kind {
                Pending::Orb | Pending::Other => !late,
                _ => late == (j >= announced),
            };
            if right_pass && wanted(j) {
                pending.push((j, kind));
            }
        };
        self.orbs.iter().for_each(|e| offer(e.join, Pending::Orb));
        self.tnts.iter().for_each(|e| offer(e.join, Pending::Tnt));
        self.falling
            .iter()
            .for_each(|e| offer(e.join, Pending::Falling));
        // Only while a mod hears entity ticks and named the type: otherwise none is offered.
        let told = self.ordered.told;
        if told.listening() {
            self.others
                .iter()
                .filter(|o| super::native_ticks::other_ticks(&o.kind) && told.alone_kind(&o.kind))
                .for_each(|o| offer(o.join, Pending::Other));
        }
        pending.sort_unstable_by_key(|&(j, _)| j);
        // EntityGetter.getEntityCollisions: the boxes that stop these moves (a
        // boat, a shulker, a happy ghast held still), gathered once for the
        // run and again after an explosion, which can move or remove them.
        let mut obstacles: Option<sunrise_client::entities::ObstacleGrid> = None;
        let mut nearby = Vec::new();
        'entity: for (join, kind) in pending.drain(..) {
            self.ordered.done = Some(join);
            // ServerLevel.tick: only entities in entity-ticking chunks tick.
            match kind {
                Pending::Tnt => {
                    let Some(i) = self.tnts.locate(join) else {
                        continue;
                    };
                    let at = self.tnts[i].position;
                    if entity_ticking(at.x.floor() as i32, at.z.floor() as i32) {
                        let t = &self.tnts[i];
                        let area = t.bounds().expand_towards(t.velocity);
                        obstacles
                            .get_or_insert_with(|| {
                                sunrise_client::entities::ObstacleGrid::new(
                                    self.entity_collisions(Mover::Passive),
                                )
                            })
                            .near(area.inflate(Vec3::new(1.0, 1.0, 1.0)), &mut nearby);
                        let count = self.tnts.len();
                        if let Err(e) = self.tick_one_tnt(i, &nearby) {
                            first_error.get_or_insert(e);
                        }
                        if self.tnts.len() < count {
                            obstacles = None;
                        }
                    }
                }
                Pending::Falling => {
                    let Some(i) = self.falling.locate(join) else {
                        continue;
                    };
                    let at = self.falling[i].position;
                    if entity_ticking(at.x.floor() as i32, at.z.floor() as i32) {
                        events = true;
                        let f = &self.falling[i];
                        let area = sunrise_client::falling_block::bounds(f.position)
                            .expand_towards(f.velocity);
                        obstacles
                            .get_or_insert_with(|| {
                                sunrise_client::entities::ObstacleGrid::new(
                                    self.entity_collisions(Mover::Passive),
                                )
                            })
                            .near(area.inflate(Vec3::new(1.0, 1.0, 1.0)), &mut nearby);
                        if let Err(e) = self.tick_one_falling(i, &nearby) {
                            first_error.get_or_insert(e);
                        }
                    }
                }
                Pending::Other => {
                    let Some(i) = self.others.locate(join) else {
                        continue;
                    };
                    let at = self.others[i].position;
                    if entity_ticking(at.x.floor() as i32, at.z.floor() as i32) {
                        self.native_other_tick_alone(told, i);
                    }
                }
                Pending::Orb => {
                    let Some(i) = self.orbs.locate(join) else {
                        continue;
                    };
                    let at = self.orbs[i].position;
                    if !entity_ticking(at.x.floor() as i32, at.z.floor() as i32) {
                        continue;
                    }
                    // ExperienceOrb.tick runs scanForMerges before it moves.
                    // An orb absorbed here is gone before its own turn's
                    // find_join finds it.
                    if self.remote.is_none() {
                        self.merge_orb(i);
                    }
                    let Some(i) = self.orbs.locate(join) else {
                        continue;
                    };
                    let followed = followed.get_or_insert_with(|| self.orb_followed());
                    let target = self.orb_target(i, followed);
                    let Some(i) = self.orbs.locate(join) else {
                        continue;
                    };
                    // A block of slack around the move: gravity and the pull
                    // towards a player change it before it is made.
                    let o = &self.orbs[i];
                    let area = o.bounds().expand_towards(o.velocity);
                    obstacles
                        .get_or_insert_with(|| {
                            sunrise_client::entities::ObstacleGrid::new(
                                self.entity_collisions(Mover::Passive),
                            )
                        })
                        .near(area.inflate(Vec3::new(1.0, 1.0, 1.0)), &mut nearby);
                    let orb_id = self.orbs[i].id;
                    let vanilla = self.saved_refuses_damage(orb_id, "minecraft:lava", false);
                    let server = self.remote.is_none();
                    let lava_random = &mut self.ordered.lava_random;
                    // ExperienceOrb's lava section uses the orb tick's own random, not level random.
                    let mut next_float = || {
                        lava_random
                            .get_or_insert_with(random_source::entity)
                            .next_float()
                    };
                    let side = game_side!(self);
                    let motion = sunrise_client::motion::BodyMotion {
                        world: &self.world,
                        collision: self.collision,
                        facts: self.res.facts(),
                        blocks: Some(&self.res.motion().blocks),
                        context: &self.loop_context,
                        side: Some(&side),
                    };
                    let tick = self.orbs[i].move_before_fall(
                        motion,
                        target,
                        server,
                        &mut next_float,
                        Some(self.border.geometry()),
                        &self.res.motion().fluids,
                        &nearby,
                    );
                    if server {
                        self.orb_fall(i, tick.step, tick.movement.effect);
                    }
                    let Some(i) = self.orbs.locate(join) else {
                        continue;
                    };
                    let response =
                        self.orbs[i].move_restitution(self.body_motion(&game_side!(self)), tick);
                    let entity = (orb_id, "minecraft:experience_orb");
                    let mut velocity =
                        self.finish_body_restitution(entity, self.orbs[i].position, response);
                    let Some(mut i) = self.orbs.locate(join) else {
                        continue;
                    };
                    let orb = &self.orbs[i];
                    let (footprint, ground) = ((orb.position, orb.bounds()), orb.ground);
                    self.body_motion(&game_side!(self))
                        .apply_block_speed(orb.position, &mut velocity);
                    if !tick.burnt_out {
                        let writes = self.body_step_on(entity, footprint, &ground, &mut velocity);
                        for write in writes {
                            match write {
                                mod_api::EntityWrite::Hurt {
                                    damage_type,
                                    amount,
                                } => {
                                    let Some(name) =
                                        mod_api::__private::parts_installation::damage_type_name(
                                            damage_type,
                                        )
                                    else {
                                        continue;
                                    };
                                    let Some(kind) = self.health_resources.damage.id(name) else {
                                        continue;
                                    };
                                    let source = sunrise_client::health::HurtSource {
                                        damage_type: name,
                                        causing: 0,
                                        direct: 0,
                                        position: None,
                                    };
                                    let vanilla = self
                                        .resident_item_or_orb_immune(orb_id, entity.1, kind, false);
                                    let invulnerable = self.native_nonliving_invulnerable(
                                        orb_id, entity.1, source, vanilla,
                                    );
                                    let Some(current) = self.orbs.locate(join) else {
                                        continue 'entity;
                                    };
                                    i = current;
                                    if !invulnerable && self.orbs[i].hurt(amount) {
                                        self.orbs.remove(i);
                                        continue 'entity;
                                    }
                                }
                                mod_api::EntityWrite::Push([x, y, z]) => {
                                    velocity = velocity.add_vector(Vec3::new(x, y, z))
                                }
                                mod_api::EntityWrite::SetMovement([x, y, z]) => {
                                    velocity = Vec3::new(x, y, z)
                                }
                                mod_api::EntityWrite::ResetFallDistance => {
                                    self.orbs[i].fall_distance = 0.0
                                }
                                mod_api::EntityWrite::SetFireTicks(ticks) => {
                                    self.orbs[i].fire_ticks = ticks
                                }
                                _ => {}
                            }
                        }
                    }
                    let registry = self.pack.assets.registry;
                    let tags = world_items!(self);
                    let block_registries = &self.pack.block_registries;
                    let hooks = &self.damage_hooks;
                    let side = game_side!(self);
                    let o = &mut self.orbs[i];
                    o.velocity = velocity;
                    let audio = &mut self.audio;
                    let lava_random = &mut self.ordered.lava_random;
                    let mut next_float = || {
                        lava_random
                            .get_or_insert_with(random_source::entity)
                            .next_float()
                    };
                    let stays = o.finish_tick(
                        &mut &mut self.world,
                        (self.collision, self.res.facts(), &self.loop_context, &side),
                        &self.res.motion().fluids,
                        tick,
                        server,
                        sunrise_client::entities::OrbLavaEffects {
                            next_float: &mut next_float,
                            on_sound: &mut |pos, pitch| {
                                audio.play("minecraft:entity.generic.burn", Some(pos), 0.4, pitch)
                            },
                            invulnerable: &mut |world| {
                                let source = sunrise_client::health::HurtSource {
                                    damage_type: "minecraft:lava",
                                    causing: 0,
                                    direct: 0,
                                    position: None,
                                };
                                sunrise_client::health::DamageLevel::of(
                                    world,
                                    registry,
                                    Some(tags),
                                    Some(block_registries),
                                    hooks,
                                )
                                .no_push()
                                .invulnerability_check(
                                    sunrise_client::health::Hurting::Entity(
                                        orb_id as i32,
                                        "minecraft:experience_orb",
                                        source,
                                    ),
                                    vanilla,
                                )
                            },
                        },
                    );
                    // PowderSnowBlock.entityInside during the move, before an
                    // orb that is done is removed.
                    let snow = (o.previous, o.position, o.bounds(), o.velocity);
                    self.entity_powder_snow_particles(snow.0, snow.1, snow.2, snow.3, false);
                    // Its move is filed at once, also for an orb about to go.
                    let (from, to) = (snow.0, snow.1);
                    self.entity_section_step(orb_id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
                    if !stays {
                        self.orbs.remove(i);
                    }
                }
            }
        }
        self.ordered.pending_buf = pending;
        self.deliver_falling_ticks();
        if events && let Err(e) = self.process_level_events() {
            first_error.get_or_insert(e);
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Before a batch of another kind ticks: the three kinds' entities that
    /// joined before everything still to tick in the batches from this one on.
    pub(super) fn ordered_before(&mut self, batch: Batch) -> Result<()> {
        let cut = self.ordered.suffix_min[batch as usize];
        if cut == JoinSeq::MAX {
            return Ok(());
        }
        self.run_ordered_ticks(cut, false)
    }
}

impl Game<'_> {
    #[cfg(test)]
    pub(super) fn orb_moved(&mut self, i: usize, step: sunrise_client::entities::OrbMove) {
        let o = &self.orbs[i];
        let effect = self
            .body_motion(&game_side!(self))
            .legacy_contact(o.position, &o.ground);
        self.orb_fall(i, step, effect);
    }

    /// The authoritative impact is part of the move, before the captured state's restitution.
    fn orb_fall(
        &mut self,
        i: usize,
        step: sunrise_client::entities::OrbMove,
        effect: sunrise_client::motion::BodyContact,
    ) {
        let o = &self.orbs[i];
        let mut fall = self.vehicle_fall_reset(o.fall_distance, step.from, step.moved);
        if !step.in_water && step.moved.y < 0.0 {
            fall -= f64::from(step.moved.y as f32);
        }
        if o.ground.on_ground {
            let (id, position, bounds) = (o.id, o.position, o.bounds());
            self.orbs[i].fall_distance = 0.0;
            if fall > 0.0 {
                self.nonliving_fall_on(id, position, effect);
                self.post_landing_event(position, bounds, false);
            }
        } else {
            self.orbs[i].fall_distance = fall;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::first_join;
    use std::sync::Arc;
    use sunrise_client::chunk::Chunk;
    use sunrise_client::join_seq::{JoinList, JoinSeq, Joined as _};
    use sunrise_core::position::ChunkPos;

    #[test]
    #[ignore = "daily"]
    fn dispatch_ticks_both_owners_of_each_located_kind() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            for x in [4, 12] {
                assert!(game.run_command(&format!("summon tnt {x} 70 4 {{fuse:80}}")));
                assert!(game.run_command(&format!(
                    "summon falling_block {x} 70 8 {{BlockState:{{Name:\"minecraft:sand\"}},Time:1}}"
                )));
                assert!(game.run_command(&format!("summon experience_orb {x} 70 12 {{Value:1}}")));
            }
            // Existing residents whose spawn packets were already announced.
            game.ordered.announced = sunrise_client::join_seq::JoinSeq::MAX;
            game.tickets.update(None);
            game.begin_ordered_ticks();
            game.run_ordered_ticks(sunrise_client::join_seq::JoinSeq::MAX, false)
                .unwrap();
            assert_eq!(
                game.tnts.iter().map(|t| t.fuse).collect::<Vec<_>>(),
                [79, 79]
            );
            assert_eq!(
                game.falling.iter().map(|f| f.age).collect::<Vec<_>>(),
                [2, 2]
            );
            assert_eq!(game.orbs.iter().map(|o| o.age).collect::<Vec<_>>(), [1, 1]);
        });
    }

    /// A listener's calls, in order: the phase and the entities told.
    type Heard = std::sync::Arc<std::sync::Mutex<Vec<(&'static str, Vec<u32>)>>>;

    /// Hears every entity tick, naming `named` to `entity_tick_each`.
    struct Displays {
        named: &'static [&'static str],
        heard: Heard,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Displays {
        fn entity_tick_listened(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::EntityTickListenedEvent,
        ) {
            event.listened = true;
        }
        fn entity_tick_pre(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::EntityTickPreEvent<'_>,
        ) {
            let entities = event.entities.to_vec();
            self.heard.lock().unwrap().push(("pre", entities));
        }
        fn entity_tick_post(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::EntityTickPostEvent<'_>,
        ) {
            let entities = event.entities.to_vec();
            self.heard.lock().unwrap().push(("post", entities));
        }
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            for kind in self.named {
                r.entity_tick_each(kind);
            }
        }
    }

    /// Two text displays, which the server keeps without simulating them, through a level's
    /// entity phase: their ids and what the listener heard.
    fn display_ticks(named: &'static [&'static str]) -> (Vec<u32>, Vec<(&'static str, Vec<u32>)>) {
        let heard = Heard::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "display-ticks",
            Box::new(Displays {
                named,
                heard: heard.clone(),
            }),
        )]
        .into();
        let mut ids = Vec::new();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.build_registries();
                game.set_allow_commands(true);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                for x in [4, 12] {
                    assert!(
                        game.run_command(&format!("summon text_display {x} 70 4 {{text:\"x\"}}"))
                    );
                }
                ids = game.others.iter().map(|o| o.id).collect();
                game.tickets.update(None);
                game.begin_ordered_ticks();
                game.run_ordered_ticks(sunrise_client::join_seq::JoinSeq::MAX, false)
                    .unwrap();
                game.native_others_tick_family(game.ordered.told, game.ordered.born);
            });
        });
        let heard = std::mem::take(&mut *heard.lock().unwrap());
        (ids, heard)
    }

    /// Displays of a type a mod named are told one by one at their place in the join order, each
    /// Pre right before its Post.
    #[test]
    #[ignore = "daily"]
    fn named_displays_are_told_at_their_place_in_the_join_order() {
        let (ids, heard) = display_ticks(&["minecraft:text_display"]);
        assert_eq!(ids.len(), 2);
        assert_eq!(
            heard,
            [
                ("pre", vec![ids[0]]),
                ("post", vec![ids[0]]),
                ("pre", vec![ids[1]]),
                ("post", vec![ids[1]]),
            ]
        );
    }

    /// With no display type named, the displays are told once as a family; the walk itself
    /// offers none of them.
    #[test]
    #[ignore = "daily"]
    fn unnamed_displays_are_told_once_for_the_family() {
        let (ids, heard) = display_ticks(&[]);
        assert_eq!(heard, [("pre", ids.clone()), ("post", ids)]);
    }

    #[test]
    fn first_join_uses_join_sequence_when_storage_is_reordered() {
        struct E {
            join: JoinSeq,
        }
        impl sunrise_client::join_seq::Joined for E {
            fn join_seq(&self) -> JoinSeq {
                self.join
            }
            fn set_join_seq(&mut self, join: JoinSeq) {
                self.join = join;
            }
        }
        let mut list: JoinList<E> = JoinList::default();
        for _ in 0..3 {
            list.push(E {
                join: JoinSeq::UNJOINED,
            });
        }
        let first = list[0].join_seq();
        list.swap(0, 2);
        assert_eq!(first_join(&list), Some(first));
    }
}
