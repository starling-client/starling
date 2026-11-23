// Starling Visuals: the uniforms every program reads, and the light and sky model. Colours are
// linear and in the scene's own units: the sun gives about SUN_STRENGTH, the open sky a fraction.

#include "/lib/settings.glsl"

uniform mat4 gbufferModelViewInverse;
uniform vec3 sunPosition;
uniform vec3 shadowLightPosition;
uniform float rainStrength;
uniform float far;
uniform float nightVision;
uniform int isEyeInWater;

vec3 toLinear(vec3 c) {
    return pow(c, vec3(2.2));
}

float luma(vec3 c) {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}

// World-space directions of the sun and of the light that casts shadows (the moon at night).
vec3 sunDir() {
    return normalize(mat3(gbufferModelViewInverse) * sunPosition);
}

vec3 keyLightDir() {
    return normalize(mat3(gbufferModelViewInverse) * shadowLightPosition);
}

// 0 at night, 1 by day, crossing over while the sun is near the horizon.
float daylight(vec3 s) {
    return smoothstep(-0.12, 0.10, s.y);
}

// Sunlight once it has crossed the air: the lower the sun, the longer the path and the redder.
vec3 sunlight(vec3 s) {
    float air = 1.0 / (max(s.y, 0.0) + 0.07);
    vec3 transmitted = exp(-vec3(0.045, 0.11, 0.26) * air);
    return transmitted * smoothstep(-0.04, 0.06, s.y) * SUN_STRENGTH * (1.0 - rainStrength * 0.8);
}

vec3 moonlight(vec3 s) {
    return vec3(0.035, 0.05, 0.09) * smoothstep(0.0, -0.10, s.y) * (1.0 - rainStrength * 0.6);
}

// The sky's radiance along a world-space direction `d`, the sun at `s`.
vec3 skyRadiance(vec3 d, vec3 s) {
    float day = daylight(s);
    float lowSun = 1.0 - smoothstep(0.0, 0.4, abs(s.y));
    float up = clamp(d.y, 0.0, 1.0);
    float sunrise = pow(1.0 - up, 5.0);
    vec3 zenith = mix(vec3(0.002, 0.004, 0.010), vec3(0.045, 0.13, 0.48), day);
    // With the sun low, the long way through the air takes the blue out of the horizon too.
    vec3 rim = mix(vec3(0.006, 0.008, 0.016), vec3(0.30, 0.46, 0.78) * (1.0 - 0.4 * lowSun), day);
    vec3 color = mix(zenith, rim, sunrise);
    float toward = max(dot(d, s), 0.0);
    // A low sun warms the horizon around it; a halo surrounds it all day.
    color += vec3(1.0, 0.38, 0.10) * lowSun * day * pow(toward, 4.0) * sunrise * 2.2;
    color += sunlight(s) * (0.5 / SUN_STRENGTH) * pow(toward, 32.0);
    // Under the horizon, hazy land as far as the horizon's colour, darker looking further down.
    color *= mix(1.0, 0.5, smoothstep(0.1, 0.7, -d.y));
    color = mix(color, vec3(luma(color) * 0.8), rainStrength * 0.85);
    return color * SKY_BRIGHTNESS;
}

// The game's lightmap coordinates (block, sky) as 0..1 light levels.
vec2 lightLevels(vec2 lm) {
    return clamp((lm - 1.0 / 32.0) * (16.0 / 15.0), 0.0, 1.0);
}

// The sun's or moon's light on a surface facing `n` (world space), before shadows. `wrap` lights
// both sides of thin plants.
vec3 keyLightAt(vec3 n, float wrap) {
    vec3 s = sunDir();
    float facing = dot(n, keyLightDir());
    float lit = mix(clamp(facing, 0.0, 1.0), abs(facing) * 0.6 + 0.4, wrap);
    return (sunlight(s) + moonlight(s)) * lit;
}

// Everything else reaching it with light levels `levels`: the sky around (half its colour, the
// rest grey), the warm light the sunlit ground sends back, torches, and a floor that keeps caves
// readable.
vec3 fillLightAt(vec3 n, vec2 levels) {
    vec3 s = sunDir();
    // The sky's light is about the average of the zenith and a sunrise away from the sun, and
    // nearer white than either.
    vec3 sky = 0.5 * (skyRadiance(vec3(0.0, 1.0, 0.0), s) + skyRadiance(normalize(vec3(-s.x, 0.1, -s.z)), s));
    vec3 ambient = mix(vec3(luma(sky)), sky, 0.4) * 1.6 * (0.75 + 0.25 * n.y);
    vec3 bounce = sunlight(s) * max(s.y, 0.0) * 0.2 * (0.5 - 0.5 * n.y);
    float sky2 = levels.y * levels.y;
    float block = levels.x;
    vec3 torch = vec3(1.0, 0.56, 0.26) * (pow(block, 4.0) * 1.8 + block * 0.08) * BLOCKLIGHT_STRENGTH;
    vec3 floorLight = vec3(0.012, 0.013, 0.016) + nightVision * 0.3;
    return (ambient + bounce) * sky2 + torch + floorLight;
}

// Without the shadow map (past its reach, or shadows off): where the sky can't see, the sun
// doesn't either.
float openSky(vec2 levels) {
    return smoothstep(0.7, 0.97, levels.y);
}

// The shadow map's own distortion: points near the player get more of its texels.
vec2 distortShadow(vec2 p) {
    return p / (length(p) * SHADOW_DISTORTION + (1.0 - SHADOW_DISTORTION));
}

// The ACES film curve (Narkowicz's fit), then saturation.
vec3 tonemap(vec3 x) {
    x *= EXPOSURE;
    x = clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0);
    return clamp(mix(vec3(luma(x)), x, SATURATION), 0.0, 1.0);
}
