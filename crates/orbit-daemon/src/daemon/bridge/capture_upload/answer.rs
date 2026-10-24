//! The upload's outcome answers the request it was started for: the folder
//! image or the screenshot, once the hub holds it, or the refusal that says
//! why it never got there.
use super::super::{BridgeAsk, BridgeKind, BridgeReleaseReason, bridge_refused, result_payload};
use super::{CAPTURE_UPLOAD_TIMEOUT, CaptureUploadOutcome, PendingCapture};
use crate::daemon::ticketed::{Settlement, WorkTicket};
use crate::daemon::{Runtime, hub};
use anyhow::Result;
use orbit_core::device_bridge::Capability;
use orbit_core::wire::bridge::{BridgeResult, FileContent, FilesReadResult, ScreenshotResult};

impl Runtime {
    pub(in crate::daemon) async fn on_bridge_capture_upload(
        &mut self,
        ticket: WorkTicket,
        request_id: String,
        capture: PendingCapture,
        outcome: CaptureUploadOutcome,
    ) -> Result<()> {
        if !self.capture_upload_is_current(&ticket, &request_id, &capture) {
            return Ok(());
        }
        let Some(ask) = self.assistant.bridge_requests.get(capture.kind()).cloned() else {
            return Ok(());
        };
        let from_engine = matches!(&capture, PendingCapture::Engine { .. });
        if let (
            PendingCapture::File { capture_id, result },
            CaptureUploadOutcome::Uploaded(sha256),
        ) = (&capture, &outcome)
        {
            self.answer_folder_image(&ask, capture_id, result, sha256);
            return Ok(());
        }
        let Some(screenshot) = self.uploaded_screenshot(&ask, capture, outcome).await? else {
            return Ok(());
        };
        self.sources.bridge_uploads.take(BridgeKind::Screen);
        let subject = if from_engine {
            format!(
                "the engine observation: {} · {}",
                screenshot.application, screenshot.title
            )
        } else {
            format!(
                "the screen read: {} · {}",
                screenshot.application, screenshot.title
            )
        };
        let sent = self.send_frame(hub::command(
            &ask.id,
            "device.reply",
            result_payload(&ask.id, BridgeResult::Screenshot(screenshot)),
        ));
        self.note_bridge_send(&sent, &subject);
        self.assistant.bridge_requests.take(BridgeKind::Screen);
        Ok(())
    }

    /// Whether this outcome still answers the request being served: an
    /// upload from an older generation, a released or replaced request, or a
    /// replaced upload is logged by its cause and dropped.
    fn capture_upload_is_current(
        &mut self,
        ticket: &WorkTicket,
        request_id: &str,
        capture: &PendingCapture,
    ) -> bool {
        let generation = self.state.generation();
        let kind = capture.kind();
        let Some(upload) = self.sources.bridge_uploads.get_mut(kind) else {
            return false;
        };
        match upload.work.settle(ticket, generation) {
            Settlement::Current(()) => {}
            Settlement::Foreign => return false,
            Settlement::Expired {
                started_generation,
                current_generation,
            } => {
                if let Some(log) = &self.diagnostics {
                    let started = started_generation.to_string();
                    let current = current_generation.to_string();
                    log.record(
                        "bridge_capture_upload_stale",
                        &[
                            ("cause", "generation_changed"),
                            ("started_generation", &started),
                            ("current_generation", &current),
                        ],
                    );
                }
                return false;
            }
        }
        let wanted = if matches!(capture, PendingCapture::File { .. }) {
            Capability::FilesRead
        } else {
            Capability::Screenshot
        };
        let stale_cause = match self.assistant.bridge_requests.get(kind) {
            None => Some("request_released"),
            Some(ask) if ask.id != request_id => Some("request_replaced"),
            Some(ask) if ask.capability != wanted => Some("capability_changed"),
            Some(_)
                if self
                    .sources
                    .bridge_uploads
                    .get(kind)
                    .is_none_or(|upload| upload.capture_id != capture.id()) =>
            {
                Some("upload_replaced")
            }
            Some(_) => None,
        };
        if let Some(cause) = stale_cause {
            if let Some(log) = &self.diagnostics {
                log.record("bridge_capture_upload_stale", &[("cause", cause)]);
            }
            return false;
        }
        true
    }

