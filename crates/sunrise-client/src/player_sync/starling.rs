//! Starling's part of player_sync.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl PositionSender {
    /// The position the server was last told (the baseline of the next move).
    pub fn last_sent(&self) -> [f64; 3] {
        self.position
    }

    /// The server took a look from another packet (a teleport's acknowledgement, the view's look
    /// before a hit): the next update sends its own look if it is not that one, even when the
    /// baseline has it. With Free Direction steering the look sent is not the view's, and an
    /// acknowledgement with the view's look would otherwise stand for every move after it.
    pub fn told_rotation(&mut self, rotation: (f32, f32)) {
        self.told = Some(rotation);
    }
}
