//! The app's orders to the assistant: a message, a new conversation or a
//! handover to another agent, and the answers to the agent's questions.
use super::*;

impl Runtime {
    /// Routes an order to the assistant: the conversation, its answers, its
    /// readings, the question during a meeting and the folders it may read.
    pub(super) async fn assistant_order(&mut self, command: Command) -> Result<()> {
        match command {
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
            off_loop_orders!()
            | device_orders!()
            | recording_orders!()
            | note_orders!()
            | agenda_orders!() => return Err(super::misrouted()),
        }
        Ok(())
    }

    /// A message from the app: refused while a voice turn runs, and checked
    /// against the hub's attachment ceilings before any frame leaves.
    pub(super) async fn chat_command(
        &mut self,
        text: String,
        attachments: Vec<orbit_core::attachments::Attachment>,
        primary: bool,
    ) -> Result<()> {
        if self.capture.buffer.voice.is_some() || self.assistant.voice_task.active().is_some() {
            bail!("Termina o interrumpe el turno actual antes de enviar otro mensaje");
        }
        // The message's files against the hub's ceilings, before any
        // frame leaves; a 0.9.0 hub takes none.
        orbit_core::attachments::admit_message(
            self.assistant.hub_attachments.as_ref(),
            &attachments,
        )?;
        // Nobody named: the primary answers, in a conversation of its
        // own when the open one is another agent's.
        let default = self.settings.roster().default_agent_id().to_owned();
        if primary && default != self.conversation_agent_id() {
            self.ensure_conversation_can_change()?;
            self.apply(Event::NewSession {
                agent_id: Some(default),
            })
            .await?;
        }
        self.apply(Event::Submit {
            id: id(),
            text,
            attachments,
        })
        .await?;
        Ok(())
    }

    /// A new conversation with an agent the hub listed.
    pub(super) async fn new_session_command(&mut self, agent_id: Option<String>) -> Result<()> {
        self.ensure_known_agent(agent_id.as_deref())?;
        self.ensure_conversation_can_change()?;
        self.apply(Event::NewSession { agent_id }).await?;
        Ok(())
    }

    /// The open conversation goes on with another agent, which takes it over
    /// with its first turn.
    pub(super) async fn handover_command(
        &mut self,
        session_id: String,
        agent_id: String,
        text: String,
    ) -> Result<()> {
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
        if self.capture.buffer.voice.is_some() || self.assistant.voice_task.active().is_some() {
            bail!("Termina o interrumpe el turno actual antes de seguir con otro asistente");
        }
        self.ensure_known_agent(Some(&agent_id))?;
        self.ensure_conversation_can_change()?;
        self.apply(Event::NewSession {
            agent_id: Some(agent_id),
        })
        .await?;
        self.assistant.handover_from = Some(session_id);
        let submitted = self
            .apply(Event::Submit {
                id: id(),
                text,
                attachments: Vec::new(),
            })
            .await;
        // Only the first turn carries it, sent or not.
        self.assistant.handover_from = None;
        submitted?;
        Ok(())
    }

    /// The user allows or denies what the agent asked, with one of the
    /// choices it offered.
    pub(super) async fn approval_command(
        &mut self,
        approved: bool,
        choice: Option<String>,
    ) -> Result<()> {
        let approval = self
            .state
            .durable
            .assistant
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
                bail!("La opción «{choice}» contradice approved={approved}; sólo «deny» rechaza");
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
        Ok(())
    }

    /// The user's written answer to the agent's question.
    pub(super) async fn clarify_command(
        &mut self,
        answer: String,
        question_id: Option<String>,
    ) -> Result<()> {
        let approval = self
            .state
            .durable
            .assistant
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
        Ok(())
    }

    /// The panel's «Preguntar a {principal}» names nobody: the primary answers.
    pub(super) async fn meeting_ask_command(
        &mut self,
        question: String,
        minutes: u16,
    ) -> Result<()> {
        let primary = self.settings.roster().default_agent_id().to_owned();
        self.ask_meeting(
            question,
            minutes,
            orbit_core::meeting_question::QuestionOrigin::Typed,
            Some(primary),
        )
        .await?;
        Ok(())
    }
}
