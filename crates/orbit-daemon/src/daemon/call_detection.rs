//! What the daemon does with the call detection's decisions
//! (`orbit_core::call_detection`): it feeds it the helper's
//! `microphone_activity`, the user's answers and the time, reads the agenda
//! around a call, starts the recording linked to the call's event, and ends
//! one the call started when the detection says so.
//!
//! RAM only, except the user's preference, which the store keeps encrypted. A
//! restart knows nothing until the helper's first list.
use super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::{
    calendar::{CalendarOccurrence, CalendarWindow},
    call_detection::{
        CallApp, CallDetector, CallEffect, CallFacts, CallInput, CallPanel, CallPreference,
        EVENT_LOOKAHEAD_MS, FinishCause, LinkedEvent, choose_event,
    },
    protocol::MeetingAudio,
    topics::recording as topic,
    wire::native::NativeEvent,
};

/// The agenda around a call, as the calendar read it.
pub(super) struct CallEvents {
    pub(super) events: Vec<CalendarOccurrence>,
}

#[derive(Default)]
pub(super) struct CallDetectionHandle {
    pub(super) detector: CallDetector,
    /// «Al detectar una llamada». `Err` says why it could not be read; the
    /// detection then behaves as «Avisarme», which never records alone.
    pub(super) preference: Option<Result<CallPreference, String>>,
    /// The agenda around the last call application that appeared.
    pub(super) events: Option<CallEvents>,
    /// The event the recording a call started is linked to.
    pub(super) linked: Option<(String, LinkedEvent)>,
    /// A call's recording is being started: its capture is the call's.
    pub(super) starting: bool,
    /// Tests drive the detection's clock; production uses the wall clock.
    #[cfg(test)]
    pub(super) clock_ms: Option<i64>,
}

impl CallDetectionHandle {
    fn preference(&self) -> CallPreference {
        match &self.preference {
            Some(Ok(preference)) => *preference,
            Some(Err(_)) | None => CallPreference::Notify,
        }
    }
}

impl Runtime {
    fn call_facts(&self) -> CallFacts {
        CallFacts {
            now_ms: self.calls.call_now(),
            preference: self.calls.preference(),
            privacy_stopped: self.state.privacy_stopped(),
            recording_live: self.state.durable.capture.recording.as_ref().is_some_and(
                |recording| {
                    recording.ended_at_ms.is_none()
                        && !matches!(
                            recording.phase,
                            CapturePhase::Idle | CapturePhase::Interrupted
                        )
                },
            ),
        }
    }

    /// Reads the user's preference at startup. Never chosen is «Avisarme»; a
    /// preference that cannot be read is said, and the detection behaves as
    /// «Avisarme» until they choose again.
    pub(super) fn recover_call_preference(&mut self) {
        self.calls.preference = Some(match self.reader.load_call_preference() {
            Ok(preference) => Ok(preference.unwrap_or_default()),
            Err(error) => {
                let cause = format!(
                    "Could not read the video call preference ({error}); Orbit will notify without recording until it is chosen again."
                );
                self.say_failure(NoticeCode::CallPreferenceUnreadable, &error);
                Err(cause)
            }
        });
    }

    /// «Al detectar una llamada»: stored first, then in force.
    pub(super) async fn set_call_preference(&mut self, preference: CallPreference) -> Result<()> {
        self.store_actor
            .run(move |store| Ok(store.save_call_preference(preference)?))
            .await
            .context("Could not save the video call preference")?;
        self.calls.preference = Some(Ok(preference));
        Ok(())
    }

    /// One input to the detection, and its effects carried out. The first
    /// failure is returned after every effect ran: a failed start never keeps
    /// a countdown from ending its recording.
    pub(super) async fn apply_call(&mut self, input: CallInput) -> Result<()> {
        let facts = self.call_facts();
        let update = self.calls.detector.apply(input, &facts);
        let mut failure = None;
        for effect in update.effects {
            if let Err(error) = self.call_effect(effect).await {
                failure.get_or_insert(error);
            }
        }
        if let Some(refusal) = update.refusal {
            bail!("{refusal}");
        }
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(())
    }

