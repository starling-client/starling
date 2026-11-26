#version 430 compatibility

#include "/lib/settings.glsl"

layout(local_size_x = 8, local_size_y = 8) in;
const vec2 workGroupsRender = vec2(1.0, 1.0);

layout(rgba8) uniform image2D colorimg2;

void main() {
    ivec2 p = ivec2(gl_GlobalInvocationID.xy);
#ifdef EXTRA_PASS
    vec4 sum = vec4(0.0);
    for (int i = 0; i < BLUR_STEPS; i++) {
        sum += imageLoad(colorimg2, p + ivec2(i, 0));
    }
    imageStore(colorimg2, p, sum / float(BLUR_STEPS));
#endif
}
