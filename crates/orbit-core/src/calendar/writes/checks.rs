//! What a write must be before it reaches the helper, and what the helper's
//! answer to it must be before the daemon believes it.

use std::collections::HashSet;

use super::super::validation::{ascii_control, identifier};
use super::*;

type Checked = Result<(), CalendarWriteRefusal>;

fn refuse(change: CalendarInvalidChange) -> Checked {
    Err(CalendarWriteRefusal::InvalidChange(Box::new(change)))
}

fn within_limit(field: &str, value: &str, limit: usize) -> Checked {
    if value.len() > limit {
        return refuse(CalendarInvalidChange::too_large(field, value.len(), limit));
    }
    Ok(())
}

fn check_date(field: &str, value: i64) -> Checked {
    if !crate::wire::unix_ms(value) {
        return refuse(CalendarInvalidChange::value(
            field,
            CalendarChangeProblem::InvalidDate,
            value,
        ));
    }
    Ok(())
}

fn id(field: &str, value: &str) -> Checked {
    if !identifier(value) {
        return refuse(CalendarInvalidChange::problem(
            field,
            CalendarChangeProblem::InvalidIdentifier,
        ));
    }
    Ok(())
}

fn ordered(start_ms: i64, end_ms: i64) -> Checked {
    if end_ms < start_ms {
        return refuse(CalendarInvalidChange {
            start_ms: Some(start_ms),
            end_ms: Some(end_ms),
            ..CalendarInvalidChange::problem("end_ms", CalendarChangeProblem::EndBeforeStart)
        });
    }
    Ok(())
}

fn title(value: &str) -> Checked {
    if value.trim().is_empty() {
        return refuse(CalendarInvalidChange::problem(
            "title",
            CalendarChangeProblem::Empty,
        ));
    }
    within_limit("title", value, MAX_TITLE_BYTES)
}

fn time_zone(value: &str) -> Checked {
    if value.is_empty() || ascii_control(value) {
        return refuse(CalendarInvalidChange::problem(
            "time_zone",
            CalendarChangeProblem::InvalidValue,
        ));
    }
    within_limit("time_zone", value, MAX_TIMEZONE_BYTES)
}

// An edit can clear a URL; a new event omits the field instead.
fn url(value: &str, allows_empty: bool) -> Checked {
    within_limit("url", value, MAX_URL_BYTES)?;
    if ascii_control(value) || (!allows_empty && value.is_empty()) {
        return refuse(CalendarInvalidChange::problem(
            "url",
            CalendarChangeProblem::InvalidUrl,
        ));
    }
    Ok(())
}

fn alarms(values: &[u64]) -> Checked {
    if values.len() > MAX_EVENT_ALARMS {
        return refuse(CalendarInvalidChange {
            count: Some(u64::try_from(values.len()).unwrap_or(u64::MAX)),
            limit: Some(MAX_EVENT_ALARMS as u64),
            ..CalendarInvalidChange::problem("alarms_minutes", CalendarChangeProblem::TooMany)
        });
    }
    if let Some(far) = values.iter().find(|minutes| **minutes > MAX_ALARM_MINUTES) {
        return refuse(CalendarInvalidChange {
            value: Some(i64::try_from(*far).unwrap_or(i64::MAX)),
            limit: Some(MAX_ALARM_MINUTES),
            ..CalendarInvalidChange::problem("alarms_minutes", CalendarChangeProblem::InvalidValue)
        });
    }
    Ok(())
}

fn days(values: &[CalendarRecurrenceDay]) -> bool {
    let mut seen = HashSet::new();
    values.iter().all(|day| {
        day.week
            .is_none_or(|week| week != 0 && (-53..=53).contains(&week))
            && seen.insert(*day)
    })
}

