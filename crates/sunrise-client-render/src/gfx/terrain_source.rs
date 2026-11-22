// Copyright (c) 2026 Ludovic Marie and the Horizon contributors.
// SPDX-License-Identifier: MIT
// Adapted from Vasysik/sunrise-android; see THIRD_PARTY_NOTICES.md and LICENSE.
//! Legacy GLES lowering. Vulkan retains the original WGSL byte for byte.
use std::borrow::Cow;

pub(super) fn terrain_shader(gl: bool) -> Cow<'static, str> {
    let source = include_str!("../terrain.wgsl");
    if gl {
        Cow::Owned(gles_source(source))
    } else {
        Cow::Borrowed(source)
    }
}

/// `source` (the whole terrain shader, as `gfx::terrain_shader` composes it) as the backend takes
/// it: lowered for OpenGL ES, else as it is.
pub(super) fn for_backend(source: String, gl: bool) -> String {
    if gl { gles_source(&source) } else { source }
}

/// GLES 3.0 cannot declare arrays of arrays. Two adjacent vec4 entries
/// retain the original uniform byte layout and every colored-light value.
pub(super) fn gles_uniforms(source: &str) -> String {
    source
        .replace("array<array<vec4<f32>, 2>, 16>", "array<vec4<f32>, 32>")
        .replace(
            "globals.colored_lights[i][0]",
            "globals.colored_lights[2u * i]",
        )
        .replace(
            "globals.colored_lights[i][1]",
            "globals.colored_lights[2u * i + 1u]",
        )
}

fn gles_source(source: &str) -> String {
    let source = gles_uniforms(source).replace("\r\n", "\n");
    let source = lower_emissive_lookup(source);
    let start = source
        .find("fn sprite_grad(")
        .expect("terrain sprite function");
    let level = source
        .find("fn sprite_level(")
        .expect("terrain level function");
    let end = level + source[level..].find("\n}\n").expect("terrain level end") + 3;
    // The rotated grid's taps: the include unrolls them.
    let rgss = source
        .find("fn rotated_grid(")
        .expect("terrain rotated grid function");
    let rgss_end = rgss
        + source[rgss..]
            .find("\n}\n")
            .expect("terrain rotated grid end")
        + 3;
    let mut output = String::with_capacity(source.len());
    output.push_str(&source[..start]);
    output.push_str(include_str!("terrain_gles.wgsl.inc"));
    output.push_str(&source[end..rgss]);
    output.push_str(&source[rgss_end..]);
    // Adreno 304 rendered broken terrain with the separated scalar varyings: terrain.wgsl
    // itself now packs them (`TerrainWire`, its integers flat in one vector), so they stay.
    output
}

// GLES drivers can take tens of seconds to link the unrolled uniform search.
// Use a sprite-indexed texture with the same first-match and sentinel values.
fn lower_emissive_lookup(mut source: String) -> String {
    let start = source
        .find("    for (var i = 0u; i < min(globals.emissive_count.x, 128u); i++) {")
        .expect("terrain emissive lookup");
    let body = source[start..].find('{').unwrap() + start;
    let mut depth = 1;
    let mut end = body + 1;
    while depth != 0 {
        match source.as_bytes()[end] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        end += 1;
    }
    source.replace_range(
        start..end,
        "    if globals.emissive_count.x > 0u {
        let index = vec2<i32>(i32(sprite_layer & 255u), i32(sprite_layer >> 8u));
        out.emissive_layer = textureLoad(emissive_overlays, index, 0).r;
    }",
    );
    let at = source.find("fn sprite_grad(").expect("sprite function");
    source.insert_str(
        at,
        "@group(0) @binding(13) var emissive_overlays: texture_2d<u32>;\n",
    );
    source
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vulkan_source_is_unchanged_and_gles_validates_every_entrypoint() {
        assert!(matches!(terrain_shader(false), Cow::Borrowed(_)));
        assert_eq!(terrain_shader(false), include_str!("../terrain.wgsl"));
        let source = terrain_shader(true);
        let original = include_str!("../terrain.wgsl").replace("\r\n", "\n");
        assert_eq!(
            &source[source.find("struct SkyOut {").unwrap()..],
            &original[original.find("struct SkyOut {").unwrap()..],
            "sky and outline shader bodies must remain unchanged"
        );
        assert!(!source.contains("array<array<"));
        assert!(!source.contains("switch layer / SPRITE_SPLIT"));
        assert!(!source.contains("uv + offsets[i] * pixel"));
        for index in 0..7 {
            assert!(source.contains(&format!("array_index == {index}u")));
        }
        assert!(source.contains("fn rotated_grid("));
        assert!(!source.contains("emissive_layers[i]"));
        // The renderer puts the HDR prelude (`HDR_WRITE_LIGHT`) in front, as gfx.rs does.
        let module =
            naga::front::wgsl::parse_str(&(include_str!("../hdr_light.wgsl").to_owned() + &source))
                .unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let vertex = module
            .entry_points
            .iter()
            .find(|e| e.name == "vs_terrain")
            .unwrap();
        let naga::TypeInner::Struct { members, .. } =
            &module.types[vertex.function.result.as_ref().unwrap().ty].inner
        else {
            panic!("terrain result must be a struct")
        };
        assert_eq!(
            members
                .iter()
                .filter(|m| matches!(m.binding, Some(naga::Binding::Location { .. })))
                .count(),
            5
        );
    }

    #[test]
    fn windows_line_endings_produce_the_same_gles_shader() {
        let source = include_str!("../terrain.wgsl").replace("\r\n", "\n");
        assert_eq!(
            gles_source(&source),
            gles_source(&source.replace('\n', "\r\n"))
        );
    }
}
