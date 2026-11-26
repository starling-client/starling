#version 330 compatibility

layout(triangles) in;
layout(triangle_strip, max_vertices = 6) out;

in vec2 vUv[];
in vec4 vColor[];
out vec2 gUv;
out vec4 gColor;

uniform float frameTimeCounter;

void main() {
    for (int copy = 0; copy < 2; copy++) {
        for (int i = 0; i < 3; i++) {
            gl_Position = gl_in[i].gl_Position + vec4(0.0, float(copy) * 0.01 * sin(frameTimeCounter), 0.0, 0.0);
            gUv = vUv[i];
            gColor = vColor[i];
            EmitVertex();
        }
        EndPrimitive();
    }
}
