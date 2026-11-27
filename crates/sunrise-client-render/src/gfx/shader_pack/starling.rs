//! Starling's part of shader_pack.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Renderer {
    /// Whether the packs loaded from now on draw terrain's depth before shading it.
    pub fn set_pack_prepass(&self, on: bool) {
        pack_post::set_prepass(on);
    }

    /// No shader pack, and none loaded or being made for any dimension: the next frame loads
    /// the chosen one.
    pub fn forget_shader_pack(&mut self) {
        let _ = self.set_shader_pack(0, None, &[]);
        self.pack_post = None;
    }

    /// Diagnostic distributions from the existing pass timers, without another profiler.
    pub fn diagnostic_pass_times(&self) -> serde_json::Value {
        let shader = match &self.pack_post {
            Some((_, Some(post))) => post.timing.as_ref(),
            _ => None,
        };
        serde_json::json!({"world":self.own_timing.as_ref().map(pack_timing::PassTiming::distributions),
            "shader":shader.map(pack_timing::PassTiming::distributions)})
    }
}