    /// The helper's `microphone_activity`: who holds the microphone now.
    pub(super) async fn on_microphone_activity(&mut self, event: &Value) -> Result<()> {
        let decoded: NativeEvent = serde_json::from_value(event.clone())?;
        let NativeEvent::MicrophoneActivity { applications, .. } = decoded else {
            bail!("Evento microphone_activity incoherente");
        };
        let applications = applications
            .into_iter()
            .map(|application| CallApp {
                application_pid: application.application_pid,
                bundle_id: application.bundle_id,
                name: application.name,
            })
            .collect();
        if let Err(error) = self
            .apply_call(CallInput::Microphone { applications })
            .await
        {
            // A recording that could not start is said; the event itself was
            // read, and nothing else depends on it.
            let notice = super::refusal_notice(&error, NoticeCode::CallDetectionFailed);
            self.say_failure(notice, &error);
        }
        Ok(())
    }

    /// The helper went away or restarted: who holds the microphone is no
    /// longer known.
    pub(super) async fn call_signal_lost(&mut self) {
        if let Err(error) = self.apply_call(CallInput::SignalLost).await {
            let notice = super::refusal_notice(&error, NoticeCode::CallDetectionFailed);
            self.say_failure(notice, &error);
        }
    }

    /// The daemon's once-a-second upkeep: time passes, and a recording the
    /// call started that ended some other way is let go.
    pub(super) async fn maintain_call_detection(&mut self) -> Result<()> {
        if let Some(owned) = self.calls.detector.owned_recording() {
            let open = self
                .state
                .durable
                .capture
                .recording
                .as_ref()
                .is_some_and(|recording| recording.id == owned && recording.ended_at_ms.is_none());
            if !open {
                let recording_id = owned.to_owned();
                self.calls.linked = None;
                self.apply_call(CallInput::RecordingEnded { recording_id })
                    .await?;
            }
        }
        if let Err(error) = self.apply_call(CallInput::Tick).await {
            let notice = super::refusal_notice(&error, NoticeCode::CallDetectionFailed);
            self.say_failure(notice, &error);
        }
        Ok(())
    }

    /// The helper cut the capture (`capture_stopped`) with `code`. When the
    /// call's own application stopped (`system_audio_stream_stopped`) after
    /// leaving the call, the recording ends as «Terminar» with what was
    /// recorded instead of staying interrupted.
    pub(super) async fn call_capture_cut(&mut self, code: &str) {
        if code != "system_audio_stream_stopped" {
            return;
        }
        let Some(recording_id) = self.calls.detector.owned_recording().map(str::to_owned) else {
            return;
        };
        if let Err(error) = self
            .apply_call(CallInput::CaptureCut { recording_id })
            .await
        {
            let notice = super::refusal_notice(&error, NoticeCode::CallDetectionFailed);
            self.say_failure(notice, &error);
        }
    }

    async fn call_effect(&mut self, effect: CallEffect) -> Result<()> {
        match effect {
            CallEffect::ReadCalendar => {
                let now_ms = self.calls.call_now();
                self.read_calendar_for_call(CalendarWindow {
                    start_unix_ms: now_ms,
                    // Half-open: an event starting exactly at the edge counts.
                    end_unix_ms: now_ms.saturating_add(EVENT_LOOKAHEAD_MS + 1),
                });
                Ok(())
            }
            CallEffect::Start {
                call_id,
                app,
                automatic,
            } => self.start_call_recording(call_id, app, automatic).await,
            CallEffect::Finish {
                recording_id,
                app,
                cause,
            } => self.finish_call_recording(&recording_id, &app, cause).await,
        }
    }

    async fn start_call_recording(
        &mut self,
        call_id: String,
        app: CallApp,
        automatic: bool,
    ) -> Result<()> {
        let event = self.calls.call_event(&app);
        let title = call_title(&app, event.as_ref());
        self.calls.starting = true;
        let started = self
            .start_meeting(
                title,
                MeetingAudio::Application {
                    application_pid: app.application_pid,
                },
                event.clone(),
                "call_detection",
            )
            .await;
        self.calls.starting = false;
        match started {
            Ok(recording_id) => {
                self.calls.linked = event.map(|event| (recording_id.clone(), event));
                let facts = self.call_facts();
                self.calls.detector.apply(
                    CallInput::Started {
                        call_id,
                        recording_id,
                    },
                    &facts,
                );
                self.say(if automatic {
                    NoticeCode::CallRecordingStartedAutomatically
                } else {
                    NoticeCode::CallRecordingStarted
                });
                Ok(())
            }
            Err(error) => {
                let facts = self.call_facts();
                self.calls
                    .detector
                    .apply(CallInput::StartFailed { call_id }, &facts);
                Err(error.context(format!(
                    "Could not start recording the call in {}",
                    app.name
                )))
            }
        }
    }

