//! Which video-call service a calendar link opens (P9-20 «Unirse», P9-11
//! the event of a call).
//!
//! Pure: the host is read from the link itself, never fetched. A host is what
//! follows `https://` up to the path, after any `user@` and before any port,
//! so `https://meet.google.com@example.com` is `example.com`, not Meet.
use serde::{Deserialize, Serialize};

/// A video-call service a calendar link can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConferencePlatform {
    GoogleMeet,
    Zoom,
    Teams,
    Webex,
    Whereby,
    FaceTime,
}

/// The services «Unirse» opens: a host, whether its subdomains count too
/// (`us02web.zoom.us`, `acme.webex.com`), and the service it names.
const CONFERENCE_HOSTS: [(&str, bool, ConferencePlatform); 8] = [
    ("meet.google.com", false, ConferencePlatform::GoogleMeet),
    ("zoom.us", true, ConferencePlatform::Zoom),
    ("zoom.com", true, ConferencePlatform::Zoom),
    ("teams.microsoft.com", false, ConferencePlatform::Teams),
    ("teams.live.com", false, ConferencePlatform::Teams),
    ("webex.com", true, ConferencePlatform::Webex),
    ("whereby.com", true, ConferencePlatform::Whereby),
    ("facetime.apple.com", false, ConferencePlatform::FaceTime),
];

/// The service `url` opens when it is an `https` link to a known video call.
pub fn conference_platform(url: &str) -> Option<ConferencePlatform> {
    let url = url.trim();
    let scheme = url.get(..8)?;
    if !scheme.eq_ignore_ascii_case("https://") {
        return None;
    }
    let rest = url.get(8..)?;
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|authority| !authority.is_empty())?;
    let host_and_port = authority.rsplit('@').next()?;
    let host = host_and_port
        .split(':')
        .next()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    CONFERENCE_HOSTS
        .iter()
        .find(|(known, subdomains, _)| {
            host == *known
                || (*subdomains
                    && host
                        .strip_suffix(known)
                        .is_some_and(|prefix| prefix.ends_with('.')))
        })
        .map(|(_, _, platform)| *platform)
}

/// The first known video-call link written anywhere in `text` (an event's
/// location or notes, where invitations from Zoom or Google usually put it).
/// Words are split on whitespace and on the brackets and quotes mail clients
/// wrap links in; nothing is fetched.
pub fn conference_platform_in(text: &str) -> Option<ConferencePlatform> {
    text.split(|character: char| {
        character.is_whitespace() || matches!(character, '<' | '>' | '(' | ')' | '"' | '\'')
    })
    .filter(|word| word.len() > 8)
    .find_map(conference_platform)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_names_its_service_by_host_only() {
        assert_eq!(
            conference_platform("https://meet.google.com/abc-defg-hij"),
            Some(ConferencePlatform::GoogleMeet)
        );
        assert_eq!(
            conference_platform("https://us02web.zoom.us/j/123"),
            Some(ConferencePlatform::Zoom)
        );
        assert_eq!(
            conference_platform("https://meet.google.com@example.com/x"),
            None
        );
        assert_eq!(conference_platform("http://meet.google.com/x"), None);
        assert_eq!(conference_platform("https://notzoom.us/x"), None);
    }

    #[test]
    fn a_link_inside_notes_or_a_location_is_found() {
        let notes = "Únete a Zoom:\n<https://acme.zoom.us/j/42?pwd=x> ID 42";
        assert_eq!(
            conference_platform_in(notes),
            Some(ConferencePlatform::Zoom)
        );
        assert_eq!(conference_platform_in("Sala 3, planta 2"), None);
    }
}
