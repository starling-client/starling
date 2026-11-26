#version 330

#include <minecraft:fixture_blocks.glsl>

uniform sampler2D Sampler0;

layout(location = 2) in vec4 vertexColor;
layout(location = 3) in vec2 texCoord0;

#ifndef OIT_ALPHA_ONLY
layout(location = 0) out vec4 fragColor;
#endif

#define CUTOUT(alpha) \
    if ((alpha) < 0.1) { \
        discard; \
    }

#line 200
const float line_after = float(__LINE__);

void main() {
    vec4 color = texture(Sampler0, texCoord0) * vertexColor * ColorModulator;
    CUTOUT(color.a)
#if defined(IS_SEE_THROUGH)
#  if FIXTURE_TINT_STEPS >= 3
    color.rgb *= line_after / 400.0;
#  elif FIXTURE_TINT_STEPS >= 1
    color.rgb *= 0.25;
#  endif
#elif !defined IS_GUI
    color.a *= 0.75;
#endif
#ifndef OIT_ALPHA_ONLY
    fragColor = color;
#endif
}
