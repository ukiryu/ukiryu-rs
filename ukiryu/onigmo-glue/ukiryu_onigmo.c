/*
 * Glue between ukiryu and the unmodified Onigmo sources of Ruby 4.0.1
 * (../vendor/onigmo), compiled in Ruby's configuration (RUBY defined).
 *
 * 1. The Ruby runtime hooks Onigmo calls, with the behaviour of a Ruby
 *    process whose Regexp has no timeout and whose $VERBOSE is false.
 * 2. The st_table functions Onigmo uses for group names (include/ruby/st.h).
 * 3. A small scalar C API for Rust: compile, search into a caller buffer,
 *    capture count, free. Only integers and pointers cross the boundary.
 *
 * Every allocation goes through malloc/realloc/free, which is the single
 * allocator of the host program (on wasm32-unknown-unknown, the Rust
 * allocator through the C allocation ABI of the vendored YAML engine).
 */
#include "regint.h"

/* Defined by enc/utf_8.c and enc/us_ascii.c; onigmo.h declares them only
 * outside Ruby's configuration. */
extern const OnigEncodingType OnigEncodingUTF_8;
extern const OnigEncodingType OnigEncodingUS_ASCII;
extern const OnigEncodingType OnigEncodingASCII;

/* ------------------------------------------------------------------ */
/* Ruby runtime hooks                                                  */

/* Set by rb_raise while a pattern compiles (callers serialise compiles). */
static int ukiryu_onigmo_raised;

void
rb_raise(int exc, const char *fmt, ...)
{
    (void)exc;
    (void)fmt;
    ukiryu_onigmo_raised = 1;
}

void
rb_warn(const char *fmt, ...)
{
    (void)fmt;
}

void
rb_compile_warn(const char *file, int line, const char *fmt, ...)
{
    (void)file;
    (void)line;
    (void)fmt;
}

void
rb_thread_check_ints(void)
{
}

/* re.c rb_reg_timeout_p with neither Regexp.timeout nor a per-object
 * timeout: never times out. */
bool
rb_reg_timeout_p(regex_t *reg, void *end_time)
{
    (void)reg;
    (void)end_time;
    return false;
}

/* ------------------------------------------------------------------ */
/* st_table: insertion-ordered, linear lookup (name tables are small)  */

/* Out of memory with no error path back (Ruby raises NoMemoryError).
 * A trap, not abort(): the browser build has no host abort import. */
#if defined(__GNUC__) || defined(__clang__)
# define UKIRYU_ONIGMO_OOM() __builtin_trap()
#else
# define UKIRYU_ONIGMO_OOM() abort()
#endif

struct st_table_entry {
    st_data_t key;
    st_data_t value;
    st_index_t hash;
    int live;
};

st_table *
st_init_table_with_size(const struct st_hash_type *type, st_index_t size)
{
    st_table *t = (st_table *)malloc(sizeof(st_table));
    if (t == NULL) return NULL;
    t->type = type;
    t->num_entries = 0;
    t->used = 0;
    t->capacity = size < 4 ? 4 : size;
    t->entries = (struct st_table_entry *)malloc(t->capacity * sizeof(struct st_table_entry));
    if (t->entries == NULL) {
        free(t);
        return NULL;
    }
    return t;
}

static struct st_table_entry *
st_find(st_table *t, st_data_t key, st_index_t hash)
{
    st_index_t i;
    for (i = 0; i < t->used; i++) {
        struct st_table_entry *e = &t->entries[i];
        if (e->live && e->hash == hash && t->type->compare(key, e->key) == 0)
            return e;
    }
    return NULL;
}

int
st_lookup(st_table *t, st_data_t key, st_data_t *value)
{
    struct st_table_entry *e = st_find(t, key, t->type->hash(key));
    if (e == NULL) return 0;
    if (value != NULL) *value = e->value;
    return 1;
}

