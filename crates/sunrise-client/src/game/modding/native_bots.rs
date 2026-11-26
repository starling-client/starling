//! Server-side player bots for mods (docs/modding/src/how-to/native-mods.md, "Server hooks for plugin ports: player
//! bots"): a mod that drives a server player itself (a fake client sending its own moves) asks
//! where the player's movement code would take them in one tick, and which block shapes are in
//! a box. Nothing runs unless a mod calls it, and neither changes the player or the level.
use super::{Game, glide_wear, shield, worn_boots};
use mod_api::{__private::voxel_shape_from_core, PlayerInput, SimulatedMove, VoxelShape};
use std::ops::ControlFlow;
use sunrise_client::collisions::SceneCollisions;
use sunrise_client::items::ItemStack;
use sunrise_client::motion::{MotionWorld, PlayerMotion};
use sunrise_client::sprint::Keys;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_world::block_collision_scan::BlockCollisionScan;
use sunrise_world::collision_context::CollisionContext;

/// The keys a bot's input presses: the direction flags by sign (sprint and the server's input
/// packet read them), and the exact move vector when it isn't a key's (a stick's).
fn input_keys(input: &PlayerInput) -> Option<Keys> {
    if !(input.forward.is_finite() && input.strafe.is_finite()) {
        return None;
    }
    let forward = input.forward.clamp(-1.0, 1.0);
    let strafe = input.strafe.clamp(-1.0, 1.0);
    let key = |v: f32| v == 0.0 || v.abs() == 1.0;
    Some(Keys {
        forward: forward > 0.0,
        backward: forward < 0.0,
        left: strafe > 0.0,
        right: strafe < 0.0,
        jump: input.jump,
        shift: input.sneak,
        sprint: input.sprint,
        analog: (!key(forward) || !key(strafe)).then_some([strafe, forward]),
    })
}

/// `LocalPlayer.aiStep`'s sprint: it needs a forward impulse (`hasForwardImpulse`).
fn sprints(input: &PlayerInput) -> bool {
    input.sprint && input.forward > 1.0e-5
}

/// A box from `[min_x, min_y, min_z, max_x, max_y, max_z]`, when its corners are numbers in order.
fn aabb_of(b: [f64; 6]) -> Option<Aabb> {
    (b.iter().all(|v| v.is_finite()) && b[0] <= b[3] && b[1] <= b[4] && b[2] <= b[5])
        .then(|| Aabb::new(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5])))
}

