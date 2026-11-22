use super::*;
use crate::game::native_entity_refs::tests::with_game;
use sunrise_core::geometry::Vec3;

struct OneMob<'a>(&'a Mob);
impl mobs::LiveMobs for OneMob<'_> {
    fn find(&self, id: u32) -> Option<&Mob> {
        (self.0.id == id).then_some(self.0)
    }

    fn for_each(&self, visitor: &mut dyn FnMut(&Mob)) {
        visitor(self.0);
    }
}

/// Run the public target getter with the actor and the other mob outside `Game`.
pub(in crate::game) fn read_borrowed_target(
    game: &mut Game<'_>,
    warden: MobRef,
    other: u32,
    tick_conn: u32,
) -> Option<LivingRef> {
    let (_, view) = game
        .lan_guest_views()
        .into_iter()
        .find(|(conn, _)| *conn == tick_conn)
        .unwrap();
    let identity = game.guest_identity(tick_conn).unwrap();
    let mut loan = game.mobs.lift();
    let index = loan
        .iter()
        .position(|mob| mob.id == warden.entity().id())
        .unwrap();
    let result = loan
        .with_one_and_rest(index, |actor, before, after| {
            let live = OneMob(
                before
                    .iter()
                    .chain(after)
                    .find(|mob| mob.id == other)
                    .unwrap(),
            );
            let mut world = game.mob_world_base(&[]);
            world.goal_host = Some(game);
            world.live_mobs = Some(&live);
            world.player = Some(view);
            world.player_identity = Some(identity);
            warden.target(&GoalCx {
                m: actor,
                w: &world,
                host: game,
            })
        })
        .unwrap();
    game.mobs
        .restore(loan, sunrise_client::join_seq::RestoreOrder::SurvivorsFirst);
    result
}

#[test]
#[ignore = "daily"]
fn scoped_host_reads_live_other_mob_before_stale_snapshot() {
    with_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut actor = Mob::new(9001, kind.clone(), Vec3::ZERO, 0.0);
        let mut other = Mob::new(9002, kind.clone(), Vec3::ZERO, 0.0);
        let stale = Mob::new(9003, kind, Vec3::ZERO, 0.0);
        let snapshot = [
            crate::game::creatures::seen(&other, game.level.player_uuid, game.items()),
            crate::game::creatures::seen(&stale, game.level.player_uuid, game.items()),
        ];
        other.yaw = 37.0;
        other.head_pitch = -12.0;
        other.attrs.max_health = 73.0;
        let live = OneMob(&other);
        let mut world = game.mob_world_base(&snapshot);
        world.goal_host = Some(game);
        world.live_mobs = Some(&live);
        let epoch = game.native_lifecycle.epoch().unwrap();
        let entity = EntityRef::new(epoch, other.id);
        let context = GoalCx {
            m: &mut actor,
            w: &world,
            host: game,
        };
        assert_eq!(entity.rotation(&context), Some([37.0, -12.0]));
        assert_eq!(LivingRef::new(entity).max_health(&context), Some(73.0));
        assert!(context.lookup_entity(other.id).is_some());
        assert!(context.lookup_entity(stale.id).is_none());
        assert!(context.lookup_entity(9999).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn scoped_host_resolves_other_real_players_from_the_game() {
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
    };
    use sunrise_client::lan::{Joined, PlayerSender, guest::Guest};
    use sunrise_protocol::packets::configuration::ClientInformation;

    with_game(|game| {
        crate::game::native_item_places::tests::open_to_lan(game);
        for id in [12, 13] {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: id,
                    uuid: u128::from(id),
                    name: format!("guest{id}"),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                id as i32,
                [f64::from(id), 64.0, 5.0],
                10,
            );
            guest.placed = true;
            game.lan.as_mut().unwrap().hub.guests.push(guest);
        }
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut actor = Mob::new(9001, kind, Vec3::ZERO, 0.0);
        let mut world = game.mob_world_base(&[]);
        world.goal_host = Some(game);
        // Deliberately select only one guest: the other remains a real live
        // player and must resolve through the underlying game host.
        world.player_identity = Some(([0, 0, 0, 12], 12));
        let context = GoalCx {
            m: &mut actor,
            w: &world,
            host: game,
        };
        let player = context.lookup_player(13).expect("other guest is online");
        assert_eq!(player.entity().id(), 13);
        assert_eq!(context.lookup_entity(13), Some(player.entity()));
        assert!(context.lookup_living(player.entity()).is_some());
        assert_eq!(
            player.entity().position(&context),
            player.entity().position(game)
        );
        assert!(context.lookup_player(999).is_none());
    });
}

