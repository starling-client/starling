//! Starling's part of manifest.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// `[ray_tracing]`: the pack uses hardware ray queries (`rt_trace`, `rt_occluded`
/// in the material and post stages) against the world's BLAS/TLAS. Without a
/// ray-query device the pack is refused with its name in the log.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RayTracing {}

/// `[ray_tracing]`, unchecked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawRayTracing {}
