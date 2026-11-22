#version 330 compatibility

uniform sampler2D colortex0;
in vec2 uv;
layout(location = 0) out vec4 fragColor;

void main() {
    fragColor = texture(colortex0, uv);
}
