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
            DaemonEnded::Failed(cause) => anyhow::anyhow!("The Orbit service stopped: {cause}"),
            DaemonEnded::Vanished => {
                anyhow::anyhow!("The Orbit service thread ended without a result (a panic)")
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
                "The daemon did not confirm within {} ms; neither command nor read is repeated",
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
                            .unwrap_or("Operation refused without a cause")
                    );
                }
                Ok(response)
            }
            Reply::Topics(_)
            | Reply::VoiceNoteAudio(_)
            | Reply::Vault(_)
            | Reply::Agent(_)
            | Reply::Attachment(_) => {
                bail!("The daemon delivered another response kind to a command")
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
                    bail!("The daemon refused the read without a typed cause");
                }
                Ok(response)
            }
            // During recovery the actor answers a read with its typed refusal.
            Reply::Command(response) => match response.refusal {
                Some(refusal) => Err(refusal.into()),
                None => bail!("The daemon delivered a command instead of the read"),
            },
            Reply::VoiceNoteAudio(_) => bail!("The daemon delivered audio instead of the read"),
            Reply::Vault(_) => bail!("The daemon delivered a vault instead of the read"),
            Reply::Agent(_) => bail!("The daemon delivered an agent instead of the read"),
            Reply::Attachment(_) => bail!("The daemon delivered an attachment instead of the read"),
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
                            .unwrap_or("Audio refused without a cause")
                    );
                }
                response
                    .audio
                    .context("The daemon did not deliver the requested audio")
            }
            Reply::Command(response) => match response.refusal {
                Some(refusal) => Err(refusal.into()),
                None => bail!("The daemon delivered a command instead of audio"),
            },
            Reply::Topics(_) => bail!("The daemon delivered a read instead of audio"),
            Reply::Vault(_) => bail!("The daemon delivered a vault instead of the read"),
            Reply::Agent(_) => bail!("The daemon delivered an agent instead of audio"),
            Reply::Attachment(_) => bail!("The daemon delivered an attachment instead of audio"),
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
                None => bail!("The daemon delivered a command instead of an agent"),
            },
            Reply::Topics(_)
            | Reply::VoiceNoteAudio(_)
            | Reply::Attachment(_)
            | Reply::Vault(_) => {
                bail!("The daemon delivered another response kind for the agent")
            }
        }
    }

    /// One hub-owned vault call. Never retained or retried by the Mac.
    pub async fn vault(
        &self,
        call: orbit_core::vault::VaultCall,
    ) -> Result<orbit_core::vault::VaultOutcome> {
        match self.call(Command::Vault { call }, None).await? {
            Reply::Vault(response) => Ok(response.outcome),
            Reply::Command(response) => match response.refusal {
                Some(refusal) => Err(refusal.into()),
                None => bail!("The daemon refused the vault read without a cause"),
            },
            _ => bail!("The daemon delivered another response kind for the vault"),
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
                            .unwrap_or("Attachment refused without a cause")
                    );
                }
                Ok(response)
            }
            Reply::Command(response) => match response.refusal {
                Some(refusal) => Err(refusal.into()),
                None => bail!("The daemon delivered a command instead of an attachment"),
            },
            Reply::Topics(_) | Reply::VoiceNoteAudio(_) | Reply::Vault(_) | Reply::Agent(_) => {
                bail!("The daemon delivered another response kind instead of an attachment")
            }
        }
    }

    pub async fn attachment_content(&self, attachment_id: String) -> Result<AttachmentContent> {
        self.attachment(Command::AttachmentFetch { attachment_id })
            .await?
            .content
            .context("The daemon did not deliver the attachment")
    }
}
