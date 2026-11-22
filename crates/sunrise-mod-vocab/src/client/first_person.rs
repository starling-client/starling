//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// `InteractionHand`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Hand {
    Main,
    Off,
}
impl Hand {
    pub const BOTH: [Hand; 2] = [Hand::Main, Hand::Off];
}

/// The posed first-person draw context passed to [`crate::RenderHandEvent`].
pub trait HandDrawContext: BlockEntityDrawContext {
    /// Appends the local avatar's physical left or right arm in its rest pose with the vanilla
    /// outward roll, using the frame's skin, model width and that side's sleeve setting. The
    /// current draw pose and fixed light apply, not the default hand's item, swing or equip pose.
    /// Either arm can be submitted on either hand callback. This does not fire `render_arm`,
    /// change draw state or cancel the default. If the avatar arm is unavailable, draws nothing.
    fn render_arm(&mut self, arm: MainHand);
}