fn rule_shape(
    interval: u32,
    days_of_week: Option<&[CalendarRecurrenceDay]>,
    end: Option<&CalendarRecurrenceEnd>,
    start_ms: Option<i64>,
) -> Checked {
    let invalid = || {
        refuse(CalendarInvalidChange::problem(
            "recurrence",
            CalendarChangeProblem::InvalidValue,
        ))
    };
    if interval == 0 || days_of_week.is_some_and(|values| !days(values)) {
        return invalid();
    }
    if let Some(end) = end {
        match (end.until_ms, end.count) {
            (Some(until), None) => {
                check_date("recurrence", until)?;
                if start_ms.is_some_and(|start| until < start) {
                    return refuse(CalendarInvalidChange {
                        start_ms,
                        end_ms: Some(until),
                        ..CalendarInvalidChange::problem(
                            "recurrence",
                            CalendarChangeProblem::EndBeforeStart,
                        )
                    });
                }
            }
            (None, Some(count)) if count > 0 => {}
            _ => return invalid(),
        }
    }
    Ok(())
}

impl CalendarRecurrenceRule {
    fn validate(&self, start_ms: Option<i64>) -> Checked {
        rule_shape(
            self.interval,
            self.days_of_week.as_deref(),
            self.end.as_ref(),
            start_ms,
        )
    }
}

impl CalendarRecurrence {
    /// A read rule's structure; `rules_count`, when present, counts more than one.
    pub(in crate::calendar) fn validate(&self) -> Result<(), CalendarError> {
        rule_shape(
            self.interval,
            self.days_of_week.as_deref(),
            self.end.as_ref(),
            None,
        )
        .map_err(|_| CalendarError::InvalidValue)?;
        if self.rules_count.is_some_and(|count| count < 2) {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }
}

fn availability(value: CalendarAvailability) -> Checked {
    if value == CalendarAvailability::NotSupported {
        return refuse(CalendarInvalidChange::problem(
            "availability",
            CalendarChangeProblem::InvalidValue,
        ));
    }
    Ok(())
}

impl CalendarEventDraft {
    pub fn validate(&self) -> Checked {
        if let Some(calendar) = &self.calendar_id {
            id("calendar_id", calendar)?;
        }
        title(&self.title)?;
        check_date("start_ms", self.start_ms)?;
        check_date("end_ms", self.end_ms)?;
        ordered(self.start_ms, self.end_ms)?;
        if let Some(zone) = &self.time_zone {
            time_zone(zone)?;
        }
        if let Some(location) = &self.location {
            within_limit("location", location, MAX_TITLE_BYTES)?;
        }
        if let Some(notes) = &self.notes {
            within_limit("notes", notes, MAX_NOTES_BYTES)?;
        }
        if let Some(value) = &self.url {
            url(value, false)?;
        }
        if let Some(value) = self.availability {
            availability(value)?;
        }
        if let Some(rule) = &self.recurrence {
            rule.validate(Some(self.start_ms))?;
        }
        if let Some(values) = &self.alarms_minutes {
            alarms(values)?;
        }
        Ok(())
    }
}

impl CalendarEventChanges {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    pub fn validate(&self, span: CalendarSpan) -> Checked {
        if self.is_empty()
            || self.remove_recurrence == Some(false) && self.without_removal().is_empty()
        {
            return refuse(CalendarInvalidChange::problem(
                "changes",
                CalendarChangeProblem::NothingToChange,
            ));
        }
        if let Some(calendar) = &self.calendar_id {
            id("calendar_id", calendar)?;
        }
        if let Some(value) = &self.title {
            title(value)?;
        }
        if let Some(start) = self.start_ms {
            check_date("start_ms", start)?;
        }
        if let Some(end) = self.end_ms {
            check_date("end_ms", end)?;
        }
        if let (Some(start), Some(end)) = (self.start_ms, self.end_ms) {
            ordered(start, end)?;
        }
        if let Some(zone) = &self.time_zone {
            time_zone(zone)?;
        }
        if let Some(location) = &self.location {
            within_limit("location", location, MAX_TITLE_BYTES)?;
        }
        if let Some(notes) = &self.notes {
            within_limit("notes", notes, MAX_NOTES_BYTES)?;
        }
        if let Some(value) = &self.url {
            url(value, true)?;
        }
        if let Some(value) = self.availability {
            availability(value)?;
        }
        if self.recurrence.is_some() && self.remove_recurrence == Some(true) {
            return refuse(CalendarInvalidChange::problem(
                "recurrence",
                CalendarChangeProblem::InvalidValue,
            ));
        }
        if let Some(rule) = &self.recurrence {
            rule.validate(self.start_ms)?;
        }
        if (self.recurrence.is_some() || self.remove_recurrence == Some(true))
            && span == CalendarSpan::This
        {
            return refuse(CalendarInvalidChange::problem(
                "recurrence",
                CalendarChangeProblem::RecurrenceNeedsFutureSpan,
            ));
        }
        if let Some(values) = &self.alarms_minutes {
            alarms(values)?;
        }
        Ok(())
    }

