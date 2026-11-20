//! Atomic storage for copyable client state values.

use std::marker::PhantomData;
use std::sync::atomic::{
    AtomicU64,
    Ordering::{Acquire, Release},
};

use parking_lot::Mutex;

/// A copyable value that [`ValueAtomic`] and [`StaticAtomic`] store in one
/// `AtomicU64`, so reading it is one atomic load, without a lock.
///
/// Built-in implementations cover the integers up to 64 bits, `bool`, `f32`,
/// `f64`, `(f32, f32)` and `[f32; 2]`, and `#[derive(Atomic)]` implements it
/// for a fieldless enum. Any other value that fits in 64 bits only converts
/// itself to and from them; the storage, the memory ordering and the lock
/// that orders a write with its message are the crate's:
///
/// ```
/// /// Two values that always change together.
/// #[derive(Clone, Copy)]
/// struct Range {
///     low: u16,
///     high: u16,
/// }
///
/// impl egui_states::Atomic for Range {
///     fn to_atomic(self) -> u64 {
///         (u64::from(self.low) << 16) | u64::from(self.high)
///     }
///
///     fn from_atomic(bits: u64) -> Self {
///         Range {
///             low: (bits >> 16) as u16,
///             high: bits as u16,
///         }
///     }
/// }
/// ```
///
/// `from_atomic` must return the value `to_atomic` was given, for every
/// value; it is only called with bits that `to_atomic` returned. The value
/// also needs `#[egui_states::typed]` and `InitialValue` to be a state, as
/// for [`Value`](crate::Value).
///
/// [`ValueAtomic`]: crate::ValueAtomic
/// [`StaticAtomic`]: crate::StaticAtomic
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be stored in a `ValueAtomic` or `StaticAtomic`",
    label = "not an atomic value",
    note = "use `Value` or `Static` instead, or implement `Atomic` for a value that fits in 64 bits"
)]
pub trait Atomic: Copy {
    /// Converts the value to the bits it is stored as.
    fn to_atomic(self) -> u64;
    /// Converts stored bits back to the value.
    fn from_atomic(bits: u64) -> Self;
}

/// One [`Atomic`] value in an `AtomicU64`.
pub(crate) struct AtomicCell<T>(AtomicU64, PhantomData<fn() -> T>);

impl<T: Atomic> AtomicCell<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(AtomicU64::new(value.to_atomic()), PhantomData)
    }

    #[inline]
    pub(crate) fn load(&self) -> T {
        T::from_atomic(self.0.load(Acquire))
    }

    #[inline]
    pub(crate) fn store(&self, value: T) {
        self.0.store(value.to_atomic(), Release);
    }
}

/// Orders the outgoing message of a [`crate::ValueAtomic`] change with
/// storing it.
///
/// Loads use the cell directly; [`Self::update`] holds a mutex across the
/// notification and the store, so concurrent writers cannot send one value
/// and commit another.
pub(crate) struct UpdateLock<T>(Mutex<()>, AtomicCell<T>);

impl<T: Atomic> UpdateLock<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(Mutex::new(()), AtomicCell::new(value))
    }

    #[inline]
    pub(crate) fn load(&self) -> T {
        self.1.load()
    }

    /// Runs `before_store` and then stores `value`, as one update.
    #[inline]
    pub(crate) fn update(&self, value: T, before_store: impl FnOnce()) {
        let _guard = self.0.lock();
        before_store();
        self.1.store(value);
    }
}

// Built-in values --------------------------------------------------------------
macro_rules! ImplUnsigned {
    ($($t:ty),*) => {$(
        impl Atomic for $t {
            #[inline]
            fn to_atomic(self) -> u64 {
                self as u64
            }

            #[inline]
            fn from_atomic(bits: u64) -> Self {
                bits as $t
            }
        }
    )*};
}

// A signed value is stored with its unsigned bit pattern of the same width.
macro_rules! ImplSigned {
    ($($t:ty => $u:ty),*) => {$(
        impl Atomic for $t {
            #[inline]
            fn to_atomic(self) -> u64 {
                self as $u as u64
            }

            #[inline]
            fn from_atomic(bits: u64) -> Self {
                bits as $u as $t
            }
        }
    )*};
}

ImplUnsigned!(u8, u16, u32, u64);
ImplSigned!(i8 => u8, i16 => u16, i32 => u32, i64 => u64);

impl Atomic for bool {
    #[inline]
    fn to_atomic(self) -> u64 {
        self as u64
    }

