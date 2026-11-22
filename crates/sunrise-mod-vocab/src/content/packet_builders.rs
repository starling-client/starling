//! Portable vanilla play packet builders. Protocol is deliberately a test-only dependency.
use crate::{Identifier, ServerboundPlayPacket};
use sunrise_core::vanilla_bytes::Writer;

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
    /// A position, cursor coordinate or angle is not finite.
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
    /// No client connection supplies the packet's registry context.
    Unavailable {
        /// Vanilla packet name.
        packet: String,
        /// Missing context.
        field: String,
    },
}

fn range(packet: &str, field: &str) -> PacketBuildError {
    PacketBuildError::OutOfRange {
        packet: packet.into(),
        field: field.into(),
    }
}
fn ordinal(packet: &str, field: &str, value: i32, max: i32) -> Result<(), PacketBuildError> {
    if (0..=max).contains(&value) {
        Ok(())
    } else {
        Err(PacketBuildError::UnknownAction {
            packet: packet.into(),
            field: field.into(),
        })
    }
}
fn finite(
    packet: &str,
    field: &str,
    values: impl IntoIterator<Item = f64>,
) -> Result<(), PacketBuildError> {
    if values.into_iter().all(f64::is_finite) {
        Ok(())
    } else {
        Err(PacketBuildError::NonFinite {
            packet: packet.into(),
            field: field.into(),
        })
    }
}
fn position(packet: &str, pos: [i32; 3], w: &mut Writer) -> Result<(), PacketBuildError> {
    let [x, y, z] = pos;
    if !(-33_554_432..=33_554_431).contains(&x)
        || !(-33_554_432..=33_554_431).contains(&z)
        || !(-2048..=2047).contains(&y)
    {
        return Err(range(packet, "position"));
    }
    w.i64(sunrise_core::position::BlockPos::new(x, y, z).pack());
    Ok(())
}
fn build(
    packet: &str,
    write: impl FnOnce(&mut Writer) -> Result<(), PacketBuildError>,
) -> Result<ServerboundPlayPacket, PacketBuildError> {
    let mut w = Writer::new();
    write(&mut w)?;
    // Every field is fixed-size or bounded before writing (including sign lines).
    if w.data.len() > 8192 {
        return Err(range(packet, "body"));
    }
    let id = Identifier::from_parts("minecraft", packet).map_err(|_| range(packet, "id"))?;
    Ok(ServerboundPlayPacket::new(id, w.data))
}
fn flags(ground: bool, collision: bool) -> u8 {
    u8::from(ground) | (u8::from(collision) << 1)
}

