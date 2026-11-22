#version 120

#include "/lib/common.glsl"

uniform sampler2D colortex0, colortex2;
uniform float viewWidth, viewHeight;
uniform float fzPulse;
varying vec2 texcoord;

float total[2], extra = 0.0;

void main() {
    vec3 color = texture2D(colortex0, texcoord).rgb;
    for (int i = 0; i < FZ_STEPS; i++) {
        total[0] += weights[i];
    }
    Light l = Light(tint, float[2](1.0, fzPulse));
    color = mix(color, lit(l), 0.1 * total[0]) + shade * extra;
#if MC_VERSION >= 11300
    color *= texture2D(colortex2, texcoord + vec2(1.0 / viewWidth, 1.0 / viewHeight)).r;
#else
    color *= 0.5;
#endif
/* RENDERTARGETS: 0 */
    gl_FragData[0] = vec4(color, luma(color));
}
