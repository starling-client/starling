#version 120

uniform int renderStage;

varying vec4 glcolor;

void main() {
    vec4 color = glcolor;
    // The sky disc a warm orange, the stars their own cyan, the void dark red.
    if (renderStage == MC_RENDER_STAGE_SKY || renderStage == MC_RENDER_STAGE_SUNSET) color.rgb *= vec3(1.3, 0.9, 0.6);
    if (renderStage == MC_RENDER_STAGE_STARS) color.rgb = vec3(0.0, 1.0, 1.0);
    if (renderStage == MC_RENDER_STAGE_VOID) color.rgb = vec3(0.3, 0.0, 0.0);
    /* DRAWBUFFERS:0 */
    gl_FragData[0] = color;
}
