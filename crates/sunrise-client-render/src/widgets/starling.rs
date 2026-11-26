//! Starling's part of widgets.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Move the targets registered since `mark` `dx` GUI pixels right, with what drew them: a
/// screen laid out clear of a camera cutout's strip, then moved in.
pub fn shift_targets(mark: usize, dx: f32) {
    if dx == 0.0 {
        return;
    }
    TARGETS.with_borrow_mut(|(current, _, _)| {
        for target in current.iter_mut().skip(mark) {
            target.rect[0] += dx;
        }
    });
}
