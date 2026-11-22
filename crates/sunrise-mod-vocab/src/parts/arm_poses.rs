/// Client humanoid pose registrations.
pub trait ArmPosesRegistrar {
    /// Register a pose once in `Hooks::register`, keeping the returned token for
    /// `get_arm_pose`. `name` must be namespaced in a namespace owned by the registering
    /// mod (its manifest namespaces or a content package it serves via `served_by`).
    /// Duplicate or malformed names refuse registration. The flags independently
    /// select two-arm ordering and whether the opposite hand's pose gives way.
    fn register_arm_pose(
        &mut self,
        name: &str,
        uses_both_arms: bool,
        affects_offhand: bool,
    ) -> crate::CustomArmPose;
}
