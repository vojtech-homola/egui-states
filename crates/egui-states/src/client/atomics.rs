//! Atomic and lock-backed storage for copyable client state values.

use parking_lot::{Mutex, RwLock};
use std::sync::atomic::{
    AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicU8, AtomicU16, AtomicU32,
    AtomicU64,
    Ordering::{Acquire, Release},
};

/// A copyable value that can be read through a low-overhead shared lock.
///
/// # Safety
///
/// `Lock` must faithfully store and load every value of `Self` and satisfy the
/// synchronization guarantees documented by [`AtomicLockStatic`].
pub unsafe trait AtomicStatic: Copy {
    /// Storage used by [`crate::StaticAtomic`].
    type Lock: AtomicLockStatic<Self>;
}

/// Thread-safe storage backing an [`AtomicStatic`] value.
///
/// # Safety
///
/// Concurrent calls must be data-race free. A completed [`Self::store`] must
/// eventually be observable by [`Self::load`], and values must never be torn.
pub unsafe trait AtomicLockStatic<T: Copy>: Sync + Send {
    /// Creates storage containing `value`.
    fn new(value: T) -> Self;
    /// Loads the current value.
    fn load(&self) -> T;
    /// Replaces the current value.
    fn store(&self, value: T);
}

/// An [`AtomicStatic`] value whose outgoing update can be ordered with storage.
///
/// # Safety
///
/// `Lock` must implement [`AtomicLock::update`] atomically with respect to
/// concurrent updates, in addition to the guarantees of [`AtomicStatic`].
pub unsafe trait Atomic: Copy {
    /// Storage used by [`crate::ValueAtomic`].
    type Lock: AtomicLock<Self>;
}

/// Storage that orders an outgoing notification before committing a value.
///
/// # Safety
///
/// Implementations must serialize the callback and store as one update so
/// concurrent writers cannot send one value and commit another out of order.
pub unsafe trait AtomicLock<T: Copy>: AtomicLockStatic<T> {
    /// Runs `before_store` and then commits `value` as one serialized update.
    fn update<F: FnOnce()>(&self, value: T, before_store: F);
}

/// Adds mutex-serialized update ordering around an [`AtomicLockStatic`].
///
/// Loads and stores may use the inner atomic directly, but [`AtomicLock::update`]
/// is not lock-free because it holds a mutex across notification and storage.
pub struct UpdateLock<L>(Mutex<()>, L);

unsafe impl<T: Copy, L: AtomicLockStatic<T>> AtomicLockStatic<T> for UpdateLock<L> {
    #[inline]
    fn new(value: T) -> Self {
        Self(Mutex::new(()), L::new(value))
    }

    #[inline]
    fn load(&self) -> T {
        self.1.load()
    }

    #[inline]
    fn store(&self, value: T) {
        self.1.store(value);
    }
}

unsafe impl<T: Copy, L: AtomicLockStatic<T>> AtomicLock<T> for UpdateLock<L> {
    #[inline]
    fn update<F: FnOnce()>(&self, value: T, before_store: F) {
        let _guard = self.0.lock();
        before_store();
        self.1.store(value);
    }
}

/// `RwLock`-based storage for targets without a suitable native atomic.
///
/// For example, 64-bit values use this fallback when the target does not
/// advertise 64-bit atomic support.
pub struct FallbackLock<T: Copy>(RwLock<T>);

unsafe impl<T: Copy + Send + Sync> AtomicLockStatic<T> for FallbackLock<T> {
    #[inline]
    fn new(value: T) -> Self {
        Self(RwLock::new(value))
    }

    #[inline]
    fn load(&self) -> T {
        *self.0.read()
    }

    #[inline]
    fn store(&self, value: T) {
        *self.0.write() = value;
    }
}

unsafe impl<T: Copy + Send + Sync> AtomicLock<T> for FallbackLock<T> {
    #[inline]
    fn update<F: FnOnce()>(&self, value: T, before_store: F) {
        let mut write = self.0.write();
        before_store();
        *write = value;
    }
}

// Built-in 64-bit atomic storage.
/// Native atomic storage used by built-in `u64`, `f64`, and two-`f32` values.
pub struct U64Lock(pub AtomicU64);
/// Native atomic storage used by the built-in `i64` value.
pub struct I64Lock(pub AtomicI64);

