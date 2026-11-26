fn desired_pose(value: Pose, player: i32, flags: PoseFlags) -> Pose {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = (player, flags.sleeping);
    SEEN.call_once(|| fnr_lower::note("RETURN pose::desired_pose"));
    value
}