/// The body yaw is a mob's own raw `yBodyRot`, not a normalized facing or a
/// player's: the scoped goal view and the game host agree, and a stale handle
/// or another kind has none.
#[test]
#[ignore = "daily"]
fn body_yaw_is_the_raw_mob_yaw_and_none_for_others() {
    use crate::game::native_guest_turn_tests::with_guests;

    with_guests(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut mob = Mob::new(40, kind.clone(), Vec3::ZERO, 0.0);
        mob.body_yaw = -725.5;
        game.mobs.push(mob);
        let entity = game.entity(40).expect("the mob's handle");
        assert_eq!(entity.body_yaw(game), Some(-725.5));
        assert!(game.entity_set_head_yaw(entity, 45.0));
        assert_eq!(entity.body_yaw(game), Some(-725.5));

        let stale = EntityRef::new(
            mod_api::WorldEpoch::new(entity.epoch().get() + 1),
            entity.id(),
        );
        assert_eq!(stale.body_yaw(game), None);

        let players: Vec<_> = [12, 13]
            .map(|conn| game.player(conn).expect("guest player").entity())
            .into();
        for player in &players {
            assert_eq!(player.body_yaw(game), None);
        }
        game.others.push(crate::game::commands::OtherEntity {
            attachment_lifetime: Default::default(),
            knot_check: None,
            eye: None,
            join: Default::default(),
            item_transformation: None,
            interaction: None,
            id: 41,
            kind: "minecraft:marker".into(),
            position: Vec3::ZERO,
            yaw: 30.0,
            pitch: 0.0,
        });
        let marker = game.entity(41).expect("the marker's handle");
        assert_eq!(marker.body_yaw(game), None);

        let mut dragon = crate::game::dragon::Dragon::new(Vec3::ZERO, 540.75, 0);
        dragon.id = 42;
        game.dragon.replace(dragon);
        let dragon = game.entity(42).expect("the dragon's handle");
        assert!(game.mob(dragon).is_some());
        assert_eq!(dragon.body_yaw(game), Some(540.75));
        let stale_dragon = EntityRef::new(stale.epoch(), dragon.id());
        assert_eq!(stale_dragon.body_yaw(game), None);
        let part = game.entity(43).expect("the dragon head's handle");
        assert_eq!(part.body_yaw(game), None);

        let other = game
            .mobs
            .iter()
            .find(|mob| mob.id == 40)
            .expect("the mob row");
        let snapshot = [crate::game::creatures::seen(
            other,
            game.level.player_uuid,
            game.items(),
        )];
        let live = OneMob(other);
        let mut actor = Mob::new(9001, kind, Vec3::ZERO, 0.0);
        actor.body_yaw = 815.25;
        let own = EntityRef::new(entity.epoch(), actor.id);
        let mut world = game.mob_world_base(&snapshot);
        world.goal_host = Some(game);
        world.live_mobs = Some(&live);
        let context = GoalCx {
            m: &mut actor,
            w: &world,
            host: game,
        };
        assert_eq!(own.body_yaw(&context), Some(815.25));
        assert_eq!(entity.body_yaw(&context), Some(-725.5));
        assert_eq!(dragon.body_yaw(&context), Some(540.75));
        assert_eq!(stale_dragon.body_yaw(&context), None);
        assert_eq!(part.body_yaw(&context), None);
        assert_eq!(stale.body_yaw(&context), None);
        for player in players {
            assert_eq!(player.body_yaw(&context), None);
        }
        assert_eq!(marker.body_yaw(&context), None);
        context.m.removed = true;
        assert_eq!(own.body_yaw(&context), None);

        let role = game.options.execution_role;
        game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
        assert_eq!(entity.body_yaw(game), None);
        assert_eq!(dragon.body_yaw(game), None);
        game.options.execution_role = role;

        game.mobs
            .iter_mut()
            .find(|mob| mob.id == 40)
            .unwrap()
            .removed = true;
        assert_eq!(entity.body_yaw(game), None);
        game.dragon.discard();
        assert_eq!(dragon.body_yaw(game), None);
    });
}
