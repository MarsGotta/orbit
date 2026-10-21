use super::*;
use crate::{CommandReceipt, ReceiptDelivery};

impl DaemonClient {
    pub async fn send(&self, command: AppCommand) -> Result<CommandReceipt, AppError> {
        self.send_as(command, orbit_ipc::new_op()).await
    }

    /// Sends one order on behalf of the action `op` (SIM-21): the daemon, its
    /// requests to the hub and every notice the order causes carry it.
    pub async fn send_as(
        &self,
        command: AppCommand,
        op: orbit_core::notice::Op,
    ) -> Result<CommandReceipt, AppError> {
        use crate::{
            AssistantCommand, LocalCommand, NoteCommand, PermissionsCommand, StatusCommand,
        };
        let command = match command {
            AppCommand::Notes(crate::NotesCommand::NotesRefresh) => {
                self.refresh_notes().await?;
                return Ok(self.local_receipt());
            }
            AppCommand::Notes(crate::NotesCommand::NotesMore) => {
                self.read_more_notes().await?;
                return Ok(self.local_receipt());
            }
            AppCommand::Assistant(AssistantCommand::VoiceNoteAudio { message_id }) => {
                self.send_voice_note_audio(&message_id).await?;
                return Ok(self.local_receipt());
            }
            AppCommand::Note(NoteCommand::NotesOpen { recording_id }) => {
                self.read_open_note(recording_id).await?;
                return Ok(self.local_receipt());
            }
            AppCommand::Note(command) => {
                crate::features::note::map_command(command, &self.local_snapshot())?
            }
            AppCommand::Agenda(command) => {
                self.send_calendar(command).await?;
                return Ok(self.local_receipt());
            }
            AppCommand::Local(LocalCommand::OnboardingDone) => {
                self.complete_onboarding();
                return Ok(self.local_receipt());
            }
            AppCommand::Local(LocalCommand::ProfileSet { name }) => {
                self.set_profile_name(name)?;
                return Ok(self.local_receipt());
            }
            AppCommand::Local(
                LocalCommand::LoginItem { .. }
                | LocalCommand::UpdateCheck
                | LocalCommand::UpdateInstall,
            )
            | AppCommand::Status(StatusCommand::Quit | StatusCommand::OpenUrl { .. })
            | AppCommand::Permissions(PermissionsCommand::OpenSystemPrivacy { .. }) => {
                return Err(AppError::desktop_only());
            }
            AppCommand::Recording(command) => {
                let local = self.local_snapshot();
                let topics = self.inner.topics.read().await;
                let clock = self.inner.calendar.lock().await.local_clock();
                let calendar = crate::features::agenda::project_with_read(
                    &crate::features::FeatureInput::with_clock(&local, &topics, clock.clone()),
                )
                .0;
                crate::features::recording::map_command(command, &calendar, &clock)?
            }
            AppCommand::Daemon(command) => *command,
        };
        if command.feature().is_none() {
            let value = serde_json::to_value(&command)
                .map_err(|error| AppError::daemon(error.to_string()))?;
            let name = value["command"]
                .as_str()
                .ok_or_else(|| AppError::daemon("La orden interna no lleva su nombre"))?;
            return Err(AppError {
                code: "internal_command".into(),
                message: format!("La interfaz no puede enviar la orden interna «{name}»."),
                refusal: None,
            });
        }
        let library = matches!(command, Command::NoteLibrary { .. });
        let receipt = self.send_daemon(command, op).await?;
        if library {
            self.read_library().await;
        }
        Ok(receipt)
    }

    pub fn local_receipt(&self) -> CommandReceipt {
        let mut frames = self
            .inner
            .frames
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        frames.observe(&self.inner.snapshots.borrow());
        CommandReceipt {
            revision: frames.revision(),
            delivery: Some(ReceiptDelivery::Reflected),
        }
    }

    async fn send_voice_note_audio(&self, message_id: &str) -> Result<AppSnapshot, AppError> {
        let audio = self
            .inner
            .daemon
            .voice_note_audio(message_id.to_owned())
            .await
            .map_err(|error| AppError::from_daemon(error.as_ref()))?;
        if audio.message_id != message_id || audio.format != "wav" {
            return Err(AppError::daemon("Respuesta de audio no válida"));
        }
        // SIM-30: the note may be of any conversation the screen read from
        // the hub; the app keeps the last one asked for, by its message id.
        let audio = crate::VoiceNoteAudioSnapshot {
            message_id: audio.message_id,
            format: audio.format,
            bytes_base64: audio.bytes_base64,
        };
        self.update_local(|snapshot| snapshot.chat.voice_audio = Some(audio));
        Ok(self.composed_snapshot())
    }

