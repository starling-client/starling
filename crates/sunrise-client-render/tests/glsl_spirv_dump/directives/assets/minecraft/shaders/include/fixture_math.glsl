#define FIXTURE_SQUARE(x) ((x) * (x))
#define FIXTURE_MIX3(a, b, t) mix(a, b, t)
#define FIXTURE_HALF 0.5
vec4 fixture_tint(vec4 color, float amount) {
    return FIXTURE_MIX3(color, color * FIXTURE_HALF, FIXTURE_SQUARE(amount));
}
