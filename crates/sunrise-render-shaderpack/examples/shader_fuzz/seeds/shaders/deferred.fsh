#version 120

uniform sampler2D colortex0, shadowtex0;
varying vec2 texcoord;

void main() {
    float depth = texture2D(shadowtex0, texcoord).r;
/* RENDERTARGETS: 0 */
    gl_FragData[0] = texture2D(colortex0, texcoord) * depth;
}
