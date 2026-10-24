//! The app's typed access to the daemon already inside its process: calls go
//! through a channel, never the socket, and wait as long as the socket would.
use super::*;
/// App-core's typed access to the actor already inside its process. Receivers
/// do not keep topic producers alive; the thread owner alone reports its end.
#[derive(Clone)]
pub struct LocalClient {
    calls: mpsc::Sender<Call>,
    topics: BTreeMap<TopicId, watch::Receiver<TopicFrame>>,
    ended: watch::Receiver<Option<DaemonEnded>>,
}

impl LocalClient {
    pub fn new(
        calls: mpsc::Sender<Call>,
        topics: &TopicChannels,
        ended: watch::Receiver<Option<DaemonEnded>>,
    ) -> Self {
        Self {
            calls,
            topics: topics
                .iter()
                .map(|(id, sender)| (*id, sender.subscribe()))
                .collect(),
            ended,
        }
    }

    pub fn topics(&self) -> BTreeMap<TopicId, watch::Receiver<TopicFrame>> {
        self.topics.clone()
    }

    pub async fn ended(&self) -> DaemonEnded {
        let mut ended = self.ended.clone();
        match ended.wait_for(Option::is_some).await {
            Ok(value) => value.clone().unwrap_or(DaemonEnded::Vanished),
            Err(_) => DaemonEnded::Vanished,
        }
    }

    async fn stopped(&self) -> anyhow::Error {
        match self.ended().await {
            DaemonEnded::Closed => anyhow::anyhow!("orbit_closing"),
            DaemonEnded::Failed(cause) => anyhow::anyhow!("El servicio de Orbit terminó: {cause}"),
            DaemonEnded::Vanished => {
                anyhow::anyhow!("El hilo del servicio de Orbit terminó sin resultado (un pánico)")
            }
        }
    }

    async fn call(&self, command: Command, op: Option<orbit_core::notice::Op>) -> Result<Reply> {
        self.call_within(command, op, Duration::from_secs(CALL_TIMEOUT_SECONDS))
            .await
    }

    async fn call_within(
        &self,
        command: Command,
        op: Option<orbit_core::notice::Op>,
        wait: Duration,
    ) -> Result<Reply> {
        let (reply, receive) = oneshot::channel();
        let call = Call {
            request: Request {
                v: PROTOCOL_VERSION,
                id: uuid::Uuid::new_v4().to_string(),
                op,
                body: command,
            },
            reply: ReplySender(reply),
        };
        // The channel already bounds admission at 32 calls. Keep the existing
        // ten-second confirmation window, including queue time, until the
        // caller decides whether a slow order may still be running. Never retry.
        tokio::time::timeout(wait, async {
            if self.calls.send(call).await.is_err() {
                return Err(self.stopped().await);
            }
            match receive.await {
                Ok(reply) => Ok(reply),
                Err(_) => Err(self.stopped().await),
            }
        })
        .await
        .with_context(|| {
            format!(
                "El daemon no confirmó en {} ms; no se repite la orden ni la lectura",
                wait.as_millis()
            )
        })?
    }

    pub async fn request_command(&self, command: Command) -> Result<Response> {
        self.request_command_as(command, new_op()).await
    }

    pub async fn request_command_as(
        &self,
        command: Command,
        op: orbit_core::notice::Op,
    ) -> Result<Response> {
        match self.call(command, Some(op)).await? {
            Reply::Command(response) => {
                if let Some(refusal) = response.refusal.clone() {
                    return Err(refusal.into());
                }
                if !response.ok {
                    bail!(
                        "{}",
                        response
                            .error
                            .as_deref()
                            .unwrap_or("Operación rechazada sin causa")
                    );
                }
                Ok(response)
            }
            Reply::Topics(_)
            | Reply::VoiceNoteAudio(_)
            | Reply::Agent(_)
            | Reply::Attachment(_) => {
                bail!("El daemon entregó otra clase de respuesta a una orden")
            }
        }
    }