    async fn finish_call_recording(
        &mut self,
        recording_id: &str,
        app: &CallApp,
        cause: FinishCause,
    ) -> Result<()> {
        self.calls.linked = None;
        let open = self
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .is_some_and(|recording| {
                recording.id == recording_id
                    && recording.ended_at_ms.is_none()
                    && recording.phase != CapturePhase::Finalizing
            });
        if !open {
            return Ok(());
        }
        self.apply(Event::FinishMeeting).await?;
        let _ = app;
        self.say(Notice::new(NoticeCode::CallRecordingFinished).with_token(
            NoticeKey::Cause,
            match cause {
                FinishCause::CallEnded => "call_ended",
                FinishCause::CallAppClosed => "call_app_closed",
            },
        ));
        Ok(())
    }

    /// The call detection for the recording topic: the preference and what
    /// the floating panel has to say. Nothing while privacy is stopped.
    pub(super) fn call_topic(&self) -> topic::Call {
        let (preference, preference_unreadable) = match &self.calls.preference {
            Some(Ok(preference)) => (Some(*preference), None),
            Some(Err(cause)) => (None, Some(cause.clone())),
            None => (None, None),
        };
        let now_ms = self.calls.call_now();
        let panel = if self.state.privacy_stopped() {
            None
        } else {
            self.calls.detector.panel(now_ms).map(|panel| match panel {
                CallPanel::Offer { call_id, app } => topic::CallPanel::Offer {
                    call_id,
                    event_title: self.calls.call_event(&app).map(|event| event.title),
                    application: app.name,
                },
                CallPanel::Recording { recording_id, app } => topic::CallPanel::Recording {
                    event_title: self
                        .calls
                        .linked
                        .as_ref()
                        .filter(|(linked, _)| linked == &recording_id)
                        .map(|(_, event)| event.title.clone()),
                    recording_id,
                    application: app.name,
                },
                CallPanel::Ending {
                    recording_id,
                    app,
                    ends_at_ms,
                } => topic::CallPanel::Ending {
                    recording_id,
                    application: app.name,
                    ends_at_ms,
                },
            })
        };
        topic::Call {
            preference,
            preference_unreadable,
            panel,
        }
    }
}

/// The recording's title: its event's, or «Llamada en {app}» when there is
/// no event or its title does not fit a recording's.
fn call_title(app: &CallApp, event: Option<&LinkedEvent>) -> String {
    event
        .map(|event| event.title.trim())
        .filter(|title| {
            !title.is_empty() && title.chars().count() <= orbit_core::MAX_MANIFEST_TITLE_CHARACTERS
        })
        .map_or_else(
            || {
                format!("Llamada en {}", app.name)
                    .chars()
                    .take(orbit_core::MAX_MANIFEST_TITLE_CHARACTERS)
                    .collect()
            },
            str::to_owned,
        )
}

impl CallDetectionHandle {
    /// The detection's clock: Unix milliseconds.
    pub(super) fn call_now(&self) -> i64 {
        #[cfg(test)]
        if let Some(now_ms) = self.clock_ms {
            return now_ms;
        }
        now()
    }

    /// The user's stop, a revocation or closing (the retire plan's slot): the
    /// offer and the recording the call started are forgotten, and nothing
    /// resumes on its own.
    pub(super) fn retire_call_detection(&mut self) {
        self.detector.retire();
        self.linked = None;
        self.events = None;
    }

    /// Who started the capture of `recording_id`, kept with its epoch: a
    /// call or the user.
    pub(super) fn capture_origin(&self, recording_id: &str) -> &'static str {
        if self.starting || self.detector.owned_recording() == Some(recording_id) {
            "call_detection"
        } else {
            "explicit_recording_start_or_resume"
        }
    }

    /// The event of the call `app` holds, from the agenda read for it.
    fn call_event(&self, app: &CallApp) -> Option<LinkedEvent> {
        let events = self.events.as_ref()?;
        choose_event(&events.events, &app.bundle_id, self.call_now())
            .map(LinkedEvent::from_occurrence)
    }

    /// The agenda read for a call arrived.
    pub(super) fn call_calendar_read(&mut self, events: Vec<CalendarOccurrence>) {
        self.events = Some(CallEvents { events });
    }
}

#[cfg(test)]
#[path = "call_detection_tests.rs"]
mod tests;
