//! The helper's answers to Deimos's screen reads: its window listing, the
//! window chosen from it, the capture or selected text of that window, and
//! every refusal on the way, with the inventory facts that explain it.
use super::refusals::MAX_BRIDGE_DETAIL_CHARS;
use super::screen::titled_first;
use super::{BridgeAsk, bridge_refused, result_payload};
use crate::daemon::{Runtime, hub};
use anyhow::{Context, Result};
use orbit_core::device_bridge::Capability;
use orbit_core::notice::NoticeKey;
use orbit_core::refusal::Refusal;
use orbit_core::wire::bridge::{
    BridgeResult, DeviceReplyParams, ScreenSourcesResult, ScreenshotResult, SelectedTextResult,
    WindowSummary,
};
use orbit_core::wire::native::{
    NativeResponseView, SelectedTextResult as NativeSelectedTextResult,
};
use serde_json::{Value, json};

impl Runtime {
    /// A capture Deimos asked for, refused with its cause and its numbers,
    /// as fields the translator can act on.
    pub(in crate::daemon) fn refuse_bridge_capture(&mut self, refusal: Refusal) {
        let Some(ask) = self.assistant.bridge_request.take() else {
            return;
        };
        let mut said = bridge_refused(&refusal);
        if let Refusal::TooLarge {
            bytes, max_bytes, ..
        } = &refusal
        {
            said = said
                .with(NoticeKey::Bytes, *bytes)
                .with(NoticeKey::MaxBytes, *max_bytes);
        }
        self.say(said);
        let code = refusal.code().name();
        let frame = json!(DeviceReplyParams::refused(ask.id.clone(), refusal, None,));
        let sent = self.send_frame(hub::command(&ask.id, "device.reply", frame));
        self.note_bridge_send(&sent, &format!("the capture refusal ({code})"));
    }

    /// Sends the answer to a remote request once the helper has replied.
    ///
    /// Called after every source response rather than from inside the source
    /// code itself, so that the bridge cannot alter what the local path does.
    pub(in crate::daemon) async fn answer_bridge_from_sources(
        &mut self,
        event: &Value,
    ) -> Result<()> {
        let Some(ask) = self.assistant.bridge_request.clone() else {
            return Ok(());
        };
        if ask.capability == Capability::ScreenSources {
            self.answer_window_listing(&ask);
            return Ok(());
        }
        let response = NativeResponseView::from_value(event)?;
        if ask.source_id.is_none() {
            return self.capture_chosen_window(&ask, response.ok).await;
        }
        self.answer_window_capture(&ask, event, response).await
    }

    fn answer_window_listing(&mut self, ask: &BridgeAsk) {
        let payload = match self.state.views.source_inventory.as_ref() {
            Some(inventory) => {
                let windows: Vec<WindowSummary> =
                    orbit_core::device_bridge::window_summary(inventory)
                        .into_iter()
                        .map(|(application, title)| WindowSummary { application, title })
                        .collect();
                result_payload(
                    &ask.id,
                    BridgeResult::ScreenSources(ScreenSourcesResult { windows }),
                )
            }
            // Still waiting: say nothing yet rather than reporting an empty Mac.
            None if self.sources.source_operation_pending() => return,
            None => {
                let failure = self.sources.last_failure.as_ref();
                let refusal = failure
                    .map(|failure| failure.refusal.clone())
                    .unwrap_or_else(|| Self::bridge_refusal_code("source_inventory_failed"));
                self.bridge_refusal_frame(&ask.id, &refusal)
            }
        };
        let result = self.send_frame(hub::command(&ask.id, "device.reply", payload));
        self.note_bridge_send(&result, "the windows reply");
        self.assistant.bridge_request = None;
    }

