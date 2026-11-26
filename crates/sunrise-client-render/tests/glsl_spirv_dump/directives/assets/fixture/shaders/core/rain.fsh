#version 330
in vec4 vertexColor;
out vec4 fragColor;
#define KEEP(x) x
void main() {
    fragColor = KEEP(vertexColor);
#if __LINE__ > 4
    fragColor.a *= 0.5;
#endif
}
