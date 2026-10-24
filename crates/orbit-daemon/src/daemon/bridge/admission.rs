//! Where a `device.request` enters: its capability admitted, its parameters
//! checked, and made the request being served of its kind, or refused with
//! why.
use super::parameters::{
    calendar_range_refusal, invalid_parameter_refusal, tool_call_refusal, unknown_capture_filter,
};
use super::{BridgeAsk, BridgeKind, bridge_refused};
use crate::daemon::{Runtime, hub};
use anyhow::Result;
use orbit_core::device_bridge::Capability;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::refusal::Refusal;
use orbit_core::wire::bridge::{BridgeRequest, DeviceRequestData};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct HubRequestHeader<'a> {
    request_id: &'a str,
    #[serde(default)]
    capability: Option<Value>,
}

/// What a request asked for, beyond its capability, as `BridgeAsk` keeps it.
#[derive(Default)]
struct AskFields {
    application: Option<String>,
    title: Option<String>,
    full_screen: bool,
    tool_call: Option<orbit_core::computer_control::ToolCall>,
    catalogue_after: usize,
    calendar_window: Option<(i64, i64)>,
    calendar_cursor: Option<crate::daemon::calendar::Cursor>,
}

fn request_name(request: &BridgeRequest) -> &'static str {
    match request {
        BridgeRequest::Screenshot { .. } => "screenshot",
        BridgeRequest::ScreenSources { .. } => "screen_sources",
        BridgeRequest::SelectedText { .. } => "selected_text",
        BridgeRequest::ControlTools { .. } => "control_tools",
        BridgeRequest::PerformAction { .. } => "perform_action",
        BridgeRequest::CalendarRead { .. } => "calendar_read",
        BridgeRequest::FilesList { .. } => "files_list",
        BridgeRequest::FilesSearch { .. } => "files_search",
        BridgeRequest::FilesRead { .. } => "files_read",
    }
}

/// What Deimos asked for, as the notice said while it is served names it.
fn asked_code(ask: &BridgeAsk) -> &'static str {
    match ask.capability {
        Capability::Screenshot if ask.full_screen => "screenshot_full",
        Capability::Screenshot => "screenshot_window",
        Capability::ScreenSources => "screen_sources",
        Capability::SelectedText => "selected_text",
        Capability::ControlTools => "control_tools",
        Capability::PerformAction => "perform_action",
        Capability::CalendarRead => "calendar_read",
        Capability::FilesList => "files_list",
        Capability::FilesSearch => "files_search",
        Capability::FilesRead => "files_read",
    }
}

/// The request's own fields, or the refusal of the first that is invalid.
fn ask_fields(request: BridgeRequest) -> Result<AskFields, Refusal> {
    Ok(match request {
        BridgeRequest::Screenshot { parameters } => AskFields {
            application: parameters.application,
            title: parameters.title,
            full_screen: parameters.full_screen.unwrap_or(false),
            ..AskFields::default()
        },
        BridgeRequest::SelectedText { parameters } => AskFields {
            application: parameters.application,
            title: parameters.title,
            ..AskFields::default()
        },
        BridgeRequest::ControlTools { parameters } => AskFields {
            catalogue_after: parameters.after_index.unwrap_or(0),
            ..AskFields::default()
        },
        BridgeRequest::PerformAction { parameters } => {
            if parameters.action.validate().is_err() {
                return Err(tool_call_refusal(&parameters.action));
            }
            AskFields {
                tool_call: Some(parameters.action),
                ..AskFields::default()
            }
        }
        BridgeRequest::CalendarRead { parameters } => {
            if parameters.end_ms <= parameters.start_ms {
                return Err(calendar_range_refusal(
                    parameters.start_ms,
                    parameters.end_ms,
                ));
            }
            let cursor = crate::daemon::calendar::Cursor::from_parameters(&parameters)?;
            AskFields {
                calendar_window: Some((parameters.start_ms, parameters.end_ms)),
                calendar_cursor: cursor,
                ..AskFields::default()
            }
        }
        BridgeRequest::ScreenSources { .. }
        | BridgeRequest::FilesList { .. }
        | BridgeRequest::FilesSearch { .. }
        | BridgeRequest::FilesRead { .. } => AskFields::default(),
    })
}

