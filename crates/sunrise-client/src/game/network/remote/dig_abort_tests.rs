use crate::game::{Hit, native_entity_refs::tests::with_client_game};
use mod_api::{ClientHost, ClientMiningPredictEvent, ClientPacketSendEvent, Flow, Hooks, ModEntry};
use mod_platform::testing::with_mods;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_client::{chunk::Chunk, motion::PlayerMotion};
use sunrise_core::geometry::Vec3;
use sunrise_protocol::connection::Pipe;

struct CountActions {
    packets: Arc<AtomicUsize>,
    vetoes: Arc<AtomicUsize>,
}
#[mod_api::__subscriptions]
impl Hooks for CountActions {
    fn client_mining_predict(
        &self,
        _: &dyn ClientHost,
        event: &mut ClientMiningPredictEvent,
    ) -> Flow {
        assert!(!event.prediction.starting);
        assert_eq!(event.prediction.pos, [0, 64, 0]);
        assert!(event.prediction.progress > 0.5);
        self.vetoes.fetch_add(1, Ordering::Relaxed);
        Flow::Cancel
    }

    fn client_packet_send(
        &self,
        _: &dyn ClientHost,
        event: &mut ClientPacketSendEvent<'_>,
    ) -> Flow {
        if event.packet.id.path() == "player_action" {
            self.packets.fetch_add(1, Ordering::Relaxed);
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn stopping_a_retained_client_dig_aborts_only_once() {
    let count = Arc::new(AtomicUsize::new(0));
    let vetoes = Arc::new(AtomicUsize::new(0));
    let mods = vec![ModEntry::new(
        "count_actions",
        Box::new(CountActions {
            packets: count.clone(),
            vetoes: vetoes.clone(),
        }),
    )]
    .into();
    with_mods(mods, || {
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "MiningTest").unwrap();
            let pos = [0, 64, 0];
            game.set_local_mode(sunrise_client::command::args::GameMode::Survival);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.5)).unwrap());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.forced_hit = Some(Hit {
                pos,
                face: [0, 1, 0],
                point: [0.5, 65.0, 0.5],
                inside: false,
            });
            game.refresh_client_pick(1.0);
            game.forced_hit = None;
            let held = game.main_hand_key();
            let remote = game.remote.as_mut().unwrap();
            remote.destroying = Some((pos, 1, 0.5));
            remote.destroying_item = held;
            remote.is_destroying = true;
            game.remote_continue_attack(true);
            assert_eq!(vetoes.load(Ordering::Relaxed), 1);
            let remote = game.remote.as_ref().unwrap();
            assert!(remote.is_destroying);
            assert_eq!(remote.destroying, Some((pos, 1, 0.5)));
            assert_eq!(count.load(Ordering::Relaxed), 0);
            game.remote_continue_attack(false);
            game.remote_continue_attack(false);
            assert_eq!(
                game.remote.as_ref().unwrap().destroying,
                Some((pos, 1, 0.0))
            );
        })
    });
    assert_eq!(count.load(Ordering::Relaxed), 1);
}