    async fn send_daemon(
        &self,
        command: Command,
        op: orbit_core::notice::Op,
    ) -> Result<CommandReceipt, AppError> {
        let calendar = matches!(command, Command::Calendar { .. });
        let privacy_stop = matches!(command, Command::PrivacyStop);
        let response = self
            .inner
            .daemon
            .request_command_as(command, op)
            .await
            .map_err(|error| {
                if calendar {
                    AppError::from_calendar(error.as_ref())
                } else {
                    AppError::from_daemon(error.as_ref())
                }
            })?;
        // Recovery durably acknowledges the stop before topic publication has
        // begun. An acknowledgement is not a claim that a frame was reflected.
        if privacy_stop
            && response.epoch.is_none()
            && response
                .revisions
                .as_ref()
                .is_some_and(|revisions| revisions.is_empty())
            && response.status.as_ref().is_some_and(|status| {
                status.privacy_stopped
                    && matches!(
                        status.recovery,
                        orbit_core::RecoveryStatus::Recovering { .. }
                    )
            })
        {
            let mut receipt = self.local_receipt();
            receipt.delivery = None;
            return Ok(receipt);
        }
        let revisions = response.revisions.ok_or_else(|| {
            AppError::daemon("La respuesta del daemon no lleva revisiones de temas.")
        })?;
        let epoch = response.epoch.ok_or_else(|| {
            AppError::daemon("La respuesta del daemon no lleva la época de los temas.")
        })?;
        // The actor publishes its watches before replying. Read their current
        // values directly: there is no independent socket delivery to wait for.
        self.observe_current_topics().await;
        let publisher = match self.inner.frames.lock() {
            Ok(publisher) => publisher,
            Err(poisoned) => poisoned.into_inner(),
        };
        let cache = &publisher.last_topics;
        let delivery = revisions
            .iter()
            .find_map(|(topic, expected)| {
                let observed = cache.topic(*topic).map(|head| head.revision);
                (cache.epoch() != Some(epoch.as_str())
                    || observed.is_none_or(|value| value < *expected))
                .then(|| ReceiptDelivery::FrameNotObserved {
                    topic: *topic,
                    expected_revision: *expected,
                    observed_revision: observed,
                    expected_epoch: epoch.clone(),
                    observed_epoch: cache.epoch().map(str::to_owned),
                    waited_ms: 0,
                })
            })
            .unwrap_or(ReceiptDelivery::Reflected);
        Ok(CommandReceipt {
            revision: publisher.revision(),
            delivery: Some(delivery),
        })
    }

    async fn observe_current_topics(&self) {
        let mut topics = self.inner.topics.write().await;
        for receiver in self.inner.daemon.topics().values() {
            topics.observe(receiver.borrow().clone());
        }
        drop(topics);
        subscription::publish_topic_change(
            &self.inner.snapshots,
            &self.inner.topics,
            &self.inner.frames,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::{
        protocol::Response,
        topics::{Knowledge, TopicData, TopicFrame, TopicId},
    };

    #[tokio::test]
    async fn receipt_reads_the_published_watch_without_a_subscription_task()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_owned()))?;
        let (sender, mut calls) = tokio::sync::mpsc::channel::<orbit_ipc::Call>(1);
        let (status, _) = watch::channel(TopicFrame {
            epoch: "receipt-owner".into(),
            revision: 1,
            data: TopicData::not_observed(TopicId::Status),
        });
        let endpoint =
            crate::client_fixture::channel(sender, [(TopicId::Status, status.clone())].into());
        let client = DaemonClient::connect(paths, endpoint.client.clone());
        client.inner.subscription_abort.abort();
        let actor = tokio::spawn(async move {
            let call = calls.recv().await.ok_or("command absent")?;
            status.send_replace(TopicFrame {
                epoch: "receipt-owner".into(),
                revision: 4,
                data: TopicData::Status(Knowledge::Present {
                    value: orbit_core::topics::status::Value {
                        assistant_name: None,
                        hub: orbit_core::Connection::Offline,
                        account: None,
                        privacy_stopped: true,
                        helper_connected: false,
                        error_reports: None,
                    },
                    as_of_ms: None,
                }),
            });
            call.reply
                .send(Response {
                    v: 1,
                    id: call.request.id,
                    ok: true,
                    status: None,
                    error: None,
                    refusal: None,
                    epoch: Some("receipt-owner".into()),
                    revisions: Some([(TopicId::Status, 4)].into()),
                })
                .map_err(|_| "receipt receiver absent")?;
            Ok::<_, &'static str>(())
        });
        let receipt = client
            .send(AppCommand::Daemon(Box::new(Command::PrivacyStop)))
            .await?;
        assert_eq!(receipt.delivery, Some(ReceiptDelivery::Reflected));
        assert_eq!(client.bootstrap().revision, receipt.revision);
        assert!(client.take_frames().iter().any(|frame| matches!(frame,
            crate::features::FeatureFrame::Status { view, .. } if view.privacy_stopped
        )));
        actor.await??;
        Ok(())
    }
}
