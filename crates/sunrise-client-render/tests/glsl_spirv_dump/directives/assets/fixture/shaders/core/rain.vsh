#version 330
#moj_import <minecraft:fixture_blocks.glsl>
#moj_import <polytone:gpu_particle.glsl>
#define RAIN_LEVEL 2
#moj_import "../include/rain_common.glsl"

in vec3 Position;
in vec4 Params;
in vec4 Color;
out vec4 vertexColor;

void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position + Origin, 1.0);
#if defined(RAIN_HEAVY) && !defined(UNDEFINED_KNOB)
    vertexColor = RAIN_FADE(Color, Params);
#elif UNDEFINED_KNOB == 0
    vertexColor = Color;
#endif
}