    fn without_removal(&self) -> Self {
        Self {
            remove_recurrence: None,
            ..self.clone()
        }
    }
}

fn target(calendar_item_id: &str, occurrence_start_ms: i64) -> Checked {
    id("calendar_item_id", calendar_item_id)?;
    check_date("occurrence_start_ms", occurrence_start_ms)
}

impl CalendarEventChange {
    pub fn validate(&self) -> Checked {
        target(&self.calendar_item_id, self.occurrence_start_ms)?;
        self.changes.validate(self.span)
    }
}

impl CalendarSave {
    pub fn validate(&self) -> Checked {
        match self {
            Self::Create(draft) => draft.validate(),
            Self::Change(change) => change.validate(),
        }
    }
}

impl CalendarDelete {
    pub fn validate(&self) -> Checked {
        target(&self.calendar_item_id, self.occurrence_start_ms)
    }
}

/// The identifier a client gives its write, to find its outcome later.
pub fn validate_write_request_id(value: &str) -> Checked {
    id("request_id", value)
}

impl CalendarSaveResult {
    /// Structure only: the occurrence on its own terms (no window: a saved
    /// event may lie anywhere).
    pub fn validate(&self) -> Result<(), CalendarError> {
        if !crate::wire::canonical_uuid(&self.write_id)
            || !crate::wire::canonical_uuid(&self.native_instance_id)
            || self.system_timezone.is_empty()
            || self.system_timezone.len() > MAX_TIMEZONE_BYTES
        {
            return Err(CalendarError::InvalidValue);
        }
        self.occurrence.validate_alone(&self.system_timezone)?;
        super::super::wire::size(self)
    }
}

impl CalendarDeleteResult {
    pub fn validate(&self) -> Result<(), CalendarError> {
        if !crate::wire::canonical_uuid(&self.write_id)
            || !crate::wire::canonical_uuid(&self.native_instance_id)
            || !identifier(&self.calendar_item_id)
            || !crate::wire::unix_ms(self.occurrence_start_ms)
        {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }
}

/// Facts the daemon holds about the write it sent, to admit its answer.
pub struct CalendarWriteFacts<'a> {
    pub native_instance_id: &'a str,
    pub privacy_generation: u64,
}

impl CalendarSaveResult {
    pub fn validate_response(&self, facts: &CalendarWriteFacts<'_>) -> Result<(), CalendarError> {
        self.validate()?;
        if self.native_instance_id != facts.native_instance_id
            || self.privacy_generation != facts.privacy_generation
        {
            return Err(CalendarError::ScopeChanged);
        }
        Ok(())
    }
}

impl CalendarDeleteResult {
    pub fn validate_response(
        &self,
        request: &CalendarDelete,
        facts: &CalendarWriteFacts<'_>,
    ) -> Result<(), CalendarError> {
        self.validate()?;
        if self.native_instance_id != facts.native_instance_id
            || self.privacy_generation != facts.privacy_generation
            || self.calendar_item_id != request.calendar_item_id
            || self.occurrence_start_ms != request.occurrence_start_ms
            || self.span != request.span
        {
            return Err(CalendarError::ScopeChanged);
        }
        Ok(())
    }
}
