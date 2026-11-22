#[test]
#[ignore = "daily"]
fn rejected_menu_packets_retire_the_previous_client_menu() {
    use crate::game::native_entity_refs::tests::with_game;
    use crate::game::{Screen, execution_role::ExecutionRole};
    use sunrise_client::items::ItemStack;
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::{
        connection::Pipe,
        packets::play::{Clientbound as C, MountScreenOpen, OpenScreen},
    };

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "MenuTest").unwrap();
        for packet in [
            C::OpenScreen(OpenScreen {
                container: 7,
                menu: -1,
                title: Tag::String("unsupported".into()),
            }),
            C::MountScreenOpen(MountScreenOpen {
                container: 7,
                columns: 0,
                entity: i32::MAX,
            }),
        ] {
            let remote = game.remote.as_mut().unwrap();
            remote.menu.set(Some((7, "minecraft:generic_9x3".into())));
            let generation = remote.menu.generation;
            game.set_screen(Screen::Inventory);
            game.carried = Some(ItemStack::new("minecraft:stone", 1));
            game.handle_packet(packet).unwrap();
            let remote = game.remote.as_ref().unwrap();
            assert!(remote.menu.as_ref().is_none());
            assert_ne!(remote.menu.generation, generation);
            assert!(game.carried.is_none());
            assert!(game.screen == Screen::Playing);
        }
    });
}
