#version 120

uniform sampler2D colortex0;

varying vec2 texcoord;

void main() {
    vec3 color = texture2D(colortex0, texcoord).rgb;
    // Grades the whole picture, the End's sky included, which no other program here draws.
    float luma = dot(color, vec3(0.2126, 0.7152, 0.0722));
    color = mix(vec3(luma), color, 1.4) * vec3(1.05, 1.0, 0.9);
    /* DRAWBUFFERS:0 */
    gl_FragData[0] = vec4(color, 1.0);
}
