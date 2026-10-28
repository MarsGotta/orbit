//! The calendar event a recording belongs to.
//!
//! A recording is linked to an occurrence by its stable reference —
//! `calendar_item_id` and the start of the occurrence — never by the
//! `observation_id` of one read, which changes with every read. The link is
//! made when the recording starts, from the offer, from «Grabar» on an event,
//! or on its own, and travels with the recording to its note.
use crate::calendar::{
    CalendarOccurrence, ConferencePlatform, MAX_LABEL_BYTES, MAX_TITLE_BYTES, MAX_URL_BYTES,
    conference_platform, conference_platform_in,
};
use crate::meeting_audio::{CallKind, call_kind};
use serde::{Deserialize, Serialize};

/// One invitee of the calendar event linked to the recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attendee {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub is_self: bool,
}

/// The event a recording is linked to: its stable reference and what the
/// note needs of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedEvent {
    pub calendar_item_id: String,
    /// The original start of the occurrence: what names it in a recurring
    /// event even after it was moved.
    pub occurrence_start_ms: i64,
    pub title: String,
    pub starts_at_ms: i64,
    pub ends_at_ms: i64,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
    /// Whether the event repeats, as the calendar read it when the
    /// recording started. `None` is unknown — a link the app made without
    /// saying, or one stored before this field — and never «single». The
    /// series memory that read it was removed; the field stays
    /// because links already stored carry it and this type refuses unknown
    /// fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurring: Option<bool>,
}

/// Why a linked event was refused, naming the field and its numbers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkedEventRefusal {
    #[error(
        "The event identifier has {bytes} bytes; it must have between 1 and {max_bytes}, without control characters"
    )]
    Identifier { bytes: usize, max_bytes: usize },
    #[error("Event date {field} ({value} ms) is not a valid date")]
    Time { field: &'static str, value: i64 },
    #[error("The event ends ({ends_at_ms} ms) before it starts ({starts_at_ms} ms)")]
    EndsBeforeStart { starts_at_ms: i64, ends_at_ms: i64 },
    #[error("Event {field} has {bytes} bytes; the maximum is {max_bytes}")]
    TooLong {
        field: &'static str,
        bytes: usize,
        max_bytes: usize,
    },
}

impl LinkedEvent {
    /// The link to `occurrence`, as the calendar read it.
    pub fn from_occurrence(occurrence: &CalendarOccurrence) -> Self {
        let attendees = occurrence
            .invitees
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|invitee| Attendee {
                name: invitee
                    .name
                    .clone()
                    .or_else(|| invitee.email.clone())
                    .unwrap_or_default(),
                email: invitee.email.clone().unwrap_or_default(),
                is_self: invitee.is_current_user,
            })
            .collect();
        Self {
            calendar_item_id: occurrence.calendar_item_id.clone(),
            occurrence_start_ms: occurrence
                .original_occurrence_unix_ms
                .unwrap_or(occurrence.start_unix_ms),
            title: occurrence.title.clone(),
            starts_at_ms: occurrence.start_unix_ms,
            ends_at_ms: occurrence.end_unix_ms,
            attendees,
            // A helper that sends the event's details omits `recurrence` only when the
            // event does not repeat; the app reads it the same way.
            recurring: Some(occurrence.recurrence.is_some()),
        }
    }

    /// Shape only: the calendar's own bounds, which a real event always
    /// meets. A refusal names its field and numbers.
    pub fn check(&self) -> Result<(), LinkedEventRefusal> {
        let id = &self.calendar_item_id;
        if id.is_empty() || id.len() > MAX_LABEL_BYTES || id.chars().any(char::is_control) {
            return Err(LinkedEventRefusal::Identifier {
                bytes: id.len(),
                max_bytes: MAX_LABEL_BYTES,
            });
        }
        for (field, value) in [
            ("occurrence_start_ms", self.occurrence_start_ms),
            ("starts_at_ms", self.starts_at_ms),
            ("ends_at_ms", self.ends_at_ms),
        ] {
            if !crate::wire::unix_ms(value) {
                return Err(LinkedEventRefusal::Time { field, value });
            }
        }
        if self.ends_at_ms < self.starts_at_ms {
            return Err(LinkedEventRefusal::EndsBeforeStart {
                starts_at_ms: self.starts_at_ms,
                ends_at_ms: self.ends_at_ms,
            });
        }
        let too_long = |field: &'static str, text: &str, max_bytes: usize| {
            (text.len() > max_bytes).then_some(LinkedEventRefusal::TooLong {
                field,
                bytes: text.len(),
                max_bytes,
            })
        };
        if let Some(refusal) = too_long("title", &self.title, MAX_TITLE_BYTES) {
            return Err(refusal);
        }
        for attendee in &self.attendees {
            if let Some(refusal) =
                too_long("nombre de un invitado", &attendee.name, MAX_TITLE_BYTES)
                    .or_else(|| too_long("correo de un invitado", &attendee.email, MAX_URL_BYTES))
            {
                return Err(refusal);
            }
        }
        Ok(())
    }
}

