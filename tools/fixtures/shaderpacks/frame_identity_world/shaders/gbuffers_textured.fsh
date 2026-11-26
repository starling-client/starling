#version 120

uniform sampler2D texture;
uniform int renderStage;

varying vec2 texcoord;
varying vec4 glcolor;

void main() {
    vec4 color = texture2D(texture, texcoord) * glcolor;
    // The sun red, the moon green: both are the pack's own.
    if (renderStage == MC_RENDER_STAGE_SUN) color.gb *= 0.3;
    if (renderStage == MC_RENDER_STAGE_MOON) color.rb *= 0.3;
    /* DRAWBUFFERS:0 */
    gl_FragData[0] = color;
}
