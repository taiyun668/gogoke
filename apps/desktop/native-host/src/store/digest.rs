/// SHA-256 hex digest using the same pinned backend as the LPAC shim build.
pub fn sha256_hex(input: &[u8]) -> String {
    #[cfg(test)]
    let measured_start = std::time::Instant::now();
    use sha2::{Digest, Sha256};
    let output = format!("{:x}", Sha256::digest(input));
    #[cfg(test)]
    if input.len() >= 1024 * 1024 {
        eprintln!("native_timing producer=store_sha256 bytes={} elapsed_us={}",
            input.len(), measured_start.elapsed().as_micros());
    }
    output
}

pub fn content_hash(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_input_matches_known_sha256() {
        assert_eq!(
            super::sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}