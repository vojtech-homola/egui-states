use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

#[derive(Default)]
pub struct NoHashHasher(u64);

impl Hasher for NoHashHasher {
    fn write(&mut self, _: &[u8]) {
        panic!("Invalid use of NoHashHasher")
    }

    fn write_u32(&mut self, i: u32) {
        self.0 = u64::from(i);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

pub type NoHashMap<V> = HashMap<u32, V, BuildHasherDefault<NoHashHasher>>;
pub type NoHashSet = HashSet<u32, BuildHasherDefault<NoHashHasher>>;