/// The video-call service an occurrence names: its URL, else a link written
/// in its location or notes.
pub fn conference_platform_of(occurrence: &CalendarOccurrence) -> Option<ConferencePlatform> {
    occurrence
        .event_url
        .as_deref()
        .and_then(conference_platform)
        .or_else(|| {
            occurrence
                .location
                .as_deref()
                .and_then(conference_platform_in)
        })
        .or_else(|| occurrence.notes.as_deref().and_then(conference_platform_in))
}

/// The platforms a call application can be in. A browser holds Meet,
/// Whereby and the web clients of Teams and Webex.
fn platforms_of(bundle_id: &str) -> &'static [ConferencePlatform] {
    use ConferencePlatform::*;
    match bundle_id {
        "us.zoom.xos" => &[Zoom],
        "com.microsoft.teams" | "com.microsoft.teams2" => &[Teams],
        "com.cisco.webexmeetingsapp" | "Cisco-Systems.Spark" => &[Webex],
        "com.google.meetings" => &[GoogleMeet],
        "com.apple.FaceTime" => &[FaceTime],
        _ if call_kind(bundle_id) == Some(CallKind::Browser) => {
            &[GoogleMeet, Whereby, Teams, Webex]
        }
        _ => &[],
    }
}

/// Whether the event is on at `now_ms` or starts within
/// [`super::EVENT_LOOKAHEAD_MS`]: started and not ended, or about to.
fn on_now(occurrence: &CalendarOccurrence, now_ms: i64) -> bool {
    let started = occurrence.start_unix_ms <= now_ms && now_ms < occurrence.end_unix_ms;
    let soon = now_ms < occurrence.start_unix_ms
        && occurrence.start_unix_ms - now_ms <= super::EVENT_LOOKAHEAD_MS;
    started || soon
}

/// A meeting rather than a block of time: someone besides the user is invited, or
/// it has a video-call link.
fn is_meeting(occurrence: &CalendarOccurrence) -> bool {
    let guests = occurrence
        .invitees
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|invitee| !invitee.is_current_user);
    guests || conference_platform_of(occurrence).is_some()
}

fn declined_by_user(occurrence: &CalendarOccurrence) -> bool {
    occurrence
        .invitees
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|invitee| invitee.is_current_user && invitee.response_status == "declined")
}

/// The event of a call held by `bundle_id` at `now_ms`, if there is one.
///
/// Candidates are the events on now or starting within ten minutes that are
/// meetings (guests or a video link), never all-day, cancelled or declined by
/// the user. Among them, one whose link is on the same platform as the
/// application wins (Meet ↔ a browser, Zoom ↔ zoom.us…); then the one whose
/// start is nearest to now, so a meeting about to start beats one that ran
/// long. Ties go to the earlier start and then the identifier, so the choice
/// never depends on the order of the read.
pub fn choose_event<'a>(
    events: &'a [CalendarOccurrence],
    bundle_id: &str,
    now_ms: i64,
) -> Option<&'a CalendarOccurrence> {
    let platforms = platforms_of(bundle_id);
    events
        .iter()
        .filter(|occurrence| {
            !occurrence.is_all_day
                && !matches!(occurrence.status.as_str(), "cancelled" | "canceled")
                && !declined_by_user(occurrence)
                && on_now(occurrence, now_ms)
                && is_meeting(occurrence)
        })
        .min_by_key(|occurrence| {
            let same_platform = conference_platform_of(occurrence)
                .is_some_and(|platform| platforms.contains(&platform));
            (
                !same_platform,
                occurrence.start_unix_ms.abs_diff(now_ms),
                occurrence.start_unix_ms,
                occurrence.calendar_item_id.clone(),
            )
        })
}
