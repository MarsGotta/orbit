//! «Adjuntar ventana»: the frontmost window that is not Orbit's,
//! captured exactly as Orbit captures one when the assistant asks
//! (`bridge.rs`: the engine when it runs, otherwise the helper's window
//! listing and capture), and kept by the hub as an attachment. Only where the
//! answer goes differs: the bridge's `device.reply` is diverted here instead
//! of reaching the hub, and the capture it names becomes the attachment.
use super::{BridgeAsk, Runtime};
use anyhow::Result;
use orbit_core::attachments::Attachment;
use orbit_core::protocol::AttachmentResponse;
use orbit_core::refusal::{AttachmentCause, Refusal};
use serde_json::Value;

/// The window capture waiting for its answer, and who asked.
pub(super) struct PendingWindow {
    ask_id: String,
    request_id: String,
    reply: orbit_ipc::ReplySender,
}

fn answered(id: String, outcome: Result<Attachment>) -> AttachmentResponse {
    match outcome {
        Ok(attachment) => AttachmentResponse {
            v: 1,
            id,
            ok: true,
            error: None,
            refusal: None,
            attachment: Some(attachment),
            content: None,
            path: None,
        },
        Err(error) => {
            let refusal = super::refusal_from_error(&error);
            AttachmentResponse {
                v: 1,
                id,
                ok: false,
                error: Some(refusal.to_string()),
                refusal: Some(refusal),
                attachment: None,
                content: None,
                path: None,
            }
        }
    }
}

/// The attachment's name: the window as the user knows it, without what a
/// file name cannot hold. The hub takes at most 255 characters.
pub(super) fn window_file_name(application: &str, title: &str) -> String {
    let clean = |text: &str| {
        text.chars()
            .filter(|character| !character.is_control())
            .map(|character| if character == '/' { '-' } else { character })
            .collect::<String>()
            .trim()
            .to_owned()
    };
    let (application, title) = (clean(application), clean(title));
    let mut stem = match (application.is_empty(), title.is_empty()) {
        (false, false) => format!("{application} — {title}"),
        (false, true) => application,
        (true, false) => title,
        (true, true) => "Ventana".to_owned(),
    };
    if stem.chars().count() > 200 {
        stem = stem.chars().take(200).collect();
    }
    format!("{stem}.png")
}

impl Runtime {
    /// Start capturing the frontmost window for the composer; the answer
    /// goes to `reply` when the capture is an attachment, or with its cause.
    pub(super) async fn attach_window(
        &mut self,
        request_id: String,
        reply: orbit_ipc::ReplySender,
    ) {
        let context = self.attachment_context();
        let admitted = if context.limits.is_none() {
            Err(Refusal::Attachment {
                cause: AttachmentCause::Unsupported,
            })
        } else if let Err(refusal) = crate::hub::access(
            &self.settings,
            self.token.as_deref(),
            self.state.privacy_stopped() || self.closing,
            self.state.connection(),
        ) {
            Err(refusal)
        } else if self.conversation.bridge_request.is_some()
            || self.conversation.window_attachment.is_some()
        {
            // One capture at a time: the assistant's own, or another window.
            Err(Refusal::Busy {
                operation: orbit_core::refusal::Operation::Capture,
                retry_after_ms: None,
            })
        } else {
            Ok(())
        };
        if let Err(refusal) = admitted {
            let _ = reply.send(answered(request_id, Err(refusal.into())));
            return;
        }
        let ask = BridgeAsk {
            id: format!("window-{}", uuid::Uuid::new_v4()),
            capability: orbit_core::device_bridge::Capability::Screenshot,
            application: None,
            title: None,
            full_screen: false,
            source_id: None,
            tool_call: None,
            catalogue_after: 0,
            calendar_window: None,
            calendar_cursor: None,
            files: None,
            for_attachment: true,
        };
        self.conversation.bridge_request = Some(ask.clone());
        self.conversation.window_attachment = Some(PendingWindow {
            ask_id: ask.id.clone(),
            request_id,
            reply,
        });
        if let Err(refusal) = self.serve_bridge_request(&ask).await {
            self.conversation.bridge_request = None;
            if let Some(pending) = self.conversation.window_attachment.take() {
                let _ = pending
                    .reply
                    .send(answered(pending.request_id, Err(refusal.into())));
            }
        }
    }

    /// The bridge's answer to «Adjuntar ventana», taken before it reaches
    /// the hub. True when `frame` was that answer: it is not to be sent.
    pub(super) fn divert_window_reply(&mut self, frame: &Value) -> bool {
        let Some(pending) = self.conversation.window_attachment.as_ref() else {
            return false;
        };
        if frame["method"] != "device.reply"
            || frame["params"]["request_id"].as_str() != Some(pending.ask_id.as_str())
        {
            return false;
        }
        let Some(pending) = self.conversation.window_attachment.take() else {
            return false;
        };
        let params = &frame["params"];
        if let Some(refusal) = params.get("refusal") {
            let refusal = serde_json::from_value::<Refusal>(refusal.clone()).unwrap_or_else(|_| {
                Refusal::HelperReplyInvalid {
                    field: "device.reply.refusal".into(),
                    expected: "typed_refusal".into(),
                    received_type: "unreadable".into(),
                }
            });
            let _ = pending
                .reply
                .send(answered(pending.request_id, Err(refusal.into())));
            return true;
        }
        let result = &params["result"];
        let text = |key: &str| result[key].as_str().unwrap_or_default().to_owned();
        let capture_id = text("capture_id");
        let name = window_file_name(&text("application"), &text("title"));
        let context = self.attachment_context();
        let ask_id = pending.ask_id;
        self.reads.tasks.push(tokio::spawn(async move {
            let outcome = context
                .attachment_from_capture(&capture_id, &ask_id, &name)
                .await;
            let _ = pending.reply.send(answered(pending.request_id, outcome));
        }));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::window_file_name;

    #[test]
    fn a_window_is_named_as_the_user_knows_it() {
        assert_eq!(
            window_file_name("Notas", "Lista de la compra"),
            "Notas — Lista de la compra.png"
        );
        assert_eq!(window_file_name("Finder", ""), "Finder.png");
        assert_eq!(window_file_name("", ""), "Ventana.png");
        // A file name holds no `/` and no control characters.
        assert_eq!(window_file_name("Safari", "a/b\u{7}c"), "Safari — a-bc.png");
        let long = "x".repeat(400);
        assert_eq!(window_file_name("App", &long).chars().count(), 204);
    }
}
