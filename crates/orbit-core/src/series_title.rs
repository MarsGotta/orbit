//! A meeting's title as Hoy matches it with an earlier meeting of the same
//! name, when the calendar series has none (a meeting recorded by hand with
//! the event's title carries no series). The hub has the same rule
//! (`orbit_hub.meetings.series.title_key`); both are checked against
//! `contracts/fixtures/series-title-keys-v1.json`.
use sha2::{Digest, Sha256};

/// The words of `title`, lowercase, without those made only of digits:
/// «Revisión de diseño 12» and «revisión de diseño» are one meeting.
pub fn title_key(title: &str) -> String {
    title
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| {
            !word.is_empty() && !word.chars().all(|character| character.is_ascii_digit())
        })
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// What travels instead of the title: the SHA-256 of its key, in lowercase
/// hexadecimal. A title without words has none, and matches nothing.
pub fn title_digest(title: &str) -> Option<String> {
    let key = title_key(title);
    if key.is_empty() {
        return None;
    }
    Some(
        Sha256::digest(key.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Case {
        title: String,
        key: String,
        digest: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct Fixture {
        cases: Vec<Case>,
    }

    #[test]
    fn titles_have_the_hubs_keys_and_digests() -> Result<(), serde_json::Error> {
        let fixture: Fixture = serde_json::from_str(include_str!(
            "../../../contracts/fixtures/series-title-keys-v1.json"
        ))?;
        assert!(fixture.cases.len() >= 5);
        for case in fixture.cases {
            assert_eq!(title_key(&case.title), case.key, "{}", case.title);
            assert_eq!(title_digest(&case.title), case.digest, "{}", case.title);
        }
        Ok(())
    }
}
