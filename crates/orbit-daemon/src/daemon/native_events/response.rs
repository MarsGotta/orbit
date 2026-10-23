//! The helper's answers to orders, routed to the domain that sent each one:
//! the voice trigger, the mute, the audio's retired orders, the waits off the
//! loop, the folders, the audio sources, the calendar, the screen sources and
//! the capture. What no domain claims is read as the order it names.
use super::super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::native::{NativeResponseFields, NativeStateResult, decode_result_body};
use serde::Deserialize;

#[derive(Deserialize)]
struct NativePermissionsObservation {
    #[serde(default)]
    permissions: Option<Value>,
}

impl Runtime {
    pub(super) async fn on_native_response(&mut self, event: &Value) -> Result<()> {
        let response = NativeResponseFields::from_value(event)?;
        let command_name = event
            .get("_bridge_command")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| self.helper.pending.get(&response.id).cloned());
        self.helper.pending.remove(&response.id);
        if !response.ok {
            self.record_failed_order(&response, command_name.as_deref());
        }
        if self.wake_phrase_response(&response) || self.mute_response(&response) {
            return Ok(());
        }
        self.muted_start_receipt(&response);
        if let Some(stop) = self.audio.retired_stops.remove(&response.id) {
            return self.retired_stop_answered(&response, stop).await;
        }
        if self.overtaken_call_stop_answered(&response) || self.helper.answer_wait(&response) {
            return Ok(());
        }
        if self.domain_response(event).await? || self.capture_ack(&response).await? {
            return Ok(());
        }
        let command_name = command_name.or_else(|| {
            self.capture_response_command(&response.id)
                .map(String::from)
        });
        let Some(command_name) = command_name else {
            // Neither the bridge's tag nor the capture registry names this
            // reply: the bridge forwarded it unnamed after logging it once,
            // or it is a direct synthetic event. With no order to read it
            // as, it has no effect beyond this log.
            self.log("native_response_unpaired", &[("id", &response.id)]);
            return Ok(());
        };
        if self.audio.retired_start(&command_name, &response) {
            return self.retired_start_answered(&response).await;
        }
        if !response.ok {
            return self.helper_order_refused(&response, &command_name).await;
        }
        self.order_answered(event, &response, &command_name).await
    }

    /// The domains that sent their own orders claim their answers.
    async fn domain_response(&mut self, event: &Value) -> Result<bool> {
        if self.files_response(event).await? || self.audio_sources_response(event).await? {
            return Ok(true);
        }
        if self.calendar_response(event) {
            return Ok(true);
        }
        if self.source_response(event).await? {
            self.answer_bridge_from_sources(event).await?;
            return Ok(true);
        }
        Ok(false)
    }

    /// The capture registries name an ACK after the generic index has
    /// evicted it. That absence is unknown, not proof the ACK belongs to
    /// nobody. An id no registry sent is still unpaired.
    async fn capture_ack(&mut self, response: &NativeResponseFields) -> Result<bool> {
        if response.ok
            && let Some(result) = response.result.as_ref()
        {
            if self.call_start_response(&response.id, result).await? {
                return Ok(true);
            }
            if self.audio_response(&response.id, result).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// An order the helper carried out: what it says about the voice and
    /// the permissions is observed, and a start it confirmed may complete
    /// the capture's start.
    async fn order_answered(
        &mut self,
        event: &Value,
        response: &NativeResponseFields,
        command_name: &str,
    ) -> Result<()> {
        let result = response
            .result
            .as_ref()
            .context("response field=result type=absent")?;
        self.observe_voice_capabilities(result)?;
        let state_result = matches!(
            command_name,
            "hello" | "heartbeat" | "status" | "privacy_stop" | "capture_stop" | "rearm"
        );
        let observation = if state_result {
            // The bridge tags an order it actually wrote. That body is the
            // helper wire, so it has to be the state shape: `microphone` there
            // is a bool (capture active), and reading it as the permission
            // string is the false notice. A status answer that never crossed
            // the pipe can still carry only the nested `permissions` object;
            // that object is what is observed below, not the capture flag.
            if event.get("_bridge_command").is_some() {
                decode_result_body::<NativeStateResult>(command_name, result)?;
            }
            Some(NativePermissionsObservation::deserialize(result)?)
        } else {
            None
        };
        let permissions = if command_name == "permissions" {
            Some(result)
        } else {
            observation
                .as_ref()
                .and_then(|observation| observation.permissions.as_ref())
        };
        if let Some(permissions) = permissions {
            self.observe_permissions(permissions)?;
        }
        self.audio.pending_starts.retain(|id| id != &response.id);
        self.capture_started_if_ready().await?;
        Ok(())
    }

    /// The helper reports the calendar in this same payload and it used to
    /// be dropped on the floor, so the one permission Orbit had never
    /// requested was also the one never observed.
    fn observe_permissions(&mut self, permissions: &Value) -> Result<()> {
        let observed = crate::permissions::observe(permissions)?;
        self.state = decide(
            &self.state,
            Event::PermissionObserved {
                microphone: observed.microphone,
                screen: observed.screen,
                accessibility: observed.accessibility,
                calendar: observed.calendar,
            },
            now(),
        )?
        .state;
        Ok(())
    }

    /// Every order the helper answered with an error is recorded,
    /// whatever else the daemon does with the answer, so a failure on the
    /// Mac is never lost for want of a notice.
    fn record_failed_order(&self, response: &NativeResponseFields, named: Option<&str>) {
        let step = named
            .or_else(|| self.capture_response_command(&response.id))
            .or_else(|| {
                self.question
                    .wake_order_pending(&response.id)
                    .then_some("audio_wake_phrase")
            })
            .unwrap_or("unpaired");
        let code = response
            .error
            .as_ref()
            .map_or("error_absent", |error| error.code.as_str());
        let remote_source = self.remote_source_response(&response.id);
        let mut notice = Notice::new(if remote_source {
            NoticeCode::BridgeRefused
        } else {
            NoticeCode::HelperCommandFailed
        })
        .with_token(NoticeKey::Step, step)
        .with_token(NoticeKey::Code, code);
        // The macOS error under it, when the helper had one: its domain (a
        // framework's, closed) and its number. Never its text.
        let detail = response
            .error
            .as_ref()
            .and_then(|error| error.detail.as_ref());
        if let Some(domain) = detail.and_then(|detail| detail["domain"].as_str()) {
            notice = notice.with_token(NoticeKey::Cause, &domain.to_ascii_lowercase());
        }
        if let Some(status) = detail.and_then(|detail| detail["os_status"].as_i64()) {
            notice = notice.with(NoticeKey::Status, status);
        }
        self.record(notice);
    }
}
