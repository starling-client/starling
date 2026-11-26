#version 120

uniform sampler2D texture;
uniform sampler2D lightmap;
uniform sampler2D normals;
uniform int renderStage;

varying vec2 texcoord;
varying vec2 lmcoord;

void main() {
    vec4 albedo = texture2D(texture, texcoord);
    if (albedo.a < 0.1) discard;
    // The known value: every terrain pixel is this colour times the lightmap's, whatever the block's texture,
    // so a terrain draw that doesn't reach this program, or reaches it with another light, shows.
    vec3 known = vec3(0.25, 0.5, 0.75);
    vec3 light = texture2D(lightmap, lmcoord).rgb;
    // The LabPBR height in the normal map's alpha (1 where a block has no map) dims the low parts of the relief.
    float height = texture2D(normals, texcoord).a;
    // Translucent terrain (water, stained glass) falls back here too: half transparent.
    float alpha = renderStage == MC_RENDER_STAGE_TERRAIN_TRANSLUCENT ? 0.5 : 1.0;
    /* DRAWBUFFERS:0 */
    gl_FragData[0] = vec4(known * light * mix(0.5, 1.0, height), alpha);
}
