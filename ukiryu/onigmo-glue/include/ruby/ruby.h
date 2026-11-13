/*
 * The part of Ruby's C API that Onigmo uses when it is built in Ruby's own
 * configuration (RUBY defined), for the unmodified Ruby 4.0.1 sources in
 * ../../vendor/onigmo. Ruby's interpreter is not linked: the hooks below are
 * defined in ../ukiryu_onigmo.c with Ruby's default behaviour for a Regexp
 * without a timeout.
 */
#ifndef UKIRYU_ONIGMO_RUBY_H
#define UKIRYU_ONIGMO_RUBY_H 1

#include <limits.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
# include <malloc.h>
#else
# include <alloca.h>
#endif

#ifndef RUBY_EXTERN
# define RUBY_EXTERN extern
#endif
#define RUBY_SYMBOL_EXPORT_BEGIN
#define RUBY_SYMBOL_EXPORT_END

#define HAVE_STDINT_H 1
#define HAVE_INTTYPES_H 1
#define HAVE_SYS_TYPES_H 1

#if defined(_MSC_VER)
# define SIZEOF_INT 4
# define SIZEOF_LONG 4
# define SIZEOF_LONG_LONG 8
# ifdef _WIN64
#  define SIZEOF_VOIDP 8
# else
#  define SIZEOF_VOIDP 4
# endif
#else
# define SIZEOF_INT __SIZEOF_INT__
# define SIZEOF_LONG __SIZEOF_LONG__
# define SIZEOF_LONG_LONG __SIZEOF_LONG_LONG__
# define SIZEOF_VOIDP __SIZEOF_POINTER__
#endif
#define LONG_LONG long long

/* include/ruby/defines.h */
#if defined(__GNUC__) && !defined(__cplusplus)
# define RB_GNUC_EXTENSION __extension__
# define RB_GNUC_EXTENSION_BLOCK(x) __extension__ ({ x; })
#else
# define RB_GNUC_EXTENSION
# define RB_GNUC_EXTENSION_BLOCK(x) (x)
#endif

/* include/ruby/backward/2/assume.h, without the unreachable hint. */
#define UNREACHABLE_RETURN(val) return (val)

/* rb_isalnum: ASCII only. */
#define ISALNUM(c) \
  (((c) >= '0' && (c) <= '9') || ((c) >= 'A' && (c) <= 'Z') || ((c) >= 'a' && (c) <= 'z'))

/* $VERBOSE is false by default: Onigmo's verbose-only warnings stay off. */
#define ruby_verbose 0
#define RTEST(v) ((v) != 0)

/* Exception classes are not objects here; rb_raise records the failure.
 * The hooks get private names: a Ruby extension that links this library
 * must keep calling the interpreter's own rb_raise and friends. */
#define rb_eRangeError 0
#define rb_raise ukiryu_onigmo_rb_raise
#define rb_warn ukiryu_onigmo_rb_warn
#define rb_compile_warn ukiryu_onigmo_rb_compile_warn
#define rb_thread_check_ints ukiryu_onigmo_rb_thread_check_ints
#define rb_reg_timeout_p ukiryu_onigmo_rb_reg_timeout_p
void rb_raise(int exc, const char *fmt, ...);
void rb_warn(const char *fmt, ...);
void rb_compile_warn(const char *file, int line, const char *fmt, ...);
void rb_thread_check_ints(void);

#endif /* UKIRYU_ONIGMO_RUBY_H */
