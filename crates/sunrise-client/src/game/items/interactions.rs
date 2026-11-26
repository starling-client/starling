//! Server `minecraft:interaction` entities (`Interaction`): invisible boxes a
//! server (Denizen, Nexo) uses for clickable areas. They are pickable, render
//! nothing, and an interact ends at them: `SUCCESS` when they respond (the arm
//! swings), `CONSUME` otherwise (the click still goes to the server, nothing
//! behind them is used).
use super::Game;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_protocol::entity_data::Value;

/// `Interaction`'s synced data and box (`DATA_WIDTH`, `DATA_HEIGHT`, `DATA_RESPONSE`).
pub(super) struct RemoteInteraction {
    pub join: sunrise_client::join_seq::JoinSeq,
    pub position: Vec3,
    pub previous: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub previous_yaw: f32,
    pub previous_pitch: f32,
    pub width: f32,
    pub height: f32,
    pub dimensions_refreshed: bool,
    pub response: bool,
    /// `Entity.getVehicle`: ModelEngine's hitboxes ride the pivot that
    /// carries them.
    pub vehicle: Option<u32>,
}

impl RemoteInteraction {
    pub fn new(position: [f64; 3]) -> Self {
        Self::new_with_rotation(position, 0.0, 0.0)
    }
    pub fn new_with_rotation(position: [f64; 3], yaw: f32, pitch: f32) -> Self {
        let pos = Vec3::new(position[0], position[1], position[2]);
        let yaw = if yaw.is_finite() { yaw } else { 0.0 };
        let pitch = if pitch.is_finite() {
            (pitch % 360.0).clamp(-90.0, 90.0)
        } else {
            0.0
        };
        Self {
            join: sunrise_client::join_seq::JoinSeq::next(),
            position: pos,
            previous: pos,
            yaw,
            pitch,
            previous_yaw: yaw,
            previous_pitch: pitch,
            width: 1.0,
            height: 1.0,
            dimensions_refreshed: false,
            response: false,
            vehicle: None,
        }
    }
    /// Unsynced spawns keep EntityType's 0x0 eye height until DATA_POSE/WIDTH/HEIGHT triggers refreshDimensions.
    pub fn eye_height(&self) -> f32 {
        if self.dimensions_refreshed {
            self.height * 0.85
        } else {
            0.0
        }
    }
    /// Unsynced spawns keep EntityType's 0x0 dimensions until refreshDimensions runs.
    #[cfg(test)]
    pub fn seat_height(&self) -> f64 {
        if self.dimensions_refreshed {
            f64::from(self.height)
        } else {
            0.0
        }
    }
    pub fn snapshot_previous(&mut self) {
        self.previous = self.position;
        self.previous_yaw = self.yaw;
        self.previous_pitch = self.pitch;
    }
    pub fn set_rot(&mut self, yaw: f32, pitch: f32) {
        if yaw.is_finite() {
            self.yaw = yaw % 360.0;
        }
        if pitch.is_finite() {
            self.pitch = (pitch % 360.0).clamp(-90.0, 90.0);
        }
    }
    pub fn snap_to(&mut self, position: Vec3, yaw: f32, pitch: f32) {
        self.position = position;
        if yaw.is_finite() {
            self.yaw = yaw;
        }
        if pitch.is_finite() {
            self.pitch = (pitch % 360.0).clamp(-90.0, 90.0);
        }
        self.snapshot_previous();
    }
    /// `Entity.positionRider`'s move: a passenger is placed on its vehicle's
    /// seat, not moved on its own.
    pub fn ride_to(&mut self, at: glam::DVec3) {
        self.position = Vec3::new(at.x, at.y, at.z);
    }
    /// `Interaction.refreshDimensions`: `width` square about the entity's
    /// x and z, from its feet up `height` (its position is the box's bottom
    /// centre, not the model's).
    pub fn bounds(&self) -> Aabb {
        let half = f64::from(self.width) / 2.0;
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - half, p.y, p.z - half),
            Vec3::new(p.x + half, p.y + f64::from(self.height), p.z + half),
        )
    }
    pub fn metadata(&mut self, values: &[(u8, i32, Value)]) {
        for (index, _, value) in values {
            match (index, value) {
                (6, _) => self.dimensions_refreshed = true,
                (8, Value::Float(width)) => {
                    self.width = *width;
                    self.dimensions_refreshed = true;
                }
                (9, Value::Float(height)) => {
                    self.height = *height;
                    self.dimensions_refreshed = true;
                }
                (10, Value::Bool(response)) => self.response = *response,
                _ => {}
            }
        }
    }
}

