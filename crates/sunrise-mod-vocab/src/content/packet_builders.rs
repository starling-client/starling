//! Typed requests and errors for host-owned vanilla play packet construction.
use crate::ServerboundPlayPacket;

/// A value cannot be represented by the named packet field.
// no wire: reported locally while building a vanilla packet.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PacketBuildError {
    /// A number exceeds the field's wire representation.
    OutOfRange {
        /// Vanilla packet name.
        packet: String,
        /// Refused field.
        field: String,
    },
    /// A container click contains more than 128 changed slots.
    TooManyChangedSlots {
        /// Vanilla packet name.
        packet: String,
        /// Refused field.
        field: String,
    },
    /// A sign line exceeds 384 Java UTF-16 code units.
    SignLineTooLong {
        /// Vanilla packet name.
        packet: String,
        /// Refused field, including the line index.
        field: String,
    },
    /// A movement position is NaN, or a movement angle or block-hit cursor coordinate is not finite.
    NonFinite {
        /// Vanilla packet name.
        packet: String,
        /// Refused field.
        field: String,
    },
    /// An action, hand or direction has no vanilla discriminant.
    UnknownAction {
        /// Vanilla packet name.
        packet: String,
        /// Refused field.
        field: String,
    },
    /// The host cannot convert or hash a supplied component without loss.
    ComponentCannotEncode {
        /// Vanilla packet name.
        packet: String,
        /// Refused stack/component field.
        field: String,
    },
    /// The client host or connection cannot supply the packet's registry context.
    Unavailable {
        /// Vanilla packet name.
        packet: String,
        /// Missing context.
        field: String,
    },
}

/// Typed request for a host-encoded vanilla serverbound play packet.
///
/// Pass a value to
/// [`NetworkClientHost::build_play_packet`](crate::prelude::NetworkClientHost::build_play_packet)
/// to encode it with the client's vanilla protocol encoder.
// no wire: local request for the client's vanilla encoder
// exhaustive: these are the complete typed play-packet requests; protocol updates deliberately change this shape.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayPacketRequest {
    /// Use the main (0) or off (1) hand, with the caller's sequence and view angles.
    UseItem {
        /// Vanilla `InteractionHand` ordinal: 0 main, 1 off.
        hand: i32,
        /// Caller-owned prediction sequence number.
        sequence: i32,
        /// View yaw angle in degrees.
        yaw: f32,
        /// View pitch angle in degrees.
        pitch: f32,
    },
    /// Use a hand on a block. Face uses vanilla's 0..=5 direction order;
    /// cursor is relative to the block. Sequence numbers remain caller-owned.
    UseItemOn {
        /// Vanilla `InteractionHand` ordinal: 0 main, 1 off.
        hand: i32,
        /// Target block coordinates representable in vanilla's packed block position.
        pos: [i32; 3],
        /// Vanilla `Direction` ordinal (0 down, 1 up, 2 north, 3 south, 4 west, 5 east).
        face: i32,
        /// Hit position relative to the block.
        cursor: [f32; 3],
        /// Whether the player's eyes are inside the target block.
        inside: bool,
        /// Whether the hit occurred on the world border.
        world_border: bool,
        /// Caller-owned prediction sequence number.
        sequence: i32,
    },
    /// Player action: 0 start destroy, 1 change direction, 2 abort, 3 stop,
    /// 4 drop all, 5 drop one, 6 release use, 7 swap hands, 8 stab.
    PlayerAction {
        /// Vanilla `ServerboundPlayerActionPacket.Action` ordinal (0..=8).
        action: i32,
        /// Target block coordinates representable in vanilla's packed block position.
        pos: [i32; 3],
        /// Vanilla `Direction` ordinal (0..=5).
        direction: i32,
        /// Caller-owned prediction sequence number.
        sequence: i32,
    },
    /// Interact with an entity at a location, using main (0) or off (1) hand.
    /// Vanilla 26.3 uses a separate attack packet; see [`Self::Attack`].
    Interact {
        /// Target entity network ID.
        entity: i32,
        /// Vanilla `InteractionHand` ordinal: 0 main, 1 off.
        hand: i32,
        /// Hit location relative to the entity, encoded with vanilla's `LpVec3`.
        location: [f64; 3],
        /// Whether the player is using secondary action (sneaking).
        sneaking: bool,
    },
    /// The attack form of entity interaction. In 26.3 this carries only the entity ID.
    Attack {
        /// Target entity network ID.
        entity: i32,
    },
    /// Pick a block, optionally requesting its data.
    PickItemFromBlock {
        /// Target block coordinates representable in vanilla's packed block position.
        pos: [i32; 3],
        /// Whether to request block entity data.
        include_data: bool,
    },
    /// Pick an entity, optionally requesting its data.
    PickItemFromEntity {
        /// Target entity network ID.
        entity: i32,
        /// Whether to request entity data.
        include_data: bool,
    },
    /// Update four sign lines. Each line allows at most 384 UTF-16 code units,
    /// as vanilla's Java string codec does. The front/back side follows the lines.
    SignUpdate {
        /// Target sign block coordinates representable in vanilla's packed block position.
        pos: [i32; 3],
        /// Whether the edit applies to the front side (`true`) or back side (`false`).
        front: bool,
        /// Four sign text lines, each at most 384 UTF-16 code units.
        lines: [String; 4],
    },
    /// Report a player position and the ground/collision flags.
    MovePlayerPos {
        /// Player coordinates (NaN is refused).
        pos: [f64; 3],
        /// Whether the player is on the ground.
        on_ground: bool,
        /// Whether the player collided horizontally.
        horizontal_collision: bool,
    },
    /// Report a player position and rotation with ground/collision flags.
    MovePlayerPosRot {
        /// Player coordinates (NaN is refused).
        pos: [f64; 3],
        /// Finite view yaw angle in degrees.
        yaw: f32,
        /// Finite view pitch angle in degrees.
        pitch: f32,
        /// Whether the player is on the ground.
        on_ground: bool,
        /// Whether the player collided horizontally.
        horizontal_collision: bool,
    },
    /// Report finite view angles and ground/collision flags.
    MovePlayerRot {
        /// Finite view yaw angle in degrees.
        yaw: f32,
        /// Finite view pitch angle in degrees.
        pitch: f32,
        /// Whether the player is on the ground.
        on_ground: bool,
        /// Whether the player collided horizontally.
        horizontal_collision: bool,
    },
    /// Report only the ground/collision flags.
    MovePlayerStatusOnly {
        /// Whether the player is on the ground.
        on_ground: bool,
        /// Whether the player collided horizontally.
        horizontal_collision: bool,
    },
}