    /// The listing came back: the window Deimos named is chosen from it and
    /// captured, or the request is refused with why none could be.
    async fn capture_chosen_window(&mut self, ask: &BridgeAsk, listed: bool) -> Result<()> {
        if !listed {
            return self
                .refuse_bridge_request(ask, "source_inventory_failed")
                .await;
        }
        let Some(inventory) = self.state.views.source_inventory.as_ref() else {
            if self.sources.source_operation_pending() {
                return Ok(());
            }
            return self
                .refuse_bridge_request(ask, "source_inventory_failed")
                .await;
        };
        let matching: Vec<_> = inventory
            .windows
            .iter()
            .filter(|window| {
                ask.matches_window_filter(
                    &window.application_name,
                    Some(&window.bundle_id),
                    &window.title,
                )
            })
            .collect();
        let selectable = matching
            .iter()
            .copied()
            .filter(|window| window.binding.is_some() && window.unavailable_reason.is_none())
            .collect();
        let token = titled_first(selectable, |window| window.title.as_str())
            .first()
            .and_then(|window| window.binding.as_ref())
            .map(|binding| binding.selection_token.clone());
        let Some(token) = token else {
            let code = if matching.is_empty() {
                "window_not_found"
            } else {
                "screen_selection_unavailable"
            };
            return self.refuse_bridge_request(ask, code).await;
        };
        let format = match ask.capability {
            Capability::Screenshot => orbit_core::sources::SourceFormat::Png,
            Capability::SelectedText => orbit_core::sources::SourceFormat::SelectedText,
            Capability::ScreenSources
            | Capability::ControlTools
            | Capability::PerformAction
            | Capability::CalendarRead
            | Capability::FilesList
            | Capability::FilesSearch
            | Capability::FilesRead => return Ok(()),
        };
        if self
            .source_capture_as(&token, "Deimos".to_owned(), format)
            .await
            .is_err()
        {
            return self
                .refuse_bridge_request(ask, "source_capture_failed")
                .await;
        }
        let Some(source_id) = self
            .state
            .views
            .source_request
            .as_ref()
            .map(|progress| progress.id.clone())
        else {
            return self
                .refuse_bridge_request(ask, "source_capture_failed")
                .await;
        };
        if let Some(current) = self.assistant.bridge_request.as_mut() {
            current.source_id = Some(source_id);
        }
        Ok(())
    }

    /// The chosen window's capture or selection came back.
    async fn answer_window_capture(
        &mut self,
        ask: &BridgeAsk,
        event: &Value,
        response: NativeResponseView<'_>,
    ) -> Result<()> {
        if self.sources.source_operation_pending() {
            return Ok(());
        }
        if !response.ok {
            return self
                .refuse_bridge_request(ask, "source_capture_failed")
                .await;
        }
        let Some(receipt) = self.state.views.source_receipt.clone().filter(|receipt| {
            ask.source_id.as_deref() == Some(receipt.intent.id.as_str())
                && receipt.intent.annotation == "Deimos"
        }) else {
            return self
                .refuse_bridge_request(ask, "source_capture_failed")
                .await;
        };
        let (application, title, binding) = match &receipt.intent.target {
            orbit_core::sources::SourceTarget::NativeWindow {
                application_name,
                title,
                binding,
                ..
            } => (application_name.clone(), title.clone(), binding),
        };
        if ask.for_attachment {
            self.assistant
                .window_captured_by_helper(binding.application_pid, binding.window_id);
        }
        let result =
            match ask.capability {
                Capability::Screenshot => {
                    return self
                        .stage_helper_screenshot(ask, event, receipt, application, title)
                        .await;
                }
                Capability::SelectedText => {
                    let body = response
                        .result
                        .context("response selected_text field=result type=absent")?;
                    let text = orbit_core::wire::native::decode_result_body::<
                        NativeSelectedTextResult,
                    >("selected_text", body)?
                    .text;
                    BridgeResult::SelectedText(SelectedTextResult {
                        application: application.clone(),
                        title: title.clone(),
                        text,
                    })
                }
                Capability::ScreenSources
                | Capability::ControlTools
                | Capability::PerformAction
                | Capability::CalendarRead
                | Capability::FilesList
                | Capability::FilesSearch
                | Capability::FilesRead => return Ok(()),
            };
        let sent = self.send_frame(hub::command(
            &ask.id,
            "device.reply",
            result_payload(&ask.id, result),
        ));
        self.note_bridge_send(&sent, &format!("the screen read: {application} · {title}"));
        self.assistant.bridge_request = None;
        Ok(())
    }

