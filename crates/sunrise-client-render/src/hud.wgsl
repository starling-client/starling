struct Hud {
    size: vec4<f32>,
    // Blend::EndPortal: atlas rects (u0, v0, width, height) of end_sky.png and
    // end_portal.png, then vanilla's GameTime in x and PvP & HUD's glint colour in yzw
    // (y below 0: vanilla's).
    end_portal: array<vec4<f32>, 3>,
}
@group(0) @binding(0) var<uniform> hud: Hud;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
// enchanted_glint_item on its own repeating texture (vanilla's GlintSampler).
@group(0) @binding(3) var glint_tex: texture_2d<f32>;
@group(0) @binding(4) var glint_sampler: sampler;
@group(0) @binding(5) var linear_sampler: sampler;
// enchanted_glint_armor, the armour layers' GlintSampler (entity.wgsl's glint_armor_tex).
@group(0) @binding(6) var glint_armor_tex: texture_2d<f32>;
// The frame's 16x16 lightmap, sampled when a HUD item vertex carries explicit packed light.
@group(0) @binding(7) var lightmap: texture_2d<f32>;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    // projection_from_position(gl_Position): the End portal's texProj0.
    @location(2) proj: vec4<f32>,
    @location(3) glint_uv: vec2<f32>,
    // Packed explicit light flags & byte lanes:
    // bit 24: sample lightmap (bits 0..8 = block, bits 8..16 = sky);
    // bit 25: explicit item alpha cutoff 0.1 (`ITEM_CUTOUT` / `ITEM_TRANSLUCENT`).
    @location(4) @interpolate(flat) light: u32,
}
@vertex fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>, @location(3) depth: f32, @location(4) glint_uv: vec2<f32>, @location(5) light: u32) -> Out {
    let pixels = pos * hud.size.z;
    var out: Out;
    out.clip = vec4(pixels.x / hud.size.x * 2.0 - 1.0, 1.0 - pixels.y / hud.size.y * 2.0, depth, 1.0);
    out.uv = uv;
    out.color = color;
    out.proj = vec4(out.clip.xy * 0.5 + 0.5, out.clip.zw);
    out.glint_uv = glint_uv;
    out.light = light;
    return out;
}
@fragment fn fs_main(in: Out) -> @location(0) vec4<f32> {
    // u >= 2: the GLINT_TEXTURING sample over the item, masked by the item's
    // texture alpha (item.fsh's ALPHA_CUTOUT and GLINT path). u >= 4: the same
    // over a GUI model's armour, from enchanted_glint_armor (ARMOR_ENTITY_GLINT).
    if in.uv.x >= 2.0 {
        let armor = in.uv.x >= 4.0;
        let offset = select(2.0, 4.0, armor);
        let mask = textureSample(atlas, atlas_sampler, in.uv - vec2(offset, 0.0)).a;
        let item_sample = textureSample(glint_tex, glint_sampler, in.glint_uv);
        let armor_sample = textureSample(glint_armor_tex, glint_sampler, in.glint_uv);
        if mask < 0.1 { discard; }
        var glint = hud.size.w * select(item_sample, armor_sample, armor);
        let tint = hud.end_portal[2].yzw;
        if tint.x >= 0.0 {
            glint = vec4(max(glint.r, max(glint.g, glint.b)) * tint, glint.a);
        }
        // The vertex colour multiplies the glint as it does the item under it: white (every
        // glint the game draws) leaves it; a mod's tinted item (`GuiCanvas::item_tinted`) fades
        // and colours it with the item.
        return vec4(glint.rgb * glint.rgb * in.color.rgb * in.color.a, mask * in.color.a);
    }
    return textured(in);
}
// The bound texture, tinted: what `fs_main` draws below its glint markers.
fn textured(in: Out) -> vec4<f32> {
    var c = textureSample(atlas, atlas_sampler, in.uv) * in.color;
    let cutoff = select(0.004, 0.1, (in.light & 0x02000000u) != 0u);
    if c.a < cutoff { discard; }
    if (in.light & 0x01000000u) != 0u {
        let block_l = f32(in.light & 0xFFu);
        let sky_l = f32((in.light >> 8u) & 0xFFu);
        let lm_uv = clamp(vec2(block_l, sky_l) / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0));
        let lm = textureSampleLevel(lightmap, linear_sampler, lm_uv, 0.0);
        c = vec4(c.rgb * lm.rgb, c.a);
    }
    return c;
}
// A standalone picture (hud/pictures.rs) with its own texture and sampler bound as `atlas`:
// every UV is the picture's, unclamped (`innerBlit`), so a repeating picture at u >= 2 is not
// read as a glint marker.
@fragment fn fs_picture(in: Out) -> @location(0) vec4<f32> {
    return textured(in);
}
// OptiFine CIT glint layers (hud/cit_glint.rs): every rule's texture in one array, the slot in
// the vertex colour's red and the layer's intensity in its alpha. Masked by the item's texture
// as the vanilla glint is, then blended by the pipeline the rule's `blend` asks for.
struct CitSlot {
    rgba: vec4<f32>,
    // x: linear filtering (`blur`).
    blur: vec4<f32>,
}
struct CitSlots {
    slots: array<CitSlot, 256>,
}
// The glint array is bound twice, once per sampler: naga's GLSL writer pairs each image with one
// sampler, so one image sampled through both cannot run on OpenGL (#2386).
@group(1) @binding(0) var cit_tex: texture_2d_array<f32>;
@group(1) @binding(1) var cit_blur: sampler;
@group(1) @binding(2) var cit_nearest: sampler;
@group(1) @binding(3) var<uniform> cit: CitSlots;
@group(1) @binding(4) var cit_tex_sharp: texture_2d_array<f32>;
@fragment fn fs_cit_glint(in: Out) -> @location(0) vec4<f32> {
    let mask = textureSample(atlas, atlas_sampler, in.uv).a;
    let index = u32(round(in.color.r * 255.0));
    let slot = cit.slots[index];
    let blurred = textureSampleLevel(cit_tex, cit_blur, in.glint_uv, index, 0.0);
    let sharp = textureSampleLevel(cit_tex_sharp, cit_nearest, in.glint_uv, index, 0.0);
    if mask < 0.1 { discard; }
    // The layer's colour times its intensity (the glint strength option included) in alpha,
    // cut where vanilla's glint shader cuts, which CIT Resewn draws through.
    var t = select(sharp, blurred, slot.blur.x > 0.5) * slot.rgba;
    t.a *= in.color.a;
    if t.a < 0.1 { discard; }
    return t;
}
@fragment fn fs_crosshair(in: Out) -> @location(0) vec4<f32> {
    let c = textureSample(atlas, atlas_sampler, in.uv) * in.color;
    if c.a == 0.0 { discard; }
    return c;
}
// RenderPipelines.VIGNETTE (WinScreen's credits vignette): bilinear sample of
// credits_vignette.png (which has blur: true in its .mcmeta and a wrapped border in the atlas).
@fragment fn fs_vignette(in: Out) -> @location(0) vec4<f32> {
    let c = textureSample(atlas, linear_sampler, in.uv) * in.color;
    return vec4(c.rgb, 1.0);
}
// rendertype_end_portal.fsh (entity.wgsl has the same for portal blocks).
const PORTAL_COLORS = array<vec3<f32>, 16>(
    vec3(0.022087, 0.098399, 0.110818),
    vec3(0.011892, 0.095924, 0.089485),
    vec3(0.027636, 0.101689, 0.100326),
    vec3(0.046564, 0.109883, 0.114838),
    vec3(0.064901, 0.117696, 0.097189),
    vec3(0.063761, 0.086895, 0.123646),
    vec3(0.084817, 0.111994, 0.166380),
    vec3(0.097489, 0.154120, 0.091064),
    vec3(0.106152, 0.131144, 0.195191),
    vec3(0.097721, 0.110188, 0.187229),
    vec3(0.133516, 0.138278, 0.148582),
    vec3(0.070006, 0.243332, 0.235792),
    vec3(0.196766, 0.142899, 0.214696),
    vec3(0.047281, 0.315338, 0.321970),
    vec3(0.204675, 0.390010, 0.302066),
    vec3(0.080955, 0.314821, 0.661491),
);
fn portal_sample(rect: vec4<f32>, p: vec4<f32>) -> vec3<f32> {
    let st = fract(p.xy / p.w);
    return textureSampleLevel(atlas, atlas_sampler, rect.xy + st * rect.zw, 0.0).rgb;
}
fn portal_layer(p: vec4<f32>, layer: f32) -> vec4<f32> {
    let a = radians((layer * layer * 4321.0 + layer * 9.0) * 2.0);
    let c = cos(a);
    let s = sin(a);
    let k = (4.5 - layer / 4.0) * 2.0;
    var v = vec4(k * (c * p.x - s * p.y), k * (s * p.x + c * p.y), p.z, p.w);
    v = vec4(v.x + 17.0 / layer * v.w, v.y + (2.0 + layer / 1.5) * (hud.end_portal[2].x * 1.5) * v.w, v.z, v.w);
    return vec4(0.5 * v.x + 0.25 * v.w, 0.5 * v.y + 0.25 * v.w, v.z, v.w);
}
// RenderPipelines.END_PORTAL (WinScreen's poem background): end_sky and 15
// layers of end_portal.png.
@fragment fn fs_end_portal(in: Out) -> @location(0) vec4<f32> {
    var c = portal_sample(hud.end_portal[0], in.proj) * PORTAL_COLORS[0];
    for (var i = 0; i < 15; i++) {
        c += portal_sample(hud.end_portal[1], portal_layer(in.proj, f32(i + 1))) * PORTAL_COLORS[i];
    }
    return vec4(c, 1.0);
}
