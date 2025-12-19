use sha2::{Digest, Sha256};
use std::hash::Hasher;

pub(crate) struct StableHasher {
    hasher: Sha256,
}

impl StableHasher {
    pub fn new() -> Self {
        Self {
            hasher: Sha256::new(),
        }
    }
}

impl Hasher for StableHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
    }

    fn finish(&self) -> u64 {
        let result = self.hasher.clone().finalize();
        u64::from_le_bytes(result[0..8].try_into().unwrap())
    }
}