// Vec3.LP_STREAM_CODEC, also implemented by sunrise-protocol::buf::LpVec3.
// Inputs have already been checked for finiteness. The vanilla codec clamps large values.
fn lp_vec3(pos: [f64; 3], w: &mut Writer) {
    let [x, y, z] = pos.map(|v| v.clamp(-1.717_986_918_3e10, 1.717_986_918_3e10));
    let length = x.abs().max(y.abs()).max(z.abs());
    if length < 3.051_944_088_384_301e-5 {
        w.u8(0);
        return;
    }
    let scale = length.ceil() as u64;
    let partial = scale & 3 != scale;
    let markers = if partial { scale & 3 | 4 } else { scale };
    let pack = |v: f64| ((v / scale as f64 * 0.5 + 0.5) * 32766.0).round() as u64;
    let buffer = markers | pack(x) << 3 | pack(y) << 18 | pack(z) << 33;
    w.u8(buffer as u8);
    w.u8((buffer >> 8) as u8);
    w.i32((buffer >> 16) as i32);
    if partial {
        w.varint((scale >> 2) as i32);
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
    /// Use the main (0) or off (1) hand, with the caller's sequence and view angles.
    pub fn use_item(
        hand: i32,
        sequence: i32,
        yaw: f32,
        pitch: f32,
    ) -> Result<ServerboundPlayPacket, PacketBuildError> {
        let p = "use_item";
        ordinal(p, "hand", hand, 1)?;
        finite(p, "yaw", [f64::from(yaw)])?;
        finite(p, "pitch", [f64::from(pitch)])?;
        build(p, |w| {
            w.varint(hand);
            w.varint(sequence);
            w.f32(yaw);
            w.f32(pitch);
            Ok(())
        })
    }
    /// Use a hand on a block. Face uses vanilla's 0..=5 direction order;
    /// cursor is relative to the block. Sequence numbers remain caller-owned.
    pub fn use_item_on(
        hand: i32,
        pos: [i32; 3],
        face: i32,
        cursor: [f32; 3],
        inside: bool,
        world_border: bool,
        sequence: i32,
    ) -> Result<ServerboundPlayPacket, PacketBuildError> {
        let p = "use_item_on";
        ordinal(p, "hand", hand, 1)?;
        ordinal(p, "face", face, 5)?;
        finite(p, "cursor", cursor.map(f64::from))?;
        build(p, |w| {
            w.varint(hand);
            position(p, pos, w)?;
            w.varint(face);
            for v in cursor {
                w.f32(v);
            }
            w.bool(inside);
            w.bool(world_border);
            w.varint(sequence);
            Ok(())
        })
    }
    /// Player action: 0 start destroy, 1 change direction, 2 abort, 3 stop,
    /// 4 drop all, 5 drop one, 6 release use, 7 swap hands, 8 stab.
    pub fn player_action(
        action: i32,
        pos: [i32; 3],
        direction: i32,
        sequence: i32,
    ) -> Result<ServerboundPlayPacket, PacketBuildError> {
        let p = "player_action";
        ordinal(p, "action", action, 8)?;
        ordinal(p, "direction", direction, 5)?;
        build(p, |w| {
            w.varint(action);
            position(p, pos, w)?;
            w.u8(direction as u8);
            w.varint(sequence);
            Ok(())
        })
    }
    /// Interact with an entity at a location, using main (0) or off (1) hand.
    /// Vanilla 26.3 uses a separate attack packet; see [`Self::attack`].
    pub fn interact(
        entity: i32,
        hand: i32,
        location: [f64; 3],
        sneaking: bool,
    ) -> Result<ServerboundPlayPacket, PacketBuildError> {
        let p = "interact";
        ordinal(p, "hand", hand, 1)?;
        finite(p, "location", location)?;
        build(p, |w| {
            w.varint(entity);
            w.varint(hand);
            lp_vec3(location, w);
            w.bool(sneaking);
            Ok(())
        })
    }
    /// The attack form of entity interaction. In 26.3 this carries only the entity ID.
    pub fn attack(entity: i32) -> Result<ServerboundPlayPacket, PacketBuildError> {
        build("attack", |w| {
            w.varint(entity);
            Ok(())
        })
    }
    /// Pick a block, optionally requesting its data.
    pub fn pick_item_from_block(
        pos: [i32; 3],
        include_data: bool,
    ) -> Result<ServerboundPlayPacket, PacketBuildError> {
        build("pick_item_from_block", |w| {
            position("pick_item_from_block", pos, w)?;
            w.bool(include_data);
            Ok(())
        })
    }
    /// Pick an entity, optionally requesting its data.
    pub fn pick_item_from_entity(
        entity: i32,
        include_data: bool,
    ) -> Result<ServerboundPlayPacket, PacketBuildError> {
        build("pick_item_from_entity", |w| {
            w.varint(entity);
            w.bool(include_data);
            Ok(())
        })
    }
    /// Update four sign lines. Each line allows at most 384 UTF-16 code units,
    /// as vanilla's Java string codec does. The front/back side follows the lines.
    pub fn sign_update(
        pos: [i32; 3],
        front: bool,
        lines: [String; 4],
    ) -> Result<ServerboundPlayPacket, PacketBuildError> {
        let p = "sign_update";
        for (i, line) in lines.iter().enumerate() {
            if line.encode_utf16().take(385).count() > 384 {
                return Err(PacketBuildError::SignLineTooLong {
                    packet: p.into(),
                    field: format!("lines[{i}]"),
                });
            }
        }
        build(p, |w| {
            position(p, pos, w)?;
            for line in &lines {
                // The validated 384 UTF-16 units imply at most 384 * 3 UTF-8 bytes.
                w.string(line);
            }
            w.varint(i32::from(front));
            Ok(())
        })
    }
    /// Report a finite position and the ground/collision flags.
    pub fn move_player_pos(
        pos: [f64; 3],
        on_ground: bool,
        horizontal_collision: bool,
    ) -> Result<ServerboundPlayPacket, PacketBuildError> {
        let p = "move_player_pos";
        finite(p, "position", pos)?;
        build(p, |w| {
            for v in pos {
                w.f64(v);
            }
            w.u8(flags(on_ground, horizontal_collision));
            Ok(())
        })
    }
    /// Report a finite position and rotation with ground/collision flags.
    pub fn move_player_pos_rot(
        pos: [f64; 3],
        yaw: f32,
        pitch: f32,
        on_ground: bool,
        horizontal_collision: bool,
    ) -> Result<ServerboundPlayPacket, PacketBuildError> {
        let p = "move_player_pos_rot";
        finite(p, "position", pos)?;
        finite(p, "yaw", [f64::from(yaw)])?;
        finite(p, "pitch", [f64::from(pitch)])?;
        build(p, |w| {
            for v in pos {
                w.f64(v);
            }
            w.f32(yaw);
            w.f32(pitch);
            w.u8(flags(on_ground, horizontal_collision));
            Ok(())
        })
    }
    /// Report finite view angles and ground/collision flags.
    pub fn move_player_rot(
        yaw: f32,
        pitch: f32,
        on_ground: bool,
        horizontal_collision: bool,
    ) -> Result<ServerboundPlayPacket, PacketBuildError> {
        let p = "move_player_rot";
        finite(p, "yaw", [f64::from(yaw)])?;
        finite(p, "pitch", [f64::from(pitch)])?;
        build(p, |w| {
            w.f32(yaw);
            w.f32(pitch);
            w.u8(flags(on_ground, horizontal_collision));
            Ok(())
        })
    }
    /// Report only the ground/collision flags.
    pub fn move_player_status_only(
        on_ground: bool,
        horizontal_collision: bool,
    ) -> Result<ServerboundPlayPacket, PacketBuildError> {
        build("move_player_status_only", |w| {
            w.u8(flags(on_ground, horizontal_collision));
            Ok(())
        })
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
