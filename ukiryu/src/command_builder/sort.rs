//! Ruby `Enumerable#sort_by` with Integer keys, reproducing MRI's order for
//! equal keys.
//!
//! `build_args` orders positional arguments with
//! `regular_args.sort_by(&:numeric_position)`. MRI's `sort_by` is not stable,
//! so the order of arguments that share a position is whatever MRI's
//! algorithm leaves. This ports that algorithm from MRI 4.0.1 (`enum.c`,
//! revision e04267a14b):
//!
//! - When every key is a Fixnum (−2^62 ≤ k < 2^62 on 64-bit builds),
//!   `enum_sort_by` uses `rb_uniform_intro_sort_2`: return at once if the
//!   keys are already non-decreasing, otherwise an introsort with depth limit
//!   `2 * floor(log2 n)`, insertion sort for ranges of at most 16, a
//!   median-of-three Hoare partition, and heapsort once the depth runs out.
//!   This code is MRI's own, so the result does not depend on the host.
//! - Otherwise (a Bignum key) MRI calls `ruby_qsort`, which is the C
//!   library's `qsort_r`/`qsort_s`. The captured oracle (Linux, glibc 2.35)
//!   uses a stable merge sort there, and so does this port. Ruby on hosts
//!   whose C library sorts differently can order tied Bignum positions
//!   differently.
use num_bigint::BigInt;

const FIXNUM_MIN: i64 = -(1 << 62);
const FIXNUM_MAX: i64 = (1 << 62) - 1;

/// Reorder `items` as `items.sort_by { key }` does in MRI.
pub fn ruby_sort_by<T>(items: Vec<T>, mut key: impl FnMut(&T) -> BigInt) -> Vec<T> {
    let mut pairs: Vec<(BigInt, T)> = items.into_iter().map(|t| (key(&t), t)).collect();
    if pairs.len() <= 1 {
        return pairs.into_iter().map(|(_, t)| t).collect();
    }
    let fixnums: Option<Vec<i64>> = pairs
        .iter()
        .map(|(k, _)| {
            i64::try_from(k)
                .ok()
                .filter(|v| (FIXNUM_MIN..=FIXNUM_MAX).contains(v))
        })
        .collect();
    match fixnums {
        Some(keys) => {
            let mut slots: Vec<(i64, usize)> =
                keys.into_iter().enumerate().map(|(i, k)| (k, i)).collect();
            uniform_intro_sort(&mut slots);
            let mut taken: Vec<Option<T>> = pairs.into_iter().map(|(_, t)| Some(t)).collect();
            slots
                .into_iter()
                .map(|(_, i)| taken[i].take().expect("each index once"))
                .collect()
        }
        None => {
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            pairs.into_iter().map(|(_, t)| t).collect()
        }
    }
}

type Slot = (i64, usize);

/// `rb_uniform_intro_sort_2`.
fn uniform_intro_sort(a: &mut [Slot]) {
    let n = a.len();
    if a.windows(2).all(|w| w[0].0 <= w[1].0) {
        return;
    }
    let d = (usize::BITS - 1 - n.leading_zeros()) as usize;
    quicksort_intro(a, d << 1);
}

/// `rb_uniform_quicksort_intro_2`.
fn quicksort_intro(a: &mut [Slot], d: usize) {
    let n = a.len();
    if n <= 16 {
        insertion_sort(a);
        return;
    }
    if d == 0 {
        heapsort(a);
        return;
    }
    let x = med3(a[0].0, a[n >> 1].0, a[n - 1].0);
    let mut i: isize = 0;
    let mut j: isize = n as isize - 1;
    loop {
        while a[i as usize].0 < x {
            i += 1;
        }
        while x < a[j as usize].0 {
            j -= 1;
        }
        if i <= j {
            a.swap(i as usize, j as usize);
            i += 1;
            j -= 1;
        }
        if i > j {
            break;
        }
    }
    let j = (j + 1) as usize;
    let i = i as usize;
    if n - j > 1 {
        quicksort_intro(&mut a[j..], d - 1);
    }
    if i > 1 {
        quicksort_intro(&mut a[..i], d - 1);
    }
}

/// `med3_val`.
fn med3(a: i64, b: i64, c: i64) -> i64 {
    if a < b {
        if b < c {
            b
        } else if c < a {
            a
        } else {
            c
        }
    } else if c < b {
        b
    } else if a < c {
        a
    } else {
        c
    }
}

/// `rb_uniform_insertionsort_2`.
fn insertion_sort(a: &mut [Slot]) {
    if a.len() < 2 {
        return;
    }
    for index in 1..a.len() {
        let tmp = a[index];
        let mut j = index;
        if tmp.0 < a[0].0 {
            while j > 0 {
                a[j] = a[j - 1];
                j -= 1;
            }
        } else {
            while tmp.0 < a[j - 1].0 {
                a[j] = a[j - 1];
                j -= 1;
            }
        }
        a[j] = tmp;
    }
}

/// `rb_uniform_heap_down_2`.
fn heap_down(a: &mut [Slot], mut offset: usize, len: usize) {
    let tmp = a[offset];
    loop {
        let mut c = (offset << 1) + 1;
        if c > len {
            break;
        }
        if c < len && a[c].0 < a[c + 1].0 {
            c += 1;
        }
        if tmp.0 >= a[c].0 {
            break;
        }
        a[offset] = a[c];
        offset = c;
    }
    a[offset] = tmp;
}

/// `rb_uniform_heapsort_2`.
fn heapsort(a: &mut [Slot]) {
    let n = a.len();
    if n < 2 {
        return;
    }
    let mut offset = n >> 1;
    while offset > 0 {
        offset -= 1;
        heap_down(a, offset, n - 1);
    }
    let mut offset = n - 1;
    while offset > 0 {
        a.swap(0, offset);
        offset -= 1;
        heap_down(a, 0, offset);
    }
}
