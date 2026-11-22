use crate::game::{
    Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
    native_entity_refs::tests::with_game,
};
use sunrise_client::{items::ItemStack, mobs::Mob};
use sunrise_core::geometry::Vec3;
use sunrise_items::equipment::{MAINHAND, OFFHAND};
use sunrise_protocol::packets::play::{Clientbound as C, EntityEvent};

/// `LivingEntity.handleEntityEvent(55)` is `swapHandItems`: the whole
/// stacks trade hands, not just their ids.
#[test]
#[ignore = "daily"]
fn the_hand_swap_event_keeps_whole_stacks() {
    with_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.damage = 40;
        sword.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("Biter".into()));
        let mut shield = ItemStack::new("minecraft:shield", 1);
        shield.damage = 12;
        let mut mob = Mob::new(31, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        mob.extra.set_slot_stack(MAINHAND, Some(sword.clone()));
        mob.extra.set_slot_stack(OFFHAND, Some(shield.clone()));
        game.mobs.push(mob);
        let swap = |game: &mut super::super::Game| {
            game.handle_packet(C::EntityEvent(EntityEvent { id: 31, event: 55 }))
                .unwrap();
        };
        swap(game);
        let m = game.mobs.iter().find(|m| m.id == 31).unwrap();
        assert_eq!(m.extra.slot_stack(MAINHAND), Some(&shield));
        assert_eq!(m.extra.slot_stack(OFFHAND), Some(&sword));
        swap(game);
        let m = game.mobs.iter().find(|m| m.id == 31).unwrap();
        assert_eq!(m.extra.slot_stack(MAINHAND), Some(&sword));
        assert_eq!(m.extra.slot_stack(OFFHAND), Some(&shield));
    });
}

#[test]
#[ignore = "daily"]
fn joining_a_server_empties_the_off_hand_too() {
    // ClientPacketListener.handleLogin makes a new LocalPlayer, whose whole Inventory is
    // empty: a server that never sends slot 45 must not leave the last server's item.
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    let (first, _first_server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(first, "HopTest").unwrap();
    let stack = || Some(ItemStack::new("minecraft:shield", 1));
    game.inventory[0] = stack();
    game.armor[0] = stack();
    game.offhand = stack();
    let (second, _second_server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(second, "HopTest").unwrap();
    assert!(game.inventory[0].is_none() && game.armor[0].is_none());
    assert!(game.offhand.is_none());
}
