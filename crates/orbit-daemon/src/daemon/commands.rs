//! Local command dispatch.
use super::*;
use orbit_core::MAX_MANIFEST_TITLE_CHARACTERS;

/// The orders of each domain, as patterns. `Runtime::command` sends each
/// group to its domain's routing function, which serves every order of its
/// group and names the other groups as not its own. With no wildcard arm, an
/// order added to the protocol does not compile until a group takes it, and
/// a group's new order does not compile until its domain serves it.
macro_rules! off_loop_orders {
    () => {
        Command::SubscribeTopics { .. }
            | Command::Read { .. }
            | Command::Agent { .. }
            | Command::AttachmentAdd { .. }
            | Command::AttachmentFetch { .. }
            | Command::AttachmentSave { .. }
            | Command::ReminderWrite { .. }
            | Command::VoiceNoteAudio { .. }
    };
}

macro_rules! device_orders {
    () => {
        Command::Pair { .. }
            | Command::Status
            | Command::ErrorReports { .. }
            | Command::Report { .. }
            | Command::PrivacyStop
            | Command::Shutdown
            | Command::Rearm
            | Command::Permissions { .. }
    };
}

macro_rules! assistant_orders {
    () => {
        Command::SessionOpen { .. }
            | Command::Chat { .. }
            | Command::NewSession { .. }
            | Command::Handover { .. }
            | Command::Interrupt
            | Command::Approval { .. }
            | Command::Clarify { .. }
            | Command::Speak { .. }
            | Command::SpeakStop
            | Command::MeetingAsk { .. }
            | Command::MeetingQuestionClose { .. }
            | Command::MeetingQuestionHold { .. }
            | Command::AssistantFolderAdd
            | Command::AssistantFolderRemove { .. }
    };
}

macro_rules! recording_orders {
    () => {
        Command::VoiceStart
            | Command::VoiceEnd
            | Command::VoiceCancel
            | Command::VoiceRetry { .. }
            | Command::VoiceDiscard { .. }
            | Command::CallStart { .. }
            | Command::CallKeep
            | Command::MeetingKeep
            | Command::CallStop
            | Command::MeetingStart { .. }
            | Command::MeetingPause
            | Command::MeetingResume
            | Command::MeetingFinish
            | Command::MeetingDiscard { .. }
            | Command::MicrophoneMute { .. }
            | Command::AudioSourcesRefresh
    };
}

macro_rules! note_orders {
    () => {
        Command::MeetingDelete { .. }
            | Command::NotePrivateSave { .. }
            | Command::NoteLibrary { .. }
            | Command::TranscriptRetranscribe { .. }
            | Command::TranscriptRetry { .. }
            | Command::NoteEdit { .. }
            | Command::NoteRegenerate { .. }
            | Command::NoteRestoreSuperseded { .. }
            | Command::SpeakerRename { .. }
            | Command::MeetingLanguage { .. }
            | Command::MeetingRename { .. }
    };
}

macro_rules! agenda_orders {
    () => {
        Command::Calendar { .. }
            | Command::CallRecord { .. }
            | Command::CallDismiss { .. }
            | Command::RecordingKeep { .. }
            | Command::CallPreference { .. }
    };
}

#[path = "commands_assistant.rs"]
mod assistant;
#[path = "commands_audio.rs"]
mod audio;
#[path = "commands_device.rs"]
mod device;

impl Runtime {
    /// Starts a recording: by hand, from «Grabar» on an event, or for a call
    /// detected by the Mac. `event` links it to its calendar event by stable reference;
    /// the manifest keeps it, and the note asks the hub with it. Returns the
    /// recording's id.
    pub(super) async fn start_meeting(
        &mut self,
        title: String,
        audio: orbit_core::protocol::MeetingAudio,
        event: Option<orbit_core::call_detection::LinkedEvent>,
        origin: &'static str,
    ) -> Result<String> {
        // The over-long title is its own refusal, with its length
        // and the maximum: whoever reads it can fix the title in one
        // step, which the old `InvalidMeeting` never allowed.
        if title.trim().is_empty()
            || title.chars().count() > MAX_MANIFEST_TITLE_CHARACTERS
            || matches!(
                audio,
                orbit_core::protocol::MeetingAudio::Application {
                    application_pid: i32::MIN..=0
                }
            )
        {
            if title.chars().count() > MAX_MANIFEST_TITLE_CHARACTERS {
                return Err(orbit_core::refusal::Refusal::InvalidParameter {
                    parameter: orbit_core::refusal::Parameter::Title,
                    expected: orbit_core::refusal::Expected::TitleLength {
                        characters: title.chars().count() as u64,
                        max_characters: MAX_MANIFEST_TITLE_CHARACTERS as u64,
                    },
                }
                .into());
            }
            return Err(orbit_core::refusal::Refusal::InvalidParameter {
                parameter: orbit_core::refusal::Parameter::Meeting,
                expected: orbit_core::refusal::Expected::ValidMeeting,
            }
            .into());
        }
        if let Some(event) = &event {
            event
                .check()
                .context("No se empezó a grabar: el evento enlazado no es válido")?;
        }
        let recording_id = id();
        self.apply(Event::StartMeeting {
            id: recording_id.clone(),
            title: title.clone(),
            audio,
        })
        .await?;
        let application_pid = match audio {
            orbit_core::protocol::MeetingAudio::Application { application_pid } => {
                Some(application_pid)
            }
            orbit_core::protocol::MeetingAudio::System
            | orbit_core::protocol::MeetingAudio::MicrophoneOnly => None,
        };
        let mut payload = json!({"recording_id":recording_id,"title":title,"application_pid":application_pid,"origin":origin});
        if let Some(event) = event {
            payload["event"] = serde_json::to_value(event)?;
        }
        let job = Job {
            id: format!("recording-{recording_id}"),
            kind: orbit_store::QueueKind::RecordingManifest.into(),
            payload,
            state: JobState::Succeeded,
            created_at: now(),
        };
        self.store_actor
            .run(move |store| Ok(store.enqueue_job(&job)?))
            .await?;
        Ok(recording_id)
    }

