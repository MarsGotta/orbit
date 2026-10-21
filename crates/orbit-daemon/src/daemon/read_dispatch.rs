//! IPC read admission. The actor owns worker handles and the privacy epoch
//! through `Runtime::reads`.
use super::*;

/// SIM-30: a read the daemon answers with one page from the hub.
enum HubPage {
    Sessions(Option<orbit_core::topics::Cursor>),
    Messages(String, Option<orbit_core::topics::Cursor>),
}

impl Runtime {
    pub(super) fn serve_read(
        &mut self,
        read: orbit_core::reads::ReadRequest,
        request_id: String,
        reply: ipc::ReplySender,
        revisions: BTreeMap<orbit_core::topics::TopicId, u64>,
    ) {
        let command = Command::Read { read: read.clone() };
        let hub_page = if let orbit_core::reads::ReadRequest::Sessions { cursor } = &read {
            Some(HubPage::Sessions(cursor.clone()))
        } else if let orbit_core::reads::ReadRequest::SessionMessages { session_id, cursor } = &read
        {
            Some(HubPage::Messages(session_id.clone(), cursor.clone()))
        } else {
            None
        };
        if let Some(hub_page) = hub_page {
            self.say_when_conversations_need_the_hub();
            let context = reads::HubReadContext::capture(
                self,
                revisions
                    .get(&orbit_core::topics::TopicId::Assistant)
                    .copied()
                    .unwrap_or(0),
            );
            let voice_audio = self.voice_audio_reader.clone();
            let id = read.id();
            let mut privacy_epoch = self.reads.privacy_epoch_rx.clone();
            let started_at_epoch = *privacy_epoch.borrow_and_update();
            let log_root = self.paths.root.clone();
            self.reads.tasks.push(tokio::spawn(async move {
                let page = async move {
                    match hub_page {
                        HubPage::Sessions(cursor) => context.sessions(cursor).await,
                        HubPage::Messages(session_id, cursor) => {
                            context.messages(session_id, cursor, voice_audio).await
                        }
                    }
                };
                let answer = cancellable_remote_read(page, privacy_epoch, started_at_epoch).await;
                if reply
                    .send(topic_response(
                        request_id,
                        revisions,
                        Some(id),
                        Some(answer),
                    ))
                    .is_err()
                {
                    crate::diagnostics::DiagnosticLog::in_root(&log_root)
                        .record("ipc_reply_receiver_gone", &[("command", "read")]);
                }
            }));
            return;
        }

        // SIM-41: the notes are the hub's; one remote read each, off the loop.
        if let Some(remote) = self.notes_read(&read) {
            let mut privacy_epoch = self.reads.privacy_epoch_rx.clone();
            let started_at_epoch = *privacy_epoch.borrow_and_update();
            let log_root = self.paths.root.clone();
            let id = read.id();
            self.reads.tasks.push(tokio::spawn(async move {
                let mut reply = reply;
                tokio::select! {
                    _ = reply.closed() => {
                        crate::diagnostics::DiagnosticLog::in_root(&log_root)
                            .record("ipc_reply_receiver_gone", &[("command", "read")]);
                    },
                    answer = cancellable_remote_read(remote, privacy_epoch, started_at_epoch) => {
                        if reply.send(topic_response(request_id, revisions, Some(id), Some(answer))).is_err() {
                            crate::diagnostics::DiagnosticLog::in_root(&log_root)
                                .record("ipc_reply_receiver_gone", &[("command", "read")]);
                        }
                    }
                }
            }));
            return;
        }

        let revision = revisions
            .get(&orbit_core::topics::TopicId::Agenda)
            .copied()
            .unwrap_or(0);
        let preliminary = reads::dispatch(self, read.clone(), revision);
        record_ipc_reply_loss(
            reply.send(topic_response(
                request_id,
                revisions,
                Some(read.id()),
                Some(preliminary),
            )),
            &command,
            self.diagnostics.as_ref(),
        );
    }

    /// SIM-30: the conversations are the hub's. Without a link to it there is
    /// nothing to read, and Mars is told what is missing; her own stop says
    /// itself.
    pub(super) fn say_when_conversations_need_the_hub(&mut self) {
        if matches!(
            self.state.connection(),
            Connection::Offline | Connection::Connecting
        ) && !self.state.privacy_stopped()
        {
            self.say(orbit_core::notice::NoticeCode::ConversationsNeedHub);
        }
    }
}
