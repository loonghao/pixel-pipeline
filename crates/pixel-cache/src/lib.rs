//! `pixel-cache`: content-addressed cache keys (PRD §7.12).
//!
//! The cache key binds every input that can change the result so that
//! identical work is never recomputed and results stay reproducible.

use sha2::{Digest, Sha256};

/// Compute the SHA-256 hex digest of a byte slice.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Inputs that fully determine a deterministic conversion result (PRD §7.12).
#[derive(Debug, Clone, Default)]
pub struct CacheKeyInputs<'a> {
    pub input_bytes_sha256: &'a str,
    pub effective_profile: &'a str,
    pub tool_version: &'a str,
    pub provider_version: &'a str,
    pub model_version: &'a str,
    pub seed: u64,
}

/// Compute the content-addressed cache key (PRD §7.12 formula).
pub fn cache_key(inputs: &CacheKeyInputs) -> String {
    let mut hasher = Sha256::new();
    hasher.update(inputs.input_bytes_sha256.as_bytes());
    hasher.update([0]);
    hasher.update(inputs.effective_profile.as_bytes());
    hasher.update([0]);
    hasher.update(inputs.tool_version.as_bytes());
    hasher.update([0]);
    hasher.update(inputs.provider_version.as_bytes());
    hasher.update([0]);
    hasher.update(inputs.model_version.as_bytes());
    hasher.update([0]);
    hasher.update(inputs.seed.to_le_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_is_stable() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn cache_key_changes_with_seed() {
        let mut a = CacheKeyInputs {
            input_bytes_sha256: "x",
            effective_profile: "p",
            tool_version: "0.1.0",
            provider_version: "",
            model_version: "",
            seed: 1,
        };
        let k1 = cache_key(&a);
        a.seed = 2;
        let k2 = cache_key(&a);
        assert_ne!(k1, k2);
    }
}
