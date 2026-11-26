/// The bend the `pose` row stores on a part's `Poses` and the `bend` row draws: the lower half
/// of `right_arm` turned about X at the elbow by `angle` radians. A type of the mod's own, so no
/// other mod's data on the part is read for it.
pub struct Bend {
    pub angle: f32,
}
