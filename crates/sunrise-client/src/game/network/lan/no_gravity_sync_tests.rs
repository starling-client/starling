//! `Entity.setNoGravity` writes `DATA_NO_GRAVITY` (5), so every tracked entity's flag reaches
//! its clients through `ClientboundSetEntityDataPacket`, and a client's own tick reads
//! `isNoGravity`. A host sent it only for falling blocks (#5028): a dropped item or a mob with
//! the flag fell on its clients until the next position sync.
use crate::game::Game;
use crate::game::native_entity_refs::tests::with_game;
use sunrise_client::lan::entity::States;
use sunrise_client::{entities::ItemEntity, items::ItemStack, mobs::Mob};
use sunrise_core::geometry::Vec3;
use sunrise_protocol::entity_data::{Value, serializer};

/// What the host's tracking pass holds for `id` at 5: the value it sends, and whether false
/// is its default (so a flag taken away goes out too).
fn no_gravity_entry(game: &mut Game<'_>, id: u32) -> (Option<Value>, bool) {
    let mut states = States::default();
    game.lan_entities(&mut states, false, 0, None);
    let e = states
        .as_slice()
        .iter()
        .find(|e| e.id == id as i32)
        .expect("the entity is tracked");
    let sent = e
        .meta
        .iter()
        .find(|(index, _, _)| *index == 5)
        .map(|(_, _, value)| value.clone());
    let default = e
        .defaults
        .contains(&(5, serializer::BOOLEAN, Value::Bool(false)));
    (sent, default)
}

#[test]
#[ignore = "daily"]
fn an_items_and_a_mobs_no_gravity_reaches_the_tracker() {
    with_game(|game| {
        let _rx = super::world_spawn_sync_tests::host_for(game);
        // A pig and a dropped item with the flag, as a summon's `NoGravity:1b` loads them.
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let mut mob = Mob::new(101, kind, Vec3::new(1.5, 70.0, 1.5), 0.0);
        mob.no_gravity = true;
        game.mobs.push(mob);
        let stack = ItemStack::new("minecraft:diamond", 1);
        let mut drop = ItemEntity::new(102, stack, Vec3::new(2.5, 70.0, 1.5), Vec3::ZERO, 0.0);
        drop.no_gravity = true;
        game.dropped.push(drop);
        game.next_entity = 102;
        let (pig, item) = (101, 102);
        assert!(game.mobs.last().unwrap().no_gravity && game.dropped.last().unwrap().no_gravity);
        for id in [pig, item] {
            assert_eq!(
                no_gravity_entry(game, id),
                (Some(Value::Bool(true)), true),
                "{id}"
            );
        }

        // `setNoGravity(false)`: nothing past the default, which the tracker sends.
        game.mobs.last_mut().unwrap().no_gravity = false;
        game.dropped.last_mut().unwrap().no_gravity = false;
        for id in [pig, item] {
            assert_eq!(no_gravity_entry(game, id), (None, true), "{id}");
        }
    });
}
