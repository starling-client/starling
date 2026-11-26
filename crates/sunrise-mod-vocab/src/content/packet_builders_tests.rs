use super::*;

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
