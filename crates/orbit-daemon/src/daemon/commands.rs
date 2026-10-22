//! Local command dispatch.
use super::recording_notes::NoteOrder;
use super::*;
use orbit_core::MAX_MANIFEST_TITLE_CHARACTERS;

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
        let operation = if self.state.durable.active_request.is_some() {
            Some(Operation::Chat)
        } else if self.capture.buffer.voice.is_some() {
            Some(Operation::Audio)
        } else if self.conversation.voice_task.active().is_some() {
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

    pub(super) async fn command(&mut self, command: Command) -> Result<()> {
        match command {
            // Served by the IPC loop, off the actor (`serve_agent`).
            Command::SubscribeTopics { .. } | Command::Read { .. } | Command::Agent { .. } => {}
            Command::Pair {
                hub_url,
                code,
                device_name,
            } => {
                self.retire_device_request_reads().await?;
                self.pair_device(hub_url, code, device_name).await?;
            }
            Command::SessionOpen { id, agent_id } => self.open_session(id, agent_id).await?,
            Command::TranscriptRetranscribe { recording_id } => {
                self.note_order(recording_id, NoteOrder::Retranscribe)?
            }
            Command::TranscriptRetry { recording_id } => {
                self.note_order(recording_id, NoteOrder::RetryTranscript)?
            }
            Command::NotePrivateSave {
                recording_id,
                markdown,
                base_revision,
            } => {
                self.note_private_save(recording_id, markdown, base_revision)
                    .await?
            }
            Command::NoteEdit {
                recording_id,
                sections,
                done,
                base_revision,
            } => self.note_order(
                recording_id,
                NoteOrder::Edit {
                    sections,
                    done,
                    base_revision,
                },
            )?,
            Command::NoteRegenerate {
                recording_id,
                template_id,
            } => self.note_order(recording_id, NoteOrder::Write { template_id })?,
            Command::NoteRestoreSuperseded { recording_id } => {
                self.note_order(recording_id, NoteOrder::RestoreSuperseded)?
            }
            Command::SpeakerRename {
                recording_id,
                key,
                name,
                scope,
            } => self.note_order(recording_id, NoteOrder::Rename { key, name, scope })?,
            Command::MeetingLanguage {
                recording_id,
                language,
            } => self.note_order(recording_id, NoteOrder::Language { language })?,
            Command::MeetingRename {
                recording_id,
                title,
            } => self.note_order(recording_id, NoteOrder::Title { title })?,
            Command::NoteLibrary { request } => self.note_library(request),
            Command::Status => {
                // Apply receipts that already exist, but never wait here for a
                // future disk receipt: this command runs inside the select loop
                // that owes the IPC response. The projection still reports only
                // durable seals; a later Status observes later receipts.
                self.collect_ready_seals(sealing::OPPORTUNISTIC_RECEIPT_WAIT, false)?;
            }
            Command::Calendar { request } => self.calendar_command(request)?,
            Command::AudioSourcesRefresh => self.refresh_audio_sources()?,
            Command::CallRecord { call_id } => {
                self.apply_call(orbit_core::call_detection::CallInput::Record { call_id })
                    .await?
            }
            Command::CallDismiss { call_id } => {
                self.apply_call(orbit_core::call_detection::CallInput::Dismiss { call_id })
                    .await?
            }
            Command::RecordingKeep { recording_id } => {
                self.apply_call(orbit_core::call_detection::CallInput::Keep { recording_id })
                    .await?
            }
            Command::CallPreference { preference } => self.set_call_preference(preference).await?,
            Command::ErrorReports { enabled } => self.set_error_reports(enabled)?,
            Command::AssistantFolderAdd => self.add_assistant_folder()?,
            Command::AssistantFolderRemove { id } => self.remove_assistant_folder(&id).await?,
            // Already in the app's report log: it only goes to the client.
            Command::Report { report } => self
                .error_reports
                .report(&report, std::time::Instant::now()),
            Command::MeetingAsk { question, minutes } => {
                self.ask_meeting(
                    question,
                    minutes,
                    orbit_core::meeting_question::QuestionOrigin::Typed,
                    None,
                )
                .await?
            }
            Command::MeetingQuestionClose { id } => self.close_meeting_question(&id),
            Command::Chat { text, attachments } => {
                if self.capture.buffer.voice.is_some()
                    || self.conversation.voice_task.active().is_some()
                {
                    bail!("Termina o interrumpe el turno actual antes de enviar otro mensaje");
                }
                // The message's files against the hub's ceilings, before any
                // frame leaves; a 0.9.0 hub takes none.
                orbit_core::attachments::admit_message(
                    self.conversation.hub_attachments.as_ref(),
                    &attachments,
                )?;
                self.apply(Event::Submit {
                    id: id(),
                    text,
                    attachments,
                })
                .await?;
            }
            // Served off the loop by `serve_attachment`, never here.
            Command::AttachmentAdd { .. }
            | Command::AttachmentFetch { .. }
            | Command::AttachmentSave { .. } => {}
            Command::NewSession { agent_id } => {
                self.ensure_known_agent(agent_id.as_deref())?;
                self.ensure_conversation_can_change()?;
                self.apply(Event::NewSession { agent_id }).await?;
            }
            Command::Handover {
                session_id,
                agent_id,
                text,
            } => {
                if self
                    .settings
                    .hub_agents
                    .as_ref()
                    .and_then(|roster| roster.addable())
                    .is_none()
                {
                    bail!(
                        "Este hub no pasa conversaciones a otro asistente: actualízalo para seguir con otro"
                    );
                }
                if self.capture.buffer.voice.is_some()
                    || self.conversation.voice_task.active().is_some()
                {
                    bail!(
                        "Termina o interrumpe el turno actual antes de seguir con otro asistente"
                    );
                }
                self.ensure_known_agent(Some(&agent_id))?;
                self.ensure_conversation_can_change()?;
                self.apply(Event::NewSession {
                    agent_id: Some(agent_id),
                })
                .await?;
                self.conversation.handover_from = Some(session_id);
                let submitted = self
                    .apply(Event::Submit {
                        id: id(),
                        text,
                        attachments: Vec::new(),
                    })
                    .await;
                // Only the first turn carries it, sent or not.
                self.conversation.handover_from = None;
                submitted?;
            }
            Command::Interrupt => {
                self.meeting_question_interrupted();
                self.apply(Event::Interrupt).await?;
            }
            Command::PrivacyStop => {
                self.privacy_stop(orbit_core::privacy_stop::UserStop::Order)
                    .await
            }
            Command::Shutdown => {
                self.close_for_exit(
                    orbit_core::privacy_stop::StopCause::Shutdown,
                    "Orbit se está cerrando a petición del servicio.",
                )
                .await;
                self.retry_seals()?;
                // The single pending list is the only gate; every vertical
                // that still holds work is named through `has_any_pending`.
                let pending = self.has_any_pending();
                if !pending.is_empty() {
                    bail!(
                        "Orbit no sale todavía: queda pendiente de guardar {}; la captura ya está cortada y lo recibido se conserva",
                        pending.join(", ")
                    );
                }
            }
            Command::Rearm => {
                if self.state.capabilities().native_helper() {
                    // Hand it to the helper rather than refusing: this command
                    // arrived on this user's own 0600 socket on this Mac, which
                    // is the presence the latch actually requires, and the menu
                    // bar is only one way to supply it. The helper owns the
                    // durable latch, so it answers with a `local_rearm` event
                    // and that single path rearms the daemon. Remote traffic
                    // still cannot reach this socket.
                    self.native_send("rearm", json!({})).await?;
                    return Ok(());
                }
                self.apply(Event::LocalRearm).await?;
                self.connect();
            }
            Command::VoiceStart => self.apply(Event::VoiceStart).await?,
            Command::VoiceEnd => self.apply(Event::VoiceEnd).await?,
            Command::VoiceCancel => self.apply(Event::VoiceCancel).await?,
            Command::VoiceRetry { message_id } => {
                self.retry_voice_transcription(message_id).await?
            }
            Command::Speak { message_id, text } => self.speak(message_id, text)?,
            Command::SpeakStop => self.stop_speaking().await?,
            Command::VoiceNoteAudio { .. } => {}
            Command::CallStart { duration_minutes } => {
                self.apply(Event::CallStart {
                    id: id(),
                    duration_ms: i64::from(duration_minutes) * 60_000,
                    origin: orbit_core::CallOrigin::App,
                })
                .await?
            }
            Command::CallStop => self.apply(Event::CallStop).await?,
            Command::MeetingStart {
                title,
                audio,
                duration_minutes: _,
                event,
            } => {
                // «Grabar» on an event during a call records the
                // call's application when the helper says which one holds the
                // microphone; otherwise the system, as before.
                let audio = match (&event, audio) {
                    (Some(_), orbit_core::protocol::MeetingAudio::System) => self
                        .features
                        .call
                        .detector
                        .holding_call()
                        .map_or(audio, |app| {
                            orbit_core::protocol::MeetingAudio::Application {
                                application_pid: app.application_pid,
                            }
                        }),
                    _ => audio,
                };
                self.start_meeting(title, audio, event, "explicit").await?;
            }
            Command::MeetingPause => self.apply(Event::PauseMeeting).await?,
            Command::MeetingResume => self.apply(Event::ResumeMeeting).await?,
            Command::MeetingFinish => self.apply(Event::FinishMeeting).await?,
            Command::Approval { approved, choice } => {
                let approval = self
                    .state
                    .durable
                    .pending_approval
                    .as_ref()
                    .context("No hay ninguna aprobación pendiente")?;
                if approval.clarification {
                    bail!("Este turno necesita una respuesta escrita");
                }
                let mut params = json!({"session_id":self.state.session_id(),"request_id":approval.request_id,"approved":approved});
                if let Some(choice) = choice {
                    // `approved:false` is what reaches the hub's safety
                    // ledger: a choice never says the opposite of it.
                    if (choice == "deny") == approved {
                        bail!(
                            "La opción «{choice}» contradice approved={approved}; sólo «deny» rechaza"
                        );
                    }
                    let offered = if approval.choices.is_empty() {
                        vec!["once".to_owned(), "deny".to_owned()]
                    } else {
                        approval.choices.clone()
                    };
                    if !offered.contains(&choice) {
                        bail!(
                            "El agente no ofrece «{choice}»; ofrece {}: {}",
                            offered.len(),
                            offered.join(", ")
                        );
                    }
                    params["choice"] = json!(choice);
                }
                self.respond_interaction("approval.respond", params).await?;
            }
            Command::Clarify {
                answer,
                question_id,
            } => {
                let approval = self
                    .state
                    .durable
                    .pending_approval
                    .as_ref()
                    .context("No hay ninguna aclaración pendiente")?;
                if !approval.clarification {
                    bail!("Este turno necesita permitir o rechazar");
                }
                let mut params = json!({"session_id":self.state.session_id(),"request_id":approval.request_id,"answer":answer});
                if let Some(question_id) = question_id {
                    if !approval
                        .questions
                        .iter()
                        .any(|question| question.qid == question_id)
                    {
                        bail!(
                            "La pregunta «{question_id}» no está pendiente; quedan {}",
                            approval.questions.len()
                        );
                    }
                    params["question_id"] = json!(question_id);
                }
                self.respond_interaction("clarify.respond", params).await?;
            }
            Command::Permissions { request } => {
                if request
                    .iter()
                    .any(|permission| permission == crate::permissions::CALENDARS)
                {
                    if request != [crate::permissions::CALENDARS] {
                        bail!(
                            "Solicita el permiso de calendario por separado de los demás permisos"
                        );
                    }
                    self.calendar_command(
                        orbit_core::calendar_control::CalendarCommand::RequestPermission,
                    )?;
                    return Ok(());
                }
                self.native_send("permissions", json!({"request":request}))
                    .await?
            }
        }
        Ok(())
    }
}
