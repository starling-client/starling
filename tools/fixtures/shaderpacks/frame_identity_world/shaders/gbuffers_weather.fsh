#version 120

uniform sampler2D texture;

varying vec2 texcoord;
varying vec4 glcolor;

void main() {
    vec4 color = texture2D(texture, texcoord) * glcolor;
    // Rain and snow orange.
    color.rgb *= vec3(1.0, 0.45, 0.1);
    /* DRAWBUFFERS:0 */
    gl_FragData[0] = color;
}
