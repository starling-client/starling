#version 330 core

in vec3 vaPosition;
in vec4 vaColor;
in vec2 vaUV0;
in ivec2 vaUV2;
in vec3 vaNormal;

uniform mat4 modelViewMatrix, projectionMatrix;
uniform vec3 chunkOffset;

out VertexData {
    vec4 color;
    vec2 uv;
    vec3 normal;
} vertex;

void main() {
    gl_Position = projectionMatrix * modelViewMatrix * vec4(vaPosition + chunkOffset, 1.0);
    vertex.color = vaColor;
    vertex.uv = vaUV0 + vec2(vaUV2) * 0.0;
    vertex.normal = vaNormal;
}
