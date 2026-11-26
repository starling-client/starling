//! Starling's part of scan.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// The core profile's names for the matrices of [`UNIFORM_STATE`], with the type a program
/// declares them with, and what each reads: the same as the compatibility name in every kind of
/// program (the frame's, the shadow camera's, a full-screen pass's). `chunkOffset` is not here:
/// our vertices are already relative to the camera, so its zero is right.
pub(in super::super) const CORE_MATRICES: [(&str, &str, &str); 6] = [
    ("modelViewMatrix", "mat4", "hz_ModelViewMatrix"),
    (
        "modelViewMatrixInverse",
        "mat4",
        "hz_ModelViewMatrixInverse",
    ),
    ("projectionMatrix", "mat4", "hz_ProjectionMatrix"),
    (
        "projectionMatrixInverse",
        "mat4",
        "hz_ProjectionMatrixInverse",
    ),
    ("normalMatrix", "mat3", "hz_NormalMatrix"),
    ("textureMatrix", "mat4", "hz_TextureMatrix[0]"),
];

/// The largest literal `N` of `x.a < N) discard` (`<=`, `{ discard`) in `tokens`.
pub(in super::super) fn alpha_cut(tokens: &[&str]) -> Option<f32> {
    let t: Vec<&str> = tokens
        .iter()
        .copied()
        .filter(|t| !t.trim().is_empty())
        .collect();
    let cut = |end: usize| {
        let before = &t[..end];
        let before = before.strip_suffix(&["{"]).unwrap_or(before);
        let (number, before) = before.strip_suffix(&[")"])?.split_last()?;
        let before = before.strip_suffix(&["="]).unwrap_or(before);
        let before = before.strip_suffix(&["<"])?;
        let alpha = before.ends_with(&[".", "a"]) || before.ends_with(&[".", "w"]);
        alpha.then(|| number.trim_end_matches(['f', 'F']).parse::<f32>().ok())?
    };
    (0..t.len())
        .filter(|&i| t[i] == "discard")
        .filter_map(cut)
        .reduce(f32::max)
}
