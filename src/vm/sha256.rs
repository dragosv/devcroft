//! Minimal pure-Rust FIPS 180-4 SHA-256 helper for artifact, recipe, and
//! namespace content hashing with zero external dependencies.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

pub struct Sha256Hasher {
    inner: Sha256,
}

impl Default for Sha256Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256Hasher {
    pub fn new() -> Self {
        Self {
            inner: Sha256::new(),
        }
    }

    pub fn update(&mut self, input: &[u8]) {
        self.inner.update(input);
    }

    pub fn finalize_hex(self) -> String {
        let digest = self.inner.finalize();
        let mut out = String::with_capacity(64);
        for byte in digest {
            let _ = write!(&mut out, "{byte:02x}");
        }
        out
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256Hasher::new();
    h.update(bytes);
    h.finalize_hex()
}

pub fn sha256_file_hex(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256Hasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize_hex())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_fips_180_4_test_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }
}
