/* internal/sanitizers.h: no sanitizer attributes in this build. */
#ifndef NO_SANITIZE
# define NO_SANITIZE(x, y) y
#endif