macro_rules! ImplAtomic64 {
    ($t:ty, $lock:ty, $atomic:ty) => {
        unsafe impl AtomicLockStatic<$t> for $lock {
            fn new(value: $t) -> Self {
                Self(<$atomic>::new(value))
            }

            #[inline]
            fn load(&self) -> $t {
                self.0.load(Acquire)
            }

            #[inline]
            fn store(&self, value: $t) {
                self.0.store(value, Release);
            }
        }

        unsafe impl Atomic for $t {
            #[cfg(target_has_atomic = "64")]
            type Lock = UpdateLock<$lock>;
            #[cfg(not(target_has_atomic = "64"))]
            type Lock = FallbackLock<$t>;
        }

        unsafe impl AtomicStatic for $t {
            #[cfg(target_has_atomic = "64")]
            type Lock = $lock;
            #[cfg(not(target_has_atomic = "64"))]
            type Lock = FallbackLock<$t>;
        }
    };
}

ImplAtomic64!(u64, U64Lock, AtomicU64);
ImplAtomic64!(i64, I64Lock, AtomicI64);

// Built-in atomic storage for values up to 32 bits.
/// Native atomic storage used by built-in `u32` and `f32` values.
pub struct U32Lock(AtomicU32);
/// Native atomic storage used by the built-in `i32` value.
pub struct I32Lock(AtomicI32);
/// Native atomic storage used by the built-in `u16` value.
pub struct U16Lock(AtomicU16);
/// Native atomic storage used by the built-in `i16` value.
pub struct I16Lock(AtomicI16);
/// Native atomic storage used by the built-in `u8` value.
pub struct U8Lock(AtomicU8);
/// Native atomic storage used by the built-in `i8` value.
pub struct I8Lock(AtomicI8);
/// Native atomic storage used by the built-in `bool` value.
pub struct BoolLock(AtomicBool);

macro_rules! ImplAtomic {
    ($t:ty, $lock:ty, $atomic:ty) => {
        unsafe impl AtomicLockStatic<$t> for $lock {
            fn new(value: $t) -> Self {
                Self(<$atomic>::new(value))
            }

            #[inline]
            fn load(&self) -> $t {
                self.0.load(Acquire)
            }

            #[inline]
            fn store(&self, value: $t) {
                self.0.store(value, Release);
            }
        }

        unsafe impl Atomic for $t {
            type Lock = UpdateLock<$lock>;
        }

        unsafe impl AtomicStatic for $t {
            type Lock = $lock;
        }
    };
}

ImplAtomic!(u32, U32Lock, AtomicU32);
ImplAtomic!(i32, I32Lock, AtomicI32);
ImplAtomic!(u16, U16Lock, AtomicU16);
ImplAtomic!(i16, I16Lock, AtomicI16);
ImplAtomic!(u8, U8Lock, AtomicU8);
ImplAtomic!(i8, I8Lock, AtomicI8);
ImplAtomic!(bool, BoolLock, AtomicBool);

// f64
unsafe impl AtomicLockStatic<f64> for U64Lock {
    fn new(value: f64) -> Self {
        Self(AtomicU64::new(value.to_bits()))
    }

    #[inline]
    fn load(&self) -> f64 {
        f64::from_bits(self.0.load(Acquire))
    }

    #[inline]
    fn store(&self, value: f64) {
        self.0.store(value.to_bits(), Release);
    }
}

unsafe impl Atomic for f64 {
    #[cfg(target_has_atomic = "64")]
    type Lock = UpdateLock<U64Lock>;
    #[cfg(not(target_has_atomic = "64"))]
    type Lock = FallbackLock<f64>;
}

unsafe impl AtomicStatic for f64 {
    #[cfg(target_has_atomic = "64")]
    type Lock = U64Lock;
    #[cfg(not(target_has_atomic = "64"))]
    type Lock = FallbackLock<f64>;
}

// f32
unsafe impl AtomicLockStatic<f32> for U32Lock {
    fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    #[inline]
    fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Acquire))
    }

    #[inline]
    fn store(&self, value: f32) {
        self.0.store(value.to_bits(), Release);
    }
}

