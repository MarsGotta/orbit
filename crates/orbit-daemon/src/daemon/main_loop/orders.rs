//! The app's orders: served on the loop or handed to whoever answers them
//! off it, and answered with the topics' revisions they produced.
use super::*;

/// Reads, Ajustes › Agentes and the attachment orders change nothing of the
/// daemon's state: they are answered off the loop.
fn off_loop(command: &Command) -> bool {
    matches!(
        command,
        Command::SubscribeTopics { .. }
            | Command::Read { .. }
            | Command::Agent { .. }
            | Command::AttachmentAdd { .. }
            | Command::AttachmentFetch { .. }
            | Command::AttachmentSave { .. }
    )
}

impl MainLoop<'_> {
    pub(super) async fn on_call(&mut self, call: Call) {
        self.turns.commands += 1;
        self.runtime
            .turn_watch
            .begin(turn_watch::TurnSource::command(&call.request.body));
        // The order's op, or a new one for a client that sent none:
        // everything this turn says carries it.
        self.runtime.begin_turn(
            call.request.op.clone().unwrap_or_else(orbit_ipc::new_op),
            orbit_core::notice::ReportSource::Daemon,
        );
        let command = call.request.body.clone();
        // Reads, Ajustes › Agentes and the attachment orders change nothing
        // of the daemon's state; they are answered off the loop.
        let served_off_loop = off_loop(&command);
        if !served_off_loop {
            self.process.topic_board.state_changed();
        }
        let result = if served_off_loop {
            Ok(())
        } else {
            self.closing.command(command.clone(), self.runtime).await
        };
        // The transport has already attached the subscription's receivers.
        // Refresh their dirty heads before waking the caller, which
        // otherwise can accept an older turn as current.
        if !served_off_loop || matches!(&command, Command::SubscribeTopics { .. }) {
            self.runtime.report_notices();
            let now_ms = self.now_ms();
            let source = topics::TopicSource {
                state: &self.runtime.state,
                runtime: Some(&*self.runtime),
                now_ms,
            };
            self.process.topic_board.publish_command(&source);
        }
        Box::pin(self.reply(call, &command, result)).await;
    }

    /// The order's answer, or its handing to whoever answers it off the loop.
    async fn reply(&mut self, call: Call, command: &Command, result: Result<()>) {
        let id = call.request.id;
        let reply = call.reply;
        let runtime = &mut *self.runtime;
        let board = &self.process.topic_board;
        if let Command::Read { read } = command {
            runtime.serve_read(read.clone(), id, reply, board.revisions());
        } else if let Command::Agent { call } = command {
            runtime.serve_agent(call.clone(), id, reply);
        } else if matches!(
            command,
            Command::AttachmentAdd {
                source: orbit_core::attachments::AttachmentSource::Window
            }
        ) {
            // The window is captured on the loop, as the assistant's are.
            runtime.attach_window(id, reply).await;
        } else if matches!(
            command,
            Command::AttachmentAdd { .. }
                | Command::AttachmentFetch { .. }
                | Command::AttachmentSave { .. }
        ) {
            runtime.serve_attachment(command.clone(), id, reply);
        } else if matches!(command, Command::SubscribeTopics { .. }) {
            let response = topic_response(id, board.revisions(), None, None);
            record_ipc_reply_loss(reply.send(response), command, runtime.diagnostics.as_ref());
        } else if let Command::VoiceNoteAudio { message_id } = command {
            let audio = result.and_then(|()| runtime.voice_note_audio(message_id));
            let response = VoiceNoteAudioResponse {
                v: 1,
                id,
                ok: audio.is_ok(),
                error: audio.as_ref().err().map(ToString::to_string),
                refusal: audio.as_ref().err().map(refusal_from_error),
                audio: audio.ok(),
            };
            record_ipc_reply_loss(reply.send(response), command, runtime.diagnostics.as_ref());
        } else {
            let refusal = result.err().map(|error| refusal_from_error(&error));
            let status = matches!(command, Command::Status | Command::PrivacyStop)
                .then(|| DaemonStatus::from(&runtime.state));
            let epoch = Some(board.epoch().to_owned());
            let response = command_response(id, status, refusal, epoch, board.revisions());
            record_ipc_reply_loss(reply.send(response), command, runtime.diagnostics.as_ref());
        }
    }
}
