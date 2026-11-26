#version 120

uniform sampler2D texture;
uniform sampler2D lightmap;
uniform int renderStage;

varying vec2 texcoord;
varying vec2 lmcoord;

void main() {
    vec4 color = texture2D(texture, texcoord);
    if (color.a < 0.1) discard;
    color.rgb *= texture2D(lightmap, lmcoord).rgb;
    // Water and stained glass, the translucent stage, lean blue.
    if (renderStage == MC_RENDER_STAGE_TERRAIN_TRANSLUCENT) color.rgb *= vec3(0.6, 0.8, 1.2);
    /* DRAWBUFFERS:0 */
    gl_FragData[0] = color;
}
