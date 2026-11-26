#version 330
#extension GL_ARB_separate_shader_objects : require

#include <minecraft:fixture_blocks.glsl>

#if FIXTURE_TINT_STEPS * 2 > 5 && 0x10 == 16 && 010 == 8
#include <minecraft:fixture_math.glsl>
#elif UNDEFINED_KNOB
#include <minecraft:fixture_never.glsl>
#else
#include <minecraft:fixture_never.glsl>
#endif

layout(location = 0) in vec3 Position;
layout(location = 1) in vec4 Color;
layout(location = 2) in vec2 UV0;

layout(location = 2) out vec4 vertexColor;
layout(location = 3) out vec2 texCoord0;

#define OFFSET_Y 0.25
#undef OFFSET_Y
#define OFFSET_Y 0.125

void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position + ModelOffset, 1.0);
    gl_Position.y += OFFSET_Y;
#ifdef FIXTURE_FLAT
    #if UNDEFINED_KNOB == 0 && !defined(UNDEFINED_KNOB)
    vertexColor = fixture_tint(Color,
        FIXTURE_HALF);
    #else
    vertexColor = Color;
    #endif
#elif defined(IS_GRAYSCALE)
    vertexColor = Color.rrra;
#else
    vertexColor = fixture_tint(Color, float(__LINE__) / 100.0);
#endif
    texCoord0 = UV0;
}
