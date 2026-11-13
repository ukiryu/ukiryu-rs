/*
 * The st_table interface Onigmo uses (group-name tables), for Ruby's
 * configuration. Ruby's st.c needs the interpreter, so ../ukiryu_onigmo.c
 * provides these functions. Like Ruby's tables they keep insertion order.
 */
#ifndef UKIRYU_ONIGMO_ST_H
#define UKIRYU_ONIGMO_ST_H 1

#include <stdint.h>

typedef uintptr_t st_data_t;
#define ST_DATA_T_DEFINED
typedef st_data_t st_index_t;

typedef int st_compare_func(st_data_t, st_data_t);
typedef st_index_t st_hash_func(st_data_t);

struct st_hash_type {
    int (*compare)(st_data_t, st_data_t); /* 0 when equal */
    st_index_t (*hash)(st_data_t);
};

struct st_table_entry;

typedef struct st_table {
    const struct st_hash_type *type;
    st_index_t num_entries;
    st_index_t used;     /* slots in entries, including deleted ones */
    st_index_t capacity;
    struct st_table_entry *entries;
} st_table;

enum st_retval {ST_CONTINUE, ST_STOP, ST_DELETE, ST_CHECK, ST_REPLACE};

typedef int st_foreach_callback_func(st_data_t, st_data_t, st_data_t);

#define st_init_table_with_size ukiryu_onigmo_st_init_table_with_size
#define st_lookup ukiryu_onigmo_st_lookup
#define st_insert ukiryu_onigmo_st_insert
#define st_foreach ukiryu_onigmo_st_foreach
#define st_free_table ukiryu_onigmo_st_free_table

st_table *st_init_table_with_size(const struct st_hash_type *type, st_index_t size);
int st_lookup(st_table *table, st_data_t key, st_data_t *value);
int st_insert(st_table *table, st_data_t key, st_data_t value);
int st_foreach(st_table *table, st_foreach_callback_func *func, st_data_t arg);
void st_free_table(st_table *table);

#endif /* UKIRYU_ONIGMO_ST_H */
