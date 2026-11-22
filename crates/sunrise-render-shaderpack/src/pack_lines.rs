//! The block outline drawn by a shader pack's `gbuffers_line` (or `gbuffers_basic`, as in Iris),
//! so packs that style it (Complementary Reimagined, Super Duper Vanilla) draw it their way, and
//! packs that render the world into part of their buffers (SEUS PTGI HRR) move it with the rest.
//! The edges are the outline buffer Sunrise draws itself, one instance per edge, widened to a
//! strip as `outline_clip` in terrain.wgsl does.

/// What `gbuffers_line` reads from an outline edge: its camera-relative ends, ARGB colour and
/// width in pixels, and the width of the stroke drawn over it (`over_layout`). The strip's
/// corners 0 and 1 are at the start, 2 and 3 at the end (six vertices per edge: pack pipelines
/// draw triangle lists), and a vertex's `gl_VertexID % 2` is its corner's side.
///
/// The width is applied in screen space after the pack's transform, across the edge as the
/// pack draws it (a pack may turn it), unless the pack widened the line itself, as vanilla's
/// `rendertype_lines.vsh` does (Complementary Reimagined's `gbuffers_line` too). The game tells
/// by running the pack's vertex stage for each corner of the strip: a pack that only places the
/// line (a transform, a turn, a jitter, the world drawn into part of its buffers) puts both
/// corners of an end at the same place, one that widens it puts them apart across the line, by
/// any visible amount (more than 1/1024 px, a floor for rounding only). Then the stroke keeps the pack's width, and a stroke with one drawn over it (the
/// high contrast outline's black one) is widened across the drawn edge by how much wider than
/// that one the game draws it, so it still shows around it. Each run starts with the pack's
/// globals as the first did. A line program that writes images or buffers isn't run again (its
/// writes would be repeated), and the game widens its line, across the edge as the game's
/// transform puts it.
pub fn inputs() -> sunrise_shaders::GbuffersInputs {
    let declarations =
        "layout(location = 0) in vec3 hz_l_start;\nlayout(location = 1) in vec3 hz_l_end;\n\
         layout(location = 2) in vec4 hz_l_argb;\nlayout(location = 3) in float hz_l_width;\n\
         layout(location = 4) in float hz_l_over;\nint hz_l_c;\n"
            .to_owned();
    // Pack pipelines draw triangle lists: six vertices per edge, the strip's corners 0 1 2 and
    // 3 2 1, so a vertex's `gl_VertexID % 2` is its corner's side, as in vanilla's lines
    // (`rendertype_lines.vsh` offsets a vertex by that parity; Complementary's `gbuffers_line`
    // does the same). Corner `c` is also vertex `c`.
    let body = "    int hz_l_v = int(gl_VertexIndex);\n    \
         hz_l_c = hz_l_v < 4 ? hz_l_v : hz_l_v == 4 ? 2 : 1;\n    \
         hz_Vertex = vec4(hz_l_c >= 2 ? hz_l_end : hz_l_start, 1.0);\n    \
         hz_Color = hz_l_argb.zyxw;\n    \
         hz_Normal = normalize(hz_l_end - hz_l_start);\n    \
         hz_MultiTexCoord0 = vec4(0.0, 0.0, 0.0, 1.0);\n    \
         hz_MultiTexCoord1 = vec4(240.0, 240.0, 0.0, 1.0);\n    \
         hz_MultiTexCoord2 = hz_MultiTexCoord1;\n"
        .to_owned();
    // ARGB in a little-endian u32 reads as B, G, R, A.
    let attributes = [
        ("vaPosition", "hz_Vertex.xyz"),
        ("vaColor", "hz_Color"),
        ("vaNormal", "hz_Normal"),
    ];
    // `hz_l_q`, the strip's four corners where the pack puts them (with the y flip
    // `gl_Position` has here: a gbuffers program draws into the pack's buffers), each run of
    // the pack's vertex stage from the same call, this vertex's last so its outputs are its
    // own; `hz_l_s`, each in pixels. The edge on screen, `hz_l_d`, runs from the middle of the
    // start's two corners to the end's, as the pack drew it (turned, moved, shortened), and
    // `hz_l_n` is its unit perpendicular. `hz_l_a0` and `hz_l_a1` are how far apart across it
    // the pack put the even and the odd corner of the start and of the end: more than 1/1024
    // px at either, and the pack widened the line. The stroke then keeps the pack's width and sides, and the
    // game adds only what this stroke has over the one drawn on it, to the side of this
    // vertex's corner from its partner (none where the pack put an end's two corners at one
    // place: that end stays a point). Otherwise the game widens it to the side of its corner.
    // A pack that only places its line runs the same code for each corner, on the same inputs
    // but a vertex index it doesn't read, so the two corners of an end are one point; the
    // 1/1024 px floor is only for a driver that compiles the four runs apart and rounds them
    // differently (contracting multiplies and adds into FMAs in one copy and not another),
    // which can leave them a millionth of a pixel apart. It is about 500 times below half a
    // pixel and far below any width that shows. A stage that isn't run again (`hz_vertex_reruns`) gets the game's own transform
    // for its corners, and the game's widening.
    // A rendered edge of no length (or not finite) has no direction to widen across; it covers
    // nothing, and `hz_l_d` is any unit vector there.
    let after = "    {\n        \
         int hz_l_k = hz_l_c;\n        \
         vec4 hz_l_q[4];\n        \
         if (hz_vertex_reruns) {\n            \
         for (int hz_l_i = 1; hz_l_i <= 4; hz_l_i++) {\n                \
         int hz_l_j = hz_l_k ^ (hz_l_i & 3);\n                \
         hz_vertex_index = hz_l_i == 4 ? gl_VertexIndex : uint(hz_l_j);\n                \
         hz_vertex();\n                \
         hz_l_q[hz_l_j] = gl_Position;\n            \
         }\n        \
         } else {\n            \
         vec4 hz_l_a = hz_ModelViewProjectionMatrix * vec4(hz_l_start, 1.0);\n            \
         vec4 hz_l_b = hz_ModelViewProjectionMatrix * vec4(hz_l_end, 1.0);\n            \
         hz_l_a.y = -hz_l_a.y;\n            \
         hz_l_b.y = -hz_l_b.y;\n            \
         hz_l_q[0] = hz_l_a;\n            \
         hz_l_q[1] = hz_l_a;\n            \
         hz_l_q[2] = hz_l_b;\n            \
         hz_l_q[3] = hz_l_b;\n        \
         }\n        \
         vec2 hz_l_s[4];\n        \
         for (int hz_l_i = 0; hz_l_i < 4; hz_l_i++) {\n            \
         hz_l_s[hz_l_i] = hz_l_q[hz_l_i].xy / hz_l_q[hz_l_i].w * hz_target.xy * 0.5;\n        \
         }\n        \
         vec2 hz_l_d = (hz_l_s[2] + hz_l_s[3] - hz_l_s[0] - hz_l_s[1]) * 0.5;\n        \
         float hz_l_l = dot(hz_l_d, hz_l_d);\n        \
         hz_l_d = hz_l_l > 0.0 && hz_l_l <= 3.4028235e38 ? hz_l_d * inversesqrt(hz_l_l) : vec2(1.0, 0.0);\n        \
         vec2 hz_l_n = vec2(-hz_l_d.y, hz_l_d.x);\n        \
         float hz_l_a0 = dot(hz_l_s[0] - hz_l_s[1], hz_l_n);\n        \
         float hz_l_a1 = dot(hz_l_s[2] - hz_l_s[3], hz_l_n);\n        \
         bool hz_l_own = abs(hz_l_a0) > 0.0009765625 || abs(hz_l_a1) > 0.0009765625;\n        \
         float hz_l_e = hz_l_k < 2 ? hz_l_a0 : hz_l_a1;\n        \
         float hz_l_even = !hz_l_own ? 1.0 : abs(hz_l_e) > 0.0009765625 ? sign(hz_l_e) : 0.0;\n        \
         float hz_l_side = (hz_l_k & 1) == 0 ? hz_l_even : -hz_l_even;\n        \
         float hz_l_w = hz_l_own ? hz_l_width - hz_l_over : hz_l_width;\n        \
         gl_Position.xy += hz_l_n * (hz_l_side * hz_l_w) / hz_target.xy * gl_Position.w;\n    }\n"
        .to_owned();
    sunrise_shaders::GbuffersInputs {
        declarations,
        body,
        attributes: attributes
            .iter()
            .map(|&(n, v)| (n.to_owned(), v.to_owned()))
            .collect(),
        after,
        alpha_test: None,
        parts: None,
        item_id_declared: false,
        far_depth: false,
        paged_maps: false,
        water_mask: false,
        water_patches: false,
    }
}

/// The outline edges as the pack's lines program reads them: one 32-byte instance per edge.
pub fn layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4, 3 => Float32];
    wgpu::VertexBufferLayout {
        array_stride: 32,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

/// The width of the stroke drawn over each edge (`hz_l_over`): the outline buffer bound a
/// second time, from the next stroke's first edge for a high contrast outline's black stroke,
/// from its own otherwise, so that an edge with no stroke over it reads its own width.
pub fn over_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 28,
        shader_location: 4,
    }];
    wgpu::VertexBufferLayout {
        array_stride: 32,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}
