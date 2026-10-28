//! Why a write was not applied, or may not have been: by field and with its
//! numbers, as the helper reports it and as the user reads it.

use serde::{Deserialize, Serialize};

use super::super::validation::{ascii_control, identifier};
use super::*;

/// Why an event cannot be changed from Orbit.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalendarNotEditable {
    /// The calendar does not admit changes (a subscription, birthdays, a
    /// shared calendar with read access).
    CalendarReadOnly,
    /// Someone else organized it: an invitation is moved by whoever sent it.
    OrganizedBySomeoneElse,
}

/// What is wrong with one field of a write.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalendarChangeProblem {
    /// Empty or only blanks where a value is required (`title`).
    Empty,
    /// Longer than its limit: `bytes` against `limit`.
    TooLarge,
    /// More items than the limit: `count` against `limit`.
    TooMany,
    /// The end comes before the start: `start_ms` and `end_ms`.
    EndBeforeStart,
    /// Not a date Orbit can represent (years 1–9999): `value`.
    InvalidDate,
    /// Not an identifier: empty, too long or with control characters.
    InvalidIdentifier,
    /// Malformed: control characters, a value out of its range (`value`), or
    /// two ways of saying the same thing.
    InvalidValue,
    /// No calendar with that identifier on this Mac.
    UnknownCalendar,
    /// macOS has no default calendar for new events.
    NoDefaultCalendar,
    /// Not a time zone macOS knows.
    UnknownTimeZone,
    /// Not a URL macOS can parse.
    InvalidUrl,
    /// A change of the repetition reaches this occurrence and the ones after
    /// it (`future`), never this one alone.
    RecurrenceNeedsFutureSpan,
    /// A change that sets nothing.
    NothingToChange,
}

/// One field of a write that cannot be applied, and why, with its numbers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarInvalidChange {
    pub field: String,
    pub problem: CalendarChangeProblem,
    #[serde(
        default,
        deserialize_with = "crate::calendar::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub bytes: Option<u64>,
    #[serde(
        default,
        deserialize_with = "crate::calendar::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub count: Option<u64>,
    #[serde(
        default,
        deserialize_with = "crate::calendar::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub limit: Option<u64>,
    #[serde(
        default,
        deserialize_with = "crate::calendar::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub value: Option<i64>,
    #[serde(
        default,
        deserialize_with = "crate::calendar::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub start_ms: Option<i64>,
    #[serde(
        default,
        deserialize_with = "crate::calendar::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub end_ms: Option<i64>,
}

impl CalendarInvalidChange {
    pub(super) fn problem(field: &str, problem: CalendarChangeProblem) -> Self {
        Self {
            field: field.into(),
            problem,
            bytes: None,
            count: None,
            limit: None,
            value: None,
            start_ms: None,
            end_ms: None,
        }
    }

    pub(super) fn too_large(field: &str, bytes: usize, limit: usize) -> Self {
        Self {
            bytes: Some(u64::try_from(bytes).unwrap_or(u64::MAX)),
            limit: Some(u64::try_from(limit).unwrap_or(u64::MAX)),
            ..Self::problem(field, CalendarChangeProblem::TooLarge)
        }
    }

    pub(super) fn value(field: &str, problem: CalendarChangeProblem, value: i64) -> Self {
        Self {
            value: Some(value),
            ..Self::problem(field, problem)
        }
    }
}

/// A write the helper (or Orbit, before sending it) did not apply, or whose
/// outcome nobody can know. The `code` is the native error code; the rest is
/// the error's `detail`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "code", deny_unknown_fields)]
pub enum CalendarWriteRefusal {
    /// No occurrence of that event starts (originally) at that date.
    #[serde(rename = "calendar_event_not_found")]
    EventNotFound {
        calendar_item_id: String,
        occurrence_start_ms: i64,
    },
    #[serde(rename = "calendar_not_editable")]
    NotEditable {
        reason: CalendarNotEditable,
        calendar_id: String,
    },
    /// Boxed: the field's numbers make it the largest refusal by far.
    #[serde(rename = "calendar_invalid_change")]
    InvalidChange(Box<CalendarInvalidChange>),
    /// The macOS permission as EventKit reports it now.
    #[serde(rename = "calendar_permission_denied")]
    PermissionDenied {
        authorization: CalendarAuthorization,
    },
    /// EventKit refused the save or the removal: its error domain and code,
    /// never its text.
    #[serde(rename = "calendar_save_failed")]
    SaveFailed {
        error_domain: String,
        error_code: i64,
    },
    /// The write may or may not have reached the store: its answer was lost.
    /// Orbit reads the agenda again and never sends it a second time.
    #[serde(rename = "calendar_write_outcome_unknown")]
    OutcomeUnknown { cause: String },
    /// Known not to have been written: refused before EventKit was asked
    /// (`code` says why: `privacy_stopped`, `calendar_busy`…).
    #[serde(rename = "calendar_not_written")]
    NotWritten { cause: String },
    /// The daemon already follows `limit` writes waiting or with EventKit;
    /// this one was not sent (the daemon's own refusal, never the helper's).
    #[serde(rename = "calendar_write_queue_full")]
    QueueFull { unfinished: u64, limit: u64 },
}