/* 1 when the key was present (its value is replaced, the old key kept). */
int
st_insert(st_table *t, st_data_t key, st_data_t value)
{
    st_index_t hash = t->type->hash(key);
    struct st_table_entry *e = st_find(t, key, hash);
    if (e != NULL) {
        e->value = value;
        return 1;
    }
    if (t->used == t->capacity) {
        st_index_t capacity = t->capacity * 2;
        struct st_table_entry *grown = (struct st_table_entry *)
            realloc(t->entries, capacity * sizeof(struct st_table_entry));
        if (grown == NULL) UKIRYU_ONIGMO_OOM();
        t->entries = grown;
        t->capacity = capacity;
    }
    e = &t->entries[t->used++];
    e->key = key;
    e->value = value;
    e->hash = hash;
    e->live = 1;
    t->num_entries++;
    return 0;
}

int
st_foreach(st_table *t, st_foreach_callback_func *func, st_data_t arg)
{
    st_index_t i;
    for (i = 0; i < t->used; i++) {
        struct st_table_entry *e = &t->entries[i];
        if (!e->live) continue;
        switch (func(e->key, e->value, arg)) {
          case ST_STOP:
            return 0;
          case ST_DELETE:
            e->live = 0;
            t->num_entries--;
            break;
          default:
            break;
        }
    }
    return 0;
}

void
st_free_table(st_table *t)
{
    free(t->entries);
    free(t);
}

/* ------------------------------------------------------------------ */
/* Scalar API for Rust                                                 */

/* ukiryu_onigmo_new: error code when rb_raise was called (Ruby raises
 * RangeError from onigenc_single_byte_code_to_mbc). */
#define UKIRYU_ONIGMO_RAISED (-100000)

/* Compile like re.c make_regexp/onig_new_with_source: Ruby syntax, default
 * case-fold flag, `options` already masked to IGNORECASE|EXTEND|MULTILINE.
 * encoding: 0 US-ASCII, 1 UTF-8, 2 ASCII-8BIT (Ruby BINARY).
 * Returns ONIG_NORMAL (0) or an Onigmo error code; on error `message`
 * (ONIG_MAX_ERROR_MESSAGE_LEN bytes) holds onig_error_code_to_str. */
int
ukiryu_onigmo_new(regex_t **out, const unsigned char *pattern, size_t len,
                  unsigned int options, int encoding, unsigned char *message)
{
    OnigErrorInfo einfo;
    int r;

    *out = NULL;
    message[0] = '\0';
    ukiryu_onigmo_raised = 0;
    r = onig_new(out, pattern, pattern + len, (OnigOptionType)options,
                 encoding == 2 ? &OnigEncodingASCII :
                 encoding == 1 ? &OnigEncodingUTF_8 : &OnigEncodingUS_ASCII,
                 &OnigSyntaxRuby, &einfo);
    if (r != ONIG_NORMAL) {
        onig_error_code_to_str(message, r, &einfo);
        return r;
    }
    if (ukiryu_onigmo_raised) {
        onig_free(*out);
        *out = NULL;
        return UKIRYU_ONIGMO_RAISED;
    }
    return ONIG_NORMAL;
}

/* onig_number_of_captures + 1: the size of a match region. */
int
ukiryu_onigmo_regions(regex_t *reg)
{
    return onig_number_of_captures(reg) + 1;
}

/* re.c reg_onig_search over the whole string (pos 0, range = length).
 * Returns the match start, ONIG_MISMATCH, or an error code. On a match,
 * spans[2*i], spans[2*i+1] hold the byte offsets of group i (-1 when the
 * group did not participate) for i < min(regions, capacity). */
long long
ukiryu_onigmo_search(regex_t *reg, const unsigned char *str, size_t len,
                     long long *spans, int capacity)
{
    OnigRegion region;
    OnigPosition r;
    int i;

    onig_region_init(&region);
    r = onig_search(reg, str, str + len, str, str + len, &region,
                    ONIG_OPTION_NONE);
    if (r >= 0) {
        for (i = 0; i < region.num_regs && i < capacity; i++) {
            spans[2 * i] = (long long)region.beg[i];
            spans[2 * i + 1] = (long long)region.end[i];
        }
    }
    onig_region_free(&region, 0);
    return (long long)r;
}

void
ukiryu_onigmo_free(regex_t *reg)
{
    onig_free(reg);
}