    #[inline]
    fn from_atomic(bits: u64) -> Self {
        bits != 0
    }
}

// Floats keep their exact bits, so NaN payloads and -0.0 survive.
impl Atomic for f32 {
    #[inline]
    fn to_atomic(self) -> u64 {
        self.to_bits() as u64
    }

    #[inline]
    fn from_atomic(bits: u64) -> Self {
        f32::from_bits(bits as u32)
    }
}

impl Atomic for f64 {
    #[inline]
    fn to_atomic(self) -> u64 {
        self.to_bits()
    }

    #[inline]
    fn from_atomic(bits: u64) -> Self {
        f64::from_bits(bits)
    }
}

// Two f32 in one u64, so both components change atomically.
impl Atomic for (f32, f32) {
    #[inline]
    fn to_atomic(self) -> u64 {
        ((self.0.to_bits() as u64) << 32) | (self.1.to_bits() as u64)
    }

    #[inline]
    fn from_atomic(bits: u64) -> Self {
        (
            f32::from_bits((bits >> 32) as u32),
            f32::from_bits(bits as u32),
        )
    }
}

impl Atomic for [f32; 2] {
    #[inline]
    fn to_atomic(self) -> u64 {
        (self[0], self[1]).to_atomic()
    }

    #[inline]
    fn from_atomic(bits: u64) -> Self {
        let (first, second) = <(f32, f32)>::from_atomic(bits);
        [first, second]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: Atomic + PartialEq + std::fmt::Debug>(values: &[T]) {
        let cell = AtomicCell::new(values[0]);
        for &value in values {
            cell.store(value);
            assert_eq!(cell.load(), value);
        }
    }

    #[test]
    fn integers_and_bool_round_trip_at_their_limits() {
        round_trip(&[0_u8, 1, u8::MAX]);
        round_trip(&[0_u16, 1, u16::MAX]);
        round_trip(&[0_u32, 1, u32::MAX]);
        round_trip(&[0_u64, 1, u64::MAX]);
        round_trip(&[0_i8, -1, i8::MIN, i8::MAX]);
        round_trip(&[0_i16, -1, i16::MIN, i16::MAX]);
        round_trip(&[0_i32, -1, i32::MIN, i32::MAX]);
        round_trip(&[0_i64, -1, i64::MIN, i64::MAX]);
        round_trip(&[false, true, false]);
    }

    #[test]
    fn float_storage_preserves_bits_including_signed_zero_and_nan() {
        let f32_cell = AtomicCell::new(0.0_f32);
        for bits in [0, 0x80000000, 0x7f800000, 0x7fc01234, 1] {
            f32_cell.store(f32::from_bits(bits));
            assert_eq!(f32_cell.load().to_bits(), bits);
        }
        let f64_cell = AtomicCell::new(0.0_f64);
        for bits in [
            0,
            0x8000000000000000,
            0x7ff0000000000000,
            0x7ff8000000001234,
            1,
        ] {
            f64_cell.store(f64::from_bits(bits));
            assert_eq!(f64_cell.load().to_bits(), bits);
        }
        let pair = AtomicCell::new((-0.0_f32, f32::from_bits(0x7fc01234)));
        let (first, second) = pair.load();
        assert_eq!(
            (first.to_bits(), second.to_bits()),
            (0x80000000, 0x7fc01234)
        );
    }

    /// An update notifies before it stores, so the notification sees the
    /// previous value.
    #[test]
    fn an_update_notifies_before_it_stores() {
        let lock = UpdateLock::new(1_u16);
        let mut previous = 1_u16;
        for value in [2_u16, u16::MAX, 0] {
            let mut notified = false;
            lock.update(value, || {
                assert_eq!(lock.load(), previous);
                notified = true;
            });
            assert!(notified);
            assert_eq!(lock.load(), value);
            previous = value;
        }
    }

    #[test]
    fn concurrent_pair_storage_never_tears() {
        if crate::test_support::isolated() {
            return;
        }
        let cell = std::sync::Arc::new(AtomicCell::new([1.0_f32, -1.0]));
        let writer = cell.clone();
        let thread = std::thread::spawn(move || {
            for n in 1..50_000 {
                let v = n as f32;
                writer.store([v, -v]);
            }
        });
        for _ in 0..50_000 {
            let pair: [f32; 2] = cell.load();
            assert_eq!(pair[0], -pair[1]);
        }
        thread.join().unwrap();
        assert_eq!(cell.load(), [49_999.0, -49_999.0]);
    }
}
