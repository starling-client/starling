/*
 * Starling's `java` and `javac` on Android, for sunrise-prepare (the game files prepared on the
 * phone). An app may run programs only from its APK's library folder, so build_apk.py puts this
 * there twice, as libstarling_java.so and libstarling_javac.so (built with -DJAVAC), and the game
 * names them as the Java tools (sunrise_prepare::java::supply). Each starts the runtime Starling
 * downloaded (STARLING_JAVA_HOME, starling_account::java) as the JDK's own launchers do, through
 * JLI_Launch in its libjli.so; the runtime finds its home from where that library is.
 */
#include <dlfcn.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef int (*jli_launch)(int argc, char **argv, int jargc, const char **jargv, int appclassc,
                          const char **appclassv, const char *fullversion, const char *dotversion,
                          const char *pname, const char *lname, unsigned char javaargs,
                          unsigned char cpwildcard, unsigned char javaw, int ergo);

static void *open_library(const char *path) {
    void *library = dlopen(path, RTLD_NOW | RTLD_GLOBAL);
    if (library == NULL) {
        fprintf(stderr, "starling java: %s\n", dlerror());
    }
    return library;
}

int main(int argc, char **argv) {
    const char *home = getenv("STARLING_JAVA_HOME");
    if (home == NULL || home[0] == '\0') {
        fprintf(stderr, "starling java: STARLING_JAVA_HOME is not set\n");
        return 2;
    }
    char path[PATH_MAX];
    // libc++_shared.so, which the runtime's libraries need and do not carry, is beside this
    // program: loaded first, it answers their dependency by name.
    ssize_t length = readlink("/proc/self/exe", path, sizeof(path) - 1);
    if (length <= 0) {
        fprintf(stderr, "starling java: cannot find its own folder\n");
        return 2;
    }
    path[length] = '\0';
    char *slash = strrchr(path, '/');
    if (slash == NULL || (size_t)(slash - path) + sizeof("/libc++_shared.so") > sizeof(path)) {
        return 2;
    }
    strcpy(slash, "/libc++_shared.so");
    if (open_library(path) == NULL) {
        return 2;
    }
    if (snprintf(path, sizeof(path), "%s/lib/libjli.so", home) >= (int)sizeof(path)) {
        return 2;
    }
    void *jli = open_library(path);
    if (jli == NULL) {
        return 2;
    }
    jli_launch launch = (jli_launch)dlsym(jli, "JLI_Launch");
    if (launch == NULL) {
        fprintf(stderr, "starling java: %s has no JLI_Launch\n", path);
        return 2;
    }
#ifdef JAVAC
    // The JDK's javac launcher: the compiler's module, -J options for the runtime.
    static const char *javac_args[] = {"-J-ms8m", "-m", "jdk.compiler/com.sun.tools.javac.Main"};
    return launch(argc, argv, 3, javac_args, 0, NULL, "25", "25", "javac", "javac", 1, 1, 0, 0);
#else
    return launch(argc, argv, 0, NULL, 0, NULL, "25", "25", "java", "java", 0, 1, 0, 0);
#endif
}