    /// The helper's screenshot goes up to the hub; the answer waits for the
    /// hub to hold it.
    async fn stage_helper_screenshot(
        &mut self,
        ask: &BridgeAsk,
        event: &Value,
        receipt: orbit_core::sources::SourceReceipt,
        application: String,
        title: String,
    ) -> Result<()> {
        let (width, height) = match receipt.metadata.media {
            orbit_core::sources::SourceMedia::Png { width, height } => (width, height),
            orbit_core::sources::SourceMedia::SelectedText { .. } => {
                return self
                    .refuse_bridge_request(ask, "source_capture_failed")
                    .await;
            }
        };
        self.prepare_bridge_capture_upload(event);
        let Some(upload) = self.sources.bridge_upload.as_mut() else {
            return self.refuse_capture_upload(ask, None).await;
        };
        upload.screenshot = Some(ScreenshotResult {
            capture_id: receipt.intent.id,
            application,
            title,
            width,
            height,
            mime_type: "image/png".into(),
            sha256: receipt.sha256,
            coordinates: None,
        });
        Ok(())
    }

    pub(super) async fn refuse_bridge_request(
        &mut self,
        ask: &BridgeAsk,
        code: &str,
    ) -> Result<()> {
        if let Some(mut upload) = self.sources.bridge_upload.take() {
            upload.work.cancel();
        }
        let source_failure = self.sources.last_failure.clone().filter(|failure| {
            (code == "source_inventory_failed" && ask.source_id.is_none())
                || (code == "source_capture_failed"
                    && ask.source_id.as_deref() == Some(failure.operation_id.as_str()))
        });
        let refusal = source_failure
            .as_ref()
            .map(|failure| failure.refusal.clone())
            .or_else(|| self.selection_refusal(ask, code))
            .unwrap_or_else(|| Self::bridge_refusal_code(code));
        self.say(bridge_refused(&refusal));
        self.assistant.bridge_request = None;
        let payload = self.bridge_refusal_frame(&ask.id, &refusal);
        self.send(hub::command(&ask.id, "device.reply", payload)?)
            .await
    }

    /// No window Deimos named could be taken: the counts of the inventory,
    /// the filters it asked with and a bounded sample of what was there.
    fn selection_refusal(&self, ask: &BridgeAsk, code: &str) -> Option<Refusal> {
        use orbit_core::refusal::{
            Expected, Parameter, SelectionFacts, SelectionFailure, SelectionWindow,
        };
        if code != "window_not_found" && code != "screen_selection_unavailable" {
            return None;
        }
        let inventory = self.state.views.source_inventory.as_ref()?;
        let matching = inventory
            .windows
            .iter()
            .filter(|window| {
                ask.matches_window_filter(
                    &window.application_name,
                    Some(&window.bundle_id),
                    &window.title,
                )
            })
            .count();
        let mut sample = Vec::new();
        // Preserve the former 400-character inventory detail budget:
        // it bounds only the sample; the complete counts travel separately.
        for window in &inventory.windows {
            sample.push(SelectionWindow {
                application: window.application_name.clone(),
                title: window.title.clone(),
                unavailable_reason: window.unavailable_reason.clone(),
                has_binding: window.binding.is_some(),
            });
            if serde_json::to_string(&sample)
                .map_or(true, |text| text.chars().count() > MAX_BRIDGE_DETAIL_CHARS)
            {
                sample.pop();
                break;
            }
        }
        // Filter copies share the former detail's character budget.
        // Original byte counts remain exact even when the copy is short.
        let mut remaining = MAX_BRIDGE_DETAIL_CHARS;
        let mut shortened = false;
        let mut bounded_filter = |filter: &Option<String>| {
            filter.as_ref().map(|text| {
                let kept: String = text.chars().take(remaining).collect();
                remaining -= kept.chars().count();
                shortened |= kept.len() != text.len();
                kept
            })
        };
        let application = bounded_filter(&ask.application);
        let title = bounded_filter(&ask.title);
        Some(Refusal::InvalidParameter {
            parameter: Parameter::Selection,
            expected: Expected::Selection(Box::new(SelectionFacts {
                reason: if code == "window_not_found" {
                    SelectionFailure::WindowNotFound
                } else {
                    SelectionFailure::Unavailable
                },
                application_bytes: ask.application.as_ref().map(|s| s.len() as u64),
                title_bytes: ask.title.as_ref().map(|s| s.len() as u64),
                filters_truncated: shortened,
                application,
                title,
                windows: inventory.windows.len() as u64,
                matching: matching as u64,
                selectable: 0,
                omitted: inventory.omitted_count,
                inventory: sample,
            })),
        })
    }
}
