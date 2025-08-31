use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::marker::PhantomData;

pub(crate) trait NoHashKey {}

#[derive(Default)]
pub struct NoHashHasher<K>(u64, PhantomData<K>);

impl<K: NoHashKey> Hasher for NoHashHasher<K> {
    fn write(&mut self, _: &[u8]) {
        panic!("Invalid use of NoHashHasher")
    }

    fn write_u32(&mut self, i: u32) {
        self.0 = u64::from(i);
    }

    fn write_u16(&mut self, i: u16) {
        self.0 = u64::from(i);
    }

    fn write_u8(&mut self, i: u8) {
        self.0 = u64::from(i);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

pub type NoHashMap<K, V> = HashMap<K, V, BuildHasherDefault<NoHashHasher<K>>>;
pub type NoHashSet<K> = HashSet<K, BuildHasherDefault<NoHashHasher<K>>>;

macro_rules! impl_basic_item {
    ($($t:ty),*) => {
        $(
            impl NoHashKey for $t {}
        )*
    };
}

impl_basic_item!(u32, u16, u8);