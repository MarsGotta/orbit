//! One shape of wire for the core. The seven UUID validators, the SHA-256 hex
//! check, the bounded counters and the decoders used to live next to each
//! vertical; they now share this single home. Each vertical delegates here.
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use std::io;

pub mod bridge;
pub mod hub;
pub mod native;

/// Upper bound a unix timestamp can reach and still be valid (inclusive).
pub use crate::calendar::MAX_UNIX_MS as UNIX_MS_CEILING;
/// Lower bound a unix timestamp can reach and still be valid (inclusive).
pub use crate::calendar::MIN_UNIX_MS as UNIX_MS_FLOOR;

/// Lowercase hex SHA-256 of these bytes.
pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// Canonical lowercase hex digest of exactly 64 bytes.
pub fn sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Canonical RFC4122 UUID, lowercase only: 36 chars, dashes at 8/13/18/23 and
/// lowercase hexadecimal digits elsewhere. Mixed case and non-canonical
/// hyphens are rejected so the same identifier cannot be accepted in one
/// vertical and refused in another. The Swift peer `.lowercased()` its input
/// before calling in, so the canonical form is lowercase; for fields that
/// originate from a native source with its own casing, the caller should
/// lowercase before validating.
pub fn canonical_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

/// Unix milliseconds inside the supported calendar range.
pub fn unix_ms(value: i64) -> bool {
    (UNIX_MS_FLOOR..=UNIX_MS_CEILING).contains(&value)
}

/// A short non-empty string bounded by `max` bytes. Used for opaque tokens
/// that the protocol must accept when present and reject otherwise.
pub fn present(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max
}

/// Streaming size check that aborts before producing a Value or copying input
/// strings. Returns `Limit` if the serialized payload would exceed the budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundedError {
    Limit,
}

pub fn bounded<T>(value: &T, limit: usize) -> Result<(), BoundedError>
where
    T: Serialize + ?Sized,
{
    struct Counter(usize);
    impl io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self
                .0
                .checked_sub(bytes.len())
                .ok_or_else(|| io::Error::other("wire limit"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter(limit), value).map_err(|_| BoundedError::Limit)
}

/// Same bound as [`bounded`] but returns the serialized bytes (with an
/// optional trailing newline) for callers that need to ship the encoded view.
pub fn bounded_json<T>(value: &T, limit: usize, newline: bool) -> Result<Vec<u8>, BoundedError>
where
    T: Serialize + ?Sized,
{
    struct Writer {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
                return Err(io::Error::other("wire limit"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer {
        bytes: Vec::new(),
        limit: limit.saturating_sub(usize::from(newline)),
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| BoundedError::Limit)?;
    if newline {
        writer.bytes.push(b'\n');
    }
    Ok(writer.bytes)
}

/// Swift omits absent optionals. A present null is not that wire form and must
/// not silently become an omitted category, field, failure or scope. This is
/// the deserializer companion to [`present`]; both share the same name space.
pub fn optional<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Generic pagination cursor shared by the history views. Each vertical
/// decides how to map its own identifier onto [`Self::before_revision`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryCursor {
    pub before_revision: u64,
}

impl HistoryCursor {
    /// Positive revision only: zero would page from the start.
    pub fn is_valid(&self) -> bool {
        (1..=i64::MAX as u64).contains(&self.before_revision)
    }
}

/// Trait every decoded command implements so [`decode_command`] can apply a
/// single bounded decode and a single validation pass.
pub trait WireCommand: DeserializeOwned {
    const MAX_BYTES: usize;
    type Error;
    fn validate(&self) -> Result<(), Self::Error>;
}

/// Errors that [`decode_command`] can produce before the command type's own
/// `validate` ever runs. The trait's associated `Error` must be reachable
/// from this enum so the limit and invalid cases reach the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    Limit,
    Invalid,
}

/// Decode a bounded command payload, then run its own validator. The caller
/// supplies the conversion from this module's flat error space into its own.
pub fn decode_command<C>(bytes: &[u8]) -> Result<C, C::Error>
where
    C: WireCommand,
    C::Error: From<DecodeError>,
{
    if bytes.is_empty() || bytes.len() > C::MAX_BYTES {
        return Err(DecodeError::Limit.into());
    }
    let command: C = serde_json::from_slice(bytes).map_err(|_| DecodeError::Invalid)?;
    command.validate()?;
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_rejects_wrong_length_or_case() {
        assert!(sha256_hex(&"a".repeat(64)));
        assert!(!sha256_hex(&"A".repeat(64)));
        assert!(!sha256_hex(&"a".repeat(63)));
        assert!(!sha256_hex(""));
        assert!(!sha256_hex(&"z".repeat(64)));
    }

    #[test]
    fn canonical_uuid_accepts_lowercase_and_rejects_everything_else() {
        let lower = "abcdef01-2345-4678-89ab-cdef01234567";
        assert!(canonical_uuid(lower));
        let upper = lower.to_ascii_uppercase();
        assert!(!canonical_uuid(&upper));
        assert!(!canonical_uuid("11111111111141118111111111111111"));
        assert!(!canonical_uuid(""));
        assert!(!canonical_uuid("11111111-1111-4111-8111-11111111111z"));
    }

    #[test]
    fn present_requires_non_empty_and_within_the_budget() {
        assert!(present("x", 1));
        assert!(present("xx", 2));
        assert!(!present("", 1));
        assert!(!present("xxx", 2));
    }

    #[test]
    fn bounded_aborts_before_exceeding_the_budget() {
        let small = serde_json::json!({"k": "v"});
        assert!(bounded(&small, 64).is_ok());
        assert!(bounded(&small, 1).is_err());
    }
}
