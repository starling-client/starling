// Copyright (c) 2026 Ludovic Marie and the Horizon contributors.
// SPDX-License-Identifier: MIT
// Adapted from Vasysik/horizon-android; see THIRD_PARTY_NOTICES.md and LICENSE.
// Legacy GLES sprite selection; included into terrain.wgsl, not a standalone shader.
fn sprite_grad(uv: vec2<f32>, layer: u32, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    let l = i32(layer % SPRITE_SPLIT);
    let array_index = layer / SPRITE_SPLIT;
    if SPRITE_ARRAYS == 1u { return textureSampleGrad(sprites, sprite_sampler, uv, l, dx, dy); }
    if array_index == 0u { return textureSampleGrad(sprites, sprite_sampler, uv, l, dx, dy); }
    if array_index == 1u { return textureSampleGrad(sprites_1, sprite_sampler, uv, l, dx, dy); }
    if array_index == 2u { return textureSampleGrad(sprites_2, sprite_sampler, uv, l, dx, dy); }
    if array_index == 3u { return textureSampleGrad(sprites_3, sprite_sampler, uv, l, dx, dy); }
    if array_index == 4u { return textureSampleGrad(sprites_4, sprite_sampler, uv, l, dx, dy); }
    if array_index == 5u { return textureSampleGrad(sprites_5, sprite_sampler, uv, l, dx, dy); }
    if array_index == 6u { return textureSampleGrad(sprites_6, sprite_sampler, uv, l, dx, dy); }
    return textureSampleGrad(sprites_7, sprite_sampler, uv, l, dx, dy);
}
fn sprite_level(uv: vec2<f32>, layer: u32, lod: f32) -> vec4<f32> {
    let l = i32(layer % SPRITE_SPLIT);
    let array_index = layer / SPRITE_SPLIT;
    if SPRITE_ARRAYS == 1u { return textureSampleLevel(sprites, sprite_sampler, uv, l, lod); }
    if array_index == 0u { return textureSampleLevel(sprites, sprite_sampler, uv, l, lod); }
    if array_index == 1u { return textureSampleLevel(sprites_1, sprite_sampler, uv, l, lod); }
    if array_index == 2u { return textureSampleLevel(sprites_2, sprite_sampler, uv, l, lod); }
    if array_index == 3u { return textureSampleLevel(sprites_3, sprite_sampler, uv, l, lod); }
    if array_index == 4u { return textureSampleLevel(sprites_4, sprite_sampler, uv, l, lod); }
    if array_index == 5u { return textureSampleLevel(sprites_5, sprite_sampler, uv, l, lod); }
    if array_index == 6u { return textureSampleLevel(sprites_6, sprite_sampler, uv, l, lod); }
    return textureSampleLevel(sprites_7, sprite_sampler, uv, l, lod);
}

// Adreno 304 V@269 crashes while linking a locally indexed RGSS offset array.
// The four original samples and their accumulation order are unchanged.
fn rotated_grid(uv: vec2<f32>, layer: u32, pixel: vec2<f32>, du: vec2<f32>,
                dv: vec2<f32>) -> vec4<f32> {
    let min_pixel = min(pixel.x, pixel.y);
    let du_len = length(du);
    let dv_len = length(dv);
    let effective = sqrt(min(du_len, dv_len) * max(du_len, dv_len));
    let exact = max(0.0, log2(effective / min_pixel));
    let single = globals.filtering.z;
    if single > 0.0 && floor(exact) >= single {
        return sprite_level(uv, layer, floor(exact));
    }
    var sum = vec4(0.0);
    sum += sprite_level(uv + vec2<f32>(0.125, 0.375) * pixel, layer, exact);
    sum += sprite_level(uv + vec2<f32>(-0.125, -0.375) * pixel, layer, exact);
    sum += sprite_level(uv + vec2<f32>(0.375, -0.125) * pixel, layer, exact);
    sum += sprite_level(uv + vec2<f32>(-0.375, 0.125) * pixel, layer, exact);
    return sum * 0.25;
}
