//! Local command dispatch.
use super::*;
use orbit_core::MAX_MANIFEST_TITLE_CHARACTERS;

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

    /// Routes one order from the app to the domain that serves it, one arm
    /// per order. Orders served off the loop (topics, reads, agents,
    /// attachments, reminders, voice-note audio) never reach here.
    pub(super) async fn command(&mut self, command: Command) -> Result<()> {
        use orbit_core::call_detection::CallInput;
        match command {
            Command::SubscribeTopics { .. } | Command::Read { .. } | Command::Agent { .. } => {}
            Command::AttachmentAdd { .. }
            | Command::AttachmentFetch { .. }
            | Command::AttachmentSave { .. }
            | Command::ReminderWrite { .. }
            | Command::VoiceNoteAudio { .. } => {}
            Command::Pair {
                hub_url,
                code,
                device_name,
            } => self.pair_command(hub_url, code, device_name).await?,
            Command::Status => self.status_command()?,
            Command::ErrorReports { enabled } => self.set_error_reports(enabled)?,
            // Already in the app's report log: it only goes to the client.
            Command::Report { report } => self
                .error_reports
                .report(&report, std::time::Instant::now()),
            Command::PrivacyStop => {
                self.privacy_stop(orbit_core::privacy_stop::UserStop::Order)
                    .await
            }
            Command::Shutdown => self.shutdown_command().await?,
            Command::Rearm => self.rearm_command().await?,
            Command::Permissions { request } => self.permissions_command(request).await?,
            Command::SessionOpen { id, agent_id } => self.open_session(id, agent_id).await?,
            Command::Chat {
                text,
                attachments,
                primary,
            } => self.chat_command(text, attachments, primary).await?,
            Command::NewSession { agent_id } => self.new_session_command(agent_id).await?,
            Command::Handover {
                session_id,
                agent_id,
                text,
            } => self.handover_command(session_id, agent_id, text).await?,
            Command::Interrupt => {
                self.meeting_question_interrupted();
                self.apply(Event::Interrupt).await?;
            }
            Command::Approval { approved, choice } => {
                self.approval_command(approved, choice).await?
            }
            Command::Clarify {
                answer,
                question_id,
            } => self.clarify_command(answer, question_id).await?,
            Command::Speak { message_id, text } => self.speak(message_id, text)?,
            Command::SpeakStop => self.stop_speaking().await?,
            Command::MeetingAsk { question, minutes } => {
                self.meeting_ask_command(question, minutes).await?
            }
            Command::MeetingQuestionClose { id } => self.question.close_meeting_question(&id),
            Command::MeetingQuestionHold { id, held } => {
                self.question.hold_meeting_question(&id, held)
            }
            Command::AssistantFolderAdd => self.add_assistant_folder()?,
            Command::AssistantFolderRemove { id } => self.remove_assistant_folder(&id).await?,
            Command::VoiceStart => self.apply(Event::VoiceStart).await?,
            Command::VoiceEnd => self.apply(Event::VoiceEnd).await?,
            Command::VoiceCancel => self.apply(Event::VoiceCancel).await?,
            Command::VoiceRetry { message_id } => {
                self.retry_voice_transcription(message_id).await?
            }
            Command::VoiceDiscard { message_id } => self.discard_voice_note(message_id).await?,
            Command::CallStart { duration_minutes } => {
                self.call_start_command(duration_minutes).await?
            }
            Command::CallStop => self.apply(Event::CallStop).await?,
            Command::MeetingStart {
                title,
                audio,
                duration_minutes: _,
                event,
            } => self.meeting_start_command(title, audio, event).await?,
            Command::MeetingPause => self.apply(Event::PauseMeeting).await?,
            Command::MeetingResume => self.apply(Event::ResumeMeeting).await?,
            Command::MeetingFinish => self.apply(Event::FinishMeeting).await?,
            Command::MeetingDiscard { recording_id } => self.discard_meeting(recording_id).await?,
            Command::MeetingDelete { recording_id } => self.delete_meeting(recording_id).await?,
            Command::MicrophoneMute { muted } => self.mute_microphone(muted).await?,
            Command::AudioSourcesRefresh => self.refresh_audio_sources()?,
            Command::NotePrivateSave {
                recording_id,
                markdown,
                base_revision,
            } => {
                self.note_private_save(recording_id, markdown, base_revision)
                    .await?
            }
            Command::NoteLibrary { request } => self.note_library(request).await?,
            command @ (Command::TranscriptRetranscribe { .. }
            | Command::TranscriptRetry { .. }
            | Command::NoteEdit { .. }
            | Command::NoteRegenerate { .. }
            | Command::NoteRestoreSuperseded { .. }
            | Command::SpeakerRename { .. }
            | Command::MeetingLanguage { .. }
            | Command::MeetingRename { .. }) => self.note_command(command).await?,
            Command::Calendar { request } => self.calendar_command(request)?,
            Command::CallRecord { call_id } => {
                self.apply_call(CallInput::Record { call_id }).await?
            }
            Command::CallDismiss { call_id } => {
                self.apply_call(CallInput::Dismiss { call_id }).await?
            }
            Command::RecordingKeep { recording_id } => {
                self.apply_call(CallInput::Keep { recording_id }).await?
            }
            Command::CallPreference { preference } => self.set_call_preference(preference).await?,
        }
        Ok(())
    }
}