/// The codes that are write refusals with a typed detail.
const WRITE_REFUSAL_CODES: [&str; 6] = [
    "calendar_event_not_found",
    "calendar_not_editable",
    "calendar_invalid_change",
    "calendar_permission_denied",
    "calendar_save_failed",
    "calendar_write_outcome_unknown",
];

impl std::fmt::Display for CalendarWriteRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.notice())
    }
}

/// A refusal is also the error a daemon command returns: its text is the
/// notice, with every number.
impl std::error::Error for CalendarWriteRefusal {}

impl CalendarWriteRefusal {
    pub fn code(&self) -> &'static str {
        match self {
            Self::EventNotFound { .. } => "calendar_event_not_found",
            Self::NotEditable { .. } => "calendar_not_editable",
            Self::InvalidChange(_) => "calendar_invalid_change",
            Self::PermissionDenied { .. } => "calendar_permission_denied",
            Self::SaveFailed { .. } => "calendar_save_failed",
            Self::OutcomeUnknown { .. } => "calendar_write_outcome_unknown",
            Self::NotWritten { .. } => "calendar_not_written",
            Self::QueueFull { .. } => "calendar_write_queue_full",
        }
    }

    /// Whether the store is known to be unchanged by this write.
    pub fn known_not_written(&self) -> bool {
        !matches!(self, Self::OutcomeUnknown { .. })
    }

    /// The refusal a helper error (`code` and its `detail`) stands for. A code
    /// that is not a write refusal — `privacy_stopped`, `calendar_busy`,
    /// `calendar_invalidated` before admission, `invalid_calendar_request` —
    /// was refused before EventKit was asked, and is kept by name. A refusal
    /// whose detail does not match its code is `Err`: the reply broke the
    /// contract and is not a fact about the store.
    pub fn from_native(
        code: &str,
        detail: Option<&serde_json::Value>,
    ) -> Result<Self, CalendarError> {
        if !WRITE_REFUSAL_CODES.contains(&code) {
            if code.is_empty() || code.len() > 64 || ascii_control(code) {
                return Err(CalendarError::InvalidWire);
            }
            return Ok(Self::NotWritten { cause: code.into() });
        }
        let mut object = match detail {
            Some(serde_json::Value::Object(fields)) => fields.clone(),
            None => serde_json::Map::new(),
            Some(_) => return Err(CalendarError::InvalidWire),
        };
        if object.contains_key("code") {
            return Err(CalendarError::InvalidWire);
        }
        object.insert("code".into(), serde_json::Value::from(code));
        let refusal: Self = serde_json::from_value(serde_json::Value::Object(object))
            .map_err(|_| CalendarError::InvalidWire)?;
        refusal.validate()?;
        Ok(refusal)
    }

    /// The detail object that travels next to the code on the native wire.
    pub fn detail(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        if let Some(fields) = value.as_object_mut() {
            fields.remove("code");
        }
        value
    }

    pub fn validate(&self) -> Result<(), CalendarError> {
        let valid = match self {
            Self::EventNotFound {
                calendar_item_id,
                occurrence_start_ms,
            } => identifier(calendar_item_id) && crate::wire::unix_ms(*occurrence_start_ms),
            Self::NotEditable { calendar_id, .. } => identifier(calendar_id),
            Self::InvalidChange(change) => {
                !change.field.is_empty()
                    && change.field.len() <= 64
                    && !ascii_control(&change.field)
            }
            Self::PermissionDenied { authorization } => {
                *authorization != CalendarAuthorization::FullAccess
            }
            Self::SaveFailed { error_domain, .. } => {
                !error_domain.is_empty()
                    && error_domain.len() <= 256
                    && !ascii_control(error_domain)
            }
            Self::OutcomeUnknown { cause } | Self::NotWritten { cause } => {
                !cause.is_empty() && cause.len() <= 64 && !ascii_control(cause)
            }
            Self::QueueFull { unfinished, limit } => unfinished >= limit,
        };
        if valid {
            Ok(())
        } else {
            Err(CalendarError::InvalidValue)
        }
    }

    /// English compatibility text, with every number the refusal has.
    /// Current interfaces render the typed code and facts in their own language.
    pub fn notice(&self) -> String {
        match self {
            Self::EventNotFound {
                calendar_item_id,
                occurrence_start_ms,
            } => format!(
                "Event {calendar_item_id} has no occurrence starting at {occurrence_start_ms} (Unix ms) on this Mac: it was moved or deleted outside Orbit. Nothing changed; the calendar is reread."
            ),
            Self::NotEditable {
                reason: CalendarNotEditable::CalendarReadOnly,
                calendar_id,
            } => format!(
                "Calendar {calendar_id} is read only in macOS (a subscription, birthdays or a shared calendar without write access). Nothing changed."
            ),
            Self::NotEditable {
                reason: CalendarNotEditable::OrganizedBySomeoneElse,
                calendar_id,
            } => format!(
                "This event in calendar {calendar_id} was organized by someone else: only the organizer can move or change it. Nothing changed."
            ),
            Self::InvalidChange(change) => change.notice(),
            Self::PermissionDenied { authorization } => format!(
                "macOS does not grant Orbit full Calendar access (EventKit status: {}). Grant it in System Settings > Privacy & Security > Calendars. Nothing changed.",
                serde_json::to_value(authorization)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "unknown".into())
            ),
            Self::SaveFailed {
                error_domain,
                error_code,
            } => format!(
                "EventKit refused the change (error {error_domain} {error_code}). Nothing changed."
            ),
            Self::OutcomeUnknown { cause } => format!(
                "Whether the change reached the calendar is unknown ({cause}): its response was lost. Orbit rereads the calendar and does not repeat it."
            ),
            Self::NotWritten { cause } if cause == "calendar_helper_read_only" => {
                "This Mac's helper advertises `calendar_read_only`: it reads the calendar but cannot write it. Update Orbit on this Mac. Nothing changed.".into()
            }
            Self::NotWritten { cause } => {
                format!("The change was not made ({cause}); the calendar is unchanged.")
            }
            Self::QueueFull { unfinished, limit } => format!(
                "There are {unfinished} calendar changes waiting or in progress, the maximum Orbit tracks concurrently ({limit}); this one was not sent. The calendar is unchanged."
            ),
        }
    }
}

