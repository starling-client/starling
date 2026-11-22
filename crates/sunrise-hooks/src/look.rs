//! Per-seat visual knobs a crate sets and the engine reads (docs/layers.md, "Look").
use crate::seat;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

/// What the engine draws differently. `Default` is vanilla's.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    /// The player's name tag shows in the camera's own view.
    pub camera_name_tag: bool,
    /// The chat's look.
    pub chat: Chat,
}

/// The chat's knobs.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chat {
    /// New lines slide in from the left (vanilla pops them in).
    pub slide_in: bool,
    /// The chat's trust indicator beside a line is not drawn.
    pub hide_indicator: bool,
}

static LOOKS: Mutex<Vec<Look>> = Mutex::new(Vec::new());
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Changes the current seat's look ([`crate::seat`]); crates share it, each keeping to its fields.
pub fn update(change: impl FnOnce(&mut Look)) {
    let seat = seat();
    let mut looks = LOOKS.lock().unwrap_or_else(PoisonError::into_inner);
    if looks.len() <= seat {
        looks.resize(seat + 1, Look::default());
    }
    change(&mut looks[seat]);
    GENERATION.fetch_add(1, Ordering::Release);
}

/// The engine's side. Only the engine calls it.
pub mod engine {
    use super::*;

    /// Changes whenever any look does; one atomic load.
    pub fn generation() -> u64 {
        GENERATION.load(Ordering::Acquire)
    }

    /// Player `seat`'s look: vanilla's when nothing changed it.
    pub fn get(seat: usize) -> Look {
        let looks = LOOKS.lock().unwrap_or_else(PoisonError::into_inner);
        looks.get(seat).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_writes_only_the_current_seat() {
        let before = engine::generation();
        crate::engine::set_seat(3);
        update(|look| look.chat.slide_in = true);
        crate::engine::set_seat(0);
        assert!(engine::generation() > before);
        assert!(engine::get(3).chat.slide_in);
        assert_eq!(engine::get(0), Look::default());
        assert_eq!(engine::get(2), Look::default());
        assert_eq!(engine::get(9), Look::default());
    }
}