unsafe impl Atomic for f32 {
    type Lock = UpdateLock<U32Lock>;
}

unsafe impl AtomicStatic for f32 {
    type Lock = U32Lock;
}

// Pack two f32 bit patterns into one u64 so both components change atomically.
unsafe impl AtomicLockStatic<(f32, f32)> for U64Lock {
    fn new(value: (f32, f32)) -> Self {
        let combined = ((value.0.to_bits() as u64) << 32) | (value.1.to_bits() as u64);
        Self(AtomicU64::new(combined))
    }

    fn load(&self) -> (f32, f32) {
        let combined = self.0.load(Acquire);
        let first = f32::from_bits((combined >> 32) as u32);
        let second = f32::from_bits(combined as u32);
        (first, second)
    }

    fn store(&self, value: (f32, f32)) {
        self.0.store(
            ((value.0.to_bits() as u64) << 32) | (value.1.to_bits() as u64),
            Release,
        );
    }
}

unsafe impl Atomic for (f32, f32) {
    #[cfg(target_has_atomic = "64")]
    type Lock = UpdateLock<U64Lock>;
    #[cfg(not(target_has_atomic = "64"))]
    type Lock = FallbackLock<(f32, f32)>;
}

unsafe impl AtomicStatic for (f32, f32) {
    #[cfg(target_has_atomic = "64")]
    type Lock = U64Lock;
    #[cfg(not(target_has_atomic = "64"))]
    type Lock = FallbackLock<(f32, f32)>;
}

unsafe impl AtomicLockStatic<[f32; 2]> for U64Lock {
    fn new(value: [f32; 2]) -> Self {
        let combined = ((value[0].to_bits() as u64) << 32) | (value[1].to_bits() as u64);
        Self(AtomicU64::new(combined))
    }

    fn load(&self) -> [f32; 2] {
        let combined = self.0.load(Acquire);
        let first = f32::from_bits((combined >> 32) as u32);
        let second = f32::from_bits(combined as u32);
        [first, second]
    }

    fn store(&self, value: [f32; 2]) {
        self.0.store(
            ((value[0].to_bits() as u64) << 32) | (value[1].to_bits() as u64),
            Release,
        );
    }
}

unsafe impl Atomic for [f32; 2] {
    #[cfg(target_has_atomic = "64")]
    type Lock = UpdateLock<U64Lock>;
    #[cfg(not(target_has_atomic = "64"))]
    type Lock = FallbackLock<[f32; 2]>;
}

unsafe impl AtomicStatic for [f32; 2] {
    #[cfg(target_has_atomic = "64")]
    type Lock = U64Lock;
    #[cfg(not(target_has_atomic = "64"))]
    type Lock = FallbackLock<[f32; 2]>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_storage_preserves_bits_including_signed_zero_and_nan() {
        let f32_lock = <f32 as Atomic>::Lock::new(0.0);
        for bits in [0, 0x80000000, 0x7f800000, 0x7fc01234, 1] {
            f32_lock.store(f32::from_bits(bits));
            assert_eq!(AtomicLockStatic::<f32>::load(&f32_lock).to_bits(), bits);
        }
        let f64_lock = <f64 as Atomic>::Lock::new(0.0);
        for bits in [
            0,
            0x8000000000000000,
            0x7ff0000000000000,
            0x7ff8000000001234,
            1,
        ] {
            f64_lock.store(f64::from_bits(bits));
            assert_eq!(AtomicLockStatic::<f64>::load(&f64_lock).to_bits(), bits);
        }
    }

    #[test]
    fn concurrent_pair_storage_never_tears() {
        if crate::test_support::isolated() {
            return;
        }
        let lock = std::sync::Arc::new(<[f32; 2] as Atomic>::Lock::new([1.0, -1.0]));
        let writer = lock.clone();
        let thread = std::thread::spawn(move || {
            for n in 1..50_000 {
                let v = n as f32;
                writer.store([v, -v]);
            }
        });
        for _ in 0..50_000 {
            let pair: [f32; 2] = lock.load();
            assert_eq!(pair[0], -pair[1]);
        }
        thread.join().unwrap();
        assert_eq!(
            AtomicLockStatic::<[f32; 2]>::load(&*lock),
            [49_999.0, -49_999.0]
        );
    }
}