impl Runtime {
    /// Answers a remote assistant asking this Mac to observe or act.
    ///
    /// Every reachable path replies: a request that is silently dropped leaves
    /// the assistant waiting on a Mac that already decided, and "no answer"
    /// reads as a broken bridge rather than as the refusal it is.
    ///
    /// One case is deliberately unreachable. While privacy is latched
    /// `hub_event` returns before dispatching anything at all, so a stopped Mac
    /// says nothing rather than saying no. That invariant is older and stronger
    /// than this bridge — a latched Mac is silent to the hub, full stop — and
    /// punching a hole in it for a politer error would be a bad trade. Naming
    /// the caller's timeout as the answer is the hub's job, not this one's.
    pub(in crate::daemon) async fn device_request_raw(&mut self, raw: &Value) -> Result<()> {
        let Ok(header) = HubRequestHeader::deserialize(raw) else {
            // Nothing to reply to: without an id the answer has no destination.
            return Ok(());
        };
        let request_id = header.request_id;
        let capability = match orbit_core::device_bridge::admits(
            header
                .capability
                .as_ref()
                .and_then(Value::as_str)
                .unwrap_or(""),
            self.bridge_conditions(),
        ) {
            Ok(capability) => capability,
            Err(refusal) => {
                self.refuse_bridge_admission(request_id, &refusal).await?;
                return Ok(());
            }
        };
        let encoded = serde_json::to_vec(raw)?;
        let mut decoder = serde_json::Deserializer::from_slice(&encoded);
        let typed: DeviceRequestData = match serde_path_to_error::deserialize(&mut decoder) {
            Ok(request) => request,
            Err(_) => {
                let refusal = unknown_capture_filter(raw, capability)
                    .unwrap_or_else(|| invalid_parameter_refusal(raw, capability));
                self.refuse_bridge_admission(request_id, &refusal).await?;
                return Ok(());
            }
        };
        self.device_request(&typed).await
    }

    fn bridge_conditions(&self) -> orbit_core::device_bridge::Conditions {
        orbit_core::device_bridge::Conditions {
            privacy_stopped: self.state.privacy_stopped(),
            // What macOS grants, as the helper last reported it and
            // as the daemon asks it again every few seconds. Nothing in Orbit's
            // settings is read: there is no switch left to consult.
            helper_connected: self.state.capabilities().native_helper(),
            accessibility: self.state.capabilities().accessibility_state(),
            screen_recording: self.state.capabilities().screen_state(),
            // Read from the live permission the Mac reports, not from
            // a switch in Orbit. This is the only decision the user makes about
            // their calendar, so it is the only thing consulted here. In
            // three values — a permission the Mac has not reported yet is
            // neither granted nor refused.
            calendar: orbit_core::device_bridge::CalendarPermission::from_authorization(
                self.calendar.view.authorization,
            ),
        }
    }

    pub(super) async fn refuse_bridge_admission(
        &mut self,
        request_id: &str,
        refusal: &Refusal,
    ) -> Result<()> {
        self.say(bridge_refused(refusal));
        self.send(hub::command(
            request_id,
            "device.reply",
            self.bridge_refusal_frame(request_id, refusal),
        )?)
        .await
    }

    pub(in crate::daemon) async fn device_request(
        &mut self,
        data: &DeviceRequestData,
    ) -> Result<()> {
        let request_id = data.request_id.clone();
        let admitted = orbit_core::device_bridge::admits(
            request_name(&data.request),
            self.bridge_conditions(),
        )
        .and_then(|capability| {
            let files = crate::daemon::assistant_files::FilesAsk::from_request(&data.request)?;
            Ok((capability, files, ask_fields(data.request.clone())?))
        });
        let (capability, files, fields) = match admitted {
            Ok(admitted) => admitted,
            Err(refusal) => {
                self.refuse_bridge_admission(&request_id, &refusal).await?;
                return Ok(());
            }
        };
        // One at a time of each kind: another kind's request in course
        // does not make this one wait.
        if self
            .assistant
            .bridge_requests
            .get(BridgeKind::of(capability))
            .is_some()
        {
            let refusal = Refusal::Busy {
                operation: orbit_core::refusal::Operation::Interaction,
                retry_after_ms: None,
            };
            return self.refuse_bridge_admission(&request_id, &refusal).await;
        }
        let ask = BridgeAsk {
            for_attachment: false,
            id: request_id.clone(),
            capability,
            application: fields.application,
            title: fields.title,
            full_screen: fields.full_screen,
            source_id: None,
            tool_call: fields.tool_call,
            catalogue_after: fields.catalogue_after,
            calendar_window: fields.calendar_window,
            calendar_cursor: fields.calendar_cursor,
            files,
        };
        self.assistant.bridge_requests.put(ask.clone());
        self.start_bridge_request(&ask).await
    }

    /// Serves an admitted request and says so; a start that fails is
    /// answered here and now. Leaving the request open would spend the
    /// caller's whole timeout on a Mac that already knows it will not be
    /// answering.
    async fn start_bridge_request(&mut self, ask: &BridgeAsk) -> Result<()> {
        let started = self.serve_bridge_request(ask).await;
        // Written AFTER starting the work, never before. The local operation may
        // set this same field while it starts.
        match started {
            Ok(()) => {
                // What Deimos asked for, said while it is served: Deimos never
                // looks at this Mac quietly.
                self.say(
                    Notice::new(NoticeCode::BridgeServing)
                        .with_token(NoticeKey::Code, asked_code(ask)),
                );
            }
            Err(failure) => {
                self.assistant.bridge_requests.take(ask.kind());
                self.log("bridge_start_failed", &[("code", failure.code().name())]);
                self.say(
                    Notice::new(NoticeCode::BridgeRefused)
                        .with_token(NoticeKey::Code, failure.code().name()),
                );
                self.send(hub::command(
                    &ask.id,
                    "device.reply",
                    self.bridge_start_frame(&ask.id, &failure),
                )?)
                .await?;
            }
        }
        Ok(())
    }
}
