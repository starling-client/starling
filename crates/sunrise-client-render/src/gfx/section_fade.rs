//! How far through vanilla's chunk fade-in a section is, and the instance that carries it to `terrain.wgsl`.
use super::SectionKey;
use crate::fast_hash::FxHashMap;
use std::{cmp::Reverse, collections::BinaryHeap, time::Instant};

pub(super) use sunrise_render_core::section_instance;
pub use sunrise_render_core::{CHUNK_VISIBILITY_THRESHOLD, chunk_visibility, fade_millis};
/// `chunk_visibility` at `now` of a section first uploaded at `uploaded`
/// (`Renderer::section_uploaded`; none: one that fades in over far terrain, or
/// whose column left), with the chunk fade-in `fade_ms`.
pub(super) fn section_visibility(uploaded: Option<&Instant>, fade_ms: u64, now: Instant) -> f32 {
    uploaded.map_or(1.0, |&t| {
        let elapsed = now.saturating_duration_since(t).as_millis() as u64;
        chunk_visibility(elapsed, fade_ms)
    })
}
/// `LevelRenderer.isSectionCompiledAndVisible` for one section: it has its mesh
/// (`compiled`), and `getVisibility` from its first upload `uploaded` (none: whole, or
/// fading in over far terrain) has reached `CHUNK_VISIBILITY_THRESHOLD`.
pub(super) fn compiled_and_visible(
    compiled: bool,
    uploaded: Option<&Instant>,
    fade_ms: u64,
    now: Instant,
) -> bool {
    compiled && section_visibility(uploaded, fade_ms, now) >= CHUNK_VISIBILITY_THRESHOLD
}
/// What a section's fade depends on besides the section itself
/// (`SectionGpu::drawn_faded`): the time, the chunk fade-in time, the sections'
/// first uploads (`Renderer::section_uploaded`) and those held while far
/// terrain comes in (`Renderer::lod_retiring`).
#[derive(Clone, Copy)]
pub(super) struct FadeInputs<'a> {
    pub(super) now: Instant,
    pub(super) fade_ms: u64,
    pub(super) uploaded: &'a FxHashMap<super::SectionKey, Instant>,
    pub(super) retiring: &'a crate::fast_hash::FxHashSet<super::SectionKey>,
}
/// The flagged sections by when their fade is over at the latest (gfx.rs
/// `SectionGpu::fade_end`), for gfx.rs `mark_fades` to look at them again
/// then, and the chunk fade-in time their flags were decided with: `None`
/// until a frame has decided them, just after GPU culling was switched on.
/// The ends of sections that went or were handed back are skipped as they come.
#[derive(Default)]
pub(crate) struct Fades {
    due: BinaryHeap<Reverse<(Instant, SectionKey)>>,
    pub fade_ms: Option<u64>,
}

impl Fades {
    /// The section at `key` is looked at again at `end`.
    pub fn push(&mut self, end: Instant, key: SectionKey) {
        self.due.push(Reverse((end, key)));
    }
    /// The section of the earliest end at or before `now`, taken out.
    pub fn pop_due(&mut self, now: Instant) -> Option<SectionKey> {
        let &Reverse((end, key)) = self.due.peek()?;
        if end > now {
            return None;
        }
        self.due.pop();
        Some(key)
    }
    /// Every end goes; `fade_ms` stays.
    pub fn clear(&mut self) {
        self.due.clear();
    }
    /// The sections an end is waited for.
    #[cfg(debug_assertions)]
    pub fn pending(&self) -> impl Iterator<Item = SectionKey> {
        self.due.iter().map(|r| r.0.1)
    }
}
