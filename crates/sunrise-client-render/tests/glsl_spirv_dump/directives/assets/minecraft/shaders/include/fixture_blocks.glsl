// The fixture's own declarations of vanilla's blocks, guarded as an include is.
#ifndef FIXTURE_BLOCKS
#define FIXTURE_BLOCKS

layout(std140) uniform DynamicTransforms {
    mat4 ModelViewMat;
    vec4 ColorModulator;
    vec3 ModelOffset;
    mat4 TextureMat;
};

layout(std140) uniform Projection {
    mat4 ProjMat;
};

// A macro this file defines governs the file that includes it.
#define FIXTURE_TINT_STEPS 3
#if defined(IS_GUI) || defined IS_SEE_THROUGH
#define FIXTURE_FLAT
#endif

#endif
