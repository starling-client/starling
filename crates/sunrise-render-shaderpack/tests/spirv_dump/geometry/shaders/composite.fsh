#version 120

uniform sampler2D colortex0;
uniform sampler2D shadowtex0;
varying vec2 texcoord;

void main() {
/* RENDERTARGETS: 0 */
    gl_FragData[0] = texture2D(colortex0, texcoord) * texture2D(shadowtex0, texcoord).r;
}
