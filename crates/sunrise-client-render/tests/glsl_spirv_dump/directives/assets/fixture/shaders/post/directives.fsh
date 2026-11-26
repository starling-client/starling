#version 330

#include <fixture:post_common.glsl>
#if 0
#include <fixture:post_never.glsl>
#endif

uniform sampler2D InSampler;

layout(std140) uniform DirectivesConfig {
    vec2 Strength;
};

layout(location = 0) in vec2 texCoord;
layout(location = 0) out vec4 fragColor;

/* A block comment
   over lines */
#define PICK(a, b) b
#define TAPS 4

#line 40
const float line_here = float(__LINE__);

void main() {
    vec4 color = texture(InSampler, texCoord);
#if TAPS > 2 && defined(POST_COMMON)
#  if UNDEFINED_KNOB
    color = vec4(0.0);
#  elif TAPS == 4
    color.rgb = POST_SCALE(color.rgb) * PICK(Strength.x, Strength.y);
#  else
    color = vec4(1.0);
#  endif
#elif defined POST_NEVER
    color = vec4(post_never);
#endif
    fragColor = vec4(color.rgb, line_here / 64.0);
}
