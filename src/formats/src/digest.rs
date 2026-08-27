//! Content digests.
//!
//! Unknown and opaque spans are reported by digest, never by payload bytes:
//! it keeps the normalized output bounded, and it is the only form a
//! private-fixture expectation may carry without becoming a copy of client
//! data.

use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256 of a byte range.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    crate::reader::to_hex(&digest)
}

/// SHA-256 after applying one XOR byte without allocating a decoded copy.
pub fn sha256_xor_hex(bytes: &[u8], key: u8) -> String {
    let mut digest = Sha256::new();
    for byte in bytes {
        digest.update([byte ^ key]);
    }
    crate::reader::to_hex(&digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_published_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn xor_digest_streams_the_decoded_bytes() {
        let encoded: Vec<u8> = b"abc".iter().map(|byte| byte ^ 0x73).collect();
        assert_eq!(sha256_xor_hex(&encoded, 0x73), sha256_hex(b"abc"));
    }
}