impl Game<'_> {
    /// An `Interaction` keeps `Entity`'s no-op interpolation handler: the
    /// server's position replaces its own at once.
    pub(super) fn move_remote_interaction(&mut self, id: i32, position: [f64; 3]) {
        if let Some(i) = self.interactions.get_mut(&id) {
            i.position = Vec3::new(position[0], position[1], position[2]);
            self.pick_section_move(id as u32, position);
        }
    }
    pub(super) fn remote_interaction_data(&mut self, id: i32, values: &[(u8, i32, Value)]) -> bool {
        let Some(i) = self.interactions.get_mut(&id) else {
            return false;
        };
        i.metadata(values);
        true
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    #[test]
    fn the_box_is_centred_on_the_feet_and_grows_with_the_data() {
        let mut i = RemoteInteraction::new([10.0, 64.0, -3.0]);
        let b = i.bounds();
        assert_eq!(
            (b.min_position().components(), b.max_position().components()),
            ([9.5, 64.0, -3.5], [10.5, 65.0, -2.5])
        );
        i.metadata(&[
            (8, 0, Value::Float(2.2)),
            (9, 0, Value::Float(2.2)),
            (10, 0, Value::Bool(true)),
        ]);
        let b = i.bounds();
        let half = f64::from(2.2f32) / 2.0;
        let height = f64::from(2.2f32);
        let (lo, hi) = (b.min_position().components(), b.max_position().components());
        for (got, want) in lo.iter().zip([10.0 - half, 64.0, -3.0 - half]) {
            assert!((got - want).abs() < 1e-6, "{lo:?}");
        }
        for (got, want) in hi.iter().zip([10.0 + half, 64.0 + height, -3.0 + half]) {
            assert!((got - want).abs() < 1e-6, "{hi:?}");
        }
        assert!(i.response);
    }
    #[test]
    fn a_riding_hitbox_moves_with_its_pivot() {
        let mut i = RemoteInteraction::new([10.0, 64.0, -3.0]);
        i.vehicle = Some(9);
        i.ride_to(glam::DVec3::new(12.0, 65.5, -3.0));
        assert_eq!(i.position, Vec3::new(12.0, 65.5, -3.0));
        let b = i.bounds();
        assert_eq!(
            (b.min_position().components(), b.max_position().components()),
            ([11.5, 65.5, -3.5], [12.5, 66.5, -2.5])
        );
    }

    fn parity_witness() -> serde_json::Value {
        let path = std::env::var("SUNRISE_CAMERA_PARITY_WITNESS")
            .expect("run through tools/test_camera_parity.py, which records the witness");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "failed to read SUNRISE_CAMERA_PARITY_WITNESS={path} (run through tools/test_camera_parity.py): {err}"
            )
        });
        serde_json::from_str(&raw).expect("valid 26.3 camera parity witness")
    }

    fn hex_f64(v: &serde_json::Value) -> u64 {
        u64::from_str_radix(v.as_str().unwrap(), 16).unwrap()
    }

    #[test]
    #[ignore = "run by tools/test_camera_parity.py"]
    fn interaction_unsynced_default_sync_and_pose_sync_seat_height_26_3() {
        run_interaction_unsynced_default_sync_and_pose_sync_seat_height_26_3();
    }

    pub(in crate::game) fn run_interaction_unsynced_default_sync_and_pose_sync_seat_height_26_3() {
        let witness = parity_witness();
        let life = &witness["interaction_lifecycle"];

        let mut i = RemoteInteraction::new([0.5, 300.0, 0.5]);
        assert_eq!(
            i.seat_height().to_bits(),
            hex_f64(&life["unsynced_spawn"]["seat_height_bits"]),
            "unsynced interaction seat height"
        );

        // Explicit default (1.0, 1.0) triggers refreshDimensions() in vanilla 26.3.
        i.metadata(&[(8, 3, Value::Float(1.0)), (9, 3, Value::Float(1.0))]);
        assert_eq!(
            i.seat_height().to_bits(),
            hex_f64(&life["sync_default_1x1"]["seat_height_bits"]),
            "explicit 1x1 sync must refresh seat height to 1.0"
        );

        // DATA_POSE (6) alone also triggers refreshDimensions() on an unsynced Interaction.
        let mut pose_only = RemoteInteraction::new([0.5, 300.0, 0.5]);
        pose_only.metadata(&[(6, 21, Value::Id(0))]);
        assert_eq!(
            pose_only.seat_height().to_bits(),
            hex_f64(&life["sync_pose_only"]["seat_height_bits"]),
            "DATA_POSE sync must refresh seat height to 1.0"
        );
    }

    #[test]
    #[ignore = "run by tools/test_camera_parity.py"]
    fn interaction_negative_height_bounds_invert_downward_26_3() {
        run_interaction_negative_height_bounds_invert_downward_26_3();
    }

    pub(in crate::game) fn run_interaction_negative_height_bounds_invert_downward_26_3() {
        let witness = parity_witness();
        let life = &witness["interaction_lifecycle"];
        for (case_name, width, height) in [
            ("sync_negative_height", 0.5f32, -1.5f32),
            ("sync_negative_width_and_height", -0.4f32, -2.0f32),
        ] {
            let mut i = RemoteInteraction::new([10.0, 64.0, -3.0]);
            i.metadata(&[(8, 3, Value::Float(width)), (9, 3, Value::Float(height))]);
            let b = i.bounds();
            let got = [
                b.min_position().x.to_bits(),
                b.min_position().y.to_bits(),
                b.min_position().z.to_bits(),
                b.max_position().x.to_bits(),
                b.max_position().y.to_bits(),
                b.max_position().z.to_bits(),
            ];
            let want: Vec<u64> = life[case_name]["aabb_bits"]
                .as_array()
                .unwrap()
                .iter()
                .map(hex_f64)
                .collect();
            assert_eq!(got.as_slice(), want.as_slice(), "{case_name}");
        }
    }
}

