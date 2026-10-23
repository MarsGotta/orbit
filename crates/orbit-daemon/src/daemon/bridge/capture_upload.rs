//! An image Deimos asked for goes to the hub out of band, and its answer
//! waits for the hub to hold it: a window or the desktop through the engine,
//! a window through the helper, or an image from the assistant's folders.
use super::BridgeAsk;
use crate::daemon::ticketed::{DeadlineKind, Ticketed};
use crate::daemon::{Runtime, WorkResult, deliver_work, hub};
use anyhow::Result;
use orbit_core::device_bridge::Capability;
use orbit_core::wire::bridge::FilesReadResult;
use orbit_core::wire::native::NativeResponseView;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

mod answer;

#[derive(Deserialize)]
struct EncodedCapture<'a> {
    data_base64: &'a str,
}

/// How long either capture upload may take to reach the hub before the caller
/// is told that it did not. This bounds the bridge reply, not local sealing or
/// the HTTP client's general 90 s limit.
///
/// The hub gives a bridge request 30 s (`REQUEST_TIMEOUT_SECONDS`), so an
/// upload has to give up with enough margin for the refusal to travel. A
/// capture still uploading at 20 s would not have been useful anyway, and a
/// wait that outlives the caller's own is a reply that never leaves the Mac.
const CAPTURE_UPLOAD_TIMEOUT: DeadlineKind = DeadlineKind::new(
    "capture_upload",
    Duration::from_secs(orbit_core::limits::CAPTURE_UPLOAD_TIMEOUT_SECONDS),
    "One capture upload; 20 s leaves 10 s before the hub request limit; not local sealing, data size, or retry",
);

pub(in crate::daemon) enum PendingCapture {
    Engine {
        capture_id: String,
        application: String,
        title: String,
        width: u32,
        height: u32,
        mime_type: String,
    },
    Helper {
        source_id: String,
    },
    /// An image from the assistant's folders, fitted by the helper; the
    /// answer waits for the hub to hold it, as a screenshot's does.
    File {
        capture_id: String,
        result: Box<FilesReadResult>,
    },
}

impl PendingCapture {
    fn id(&self) -> &str {
        match self {
            Self::Engine { capture_id, .. } | Self::File { capture_id, .. } => capture_id,
            Self::Helper { source_id } => source_id,
        }
    }
}

pub(in crate::daemon) enum CaptureUploadOutcome {
    Uploaded(String),
    Failed(anyhow::Error),
    TimedOut { waited_ms: u64 },
}

async fn bounded_capture_upload(
    hub_url: &str,
    token: &str,
    development: bool,
    capture_id: &str,
    request_id: &str,
    bytes: Vec<u8>,
    op: &orbit_core::notice::Op,
) -> CaptureUploadOutcome {
    capture_upload_outcome(hub::upload_capture(
        hub_url,
        token,
        development,
        capture_id,
        request_id,
        bytes,
        op,
    ))
    .await
}

async fn capture_upload_outcome(
    upload: impl std::future::Future<Output = Result<String>>,
) -> CaptureUploadOutcome {
    let deadline = CAPTURE_UPLOAD_TIMEOUT.start();
    match tokio::time::timeout_at(deadline.at().into(), upload).await {
        Ok(Ok(sha256)) => CaptureUploadOutcome::Uploaded(sha256),
        Ok(Err(error)) => CaptureUploadOutcome::Failed(error),
        // Tokio's timer reached the deadline even when a paused test clock
        // leaves std::Instant stationary. A timeout therefore waited at least
        // its budget; a delayed poll may have waited longer on the real clock.
        Err(_) => CaptureUploadOutcome::TimedOut {
            waited_ms: deadline.waited_ms().max(deadline.budget_ms()),
        },
    }
}

/// What one upload needs to reach the hub, apart from its bytes.
struct UploadTarget {
    hub_url: String,
    token: String,
    capture_id: String,
    request_id: String,
}

impl Runtime {
    /// The bytes go out of band, exactly as the helper's do. Inlining them
    /// put a 933 KiB frame on a link that admits 131 072, so "Deimos ve mi
    /// pantalla" failed on every capture worth taking. The capture's id when
    /// the upload started; `None` when it was refused at once.
    pub(super) async fn upload_engine_capture(
        &mut self,
        ask: &BridgeAsk,
        bytes: Vec<u8>,
        (width, height): (u32, u32),
        mime_type: String,
        application: &str,
        title: &str,
    ) -> Result<Option<String>> {
        let capture_id = uuid::Uuid::new_v4().to_string();
        let capture = PendingCapture::Engine {
            capture_id: capture_id.clone(),
            application: application.to_owned(),
            title: title.to_owned(),
            width,
            height,
            mime_type,
        };
        self.start_capture_upload(ask, bytes, capture_id, capture)
            .await
    }

