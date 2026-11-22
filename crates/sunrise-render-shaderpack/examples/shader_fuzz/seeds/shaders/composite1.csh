#version 430 compatibility

layout(local_size_x = 8, local_size_y = 8) in;
const ivec3 workGroups = ivec3(4, 4, 1);

layout(std430, binding = 0) buffer Counts {
    uint total;
    uint bins[16];
} counts;

layout(rgba8) uniform image2D colorimg0;

void main() {
    ivec2 p = ivec2(gl_GlobalInvocationID.xy);
    vec4 c = imageLoad(colorimg0, p);
    atomicAdd(counts.bins[int(c.r * 15.0)], 1u);
}
