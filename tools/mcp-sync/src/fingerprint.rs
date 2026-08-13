// TODO(AGNT-0002.T01): Verify deterministic fingerprint coverage.
use sha2::{Digest, Sha256};

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::sha256_hex;

    #[test]
    fn matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hashes_empty_input() {
        assert_eq!(
            sha256_hex(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn is_deterministic() {
        assert_eq!(sha256_hex(b"abc"), sha256_hex(b"abc"));
    }

    #[test]
    fn unequal_inputs_have_unequal_fingerprints() {
        assert_ne!(sha256_hex(b"abc"), sha256_hex(b"abd"));
    }
}
