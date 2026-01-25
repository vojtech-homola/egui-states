use std::sync::atomic::{
    AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicU8, AtomicU16, AtomicU32,
    AtomicU64,
    Ordering::{Acquire, Release},
};

pub unsafe trait Atomic: Copy {
    type Lock: AtomicLock<Self>;
}

pub unsafe trait AtomicLock<T: Copy>: Sync + Send {
    fn new(value: T) -> Self;
    fn load(&self) -> T;
    fn store(&self, value: T);
}

// ----------------------------------------------------
// implemntation basic --------------------------------
// 64
pub struct U64Lock(AtomicU64);
pub struct I64Lock(AtomicI64);

macro_rules! ImplAtomic64 {
    ($t:ty, $lock:ty, $atomic:ty) => {
        unsafe impl AtomicLock<$t> for $lock {
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
            type Lock = $lock;
        }
    };
}

ImplAtomic64!(u64, U64Lock, AtomicU64);
ImplAtomic64!(i64, I64Lock, AtomicI64);

// basics
pub struct U32Lock(AtomicU32);
pub struct I32Lock(AtomicI32);
pub struct U16Lock(AtomicU16);
pub struct I16Lock(AtomicI16);
pub struct U8Lock(AtomicU8);
pub struct I8Lock(AtomicI8);
pub struct BoolLock(AtomicBool);

macro_rules! ImplAtomic {
    ($t:ty, $lock:ty, $atomic:ty) => {
        unsafe impl AtomicLock<$t> for $lock {
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
unsafe impl AtomicLock<f64> for U64Lock {
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
    type Lock = U64Lock;
}

// f32
unsafe impl AtomicLock<f32> for U32Lock {
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
    type Lock = U32Lock;
}

// F32F32
#[cfg(target_has_atomic = "64")]
unsafe impl AtomicLock<(f32, f32)> for U64Lock {
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
    type Lock = U64Lock;
}

unsafe impl AtomicLock<[f32; 2]> for U64Lock {
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
    type Lock = U64Lock;
}
