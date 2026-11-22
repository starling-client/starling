use super::*;
use sunrise_protocol::{buf::Reader, packets::play::*};

fn matches(
    packet: Result<ServerboundPlayPacket, PacketBuildError>,
    expected: Serverbound,
    name: &str,
) {
    let packet = packet.unwrap();
    assert_eq!(packet.id.to_string(), format!("minecraft:{name}"));
    let bytes = expected.encode();
    let mut reader = Reader::new(&bytes);
    reader.varint().unwrap();
    assert_eq!(packet.body, reader.rest(), "{name}");
}

#[test]
fn every_portable_builder_matches_the_game_encoder() {
    type P = ServerboundPlayPacket;
    matches(
        P::use_item(1, 7, 90.0, -12.5),
        Serverbound::UseItem(UseItem {
            hand: 1,
            sequence: 7,
            yaw: 90.0,
            pitch: -12.5,
        }),
        "use_item",
    );
    matches(
        P::use_item_on(0, [1, 64, -2], 1, [0.5, 1.0, 0.25], true, false, 8),
        Serverbound::UseItemOn(UseItemOn {
            hand: 0,
            pos: [1, 64, -2],
            face: 1,
            cursor: [0.5, 1.0, 0.25],
            inside: true,
            world_border: false,
            sequence: 8,
        }),
        "use_item_on",
    );
    for action in 0..=8 {
        matches(
            P::player_action(action, [1, 64, -2], 3, 7),
            Serverbound::PlayerAction(PlayerAction {
                action,
                pos: [1, 64, -2],
                direction: 3,
                sequence: 7,
            }),
            "player_action",
        );
    }
    for location in [
        [0.25, 1.0, -0.5],
        [0.0; 3],
        [1.5, 64.0, -2.25],
        [1.0e20, -1.0e20, 0.0],
    ] {
        matches(
            P::interact(9, 1, location, true),
            Serverbound::Interact(Interact {
                id: 9,
                hand: 1,
                location,
                secondary: true,
            }),
            "interact",
        );
    }
    matches(
        P::attack(9),
        Serverbound::Attack(Attack { id: 9 }),
        "attack",
    );
    matches(
        P::pick_item_from_block([1, 64, -2], true),
        Serverbound::PickItemFromBlock(PickItemFromBlock {
            pos: [1, 64, -2],
            include_data: true,
        }),
        "pick_item_from_block",
    );
    matches(
        P::pick_item_from_entity(9, false),
        Serverbound::PickItemFromEntity(PickItemFromEntity {
            id: 9,
            include_data: false,
        }),
        "pick_item_from_entity",
    );
    for front in [false, true] {
        matches(
            P::sign_update(
                [1, 64, -2],
                front,
                ["Hello".into(), "世界".into(), "🦀".into(), "".into()],
            ),
            Serverbound::SignUpdate(SignUpdate {
                pos: [1, 64, -2],
                front,
                lines: ["Hello".into(), "世界".into(), "🦀".into(), "".into()],
            }),
            "sign_update",
        );
    }
    matches(
        P::move_player_pos([1.5, 64.0, -2.25], true, false),
        Serverbound::MovePlayerPos(MovePlayerPos {
            x: 1.5,
            y: 64.0,
            z: -2.25,
            flags: 1,
        }),
        "move_player_pos",
    );
    matches(
        P::move_player_pos_rot([1.5, 64.0, -2.25], 90.0, -12.5, false, true),
        Serverbound::MovePlayerPosRot(MovePlayerPosRot {
            x: 1.5,
            y: 64.0,
            z: -2.25,
            yaw: 90.0,
            pitch: -12.5,
            flags: 2,
        }),
        "move_player_pos_rot",
    );
    matches(
        P::move_player_rot(90.0, -12.5, true, true),
        Serverbound::MovePlayerRot(MovePlayerRot {
            yaw: 90.0,
            pitch: -12.5,
            flags: 3,
        }),
        "move_player_rot",
    );
    for (ground, collision, flags) in [
        (false, false, 0),
        (true, false, 1),
        (false, true, 2),
        (true, true, 3),
    ] {
        matches(
            P::move_player_status_only(ground, collision),
            Serverbound::MovePlayerStatus(MovePlayerStatus { flags }),
            "move_player_status_only",
        );
    }
}

#[test]
fn invalid_fields_are_refused_by_packet_and_field() {
    type P = ServerboundPlayPacket;
    assert_eq!(
        P::move_player_pos([f64::NAN, 64.0, -2.25], true, false),
        Err(PacketBuildError::NonFinite {
            packet: "move_player_pos".into(),
            field: "position".into()
        })
    );
    assert_eq!(
        P::sign_update(
            [1, 64, -2],
            true,
            ["a".repeat(385), "".into(), "".into(), "".into()]
        ),
        Err(PacketBuildError::SignLineTooLong {
            packet: "sign_update".into(),
            field: "lines[0]".into()
        })
    );
    assert!(
        P::sign_update(
            [1, 64, -2],
            true,
            ["🦀".repeat(192), "a".repeat(384), "".into(), "".into()]
        )
        .is_ok()
    );
    assert!(
        P::sign_update(
            [1, 64, -2],
            true,
            ["🦀".repeat(193), "".into(), "".into(), "".into()]
        )
        .is_err()
    );
    assert_eq!(
        P::player_action(9, [1, 64, -2], 0, 7),
        Err(PacketBuildError::UnknownAction {
            packet: "player_action".into(),
            field: "action".into()
        })
    );
    assert!(P::use_item(2, 7, 0.0, 0.0).is_err());
    assert!(P::use_item(0, 7, f32::INFINITY, 0.0).is_err());
    assert!(P::use_item_on(0, [1, 64, -2], 6, [0.0; 3], false, false, 7).is_err());
    assert!(P::use_item_on(0, [1, 64, -2], 1, [f32::NAN; 3], false, false, 7).is_err());
    assert!(P::interact(9, 0, [f64::NAN; 3], false).is_err());
    assert!(P::move_player_pos_rot([0.0; 3], 0.0, f32::NAN, false, false).is_err());
    assert!(P::move_player_rot(f32::NAN, 0.0, false, false).is_err());
    assert!(P::pick_item_from_block([33_554_432, 64, 0], false).is_err());
    assert!(P::pick_item_from_block([0, 2048, 0], false).is_err());
}

#[test]
fn container_limits_preserve_wire_ranges_and_refuse_129_slots() {
    type P = ServerboundPlayPacket;
    assert!(P::validate_container_click_limits(0, 5, 36, 128).is_ok());
    assert!(
        P::validate_container_click_limits(i64::from(i32::MIN), i64::from(i32::MAX), -999, 0)
            .is_ok()
    );
    assert_eq!(
        P::validate_container_click_limits(0, 5, 36, 129),
        Err(PacketBuildError::TooManyChangedSlots {
            packet: "container_click".into(),
            field: "changed_slots".into()
        })
    );
    for (container, state, slot, field) in [
        (i64::MAX, 5, 36, "container_id"),
        (0, i64::MAX, 36, "state_id"),
        (0, 5, 32768, "slot"),
    ] {
        assert_eq!(
            P::validate_container_click_limits(container, state, slot, 0),
            Err(PacketBuildError::OutOfRange {
                packet: "container_click".into(),
                field: field.into()
            })
        );
    }
}
