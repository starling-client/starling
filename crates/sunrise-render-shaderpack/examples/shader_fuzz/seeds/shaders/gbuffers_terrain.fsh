#version 120

uniform sampler2D texture, lightmap;

varying vec2 texcoord, lmcoord;
varying vec4 glcolor;
varying float blockId;

void main() {
    vec4 color = texture2D(texture, texcoord) * glcolor;
    color *= texture2D(lightmap, lmcoord);
    if (blockId > 10000.0) color.rgb = vec3(1.0);
/* DRAWBUFFERS:02 */
    gl_FragData[0] = color;
    gl_FragData[1] = vec4(lmcoord, 0.0, 1.0);
}
