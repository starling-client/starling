//! Sunrise-written, interface-only GLSL stubs for `minecraft:*` shader includes and standard
//! vanilla vertex shaders.
//!
//! Used only when prepared vanilla resources are absent (such as `cargo sunrise-mod check` in CI
//! or the registry release check): each file declares only the std140 uniform block layouts and
//! public function/attribute interfaces that Sunrise's renderer binds, with trivial stub bodies.
//! Nothing is copied from Mojang's files; the game's resource load remains authoritative.

const GLOBALS_GLSL: &str = "\
#ifndef SUNRISE_STUB_GLOBALS_GLSL
#define SUNRISE_STUB_GLOBALS_GLSL
layout(std140) uniform Globals {
    ivec3 CameraBlockPos;
    float GlintAlpha;
    vec3 CameraOffset;
    float GameTime;
    vec2 ScreenSize;
    int MenuBlurRadius;
    int UseRgss;
};
#endif
";

const PROJECTION_GLSL: &str = "\
#ifndef SUNRISE_STUB_PROJECTION_GLSL
#define SUNRISE_STUB_PROJECTION_GLSL
layout(std140) uniform Projection {
    mat4 ProjMat;
};
vec4 projection_from_position(vec4 position) {
    return position;
}
#endif
";

const DYNAMICTRANSFORMS_GLSL: &str = "\
#ifndef SUNRISE_STUB_DYNAMICTRANSFORMS_GLSL
#define SUNRISE_STUB_DYNAMICTRANSFORMS_GLSL
layout(std140) uniform DynamicTransforms {
    mat4 ModelViewMat;
    mat4 TextureMat;
    vec4 ColorModulator;
    vec3 ModelOffset;
};
#endif
";

const FOG_GLSL: &str = "\
#ifndef SUNRISE_STUB_FOG_GLSL
#define SUNRISE_STUB_FOG_GLSL
layout(std140) uniform Fog {
    vec4 FogColor;
    float FogEnvironmentalStart;
    float FogEnvironmentalEnd;
    float FogRenderDistanceStart;
    float FogRenderDistanceEnd;
    float FogSkyEnd;
    float FogCloudsEnd;
};
float linear_fog_value(float vertexDistance, float fogStart, float fogEnd) {
    return 0.0;
}
float total_fog_value(
    float sphericalVertexDistance,
    float cylindricalVertexDistance,
    float environmentalStart,
    float environmentalEnd,
    float renderDistanceStart,
    float renderDistanceEnd
) {
    return 0.0;
}
vec4 apply_fog(
    vec4 inColor,
    float sphericalVertexDistance,
    float cylindricalVertexDistance,
    float environmentalStart,
    float environmentalEnd,
    float renderDistanceStart,
    float renderDistanceEnd,
    vec4 fogColor
) {
    return inColor;
}
float fog_spherical_distance(vec3 pos) {
    return 0.0;
}
float fog_cylindrical_distance(vec3 pos) {
    return 0.0;
}
#endif
";

const MATRIX_GLSL: &str = "\
#ifndef SUNRISE_STUB_MATRIX_GLSL
#define SUNRISE_STUB_MATRIX_GLSL
mat2 mat2_rotate_z(float radians) {
    return mat2(1.0);
}
#endif
";

const OIT_GLSL: &str = "\
#ifndef SUNRISE_STUB_OIT_GLSL
#define SUNRISE_STUB_OIT_GLSL
#if defined(OIT) && defined(OIT_DEPTH_BOUNDS)
layout(location = 0) out vec4 sunriseOitDepthBounds;
void executeAlphaOnlyPhase(float deviceDepth, float alpha) {
    if (alpha < 0.01) {
        discard;
    }
    sunriseOitDepthBounds = vec4(-deviceDepth, deviceDepth, deviceDepth, alpha);
}
#elif defined(OIT) && defined(OIT_TRANSMITTANCE)
uniform sampler2D DepthBoundsSampler;
layout(location = 0) out vec4 sunriseOitCoeff0;
layout(location = 1) out vec4 sunriseOitCoeff1;
void executeAlphaOnlyPhase(float deviceDepth, float alpha) {
#ifdef OIT_ADDITIVE
    discard;
#else
    vec4 b = texelFetch(DepthBoundsSampler, ivec2(gl_FragCoord.xy), 0);
    sunriseOitCoeff0 = vec4(alpha, deviceDepth, b.r, b.g);
    sunriseOitCoeff1 = vec4(0.0);
#endif
}
#elif defined(OIT) && defined(OIT_ACCUMULATE)
uniform sampler2D DepthBoundsSampler;
uniform sampler2D Coeff0;
uniform sampler2D Coeff1;
vec4 sampleColorForAccumulation(vec4 color) {
    ivec2 p = ivec2(gl_FragCoord.xy);
    float d = texelFetch(DepthBoundsSampler, p, 0).r;
    float c0 = texelFetch(Coeff0, p, 0).r;
    float c1 = texelFetch(Coeff1, p, 0).r;
    float t = clamp(exp(-max(c0 + c1 + d * 0.0, 0.0)), 0.0001, 1.0);
#ifdef OIT_ADDITIVE
    return vec4(color.rgb * color.a, 0.0) * t;
#else
    return vec4(color.rgb * color.a, color.a) * t;
#endif
}
#else
void executeAlphaOnlyPhase(float deviceDepth, float alpha) {}
vec4 sampleColorForAccumulation(vec4 color) {
    return color;
}
#endif
#endif
";

const POSITION_TEX_COLOR_VSH: &str = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec2 texCoord0;
out vec4 vertexColor;
void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    texCoord0 = UV0;
    vertexColor = Color;
}
";

const POSITION_COLOR_VSH: &str = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>
in vec3 Position;
in vec4 Color;
out vec4 vertexColor;
void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    vertexColor = Color;
}
";

const POSITION_TEX_VSH: &str = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>
in vec3 Position;
in vec2 UV0;
out vec2 texCoord0;
void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    texCoord0 = UV0;
}
";

const POSITION_VSH: &str = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>
#include <minecraft:fog.glsl>
in vec3 Position;
out float sphericalVertexDistance;
out float cylindricalVertexDistance;
void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    sphericalVertexDistance = fog_spherical_distance(Position);
    cylindricalVertexDistance = fog_cylindrical_distance(Position);
}
";

/// Look up a Sunrise-written interface stub by resource id (`minecraft:shaders/include/...` or
/// `minecraft:shaders/core/position*.vsh`).
pub fn read(id: &str) -> Option<&'static str> {
    match id {
        "minecraft:shaders/include/globals.glsl" => Some(GLOBALS_GLSL),
        "minecraft:shaders/include/projection.glsl" => Some(PROJECTION_GLSL),
        "minecraft:shaders/include/dynamictransforms.glsl" => Some(DYNAMICTRANSFORMS_GLSL),
        "minecraft:shaders/include/fog.glsl" => Some(FOG_GLSL),
        "minecraft:shaders/include/matrix.glsl" => Some(MATRIX_GLSL),
        "minecraft:shaders/include/oit.glsl" => Some(OIT_GLSL),
        "minecraft:shaders/core/position_tex_color.vsh" => Some(POSITION_TEX_COLOR_VSH),
        "minecraft:shaders/core/position_color.vsh" => Some(POSITION_COLOR_VSH),
        "minecraft:shaders/core/position_tex.vsh" => Some(POSITION_TEX_VSH),
        "minecraft:shaders/core/position.vsh" => Some(POSITION_VSH),
        _ => None,
    }
}
