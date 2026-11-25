// Map quads carry page and texel coordinates in a reserved range of U.
@group(0) @binding(8) var map_page_0: texture_2d<f32>;
@group(0) @binding(9) var map_page_1: texture_2d<f32>;
@group(0) @binding(10) var map_page_2: texture_2d<f32>;
@group(0) @binding(11) var map_page_3: texture_2d<f32>;

fn separate_map_texel(uv: vec2<f32>) -> vec4<f32> {
    let at = vec2<i32>(uv + vec2(16384.0, 0.0));
    let texel = vec2(at.x % 1024, at.y);
    // Missing pages bind a zero-initialized 1x1 texture.
    switch at.x / 1024 {
        case 0: { if textureDimensions(map_page_0).x == 1024u { return textureLoad(map_page_0, texel, 0); } }
        case 1: { if textureDimensions(map_page_1).x == 1024u { return textureLoad(map_page_1, texel, 0); } }
        case 2: { if textureDimensions(map_page_2).x == 1024u { return textureLoad(map_page_2, texel, 0); } }
        case 3: { if textureDimensions(map_page_3).x == 1024u { return textureLoad(map_page_3, texel, 0); } }
        default: {}
    }
    return vec4(0.0);
}
