use crate::game::remote::{look_at_rotation, snapshot_value};

/// An NPC plugin's scale: a base the plugin sets, or a modifier on 1.
#[test]
fn attribute_values_add_then_multiply_as_vanilla() {
    use sunrise_protocol::packets::play::{AttributeModifier, AttributeSnapshot};
    let modifier = |amount, operation| AttributeModifier {
        id: "minecraft:proof".into(),
        amount,
        operation,
    };
    let snapshot = |base, modifiers| AttributeSnapshot {
        attribute: 0,
        base,
        modifiers,
    };
    assert_eq!(snapshot_value(&snapshot(2.0, vec![])), 2.0);
    assert_eq!(snapshot_value(&snapshot(1.0, vec![modifier(1.0, 0)])), 2.0);
    // (1 + 1) * (1 + 0.5) from the base, then * (1 + 1) of the total.
    let all = vec![modifier(1.0, 0), modifier(0.5, 1), modifier(1.0, 2)];
    assert_eq!(snapshot_value(&snapshot(1.0, all)), 6.0);
}

/// `Entity.lookAt`: yaw 0 faces +Z, -90 faces +X; pitch -90 looks up.
#[test]
fn look_at_turns_towards_the_target() {
    let close = |(yaw, pitch): (f32, f32), want: (f32, f32)| {
        assert!(
            (yaw - want.0).abs() < 1e-3 && (pitch - want.1).abs() < 1e-3,
            "{yaw} {pitch}, want {want:?}"
        );
    };
    let at = [10.0, 64.0, -5.0];
    let off = |d: [f64; 3]| [at[0] + d[0], at[1] + d[1], at[2] + d[2]];
    close(look_at_rotation(at, off([0.0, 0.0, 3.0])), (0.0, 0.0));
    close(look_at_rotation(at, off([3.0, 0.0, 0.0])), (-90.0, 0.0));
    close(look_at_rotation(at, off([-3.0, 0.0, 0.0])), (90.0, 0.0));
    close(look_at_rotation(at, off([0.0, 0.0, -3.0])), (-180.0, 0.0));
    close(look_at_rotation(at, off([0.0, 2.0, 2.0])), (0.0, -45.0));
    close(look_at_rotation(at, off([0.0, -4.0, 4.0])), (0.0, 45.0));
}

/// `handleMovePlayer` moves the player's pitch and the model's old pitch
/// (`xRotO`) alike: a relative change keeps the ease a tick began, each
/// end clamped on its own, and an absolute one snaps both (#3844).
#[test]
fn a_position_packet_moves_both_pitch_ends() {
    use super::super::position_packet_pitch as pitch;
    // The tick is laying the head down from 30 to 0; the packet turns 10.
    assert_eq!(
        (pitch(30.0, 10.0, true), pitch(0.0, 10.0, true)),
        (40.0, 10.0)
    );
    // Each end clamps on its own.
    assert_eq!(
        (pitch(80.0, 20.0, true), pitch(60.0, 20.0, true)),
        (90.0, 80.0)
    );
    // An absolute pitch ignores both ends.
    assert_eq!(
        (pitch(30.0, -45.0, false), pitch(0.0, -45.0, false)),
        (-45.0, -45.0)
    );
    assert_eq!(pitch(0.0, 120.0, false), 90.0);
}
