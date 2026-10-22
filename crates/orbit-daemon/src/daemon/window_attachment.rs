//! «Adjuntar ventana»: the frontmost window that is not Orbit's,
//! captured exactly as Orbit captures one when the assistant asks
//! (`bridge.rs`: the engine when it runs, otherwise the helper's window
//! listing and capture), and kept by the hub as an attachment. Only where the
//! answer goes differs: the bridge's `device.reply` is diverted here instead
//! of reaching the hub, and the capture it names becomes the attachment.
//!
//! The window's text goes next to it as a second attachment, a `.txt` named
//! like the capture: the engine reads it right after its capture, and the
//! helper (`window_text`) once its capture is kept. A window without
//! readable text sends the capture alone and the answer says why. The text
//! is the user's content: it is uploaded and never logged.
use super::helper_waits::{self, HelperAnswer};
use super::{BridgeAsk, Runtime};
use anyhow::Result;
use orbit_core::attachments::{Attachment, WindowText};
use orbit_core::protocol::AttachmentResponse;
use orbit_core::refusal::{AttachmentCause, Operation, Refusal};
use orbit_core::window_text::{
    WINDOW_TEXT_DEADLINE_MS, WindowTextLimits, WindowTextReply, kept, without_menu_bar,
};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::oneshot;

/// How long «Adjuntar ventana» waits for the helper's text.
///
/// Protects: the person waiting on the chip when the helper does not
/// answer. Why: the helper's own walk stops at `WINDOW_TEXT_DEADLINE_MS`,
/// and two seconds more find the window and carry the answer back. Past it
/// the capture goes alone with `timeout`; it does not limit the capture.
const TEXT_WAIT: Duration = Duration::from_millis(WINDOW_TEXT_DEADLINE_MS + 2_000);

/// The window capture waiting for its answer, and who asked.
pub(super) struct PendingWindow {
    ask_id: String,
    request_id: String,
    reply: orbit_ipc::ReplySender,
    text: WindowTextSource,
}

/// Where the window's text comes from, once a capture says which window it
/// took.
enum WindowTextSource {
    /// No capture has said yet.
    Unknown,
    /// The engine read the window's tree after its capture, or the cause
    /// it could not.
    Engine(std::result::Result<String, String>),
    /// The helper captured this window; it reads the text next.
    Helper {
        application_pid: i32,
        window_id: u32,
    },
}

/// The window's text as the task that keeps the capture finds it.
enum TextRead {
    Read(std::result::Result<String, String>),
    Asked(oneshot::Receiver<HelperAnswer>),
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
            window_text: None,
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
                window_text: None,
            }
        }
    }
}

/// The capture's name: the window as the user knows it, without what a
/// file name cannot hold. The hub takes at most 255 characters.
pub(super) fn window_file_name(application: &str, title: &str) -> String {
    format!("{}.png", window_stem(application, title))
}

/// The window's text's name: the capture's, as `.txt`.
fn window_text_name(application: &str, title: &str) -> String {
    format!("{}.txt", window_stem(application, title))
}

fn window_stem(application: &str, title: &str) -> String {
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
    stem
}

/// The text kept from the engine's tree: without the menu bar, within its
/// ceiling; `empty` when nothing readable is left.
fn engine_window_text(
    read: std::result::Result<String, String>,
) -> std::result::Result<String, String> {
    read.and_then(|tree| kept(&without_menu_bar(&tree), false).ok_or_else(|| "empty".into()))
}