    /// A conversation is with an agent the hub listed (the last list it
    /// gave). Refused with the id asked for and the ones there are, as the
    /// hub's own `agent_not_found` would say.
    pub(super) fn ensure_known_agent(&self, agent_id: Option<&str>) -> Result<()> {
        let Some(agent_id) = agent_id else {
            return Ok(());
        };
        let roster = self.settings.roster();
        if roster.agent(agent_id).is_none() {
            let known = roster
                .agents()
                .iter()
                .map(|agent| agent.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "El hub no tiene el agente «{agent_id}»; tiene {} ({known})",
                roster.agents().len()
            );
        }
        Ok(())
    }

    pub(super) fn ensure_conversation_can_change(&self) -> Result<()> {
        use orbit_core::refusal::{Operation, Refusal};
        let operation = if self.state.durable.assistant.active_request.is_some() {
            Some(Operation::Chat)
        } else if self.capture.buffer.voice.is_some() {
            Some(Operation::Audio)
        } else if self.assistant.voice_task.active().is_some() {
            Some(Operation::VoiceTranscription)
        } else {
            None
        };
        if let Some(operation) = operation {
            return Err(Refusal::Busy {
                operation,
                retry_after_ms: None,
            }
            .into());
        }
        Ok(())
    }

    /// «Enviar informes de errores anónimos»: stored with the other
    /// settings first, then in force at once.
    pub(super) fn set_error_reports(&mut self, enabled: bool) -> Result<()> {
        let settings = Settings {
            error_reports: enabled,
            ..self.settings.clone()
        };
        self.paths
            .save_settings(&settings)
            .context("No se pudo guardar «Enviar informes de errores anónimos»")?;
        self.settings = settings;
        self.error_reports.set_enabled(enabled);
        Ok(())
    }

    /// Routes one order from the app to the domain that serves it. Orders
    /// served off the loop (topics, reads, agents, attachments, reminders,
    /// voice-note audio) never reach here.
    pub(super) async fn command(&mut self, command: Command) -> Result<()> {
        match command {
            off_loop_orders!() => Ok(()),
            command @ device_orders!() => self.device_order(command).await,
            command @ assistant_orders!() => self.assistant_order(command).await,
            command @ recording_orders!() => self.recording_order(command).await,
            command @ note_orders!() => self.notes_order(command).await,
            command @ agenda_orders!() => self.agenda_order(command).await,
        }
    }

    /// The orders on the meetings' notes and their deletion.
    async fn notes_order(&mut self, command: Command) -> Result<()> {
        match command {
            Command::MeetingDelete { recording_id } => self.delete_meeting(recording_id).await,
            Command::NotePrivateSave {
                recording_id,
                markdown,
                base_revision,
            } => {
                self.note_private_save(recording_id, markdown, base_revision)
                    .await
            }
            Command::NoteLibrary { request } => self.note_library(request).await,
            command @ (Command::TranscriptRetranscribe { .. }
            | Command::TranscriptRetry { .. }
            | Command::NoteEdit { .. }
            | Command::NoteRegenerate { .. }
            | Command::NoteRestoreSuperseded { .. }
            | Command::SpeakerRename { .. }
            | Command::MeetingLanguage { .. }
            | Command::MeetingRename { .. }) => self.note_command(command).await,
            off_loop_orders!()
            | device_orders!()
            | assistant_orders!()
            | recording_orders!()
            | agenda_orders!() => Err(misrouted()),
        }
    }

    /// The orders on the agenda and on the calls the Mac detects.
    async fn agenda_order(&mut self, command: Command) -> Result<()> {
        use orbit_core::call_detection::CallInput;
        match command {
            Command::Calendar { request } => self.calendar_command(request),
            Command::CallRecord { call_id } => self.apply_call(CallInput::Record { call_id }).await,
            Command::CallDismiss { call_id } => {
                self.apply_call(CallInput::Dismiss { call_id }).await
            }
            Command::RecordingKeep { recording_id } => {
                self.apply_call(CallInput::Keep { recording_id }).await
            }
            Command::CallPreference { preference } => self.set_call_preference(preference).await,
            off_loop_orders!()
            | device_orders!()
            | assistant_orders!()
            | recording_orders!()
            | note_orders!() => Err(misrouted()),
        }
    }
}

/// An order sent to a domain that does not serve it: `command` routes each
/// group to its own domain, so only a routing mistake gets here.
fn misrouted() -> anyhow::Error {
    anyhow::anyhow!("Esta orden no es de esta parte de Orbit")
}
