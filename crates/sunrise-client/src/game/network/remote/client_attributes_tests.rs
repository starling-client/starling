/// After `ClientboundUpdateAttributes`, the local player's `LivingRef`
/// reads its attribute value, base, modifiers and attribute list from the
/// client's own copy of the synced instances. A mob the server tracks
/// keeps no attribute state of its own on the client, so its reads are
/// `None` there.
#[test]
#[ignore = "daily"]
fn update_attributes_answers_the_local_players_living_reads() {
    use crate::game::native_entity_refs::tests::with_client_game;
    use sunrise_client::{mobs::Mob, motion::PlayerMotion};
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::{
        buf::Reader,
        connection::Pipe,
        packets::play::{self, Clientbound as C},
    };

    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "attributes-client").unwrap();
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.player.as_mut().unwrap().entity_id = 42;
        game.mobs.push(Mob::new(60, zombie, Vec3::ZERO, 0.0));
        let remote = game.remote.as_mut().unwrap();
        remote.entity_id = 42;
        remote.logged_in = true;
        remote.entities.insert(
            60,
            crate::game::remote::Tracked {
                kind: "minecraft:zombie".into(),
                position: [0.0; 3],
                codec_base: [0.0; 3],
                on_fire: false,
                mod_fire_immune: false,
                silent: false,
                vehicle: None,
            },
        );

        let health = "minecraft:max_health";
        let living = game.living(game.entity(42).unwrap()).unwrap();
        // Before the server says anything, the client's own map holds the
        // type's defaults, as a vanilla client's does before its sync.
        assert_eq!(living.attribute_value(game, health), Some(20.0));
        assert_eq!(living.attribute_modifiers(game, health), Some(Vec::new()));

        let registry_id = game.motion.attributes.id(health).unwrap().registry_id();
        let packet = C::UpdateAttributes(play::UpdateAttributes {
            id: 42,
            values: vec![play::AttributeSnapshot {
                attribute: registry_id,
                base: 30.0,
                modifiers: vec![play::AttributeModifier {
                    id: "test:bonus".into(),
                    amount: 5.0,
                    operation: 0,
                }],
            }],
        });
        let body = packet.encode();
        let mut reader = Reader::new(&body);
        let id = reader.varint().unwrap();
        game.handle_packet(C::decode(id, reader.rest()).unwrap())
            .unwrap();

        assert_eq!(living.attribute_value(game, health), Some(35.0));
        assert_eq!(living.attribute_base(game, health), Some(30.0));
        assert_eq!(
            living.attribute_modifiers(game, health),
            Some(vec![mod_api::AttributeModifier::new(
                mod_api::Identifier::parse("test:bonus").unwrap(),
                5.0,
                mod_api::AttributeOperation::AddValue,
            )])
        );
        let listed = living.attributes(game).unwrap();
        assert!(listed.contains(&mod_api::Identifier::parse(health).unwrap()));

        // The mob's attributes are the server's business: no client state,
        // no read.
        let mob = mod_api::LivingRef::new(game.entity(60).unwrap());
        assert_eq!(mob.attribute_value(game, health), None);
        assert_eq!(mob.attribute_base(game, health), None);
        assert_eq!(mob.attribute_modifiers(game, health), None);
        assert_eq!(mob.attributes(game), None);
    });
}
