#ifndef FZ_COMMON
#define FZ_COMMON

#define FZ_STEPS 4
#define FZ_SCALE(x) ((x) * 0.5)

struct Light {
    vec3 color;
    float strength[2];
};

const float weights[FZ_STEPS] = float[](0.1, 0.2, 0.3, 0.4);
const vec3 tint = vec3(1.0, 0.9, 0.8), shade = vec3(0.2);

float luma(vec3 c) {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}

vec3 lit(Light l) {
    return l.color * (l.strength[0] + FZ_SCALE(l.strength[1]));
}

#endif