impl CalendarInvalidChange {
    fn notice(&self) -> String {
        let field = &self.field;
        let numbers = |text: &str| {
            format!(
                "{text}{}{}{}",
                self.bytes.map_or(String::new(), |bytes| format!(
                    " ({bytes} bytes; limit {})",
                    self.limit.unwrap_or_default()
                )),
                self.count.map_or(String::new(), |count| format!(
                    " ({count}; limit {})",
                    self.limit.unwrap_or_default()
                )),
                self.value
                    .map_or(String::new(), |value| format!(" (value {value})")),
            )
        };
        let what = match self.problem {
            CalendarChangeProblem::Empty => format!("«{field}» is empty"),
            CalendarChangeProblem::TooLarge => numbers(&format!("«{field}» is too long")),
            CalendarChangeProblem::TooMany => numbers(&format!("«{field}» has too many items")),
            CalendarChangeProblem::EndBeforeStart => format!(
                "the end ({}) precedes the start ({})",
                self.end_ms.unwrap_or_default(),
                self.start_ms.unwrap_or_default()
            ),
            CalendarChangeProblem::InvalidDate => {
                numbers(&format!("«{field}» is not a valid date"))
            }
            CalendarChangeProblem::InvalidIdentifier => {
                format!("«{field}» is not a valid identifier")
            }
            CalendarChangeProblem::InvalidValue => numbers(&format!("«{field}» is invalid")),
            CalendarChangeProblem::UnknownCalendar => {
                "that calendar does not exist on this Mac".into()
            }
            CalendarChangeProblem::NoDefaultCalendar => {
                "macOS has no default calendar for new events".into()
            }
            CalendarChangeProblem::UnknownTimeZone => {
                "macOS does not recognize that time zone".into()
            }
            CalendarChangeProblem::InvalidUrl => "the URL is invalid".into(),
            CalendarChangeProblem::RecurrenceNeedsFutureSpan => {
                "changing recurrence affects this event and following events, not only this one"
                    .into()
            }
            CalendarChangeProblem::NothingToChange => "the change modifies nothing".into(),
        };
        format!("The event was not saved: {what}. Nothing changed.")
    }
}
