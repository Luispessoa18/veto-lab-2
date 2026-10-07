use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use sha2::{Digest, Sha256};
use solana_transaction::versioned::VersionedTransaction;

/// Largest raw transaction accepted. PACKET_DATA_SIZE is 1232 bytes for legacy and v0
/// transactions; v1 transactions may be larger, so 4096 is the cap.
pub const MAX_TX_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Base58,
    Base64,
}

impl Encoding {
    /// RPC names. Solana RPC defaults to base58 when the field is missing.
    pub fn parse(name: Option<&str>) -> Option<Encoding> {
        match name {
            None | Some("base58") => Some(Encoding::Base58),
            Some("base64") => Some(Encoding::Base64),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct Decoded {
    pub tx: VersionedTransaction,
    /// sha256 of the signable message bytes, lowercase hex (same as Veto's messageDigestOf).
    pub digest: String,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DecodeError {
    #[error("invalid {0} string")]
    Encoding(&'static str),
    #[error("invalid transaction: {0}")]
    Transaction(String),
    #[error("transaction too large (max {MAX_TX_BYTES} bytes)")]
    TooLarge,
}

/// Signature count is a compact-u16 ("shortvec"): 7 bits per byte, high bit = continue.
fn shortvec_len(raw: &[u8]) -> Option<(usize, usize)> {
    let mut value = 0usize;
    for (i, byte) in raw.iter().take(3).enumerate() {
        value |= ((byte & 0x7f) as usize) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None
}

/// sha256 of the signable message: the raw bytes after the signature section.
/// Hashing the raw bytes (not a re-serialization) keeps us byte-identical with Veto.
pub fn message_digest(raw: &[u8]) -> Option<String> {
    let (count, prefix) = shortvec_len(raw)?;
    let start = prefix + 64 * count;
    if start >= raw.len() {
        return None;
    }
    Some(hex::encode(Sha256::digest(&raw[start..])))
}

pub fn decode(s: &str, enc: Encoding) -> Result<Decoded, DecodeError> {
    // Cheap pre-check before decoding: both encodings take fewer than 2 chars per byte.
    if s.len() > 2 * MAX_TX_BYTES {
        return Err(DecodeError::TooLarge);
    }
    let raw = match enc {
        Encoding::Base64 => B64.decode(s).map_err(|_| DecodeError::Encoding("base64"))?,
        Encoding::Base58 => bs58::decode(s).into_vec().map_err(|_| DecodeError::Encoding("base58"))?,
    };
    if raw.len() > MAX_TX_BYTES {
        return Err(DecodeError::TooLarge);
    }
    let tx: VersionedTransaction =
        bincode::deserialize(&raw).map_err(|e| DecodeError::Transaction(e.to_string()))?;
    tx.message
        .sanitize()
        .map_err(|e| DecodeError::Transaction(e.to_string()))?;
    let digest = message_digest(&raw).ok_or(DecodeError::Transaction("truncated".into()))?;
    Ok(Decoded { tx, digest })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// shortvec(1) + one zero signature + message bytes
    fn raw_with_sigs(n: u8, message: &[u8]) -> Vec<u8> {
        let mut raw = vec![n];
        raw.extend(std::iter::repeat_n(0u8, 64 * n as usize));
        raw.extend_from_slice(message);
        raw
    }

    #[test]
    fn digest_hashes_only_the_message() {
        let msg = b"any message bytes";
        let expected = hex::encode(Sha256::digest(msg));
        assert_eq!(message_digest(&raw_with_sigs(1, msg)).unwrap(), expected);
        assert_eq!(message_digest(&raw_with_sigs(2, msg)).unwrap(), expected);
    }

    #[test]
    fn digest_rejects_truncated_input() {
        assert_eq!(message_digest(&[1u8, 0, 0]), None);
        assert_eq!(message_digest(&[]), None);
    }

    #[test]
    fn encoding_defaults_to_base58() {
        assert_eq!(Encoding::parse(None), Some(Encoding::Base58));
        assert_eq!(Encoding::parse(Some("base64")), Some(Encoding::Base64));
        assert_eq!(Encoding::parse(Some("json")), None);
    }

    #[test]
    fn oversized_input_is_rejected_before_and_after_decoding() {
        let big = vec![0u8; MAX_TX_BYTES + 1];
        assert_eq!(decode(&B64.encode(&big), Encoding::Base64).unwrap_err(), DecodeError::TooLarge);
        assert_eq!(decode(&"1".repeat(3 * MAX_TX_BYTES), Encoding::Base58).unwrap_err(), DecodeError::TooLarge);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(matches!(decode("!!!", Encoding::Base64), Err(DecodeError::Encoding(_))));
        assert!(matches!(decode(&B64.encode([1u8, 2, 3]), Encoding::Base64), Err(DecodeError::Transaction(_))));
    }
}
