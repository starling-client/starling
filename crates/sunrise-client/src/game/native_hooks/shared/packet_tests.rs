#[test]
fn client_packet_view_exposes_the_type_and_custom_payload_only() {
    use sunrise_protocol::packets::{
        configuration::{CustomPayload, Ping},
        play::Clientbound,
    };
    let ping = super::client_packet_view(&Clientbound::Ping(Ping { id: 7 }));
    assert_eq!(ping.id.to_string(), "minecraft:ping");
    assert_eq!(ping.custom_payload, None);

    let payload = super::client_packet_view(&Clientbound::CustomPayload(CustomPayload {
        channel: "example:message".into(),
        data: vec![0, 127, 255],
    }));
    assert_eq!(payload.id.to_string(), "minecraft:custom_payload");
    let payload = payload.custom_payload.unwrap();
    assert_eq!(payload.channel, "example:message");
    assert_eq!(payload.data, [0, 127, 255]);

    for id in [-1, i32::MAX] {
        let unknown = super::client_packet_view(&Clientbound::Unknown(id, vec![9]));
        assert_eq!(unknown.id.to_string(), "sunrise:unknown");
        assert_eq!(unknown.custom_payload, None);
    }
}

#[test]
fn serverbound_packet_view_names_the_packets_the_game_sends() {
    use sunrise_protocol::packets::{
        configuration::CustomPayload,
        play::{Serverbound, SetCarriedItem},
    };
    let slot =
        super::serverbound_packet_view(&Serverbound::SetCarriedItem(SetCarriedItem { slot: 3 }));
    assert_eq!(slot.id.to_string(), "minecraft:set_carried_item");
    assert_eq!(slot.custom_payload, None);

    let payload = super::serverbound_packet_view(&Serverbound::CustomPayload(CustomPayload {
        channel: "example:message".into(),
        data: vec![1, 2],
    }));
    assert_eq!(payload.id.to_string(), "minecraft:custom_payload");
    let payload = payload.custom_payload.unwrap();
    assert_eq!(
        (payload.channel.as_str(), payload.data.as_slice()),
        ("example:message", &[1, 2][..])
    );

    let unknown = super::serverbound_packet_view(&Serverbound::Unknown(-1, vec![9]));
    assert_eq!(unknown.id.to_string(), "sunrise:unknown");
}
