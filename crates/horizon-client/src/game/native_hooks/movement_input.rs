use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Local player movement input, after reading it and before applying it.
    pub(in crate::game) fn native_movement_input(
        &self,
        forward: &mut f32,
        strafe: &mut f32,
        jump: &mut bool,
        sneak: &mut bool,
    ) {
        self.fire_movement_input(forward, strafe, jump, sneak);
    }
}
