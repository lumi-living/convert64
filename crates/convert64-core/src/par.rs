//! Iterator helpers that are parallel with the `parallel` feature and
//! sequential without it, so algorithms are written once.
//!
//! With `--features parallel` the functions dispatch to `rayon`; otherwise
//! they are thin wrappers over the standard iterators. Call sites never
//! mention `rayon` directly, which keeps `wasm32-unknown-unknown` builds
//! identical in source to native ones.

/// Applies `f` to every element of `items`, in parallel when the
/// `parallel` feature is enabled.
///
/// `f` must be `Send + Sync` even in the sequential build so that enabling
/// the feature never changes which closures are accepted.
#[cfg(feature = "parallel")]
pub fn for_each<T, F>(items: &mut [T], f: F)
where
    T: Send,
    F: Fn(&mut T) + Send + Sync,
{
    use rayon::prelude::*;
    items.par_iter_mut().for_each(f);
}

/// Applies `f` to every element of `items`, in parallel when the
/// `parallel` feature is enabled.
///
/// `f` must be `Send + Sync` even in the sequential build so that enabling
/// the feature never changes which closures are accepted.
#[cfg(not(feature = "parallel"))]
pub fn for_each<T, F>(items: &mut [T], f: F)
where
    T: Send,
    F: Fn(&mut T) + Send + Sync,
{
    items.iter_mut().for_each(f);
}

/// Maps every element of `items` through `f` into a `Vec`, in parallel
/// when the `parallel` feature is enabled. Output order matches input order.
#[cfg(feature = "parallel")]
pub fn map<T, U, F>(items: &[T], f: F) -> Vec<U>
where
    T: Sync,
    U: Send,
    F: Fn(&T) -> U + Send + Sync,
{
    use rayon::prelude::*;
    items.par_iter().map(f).collect()
}

/// Maps every element of `items` through `f` into a `Vec`, in parallel
/// when the `parallel` feature is enabled. Output order matches input order.
#[cfg(not(feature = "parallel"))]
pub fn map<T, U, F>(items: &[T], f: F) -> Vec<U>
where
    T: Sync,
    U: Send,
    F: Fn(&T) -> U + Send + Sync,
{
    items.iter().map(f).collect()
}

/// Runs `a` and `b`, in parallel when the `parallel` feature is enabled, and returns
/// both results.
#[cfg(feature = "parallel")]
pub fn join<A, B, RA, RB>(a: A, b: B) -> (RA, RB)
where
    A: FnOnce() -> RA + Send,
    B: FnOnce() -> RB + Send,
    RA: Send,
    RB: Send,
{
    rayon::join(a, b)
}

/// Runs `a` and `b`, in parallel when the `parallel` feature is enabled, and returns
/// both results.
#[cfg(not(feature = "parallel"))]
pub fn join<A, B, RA, RB>(a: A, b: B) -> (RA, RB)
where
    A: FnOnce() -> RA + Send,
    B: FnOnce() -> RB + Send,
    RA: Send,
    RB: Send,
{
    (a(), b())
}

/// `true` when this build was compiled with the `parallel` feature.
#[must_use]
pub const fn is_parallel() -> bool {
    cfg!(feature = "parallel")
}

#[cfg(test)]
mod tests {
    use super::{for_each, is_parallel, join, map};

    #[test]
    fn for_each_touches_every_element() {
        let mut v = vec![1_u32, 2, 3, 4];
        for_each(&mut v, |x| *x *= 10);
        assert_eq!(v, [10, 20, 30, 40]);
    }

    #[test]
    fn map_preserves_order() {
        let v = [3_u8, 1, 2];
        assert_eq!(map(&v, |x| u32::from(*x) + 1), [4, 2, 3]);
    }

    #[test]
    fn join_returns_both() {
        assert_eq!(join(|| 1 + 1, || "b"), (2, "b"));
    }

    #[test]
    fn is_parallel_matches_feature() {
        assert_eq!(is_parallel(), cfg!(feature = "parallel"));
    }
}