// Native mods use these; nothing in Sunrise does.
impl Game<'_> {
    /// One tick of the server player `uuid`'s own movement from `input`, as their client would
    /// play it (`LocalPlayer.aiStep` and `Player.travel`): walking and strafing (`forward` and
    /// `strafe`, -1..1), jumping, sneaking (slower, and held at edges), sprinting (with a
    /// forward input only), looking `yaw` and `pitch`; gravity, friction, block speed factors,
    /// stepping up, swimming and climbing use the client's movement tick. The simulator reads
    /// the player's current gear and item use. It starts where the player is now, with
    /// `input.velocity` when given (the last simulated velocity, which the server doesn't keep),
    /// otherwise it uses the server's copy. Nothing is changed: the player stays where they are
    /// until the mod moves them (a
    /// move packet from its bot, `native_teleport_player`), and the level is only read.
    ///
    /// None off a server, for a player not here (another level than the one in place, a
    /// guest out of reach inside another player's hook, where the hub is taken out), dead, a
    /// spectator or riding, an input that is not a number, and when the movement code fails.
    /// API @API:player_bots@.
    pub(super) fn player_simulate_move(
        &mut self,
        player: mod_api::PlayerRef,
        input: PlayerInput,
    ) -> Option<SimulatedMove> {
        if !self.native_authoritative() || !(input.yaw.is_finite() && input.pitch.is_finite()) {
            return None;
        }
        if input
            .velocity
            .is_some_and(|v| !v.iter().all(|c| c.is_finite()))
        {
            return None;
        }
        let keys = input_keys(&input)?;
        self.player_name(player)?;
        let (mut body, unswapped) = self.native_bot_body(player.uuid())?;
        body.yaw = input.yaw;
        body.pitch = input.pitch.clamp(-90.0, 90.0);
        if let Some([x, y, z]) = input.velocity {
            body.velocity = Vec3::new(x, y, z);
        }
        // Game.sync_player_body's part that depends on the blocks.
        body.nudge = crate::game::closest_space_nudge(
            &body,
            &self.world,
            self.collision,
            self.level_suffocation(),
        );
        let sprinting = sprints(&input) && !body.item_use_slows_sprint;
        let mut side = game_side!(self);
        if let Some(conn) = unswapped {
            let guest = self.lan.as_ref()?.hub.guest(conn)?;
            side.player_equipment = acting_equipment!(guest);
        }
        MotionWorld {
            side: &side,
            entities: &[],
            player: &mut body,
            scene: &mut self.world,
            assets: &self.pack.assets,
            collision_binding: self.collision,
            resources: self.motion,
            health: &self.health_resources,
            tag_items: Some(world_items!(self)),
            loop_context: &self.loop_context,
            damage_hooks: &self.damage_hooks,
            block_registries: Some(&self.pack.block_registries),
        }
        .replayed_tick(keys, sprinting)
        .ok()?;
        let (p, v) = (body.position, body.velocity);
        let mut result = SimulatedMove::default();
        result.position = [p.x, p.y, p.z];
        result.velocity = [v.x, v.y, v.z];
        result.on_ground = body.ground.on_ground;
        result.horizontal_collision = body.collision.horizontal;
        result.sprinting = body.sprinting;
        Some(result)
    }

    /// A copy of server player `uuid`'s body in the level in place, and the connection to lend
    /// equipment from if the guest's state has not been swapped into Game.
    fn native_bot_body(&self, uuid: u128) -> Option<(PlayerMotion, Option<u32>)> {
        if self.lan_acting_uuid == Some(uuid) {
            self.lan_acting?;
            let p = self.player.as_ref()?;
            let mut body = (p.health.health() > 0.0 && !p.spectator && self.riding.is_none())
                .then(|| p.clone())?;
            body.glider = self.glider_worn();
            body.leather_boots = worn_boots::leather_boots(&self.armor);
            let using = self.using.and_then(|(slot, _)| self.stack_in(slot));
            self.set_bot_item_use(&mut body, using);
            return Some((body, None));
        }
        if self.lan_acting.is_some() {
            return None;
        }
        let host = self.lan.as_ref()?;
        let g = host.hub.guests.iter().find(|g| {
            g.placed
                && !g.dead
                && g.uuid == uuid
                && g.dimension == self.dim.id()
                && g.vehicle.is_none()
                && !g.spectator()
        })?;
        let mut body = g.motion.as_deref()?.clone();
        body.position = Vec3::new(g.position[0], g.position[1], g.position[2]);
        body.ground.on_ground = g.on_ground;
        body.yaw = g.yaw;
        body.pitch = g.pitch;
        body.sprinting = g.sprinting;
        body.may_fly = g.creative();
        body.flying = g.abilities.flying;
        body.flying_speed = g.abilities.flying_speed;
        body.fall_flying = g.fall_flying;
        body.glider = glide_wear::gliding_slots(
            self.res.items(),
            &g.armor,
            g.inventory.get(g.selected).and_then(Option::as_ref),
            g.offhand.as_ref(),
        )
        .next()
        .is_some();
        body.leather_boots = worn_boots::leather_boots(&g.armor);
        let using = host
            .play
            .get(&g.conn)
            .and_then(|play| play.using())
            .and_then(|(slot, _)| {
                if slot == shield::OFFHAND_SLOT {
                    g.offhand.as_ref()
                } else {
                    g.inventory.get(slot)?.as_ref()
                }
            });
        self.set_bot_item_use(&mut body, using);
        Some((body, Some(g.conn)))
    }

    /// A simulated copy reads the current stack rather than inheriting an earlier use.
    fn set_bot_item_use(&self, body: &mut PlayerMotion, using: Option<&ItemStack>) {
        let item = using.and_then(|stack| self.res.items().get(&stack.item));
        body.item_use_speed = item.map(|item| item.use_speed);
        body.item_use_slows_sprint = item.is_some_and(|item| !item.can_sprint);
    }

    /// `CollisionGetter.getBlockCollisions` with an empty context: no entities or border.
    /// The shared `BlockCollisions` cursor skips missing chunks without loading them.
    pub(super) fn level_visit_block_collisions(
        &self,
        level: mod_api::LevelRef,
        aabb: [f64; 6],
        visit: &mut dyn FnMut(&VoxelShape) -> ControlFlow<()>,
    ) -> Option<ControlFlow<()>> {
        let bounds = aabb_of(aabb)?;
        let mut scene = SceneCollisions {
            world: self.native_level_view_world(level)?,
            binding: self.collision,
            subject: None,
        };
        let scan = BlockCollisionScan::new(
            &mut scene,
            CollisionContext::empty(false),
            bounds,
            false,
            usize::MAX,
        )
        .ok()?;
        for hit in scan {
            let shape = voxel_shape_from_core(hit.ok()?.shape);
            if let ControlFlow::Break(()) = visit(&shape) {
                return Some(ControlFlow::Break(()));
            }
        }
        Some(ControlFlow::Continue(()))
    }

    /// `CollisionGetter.noBlockCollision(null, aabb)` negated, without moving full cubes.
    pub(super) fn level_has_collisions(
        &self,
        level: mod_api::LevelRef,
        aabb: [f64; 6],
    ) -> Option<bool> {
        let bounds = aabb_of(aabb)?;
        let mut scene = SceneCollisions {
            world: self.native_level_view_world(level)?,
            binding: self.collision,
            subject: None,
        };
        BlockCollisionScan::any_collision(
            &mut scene,
            CollisionContext::empty(false),
            bounds,
            false,
            usize::MAX,
        )
        .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(forward: f32, strafe: f32) -> PlayerInput {
        let mut input = PlayerInput::default();
        input.forward = forward;
        input.strafe = strafe;
        input
    }

    #[test]
    fn keys_follow_the_signs() {
        let k = input_keys(&input(1.0, -1.0)).unwrap();
        assert!(k.forward && !k.backward && k.right && !k.left);
        assert!(
            k.analog.is_none(),
            "full keys use the keyboard's own vector"
        );
        let k = input_keys(&input(-1.0, 0.0)).unwrap();
        assert!(k.backward && !k.forward && !k.left && !k.right);
    }

    #[test]
    fn partial_input_is_a_stick() {
        let k = input_keys(&input(0.5, 0.0)).unwrap();
        assert!(k.forward);
        assert_eq!(k.analog, Some([0.0, 0.5]));
        let k = input_keys(&input(3.0, -7.0)).unwrap();
        assert!(k.analog.is_none(), "clamped to full keys");
    }

    #[test]
    fn non_numbers_are_refused() {
        assert!(input_keys(&input(f32::NAN, 0.0)).is_none());
        assert!(input_keys(&input(0.0, f32::INFINITY)).is_none());
    }

    #[test]
    fn sprint_needs_forward() {
        let mut i = input(0.0, 1.0);
        i.sprint = true;
        assert!(!sprints(&i));
        i.forward = -1.0;
        assert!(!sprints(&i));
        i.forward = 1.0;
        assert!(sprints(&i));
        i.sprint = false;
        assert!(!sprints(&i));
    }

    #[test]
    #[ignore = "daily"]
    fn guest_equipment_and_item_use_shape_native_movement() {
        use crate::game::native_entity_refs::tests::with_game;
        use crate::game::native_item_places::tests::{guest, open_to_lan};
        use sunrise_client::{chunk::Chunk, items::ItemStack};

        with_game(|game| {
            open_to_lan(game);
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(0, 0));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(1, 63, 1, stone).unwrap();
            let mut guest = guest(1);
            guest.position = [1.5, 64.0, 1.5];
            guest.on_ground = true;
            guest.loaded = true;
            guest.motion = Some(Box::new(
                PlayerMotion::new(game.motion, Vec3::new(1.5, 64.0, 1.5)).unwrap(),
            ));
            guest.offhand = Some(ItemStack::new("minecraft:bow", 1));
            let uuid = guest.uuid;
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            let player = game.player(uuid).unwrap();

            let run = |game: &mut Game<'_>| {
                let mut forward = input(1.0, 0.0);
                forward.sprint = true;
                game.player_simulate_move(player, forward).unwrap()
            };
            let free = run(game);
            game.as_guest(1, |game| game.using = Some((shield::OFFHAND_SLOT, 0)))
                .unwrap();
            let using = run(game);
            assert!(free.sprinting && !using.sprinting);
            assert!(using.position[2] > 1.5 && using.position[2] < free.position[2]);

            game.as_guest(1, |game| game.using = None).unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(1, |guest| {
                    guest.position[1] = 80.0;
                    guest.on_ground = false;
                })
                .unwrap();
            let jump = |game: &mut Game<'_>| {
                let mut jump = input(0.0, 0.0);
                jump.jump = true;
                jump.velocity = Some([0.0, -0.4, 0.0]);
                game.player_simulate_move(player, jump).unwrap()
            };
            let fall = jump(game);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(1, |guest| {
                    guest.armor[1] = Some(ItemStack::new("minecraft:elytra", 1));
                })
                .unwrap();
            let glide = jump(game);
            assert_eq!(fall.velocity[2], 0.0);
            assert!(glide.velocity[2] > 0.0 && glide.velocity[1] > fall.velocity[1]);
            let live = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
            assert!(!live.motion.as_ref().unwrap().fall_flying);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn guest_movement_preview_uses_the_guests_damage_immunity() {
        use crate::game::native_entity_refs::tests::with_game;
        use crate::game::native_item_places::tests::{guest, open_to_lan};
        use std::cell::Cell;
        use sunrise_client::{chunk::Chunk, health::DamageHooks};

        // The movement result doesn't expose health. Observe committed damage to the
        // simulated body through its game-owned hook table, not through the live guest.
        thread_local! {
            static HITS: Cell<u32> = const { Cell::new(0) };
        }
        crate::game::in_own_process(
            "game::native_bots::tests::guest_movement_preview_uses_the_guests_damage_immunity",
            || {
                with_game(|game| {
                    open_to_lan(game);
                    // The fixture hasn't opened a world to bind its item predicates to the
                    // level's damage types yet.
                    let mut items = world_items!(game).clone();
                    std::sync::Arc::make_mut(&mut items.enchantments)
                        .bind_damage_catalog(&game.health_resources.damage);
                    game.server_items = Some(std::sync::Arc::new(items));
                    game.streaming
                        .insert_remote(&mut game.world, Chunk::empty(0, 0));
                    let magma = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:magma_block")
                        .unwrap();
                    game.world.set(1, 63, 1, magma).unwrap();
                    let at = Vec3::new(1.5, 64.0, 1.5);
                    let mut guest = guest(1);
                    guest.position = [at.x, at.y, at.z];
                    guest.on_ground = true;
                    guest.loaded = true;
                    let mut body = PlayerMotion::new(game.motion, at).unwrap();
                    body.entity_id = guest.entity_id;
                    guest.motion = Some(Box::new(body));
                    let uuid = guest.uuid;
                    game.lan.as_mut().unwrap().hub.guests.push(guest);
                    let player = game.player(uuid).unwrap();
                    game.damage_hooks = DamageHooks {
                        post: Some(|_, _, _| HITS.with(|hits| hits.set(hits.get() + 1))),
                        ..DamageHooks::NONE
                    };
                    let preview = |game: &mut Game<'_>| {
                        let mut falling = input(0.0, 0.0);
                        falling.velocity = Some([0.0, -0.08, 0.0]);
                        let preview = game.player_simulate_move(player, falling).unwrap();
                        assert!(preview.on_ground);
                        HITS.replace(0)
                    };
                    assert_eq!(preview(game), 1, "magma hurts the unprotected preview");

                    let mut boots = ItemStack::new("minecraft:leather_boots", 1);
                    boots.enchant("minecraft:frost_walker", 1);
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(1, |guest| guest.armor[3] = Some(boots))
                        .unwrap();
                    assert_eq!(game.lan_acting, None);
                    assert_eq!(
                        preview(game),
                        0,
                        "only the unswapped guest wears Frost Walker"
                    );
                })
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn player_simulate_move_hears_living_travel_edits() {
        use crate::game::native_entity_refs::tests::with_game;
        use crate::game::native_item_places::tests::{guest, open_to_lan};
        use mod_api::{Flow, Hooks, LivingTravelEvent, LoopContext};
        use sunrise_client::chunk::Chunk;

        struct Boost;
        #[mod_api::__subscriptions]
        impl Hooks for Boost {
            fn living_travel(
                &self,
                _: &dyn LoopContext,
                event: &mut LivingTravelEvent<'_>,
            ) -> Flow {
                event.velocity[0] += 0.5;
                Flow::Continue
            }
        }

        with_game(|game| {
            open_to_lan(game);
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(0, 0));
            let mut guest = guest(1);
            guest.position = [1.5, 64.0, 1.5];
            guest.on_ground = false;
            guest.loaded = true;
            guest.motion = Some(Box::new(
                PlayerMotion::new(game.motion, Vec3::new(1.5, 64.0, 1.5)).unwrap(),
            ));
            let uuid = guest.uuid;
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            let player = game.player(uuid).unwrap();

            let plain = game.player_simulate_move(player, input(0.0, 0.0)).unwrap();
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new("boost", Box::new(Boost))].into();
            let boosted = mod_platform::testing::with_mods(mods, || {
                game.player_simulate_move(player, input(0.0, 0.0)).unwrap()
            });
            assert!(
                (boosted.position[0] - (plain.position[0] + 0.5)).abs() < 1e-6,
                "expected living_travel velocity edit in simulated move: plain={:?} boosted={:?}",
                plain.position,
                boosted.position
            );
        });
    }

    #[test]
    fn boxes_round_trip_and_bad_boxes_are_refused() {
        let b = [0.0, 1.0, 2.0, 3.0, 4.5, 6.0];
        let bounds = aabb_of(b).unwrap();
        assert_eq!(bounds.min_position().components(), [0.0, 1.0, 2.0]);
        assert_eq!(bounds.max_position().components(), [3.0, 4.5, 6.0]);
        assert!(aabb_of([1.0, 0.0, 0.0, 0.0, 1.0, 1.0]).is_none());
        assert!(aabb_of([0.0, f64::NAN, 0.0, 1.0, 1.0, 1.0]).is_none());
    }
}
