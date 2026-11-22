#version 330 core

uniform sampler2D gtexture;

in VertexData {
    vec4 color;
    vec2 uv;
    vec3 normal;
} vertex;

/* RENDERTARGETS: 0 */
layout(location = 0) out vec4 outColor;

void main() {
    outColor = texture(gtexture, vertex.uv) * vertex.color;
    outColor.rgb *= 0.5 + 0.5 * vertex.normal.y;
}
