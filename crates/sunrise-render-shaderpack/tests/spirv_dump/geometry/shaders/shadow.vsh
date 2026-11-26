#version 330 compatibility

out vec2 vUv;
out vec4 vColor;

void main() {
    gl_Position = ftransform();
    vUv = gl_MultiTexCoord0.xy;
    vColor = gl_Color;
}
