use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::marker::PhantomData;

use sha2::{Digest, Sha256};

pub(crate) trait NoHashKey {}

#[derive(Default)]
pub struct NoHashHasher<K>(u64, PhantomData<K>);

impl<K: NoHashKey> Hasher for NoHashHasher<K> {
    fn write(&mut self, _: &[u8]) {
        panic!("Invalid use of NoHashHasher")
    }

    fn write_u64(&mut self, i: u64) {
        self.0 = i;
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

pub(crate) type NoHashMap<K, V> =
    std::collections::HashMap<K, V, BuildHasherDefault<NoHashHasher<K>>>;
#[cfg(any(feature = "server", feature = "python"))]
pub(crate) type NoHashSet<K> = std::collections::HashSet<K, BuildHasherDefault<NoHashHasher<K>>>;

macro_rules! impl_basic_item {
    ($($t:ty),*) => {
        $(
            impl NoHashKey for $t {}
        )*
    };
}

impl_basic_item!(u64, u32, u16, u8);

// Stable hashing ----------------------------------------------------------
pub(crate) struct StableHasher {
    hasher: Sha256,
}

impl StableHasher {
    pub fn new() -> Self {
        Self {
            hasher: Sha256::new(),
        }
    }

    pub fn finish_u32(&self) -> u32 {
        let result = self.hasher.clone().finalize();
        u32::from_le_bytes(result[0..4].try_into().unwrap())
    }
}

impl Hasher for StableHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
    }

    fn finish(&self) -> u64 {
        let result = self.hasher.clone().finalize();
        let hash = u64::from_le_bytes(result[0..8].try_into().unwrap());
        // Avoid reserved IDs 0-9
        10 + hash % (u64::MAX - 9)
    }
}

pub(crate) fn generate_value_id(name: &str) -> u64 {
    let mut hasher = StableHasher::new();
    name.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// State ids are computed independently by every peer and by generated
    /// bindings, so they must not change between releases or toolchains.
    #[test]
    fn state_ids_are_stable() {
        let actual: Vec<_> = ["root", "root.value", "root.nested.state", "root.ünïcødé"]
            .into_iter()
            .map(generate_value_id)
            .collect();
        assert_eq!(actual, GOLDEN_IDS, "state ids changed: {actual:?}");
    }

    const GOLDEN_IDS: [u64; 4] = [
        1439355459928577185,
        11375068530460040791,
        946401103240063378,
        3185887185576548773,
    ];

    #[test]
    fn state_ids_avoid_the_reserved_range() {
        for name in ["", "root", "a", "root.value"] {
            assert!(generate_value_id(name) > 9, "{name} uses a reserved id");
        }
    }
}
