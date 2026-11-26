#ifndef POST_COMMON
#define POST_COMMON
#define POST_GAIN 2 // the gain, a comment the body must not keep
#define POST_SCALE(v) ((v) * \
    float(POST_GAIN))
#endif