/// What became of the window's text: read, then uploaded as `name`.
async fn attach_text(
    context: &super::attachments::AttachmentContext,
    read: TextRead,
    name: &str,
) -> WindowText {
    let read = match read {
        TextRead::Read(read) => read,
        TextRead::Asked(receiver) => {
            helper_waits::wait(receiver, TEXT_WAIT)
                .await
                .and_then(|value| {
                    let reply = serde_json::from_value::<WindowTextReply>(value)
                        .map_err(|_| "window_text_reply_invalid".to_owned())?;
                    kept(&reply.text, reply.stopped.is_some()).ok_or_else(|| "empty".to_owned())
                })
        }
    };
    match read {
        Err(cause) => WindowText::Missing { cause },
        Ok(text) => match context.add_bytes(name, text.into_bytes()).await {
            Ok(attachment) => WindowText::Attached { attachment },
            Err(error) => WindowText::Refused {
                refusal: super::refusal_from_error(&error),
            },
        },
    }
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
            text: WindowTextSource::Unknown,
        });
        if let Err(refusal) = self.serve_bridge_request(&ask).await {
            self.conversation.bridge_request = None;
            self.refuse_window(refusal);
        }
    }

    /// «Adjuntar ventana» ends before its capture with `refusal`, which the
    /// app is told; the next one may start.
    pub(super) fn refuse_window(&mut self, refusal: Refusal) {
        if let Some(pending) = self.conversation.window_attachment.take() {
            let _ = pending
                .reply
                .send(answered(pending.request_id, Err(refusal.into())));
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
        let field = |key: &str| result[key].as_str().unwrap_or_default().to_owned();
        let capture_id = field("capture_id");
        let (application, title) = (field("application"), field("title"));
        let name = window_file_name(&application, &title);
        let text_name = window_text_name(&application, &title);
        let read = match pending.text {
            WindowTextSource::Engine(read) => TextRead::Read(engine_window_text(read)),
            WindowTextSource::Helper {
                application_pid,
                window_id,
            } => match self.ask_helper(
                "window_text",
                json!({"application_pid": application_pid, "window_id": window_id,
                    "limits": WindowTextLimits::ATTACHMENT}),
                Operation::Capture,
            ) {
                Ok(receiver) => TextRead::Asked(receiver),
                Err(refusal) => TextRead::Read(Err(refusal.code().name().to_owned())),
            },
            WindowTextSource::Unknown => TextRead::Read(Err("window_unknown".into())),
        };
        let context = self.attachment_context();
        let PendingWindow {
            ask_id,
            request_id,
            reply,
            ..
        } = pending;
        let mut privacy_epoch = self.reads.privacy_epoch_rx.clone();
        let started_at_epoch = *privacy_epoch.borrow_and_update();
        self.reads.tasks.push(tokio::spawn(async move {
            let id = request_id.clone();
            let keep = async move {
                let outcome = context
                    .attachment_from_capture(&capture_id, &ask_id, &name)
                    .await;
                match outcome {
                    Ok(attachment) => {
                        let window_text = attach_text(&context, read, &text_name).await;
                        let mut response = answered(id, Ok(attachment));
                        response.window_text = Some(window_text);
                        response
                    }
                    Err(error) => answered(id, Err(error)),
                }
            };
            // A privacy stop while it runs ends it: nothing more leaves, the
            // window's text included.
            let stopped = || {
                answered(
                    request_id.clone(),
                    Err(Refusal::PrivacyStopped { since_ms: None }.into()),
                )
            };
            let response = tokio::select! {
                _ = privacy_epoch.changed() => stopped(),
                response = keep => response,
            };
            let response = if started_at_epoch != *privacy_epoch.borrow() && response.ok {
                stopped()
            } else {
                response
            };
            let _ = reply.send(response);
        }));
        true
    }

    /// The helper took the capture of «Adjuntar ventana»: that window is the
    /// one whose text it reads next.
    pub(super) fn window_captured_by_helper(&mut self, application_pid: i32, window_id: u32) {
        if let Some(pending) = self.conversation.window_attachment.as_mut() {
            pending.text = WindowTextSource::Helper {
                application_pid,
                window_id,
            };
        }
    }

    /// The engine took the capture of «Adjuntar ventana» and read its tree.
    pub(super) fn window_read_by_engine(&mut self, read: std::result::Result<String, String>) {
        if let Some(pending) = self.conversation.window_attachment.as_mut() {
            pending.text = WindowTextSource::Engine(read);
        }
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
