//! Per-thread scratch buffers, so the compositor's parallel tasks do not allocate.
//!
//! A task borrows a buffer with [`with`], which hands out the thread's most recently
//! returned buffer of that element type, grown to the length asked for. Its contents are
//! whatever the last user left: callers initialise what they read. Buffers are taken off
//! a per-thread stack for the duration of the call, so nested use (a task that needs two
//! buffers, or one that runs another task while it waits) is safe.
//!
//! Single-threaded builds (wasm) have one stack; nothing here spawns threads.

use std::cell::RefCell;
use std::thread::LocalKey;

/// An element type with a scratch pool.
pub(crate) trait Scratch: Copy + Default + 'static {
    /// This thread's free buffers.
    fn pool() -> &'static LocalKey<RefCell<Vec<Vec<Self>>>>;
}

macro_rules! scratch {
    ($($t:ty => $pool:ident),* $(,)?) => {
        $(
            thread_local! {
                static $pool: RefCell<Vec<Vec<$t>>> = const { RefCell::new(Vec::new()) };
            }
            impl Scratch for $t {
                fn pool() -> &'static LocalKey<RefCell<Vec<Vec<Self>>>> {
                    &$pool
                }
            }
        )*
    };
}

scratch!(f32 => F32, f64 => F64, u16 => U16, [f32; 4] => RGBA);

/// Runs `f` on a scratch buffer of `len` elements and returns its result. The buffer's
/// contents are unspecified.
pub(crate) fn with<T: Scratch, R>(len: usize, f: impl FnOnce(&mut [T]) -> R) -> R {
    let mut buf = T::pool().with(|p| p.borrow_mut().pop()).unwrap_or_default();
    if buf.len() < len {
        buf.resize(len, T::default());
    }
    let r = f(&mut buf[..len]);
    T::pool().with(|p| p.borrow_mut().push(buf));
    r
}

#[cfg(test)]
mod tests {
    use super::with;

    #[test]
    fn buffers_are_reused_and_nest() {
        let a = with::<f32, _>(16, |b| {
            b.fill(1.0);
            let inner = with::<f32, _>(8, |c| {
                c.fill(2.0);
                c.as_ptr() as usize
            });
            assert!(
                b.iter().all(|&v| v == 1.0),
                "the inner buffer is a different one"
            );
            (b.as_ptr() as usize, inner)
        });
        let again = with::<f32, _>(4, |b| b.as_ptr() as usize);
        assert!(again == a.0 || again == a.1, "a returned buffer is reused");
    }
}