/// Whether an entity id names `minecraft:interaction`.
pub(super) fn is_interaction(kind: &str) -> bool {
    matches!(kind, "minecraft:interaction" | "interaction")
}

/// `Interaction.PlayerAction`: the player and the game time of the last
/// attack or use, as `PlayerAction.CODEC` saves it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PlayerAction {
    pub player: [i32; 4],
    pub timestamp: i64,
}

impl PlayerAction {
    fn from_tag(tag: Option<&sunrise_core::nbt::Tag>) -> Option<Self> {
        use sunrise_core::nbt::Tag;
        let tag = tag?;
        let Tag::IntArray(player) = tag.get("player")? else {
            return None;
        };
        let timestamp = sunrise_client::save::number(tag.get("timestamp"))?;
        Some(Self {
            player: player.as_slice().try_into().ok()?,
            timestamp,
        })
    }
    fn to_tag(self) -> sunrise_core::nbt::Tag {
        use sunrise_core::nbt::Tag;
        Tag::compound([
            ("player", Tag::IntArray(self.player.to_vec())),
            ("timestamp", Tag::Long(self.timestamp)),
        ])
    }
}

/// A server `Interaction`'s saved settings
/// (`Interaction.readAdditionalSaveData` / `addAdditionalSaveData`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct InteractionState {
    pub width: f32,
    pub height: f32,
    pub response: bool,
    pub attack: Option<PlayerAction>,
    pub interaction: Option<PlayerAction>,
}

impl InteractionState {
    /// `Interaction.readAdditionalSaveData`: the dimensions, optional action records and response.
    pub fn from_tag(tag: &sunrise_core::nbt::Tag) -> Self {
        let float = |key| sunrise_client::save::float(tag.get(key)).unwrap_or(1.0) as f32;
        Self {
            width: float("width"),
            height: float("height"),
            response: sunrise_client::entity_nbt::flag(tag, "response"),
            attack: PlayerAction::from_tag(tag.get("attack")),
            interaction: PlayerAction::from_tag(tag.get("interaction")),
        }
    }
    /// `Interaction.addAdditionalSaveData`: the settings, and each action when set.
    pub fn write(&self, c: &mut sunrise_client::entity_nbt::EntityTagWriter) {
        use sunrise_core::nbt::Tag;
        c.insert("width".into(), Tag::Float(self.width.to_bits()));
        c.insert("height".into(), Tag::Float(self.height.to_bits()));
        for (key, action) in [("attack", self.attack), ("interaction", self.interaction)] {
            match action {
                Some(action) => {
                    c.insert(key.into(), action.to_tag());
                }
                None => {
                    c.remove(&key.into());
                }
            }
        }
        c.insert("response".into(), Tag::Byte(i8::from(self.response)));
    }
}
