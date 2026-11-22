#version 120

attribute vec4 mc_Entity;
attribute vec4 mc_midTexCoord;
attribute vec4 at_tangent;

varying vec2 texcoord, lmcoord;
varying vec4 glcolor;
varying float blockId;

void main() {
    gl_Position = ftransform();
    texcoord = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy;
    lmcoord = (gl_TextureMatrix[1] * gl_MultiTexCoord1).xy;
    glcolor = gl_Color;
    blockId = mc_Entity.x + at_tangent.w * 0.0 + mc_midTexCoord.x * 0.0;
}
