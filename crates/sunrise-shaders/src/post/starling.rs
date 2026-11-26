//! Starling's part of post.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// `gbuffers_damagedblock` and its fallbacks (2.3, the spec's table: it falls back to
/// `gbuffers_terrain`, as in Iris). Without the chain a pack with no program of its own, Starling
/// Visuals among them, drew no breaking cracks at all.
pub(super) const DAMAGED_BLOCK: [&str; 5] = [
    "gbuffers_damagedblock",
    "gbuffers_terrain",
    "gbuffers_textured_lit",
    "gbuffers_textured",
    "gbuffers_basic",
];

/// A program that did not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// The program (`gbuffers_water`, `composite3`, `composite3_a` for a compute stage).
    pub name: String,
    /// Its errors, as `file:line: message`.
    pub reasons: Vec<String>,
}

/// Records that program `name` did not load, with its `reasons`.
pub(super) fn skip(post: &mut Post, name: &str, reasons: Vec<String>) {
    post.notes.extend(reasons.iter().cloned());
    post.notes.push(format!("{name}: skipped"));
    post.skipped.push(Skipped {
        name: name.to_owned(),
        reasons,
    });
}

/// `f` with a panic inside it turned into an error: a pack is not the game's own code, and a
/// program that makes the translator panic loses that program, not the pack.
pub(super) fn contained<T>(
    file: &str,
    f: impl FnOnce() -> Result<T, Vec<crate::Error>>,
) -> Result<T, Vec<crate::Error>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|payload| {
        let text = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("unknown");
        Err(vec![crate::Error {
            file: file.to_owned(),
            line: 0,
            message: format!("internal error (translator panicked): {text}"),
        }])
    })
}