    /// The folder image is held by the hub: its answer gets the capture's id
    /// and digest.
    fn answer_folder_image(
        &mut self,
        ask: &BridgeAsk,
        capture_id: &str,
        partial: &FilesReadResult,
        sha256: &str,
    ) {
        let mut result = partial.clone();
        if let FileContent::Image {
            capture_id: id,
            sha256: digest,
            ..
        } = &mut result.content
        {
            capture_id.clone_into(id);
            sha256.clone_into(digest);
        }
        self.sources.bridge_uploads.take(BridgeKind::Files);
        let sent = self.send_frame(hub::command(
            &ask.id,
            "device.reply",
            result_payload(&ask.id, BridgeResult::FilesRead(result)),
        ));
        self.note_bridge_send(&sent, "the folder image");
        self.assistant.bridge_requests.take(BridgeKind::Files);
    }

    /// The screenshot an upload's outcome answers with; `None` when the
    /// outcome was already answered (a refusal, a timeout) or answers nothing.
    async fn uploaded_screenshot(
        &mut self,
        ask: &BridgeAsk,
        capture: PendingCapture,
        outcome: CaptureUploadOutcome,
    ) -> Result<Option<ScreenshotResult>> {
        Ok(Some(match (capture, outcome) {
            (_, CaptureUploadOutcome::TimedOut { waited_ms }) => {
                self.release_bridge_request(
                    ask.kind(),
                    BridgeReleaseReason::CaptureUploadTimeout {
                        waited_ms,
                        deadline_ms: CAPTURE_UPLOAD_TIMEOUT.duration().as_millis() as u64,
                    },
                );
                return Ok(None);
            }
            (_, CaptureUploadOutcome::Failed(error)) => {
                self.refuse_capture_upload(ask, Some(&error)).await?;
                return Ok(None);
            }
            (
                PendingCapture::Engine {
                    capture_id,
                    application,
                    title,
                    width,
                    height,
                    mime_type,
                },
                CaptureUploadOutcome::Uploaded(sha256),
            ) => {
                // This full screen is what Deimos now sees, so its
                // scale is the one its desktop actions are mapped with.
                let desktop = self
                    .computer
                    .pending_desktop
                    .take_if(|(pending, _)| *pending == capture_id)
                    .map(|(_, scale)| scale);
                if let Some(scale) = desktop {
                    self.computer.desktop_scale = Some(scale);
                }
                ScreenshotResult {
                    capture_id,
                    application,
                    title,
                    width,
                    height,
                    mime_type,
                    sha256,
                    coordinates: desktop.map(|scale| scale.describe()),
                }
            }
            (PendingCapture::File { .. }, CaptureUploadOutcome::Uploaded(_)) => return Ok(None),
            (PendingCapture::Helper { source_id }, CaptureUploadOutcome::Uploaded(sha256)) => {
                let Some(screenshot) = self
                    .sources
                    .bridge_uploads
                    .get(BridgeKind::Screen)
                    .and_then(|upload| upload.screenshot.as_ref())
                    .filter(|screenshot| {
                        ask.source_id.as_deref() == Some(source_id.as_str())
                            && screenshot.capture_id == source_id
                            && screenshot.sha256 == sha256
                    })
                    .cloned()
                else {
                    self.refuse_bridge_request(ask, "source_capture_failed")
                        .await?;
                    return Ok(None);
                };
                screenshot
            }
        }))
    }

    pub(in crate::daemon::bridge) async fn refuse_capture_upload(
        &mut self,
        ask: &BridgeAsk,
        error: Option<&anyhow::Error>,
    ) -> Result<()> {
        if let Some(mut upload) = self.sources.bridge_uploads.take(ask.kind()) {
            upload.work.cancel();
        }
        let refusal = error
            .and_then(|error| error.downcast_ref::<orbit_core::refusal::Refusal>())
            .cloned()
            .unwrap_or_else(|| Self::bridge_refusal_code("capture_upload_failed"));
        self.say(bridge_refused(&refusal));
        self.assistant.bridge_requests.take(ask.kind());
        self.send(hub::command(
            &ask.id,
            "device.reply",
            self.bridge_refusal_frame(&ask.id, &refusal),
        )?)
        .await
    }
}
