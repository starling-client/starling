#version 330 compatibility

uniform sampler2D tex;

in vec2 gUv;
in vec4 gColor;

layout(location = 0) out vec4 shadowColor;

void main() {
    vec4 color = texture(tex, gUv) * gColor;
    if (color.a < 0.1) discard;
    shadowColor = color;
}