    /// An image read from the assistant's folders goes up the way a
    /// screenshot does; `partial` is its answer without the capture's id and
    /// digest, which the upload supplies.
    pub(in crate::daemon) async fn upload_file_capture(
        &mut self,
        ask: &BridgeAsk,
        bytes: Vec<u8>,
        partial: FilesReadResult,
    ) -> Result<Option<String>> {
        let capture_id = uuid::Uuid::new_v4().to_string();
        let capture = PendingCapture::File {
            capture_id: capture_id.clone(),
            result: Box::new(partial),
        };
        self.start_capture_upload(ask, bytes, capture_id, capture)
            .await
    }

    async fn start_capture_upload(
        &mut self,
        ask: &BridgeAsk,
        bytes: Vec<u8>,
        capture_id: String,
        capture: PendingCapture,
    ) -> Result<Option<String>> {
        let (Some(hub_url), Some(token)) = (self.settings.hub_url.clone(), self.link.token.clone())
        else {
            return self.refuse_capture_upload(ask, None).await.map(|()| None);
        };
        // A second capture replaces the first: its upload is aborted here, not
        // left running to be discarded later as `upload_replaced`.
        if let Some(mut previous) = self.sources.bridge_upload.take() {
            previous.work.cancel();
        }
        let started = capture_id.clone();
        let target = UploadTarget {
            hub_url,
            token,
            capture_id,
            request_id: ask.id.clone(),
        };
        self.spawn_capture_upload(target, bytes, capture);
        Ok(Some(started))
    }

    /// The helper's screenshot of the window Deimos named starts up as soon
    /// as its bytes arrive; its answer is staged next to the upload.
    pub(in crate::daemon) fn prepare_bridge_capture_upload(&mut self, event: &Value) {
        let Some(ask) = self.assistant.bridge_request.as_ref() else {
            return;
        };
        let Ok(response) = NativeResponseView::from_value(event) else {
            return;
        };
        if ask.capability != Capability::Screenshot
            || ask.source_id.is_none()
            || !response.ok
            || self.sources.bridge_upload.is_some()
        {
            return;
        }
        let Some(encoded) = response
            .result
            .and_then(|body| EncodedCapture::deserialize(body).ok())
            .map(|body| body.data_base64)
        else {
            return;
        };
        let Ok(bytes) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        else {
            return;
        };
        let Some(hub_url) = self.settings.hub_url.clone() else {
            return;
        };
        let Some(token) = self.link.token.clone() else {
            return;
        };
        let capture_id = ask.source_id.clone().unwrap_or_default();
        let capture = PendingCapture::Helper {
            source_id: capture_id.clone(),
        };
        let target = UploadTarget {
            hub_url,
            token,
            capture_id,
            request_id: ask.id.clone(),
        };
        self.spawn_capture_upload(target, bytes, capture);
    }

    /// The upload runs on a task of its own and its outcome comes back as
    /// work; this capture is the bridge's one upload from now on.
    fn spawn_capture_upload(
        &mut self,
        target: UploadTarget,
        bytes: Vec<u8>,
        capture: PendingCapture,
    ) {
        let UploadTarget {
            hub_url,
            token,
            capture_id,
            request_id,
        } = target;
        let upload_id = capture_id.clone();
        let generation = self.state.generation();
        let development = self.development;
        let work = self.work_events.clone();
        let mut upload_work = Ticketed::logical();
        upload_work.replace_logical(generation, self.turn.op.clone(), (), move |ticket| {
            tokio::spawn(async move {
                let outcome = bounded_capture_upload(
                    &hub_url,
                    &token,
                    development,
                    &capture_id,
                    &request_id,
                    bytes,
                    ticket.op(),
                )
                .await;
                deliver_work(
                    &work,
                    WorkResult::BridgeCaptureUpload {
                        ticket,
                        request_id,
                        capture,
                        outcome,
                    },
                )
                .await;
            })
        });
        self.sources.bridge_upload = Some(crate::daemon::sources::BridgeUpload {
            capture_id: upload_id,
            screenshot: None,
            work: upload_work,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn both_upload_paths_stop_at_the_bridge_deadline() {
        let outcome = capture_upload_outcome(std::future::pending::<Result<String>>()).await;
        let CaptureUploadOutcome::TimedOut { waited_ms } = outcome else {
            panic!("the upload did not expire with the bridge deadline");
        };
        assert_eq!(waited_ms, 20_000);
    }
}