fn range(packet: &str, field: &str) -> PacketBuildError {
    PacketBuildError::OutOfRange {
        packet: packet.into(),
        field: field.into(),
    }
}

impl ServerboundPlayPacket {
    /// Checks the wire limits for the host-backed container-click builder.
    /// Container and state IDs are signed VarInts; slots are signed shorts,
    /// including vanilla's negative outside-slot sentinel. No menu validity is checked.
    /// This does not construct or send a container click.
    pub fn validate_container_click_limits(
        container: i64,
        state: i64,
        slot: i32,
        changed_slots: u64,
    ) -> Result<(), PacketBuildError> {
        let packet = "container_click";
        i32::try_from(container).map_err(|_| range(packet, "container_id"))?;
        i32::try_from(state).map_err(|_| range(packet, "state_id"))?;
        i16::try_from(slot).map_err(|_| range(packet, "slot"))?;
        if changed_slots > 128 {
            return Err(PacketBuildError::TooManyChangedSlots {
                packet: packet.into(),
                field: "changed_slots".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "packet_builders_tests.rs"]
mod tests;

impl crate::NoBackend for PacketBuildError {
    fn no_backend() -> Self {
        Self::Unavailable {
            packet: "container_click".into(),
            field: "host".into(),
        }
    }
}

/// Plain item values for a host-built container click; no hashed values cross the API.
#[derive(Clone, Debug, Default, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: these are the complete fields of vanilla's container-click request; protocol updates deliberately change this shape.
pub struct ContainerClickRequest {
    /// Signed VarInt container ID. This does not check the currently open menu.
    pub container_id: i64,
    /// Signed VarInt menu state ID.
    pub state_id: i64,
    /// Signed-short slot, including the outside-click sentinel -999.
    pub slot: i32,
    /// Signed-byte button.
    pub button: i32,
    /// Vanilla click kind: 0 pickup, 1 quick move, 2 swap, 3 clone,
    /// 4 throw, 5 quick craft, 6 pickup all.
    pub click_type: i32,
    /// Changed slot indices and their resulting stacks; at most 128 entries.
    pub changed_slots: Vec<(i32, Option<crate::ItemStack>)>,
    /// The resulting carried stack, or empty.
    pub carried: Option<crate::ItemStack>,
}
