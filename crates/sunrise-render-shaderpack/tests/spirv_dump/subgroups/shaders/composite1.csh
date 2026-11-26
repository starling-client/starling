#version 430 compatibility

#ifdef MC_GL_KHR_shader_subgroup_arithmetic
#extension GL_KHR_shader_subgroup_arithmetic : enable
#endif

layout(local_size_x = 64) in;
const ivec3 workGroups = ivec3(1, 1, 1);

layout(rgba8) uniform image2D colorimg1;

void main() {
    ivec2 p = ivec2(gl_GlobalInvocationID.xy);
    float v = imageLoad(colorimg1, p).r;
#ifdef MC_GL_KHR_shader_subgroup_arithmetic
    v = subgroupAdd(v) / 64.0;
#else
    v *= 0.5;
#endif
    imageStore(colorimg1, p, vec4(v));
}