    pub async fn read_topic(&self, read: ReadRequest) -> Result<TopicResponse> {
        match self.call(Command::Read { read }, None).await? {
            Reply::Topics(response) => {
                if let Some(refusal) = response.refusal.clone() {
                    return Err(refusal.into());
                }
                if !response.ok {
                    bail!("El daemon rechazó la lectura sin causa tipada");
                }
                Ok(response)
            }
            // During recovery the actor answers a read with its typed refusal.
            Reply::Command(response) => match response.refusal {
                Some(refusal) => Err(refusal.into()),
                None => bail!("El daemon entregó una orden en lugar de la lectura"),
            },
            Reply::VoiceNoteAudio(_) => bail!("El daemon entregó audio en lugar de la lectura"),
            Reply::Agent(_) => bail!("El daemon entregó un agente en lugar de la lectura"),
            Reply::Attachment(_) => bail!("El daemon entregó un adjunto en lugar de la lectura"),
        }
    }

    pub async fn voice_note_audio(&self, message_id: String) -> Result<VoiceNoteAudio> {
        match self
            .call(Command::VoiceNoteAudio { message_id }, None)
            .await?
        {
            Reply::VoiceNoteAudio(response) => {
                if let Some(refusal) = response.refusal {
                    return Err(refusal.into());
                }
                if !response.ok {
                    bail!(
                        "{}",
                        response
                            .error
                            .as_deref()
                            .unwrap_or("Audio rechazado sin causa")
                    );
                }
                response
                    .audio
                    .context("El daemon no entregó el audio solicitado")
            }
            Reply::Command(response) => match response.refusal {
                Some(refusal) => Err(refusal.into()),
                None => bail!("El daemon entregó una orden en lugar del audio"),
            },
            Reply::Topics(_) => bail!("El daemon entregó una lectura en lugar del audio"),
            Reply::Agent(_) => bail!("El daemon entregó un agente en lugar del audio"),
            Reply::Attachment(_) => bail!("El daemon entregó un adjunto en lugar del audio"),
        }
    }

    /// Ajustes › Agentes: one call to the hub through the daemon, and what
    /// came of it. The daemon answers when the hub does, within the same
    /// ten-second confirmation as every other call; a later answer is lost
    /// and the call is not repeated.
    pub async fn agent(
        &self,
        call: orbit_core::agent_config::AgentCall,
    ) -> Result<orbit_core::agent_config::AgentOutcome> {
        match self.call(Command::Agent { call }, None).await? {
            Reply::Agent(response) => Ok(response.outcome),
            // During recovery the actor answers any call with its refusal.
            Reply::Command(response) => match response.refusal {
                Some(refusal) => Err(refusal.into()),
                None => bail!("El daemon entregó una orden en lugar del agente"),
            },
            Reply::Topics(_) | Reply::VoiceNoteAudio(_) | Reply::Attachment(_) => {
                bail!("El daemon entregó otra clase de respuesta al agente")
            }
        }
    }

    /// One attachment order (`AttachmentAdd`, `AttachmentFetch`,
    /// `AttachmentSave`) and its answer, or its typed refusal.
    pub async fn attachment(&self, command: Command) -> Result<AttachmentResponse> {
        match self
            .call_within(command, Some(new_op()), ATTACHMENT_CALL_TIMEOUT)
            .await?
        {
            Reply::Attachment(response) => {
                if let Some(refusal) = response.refusal.clone() {
                    return Err(refusal.into());
                }
                if !response.ok {
                    bail!(
                        "{}",
                        response
                            .error
                            .as_deref()
                            .unwrap_or("Adjunto rechazado sin causa")
                    );
                }
                Ok(response)
            }
            Reply::Command(response) => match response.refusal {
                Some(refusal) => Err(refusal.into()),
                None => bail!("El daemon entregó una orden en lugar del adjunto"),
            },
            Reply::Topics(_) | Reply::VoiceNoteAudio(_) | Reply::Agent(_) => {
                bail!("El daemon entregó otra clase de respuesta en lugar del adjunto")
            }
        }
    }

    pub async fn attachment_content(&self, attachment_id: String) -> Result<AttachmentContent> {
        self.attachment(Command::AttachmentFetch { attachment_id })
            .await?
            .content
            .context("El daemon no entregó el adjunto")
    }
}
