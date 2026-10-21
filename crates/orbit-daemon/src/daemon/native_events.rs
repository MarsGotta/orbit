//! Native helper event dispatch.
use super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::native::{
    NativeAudioFrameRef, NativeEventHeader, NativeResponseFields, NativeStateResult,
    decode_result_body,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct NativeStopFields<'a> {
    reason: Option<&'a str>,
    privacy_stopped: bool,
}

#[derive(Deserialize)]
struct RetiredAudioStartResult {
    stream_id: String,
}

#[derive(Deserialize)]
struct RetiredAudioStopResult {
    stream_id: String,
    drain_complete: bool,
}

/// The Mac accepts at most 128 UTF-8 bytes in audio_start.stream_id (the
/// current generated UUID is 36). Bound only a late readback before copying
/// it into audio_stop; this is not a capture duration or response-size limit.
pub(super) const MAX_RETIRED_START_STREAM_ID_BYTES: usize = 128;

#[derive(Deserialize)]
struct NativeStateStop {
    privacy_stopped: bool,
}

#[derive(Deserialize)]
struct NativeHotkeyFields<'a> {
    action: &'a str,
    phase: &'a str,
}

#[derive(Deserialize)]
struct NativeGapSource<'a> {
    source: &'a str,
    reason: Option<&'a str>,
    duration_ns: Option<u64>,
}

#[derive(Deserialize)]
struct NativeErrorCode<'a> {
    code: &'a str,
    os_domain: Option<&'a str>,
    os_status: Option<i64>,
}

#[derive(Deserialize)]
struct NativeHelperDisconnect<'a> {
    #[serde(default)]
    cause: Option<&'a str>,
}

/// The bridge's audio cause uses fixed labels and a schema field shorter than
/// 64 bytes; 256 bytes leaves room for its count. This bounds only the notice
/// copied from an internal disconnect event, not native wire input or capture.
const MAX_NATIVE_AUDIO_CAUSE_BYTES: usize = 256;

#[derive(Deserialize)]
struct NativePermissionsObservation {
    #[serde(default)]
    permissions: Option<Value>,
}

impl Runtime {
    pub(super) fn native_event(
        &mut self,
        mut event: Value,
    ) -> impl std::future::Future<Output = Result<()>> + '_ {
        Box::pin(async move {
            self.observe_audio_helper_liveness(&event);
            let host_update = self.observe_native_host(&event);
            self.observe_calendar_native(&event, host_update);
            let event_name = NativeEventHeader::from_value(&event)?.event.to_owned();
            match event_name.as_str() {
                "ready" => {
                    self.on_native_ready(&event).await?;
                }
                "response" => {
                    self.on_native_response(&event).await?;
                }
                "state" => {
                    self.on_native_state(&event).await?;
                }
                // The helper stopped: her brake at the helper latches as hers;
                // a latch the helper holds that the daemon did not ask for
                // now is adopted so both say the same, attributed to nobody;
                // any other stop cut the capture, and only that (REF-12
                // A2/A4b). `daemon_requested` is the echo of the daemon's own
                // order: with the daemon not stopped it is not an order of
                // hers the daemon can vouch for, so it is never hers.
                "privacy_stop" | "capture_stopped" => {
                    self.on_native_capture_stopped(&event).await?;
                }
                "local_rearm" => {
                    self.on_native_local_rearm().await?;
                }
                "helper_disconnected" => {
                    self.on_native_helper_disconnected(&event).await?;
                }
                "hotkey" => {
                    let clock = chrono::Local::now();
                    self.on_native_hotkey(
                        &event,
                        clock.timestamp_millis(),
                        Some(i64::from(clock.offset().local_minus_utc()) * 1_000),
                    )
                    .await?;
                }
                "audio" => {
                    self.on_native_audio(&mut event).await?;
                }
                "gap" => {
                    self.on_native_gap(&event).await?;
                }
                "error" => {
                    self.on_native_error(&event).await?;
                }
                "microphone_activity" => {
                    self.on_microphone_activity(&event).await?;
                }
                "assistant_question" => {
                    self.on_assistant_question(&event).await?;
                }
                "voice_trigger" => {
                    self.on_voice_trigger(&event)?;
                }
                "notice" => {
                    self.on_native_notice(&event)?;
                }
                "playback_progress" => {
                    let decoded: orbit_core::wire::native::NativeEvent =
                        serde_json::from_value(event)?;
                    let orbit_core::wire::native::NativeEvent::PlaybackProgress {
                        state,
                        playback_active,
                        ..
                    } = decoded
                    else {
                        bail!("Evento de progreso nativo incoherente");
                    };
                    let state = match state.as_str() {
                        "started" => "started",
                        "played" => "played",
                        "cancelled" => "cancelled",
                        _ => "unrecognized_state",
                    };
                    self.log(
                        "native_playback_progress",
                        &[
                            ("state", state),
                            ("active", if playback_active { "true" } else { "false" }),
                        ],
                    );
                }
                _ => {}
            }
            Ok(())
        })
    }

    async fn on_native_ready(&mut self, event: &Value) -> Result<()> {
        self.cancel_sources_with_cause("helper_restarted", "El helper se reinició antes de entregar la respuesta anterior; no se conoce aún la instancia nueva.").await?;
        let retired_call_starts = self.capture.retired_call_starts.len();
        let retired_stops = self.capture.retired_stops.len();
        if retired_call_starts > 0 || retired_stops > 0 {
            self.log(
                "retired_capture_debts_helper_restarted",
                &[
                    ("retired_call_starts", &retired_call_starts.to_string()),
                    ("retired_stops", &retired_stops.to_string()),
                ],
            );
            self.capture.retired_call_starts.clear();
            self.capture.retired_stops.clear();
        }
        self.state = decide(&self.state, Event::HelperReady, now())?.state;
        // A new helper says where its «Oye Deimos» detector is after `ready`,
        // and listens on nothing until asked again.
        self.voice_trigger_unknown();
        self.wake_helper_ready();
        // A new helper says who holds the microphone after `ready`; until
        // then it is not known.
        self.call_signal_lost().await;
        self.observe_voice_capabilities(event)?;
        // Her stop reaches a helper that came back without its
        // latch, so both say the same (REF-12 A2). A revocation
        // never writes the helper's durable latch.
        if orbit_core::privacy_stop::latches_helper(&self.state) {
            self.native_send("privacy_stop", json!({})).await?;
        }
        self.native_send("status", json!({})).await?;
        Ok(())
    }

    async fn on_native_response(&mut self, event: &Value) -> Result<()> {
        let response = NativeResponseFields::from_value(event)?;
        let command_name = event
            .get("_bridge_command")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| self.pending_native.get(&response.id).cloned());
        self.pending_native.remove(&response.id);
        if !response.ok {
            self.record_failed_order(&response, command_name.as_deref());
        }
        if self.wake_phrase_response(&response) {
            return Ok(());
        }
        if let Some(stop) = self.capture.retired_stops.remove(&response.id) {
            let outcome = if response.ok {
                match response.result.as_ref() {
                    None => "retired_audio_stop_result_absent",
                    Some(result) => {
                        match decode_result_body::<RetiredAudioStopResult>("audio_stop", result) {
                            Err(_) => "retired_audio_stop_result_malformed",
                            Ok(receipt) if !receipt.drain_complete => {
                                "retired_audio_stop_drain_incomplete"
                            }
                            Ok(receipt) if receipt.stream_id != stop.stream => {
                                "retired_audio_stop_stream_mismatch"
                            }
                            Ok(_) => "confirmed",
                        }
                    }
                }
            } else {
                response
                    .error
                    .as_ref()
                    .map_or("retired_audio_stop_error_absent", |error| {
                        error.code.as_str()
                    })
            };
            if outcome == "confirmed" {
                self.log(
                    "retired_audio_stop_confirmed",
                    &[("id", &response.id), ("stream_id", &stop.stream)],
                );
            } else if outcome == "stream_not_found" {
                self.log(
                    "retired_audio_stop_stream_absent",
                    &[("id", &response.id), ("stream_id", &stop.stream)],
                );
            } else if self.state.privacy_stopped()
                && matches!(outcome, "privacy_stopped" | "operation_cancelled_by_stop")
            {
                self.log(
                    "retired_audio_stop_cancelled_by_privacy_stop",
                    &[
                        ("id", &response.id),
                        ("stream_id", &stop.stream),
                        ("code", outcome),
                    ],
                );
            } else {
                self.log(
                    "retired_audio_stop_unconfirmed",
                    &[
                        ("id", &response.id),
                        ("stream_id", &stop.stream),
                        ("code", outcome),
                    ],
                );
                if stop.may_finish_call {
                    self.say(
                        Notice::new(NoticeCode::RetiredStopUnconfirmed)
                            .with_token(NoticeKey::Code, outcome),
                    );
                }
            }
            if stop.may_finish_call
                && matches!(outcome, "confirmed" | "stream_not_found")
                && self.closing_call_owns_stream(&stop.stream)
            {
                self.capture.mic_stream = None;
                let segments = self.capture.buffer.pause_source("mic_local")?;
                self.seal(segments)?;
                self.complete_audio_drain().await?;
            }
            return Ok(());
        }
        if !response.ok
            && response
                .error
                .as_ref()
                .is_some_and(|error| error.code == "stream_not_found")
            && self.call_stop_overtook_unanswered_start(&response.id)
        {
            self.log(
                "call_stop_overtook_unanswered_start",
                &[("id", &response.id), ("code", "stream_not_found")],
            );
            self.mark_overtaken_stop_answered(&response.id);
            return Ok(());
        }
        if self.audio_sources_response(event).await? {
            return Ok(());
        }
        if self.calendar_response(event) {
            return Ok(());
        }
        if self.source_response(event).await? {
            self.answer_bridge_from_sources(event).await?;
            return Ok(());
        }
        // The capture registries name an ACK after the generic index has
        // evicted it. That absence is unknown, not proof the ACK belongs to
        // nobody (REF-12). An id no registry sent is still unpaired, below.
        if response.ok
            && let Some(result) = response.result.as_ref()
        {
            if self.call_start_response(&response.id, result).await? {
                return Ok(());
            }
            if self.audio_response(&response.id, result).await? {
                return Ok(());
            }
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
        if command_name == "audio_start"
            && (self.capture.retired_call_starts.contains_key(&response.id)
                || !self
                    .capture
                    .pending_starts
                    .iter()
                    .any(|id| id == &response.id))
        {
            let retired_call_stream = self.capture.retired_call_starts.remove(&response.id);
            let code = response.error.as_ref().map(|error| error.code.as_str());
            self.log(
                "native_audio_start_after_retirement",
                &[
                    ("id", &response.id),
                    ("cause", "arranque ya retirado"),
                    ("code", code.unwrap_or("confirmed")),
                ],
            );
            if let Some(code) = code {
                if let Some(stream) = retired_call_stream.as_deref()
                    && self.closing_call_start_owns_stream(&response.id, stream)
                {
                    // The stop found no stream because it overtook the start;
                    // this failed start proves the expected stream never opened.
                    self.retire_pending_stream_stop(stream, true).await?;
                    if !self.state.privacy_stopped() && self.capture.audio.full_stop.is_none() {
                        self.replace_drain_wait(
                            orbit_core::notice::Notice::new(
                                orbit_core::notice::NoticeCode::AudioStartRefused,
                            )
                            .with_token(orbit_core::notice::NoticeKey::Code, code),
                        );
                    }
                    self.capture.mic_stream = None;
                    let segments = self.capture.buffer.pause_source("mic_local")?;
                    self.seal(segments)?;
                    self.complete_audio_drain().await?;
                    if self.capture.call_runtime.is_none() {
                        self.say(orbit_core::notice::NoticeCode::CallClosed);
                    }
                }
                if self.capture.audio.full_stop.is_some()
                    && !self.state.privacy_stopped()
                    && !matches!(code, "operation_cancelled_by_stop" | "privacy_stopped")
                {
                    self.say(
                        Notice::new(NoticeCode::OtherTrackStopped)
                            .with_token(NoticeKey::Code, code),
                    );
                    self.store_actor.snapshot(&self.state).await?;
                }
            } else if let Some(result) = response.result.as_ref() {
                match decode_result_body::<RetiredAudioStartResult>("audio_start", result) {
                    Ok(start) if start.stream_id.is_empty() => {
                        self.log("retired_audio_start_empty_stream", &[("id", &response.id)])
                    }
                    Ok(start) if start.stream_id.len() > MAX_RETIRED_START_STREAM_ID_BYTES => {
                        self.log(
                            "retired_audio_start_stream_too_long",
                            &[
                                ("id", &response.id),
                                ("bytes", &start.stream_id.len().to_string()),
                                ("max_bytes", &MAX_RETIRED_START_STREAM_ID_BYTES.to_string()),
                            ],
                        );
                    }
                    Ok(start)
                        if retired_call_stream
                            .as_ref()
                            .is_some_and(|expected| expected != &start.stream_id) =>
                    {
                        if let Some(expected) = retired_call_stream.as_deref() {
                            self.log(
                                "retired_call_start_stream_mismatch",
                                &[
                                    ("id", &response.id),
                                    ("expected", expected),
                                    ("observed", &start.stream_id),
                                ],
                            );
                        }
                        self.contain(
                            orbit_core::privacy_stop::StopCause::HelperIntegrityFailure,
                            "El Mac confirmó un flujo distinto del arranque de llamada retirado; se corta la captura para cerrar cualquier flujo incierto.",
                        )
                        .await;
                        if !self.state.privacy_stopped() && self.capture.audio.full_stop.is_none() {
                            self.request_full_audio_stop(super::audio::FullStop::Cut)
                                .await?;
                        }
                    }
                    Ok(start)
                        if (self.capture.mic_stream.as_deref() == Some(&start.stream_id)
                            && !self.closing_call_start_owns_stream(
                                &response.id,
                                &start.stream_id,
                            ))
                            || self.capture.remote_stream.as_deref() == Some(&start.stream_id) =>
                    {
                        self.log("retired_audio_start_live_stream", &[("id", &response.id)]);
                    }
                    Ok(start) => {
                        if self.closing_call_start_owns_stream(&response.id, &start.stream_id)
                            && !self
                                .retire_pending_stream_stop(&start.stream_id, false)
                                .await?
                        {
                            return Ok(());
                        }
                        if let Err(error) = self.request_retired_stream_stop(&start.stream_id).await
                        {
                            self.log(
                                "retired_audio_stop_request_failed",
                                &[("id", &response.id), ("error", &error.to_string())],
                            );
                        }
                    }
                    Err(error) => {
                        self.log(
                            "retired_audio_start_invalid_result",
                            &[("id", &response.id), ("error", &error.to_string())],
                        );
                        if let Some(expected) = retired_call_stream.as_deref()
                            && let Err(error) = self.request_retired_stream_stop(expected).await
                        {
                            self.log(
                                "retired_audio_stop_request_failed",
                                &[("id", &response.id), ("error", &error.to_string())],
                            );
                        }
                    }
                }
            } else if let Some(expected) = retired_call_stream.as_deref() {
                self.log("retired_call_start_result_absent", &[("id", &response.id)]);
                if let Err(error) = self.request_retired_stream_stop(expected).await {
                    self.log(
                        "retired_audio_stop_request_failed",
                        &[("id", &response.id), ("error", &error.to_string())],
                    );
                }
            }
            return Ok(());
        }
        if !response.ok {
            // A late refusal of any order sent before the brake may not
            // rewrite the cause and may not ask the helper to stop again:
            // privacy is already latched and the helper already knows.
            let error = response
                .error
                .as_ref()
                .context("response field=error type=absent")?;
            let code = error.code.as_str();
            if code == "privacy_stopped" && self.state.privacy_stopped() {
                self.log("native_rejected_after_privacy_stop", &[]);
                return Ok(());
            }
            let command_kind = command_name.as_str();
            let error_code = code;
            let outcome = orbit_core::failure::classify_native_response(
                command_kind,
                error_code,
                &self.state,
            );
            match outcome {
                Outcome::Refused { reason } => {
                    // An audio_start refused for a permission or
                    // scope reason interrupts the recording without
                    // latching; the brake stays reserved for
                    // integrity and intent. The pending start cycle
                    // is dropped so the helper's late ACK (or its
                    // absence) cannot advance the phase.
                    if command_kind == "audio_start" {
                        let refused_voice =
                            self.state.durable.voice_active
                                && !self.state.durable.recording.as_ref().is_some_and(
                                    |recording| recording.phase.interrupted_on_recovery(),
                                );
                        let empty_voice = self.state.durable.voice_active
                            && self
                                .capture
                                .buffer
                                .voice
                                .as_ref()
                                .is_some_and(|voice| voice.pcm.is_empty());
                        self.capture.pending_starts.retain(|id| id != &response.id);
                        if !refused_voice && self.capture.interruption.is_none() {
                            let cause = match reason.as_str() {
                                "microphone_permission_required" => {
                                    orbit_core::RecordingInterruptionCause::MicrophonePermissionRequired
                                }
                                "screen_permission_required" => {
                                    orbit_core::RecordingInterruptionCause::ScreenPermissionRequired
                                }
                                _ => orbit_core::RecordingInterruptionCause::CaptureStartRefused,
                            };
                            self.capture.interruption = Some(orbit_core::RecordingInterruption {
                                cause,
                                detail: Some(reason.clone()),
                                audio_available: false,
                            });
                        }
                        self.apply(Event::CaptureRefused {
                            reason: reason.clone(),
                        })
                        .await?;
                        if empty_voice {
                            // No voice transcript exists after a refused start.
                            // Flushing an empty voice buffer would replace the
                            // helper's permission cause with "No se capturó audio".
                            self.discard_refused_voice();
                        }
                        if self.capture.remote_stream.is_some() {
                            // A second track may have started in the helper.
                            // Keep its identity and incoming tail frames until
                            // the whole-capture drain is acknowledged.
                            self.request_full_audio_stop(super::audio::FullStop::Cut)
                                .await?;
                        } else if refused_voice && empty_voice {
                            self.capture.mic_stream = None;
                        } else {
                            // A one-track start was refused; no producer can
                            // still deliver audio for this recording.
                            self.capture.mic_stream = None;
                            Box::pin(self.flush_stopped_audio()).await?;
                        }
                    } else if reason == "operation_cancelled_by_stop" {
                        // The helper says this only when a stop it was given
                        // overtook the order: the stop is what happened, and
                        // it says so with its own notice. A cancelled
                        // `audio_stop` under a cut is not a refusal (SIM-57).
                        self.log(
                            "native_order_cancelled_by_stop",
                            &[("step", command_kind), ("id", &response.id)],
                        );
                    } else {
                        self.say(
                            Notice::new(NoticeCode::MacCommandRefused)
                                .with_token(NoticeKey::Step, command_kind)
                                .with_token(NoticeKey::Code, &reason),
                        );
                    }
                    return Ok(());
                }
                Outcome::Degraded { kind, detail } => {
                    // The taxonomy adds its notice through the core;
                    // the helper code keeps its own entry without
                    // erasing the call/recording context above it.
                    self.apply(Event::Degraded(kind)).await?;
                    self.say(
                        Notice::new(NoticeCode::MacCommandDegraded)
                            .with_token(NoticeKey::Step, command_kind)
                            .with_token(NoticeKey::Code, &detail),
                    );
                    return Ok(());
                }
                Outcome::IntegrityFailure { detail } => {
                    // REF-12 A4: the audio at risk is cut with the
                    // helper's code; nothing latches.
                    let message = error.message.as_str();
                    self.contain_coded(
                            orbit_core::privacy_stop::StopCause::HelperIntegrityFailure,
                            &detail,
                            &format!(
                                "El Mac no pudo garantizar el audio ({command_kind}: {detail}). {message}"
                            ),
                        )
                        .await;
                    return Ok(());
                }
            }
        }
        let result = response
            .result
            .as_ref()
            .context("response field=result type=absent")?;
        self.observe_voice_capabilities(result)?;
        let state_result = matches!(
            command_name.as_str(),
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
                decode_result_body::<NativeStateResult>(&command_name, result)?;
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
            // The helper reports the calendar in this same payload and it
            // used to be dropped on the floor, so the one permission Orbit
            // had never requested was also the one never observed.
        }
        self.capture.pending_starts.retain(|id| id != &response.id);
        self.capture_started_if_ready().await?;
        Ok(())
    }

    async fn on_native_state(&mut self, event: &Value) -> Result<()> {
        self.observe_voice_capabilities(event)?;
        // The helper's `state` is the one exact word about its
        // durable latch: set means her stop is held there, and
        // both sides must say the same (REF-12 A2).
        let state = NativeStateStop::deserialize(event)?;
        if state.privacy_stopped && !self.state.privacy_stopped() {
            self.privacy_stop(orbit_core::privacy_stop::MarsStop::HelperLatch { code: None })
                .await;
        }
        Ok(())
    }

    async fn on_native_capture_stopped(&mut self, event: &Value) -> Result<()> {
        let stop = NativeStopFields::deserialize(event)?;
        let code = stop.reason.unwrap_or("unreported");
        let echo = orbit_core::privacy_stop::DAEMON_ECHO_CODES.contains(&code);
        if orbit_core::privacy_stop::latches(code) && !echo {
            if !self.state.privacy_stopped() {
                self.privacy_stop(orbit_core::privacy_stop::MarsStop::Helper {
                    code: code.to_owned(),
                })
                .await;
            }
        } else if stop.privacy_stopped || orbit_core::privacy_stop::latches(code) {
            if !self.state.privacy_stopped() {
                self.privacy_stop(orbit_core::privacy_stop::MarsStop::HelperLatch {
                    code: Some(code.to_owned()),
                })
                .await;
            }
        } else {
            self.helper_capture_stop(code).await;
            // P9-11: the call's application closed after its call.
            self.call_capture_cut(code).await;
        }
        Ok(())
    }

    async fn on_native_local_rearm(&mut self) -> Result<()> {
        self.apply(Event::LocalRearm).await?;
        self.features.calendar.read_failure = None;
        self.connect();
        Ok(())
    }

    async fn on_native_helper_disconnected(&mut self, event: &Value) -> Result<()> {
        let cause = NativeHelperDisconnect::deserialize(event)?;
        // The line that explains a menu bar icon vanishing without a
        // crash report, which is otherwise invisible after the fact.
        self.log(
            "helper_disconnected",
            &[(
                "recording",
                if self.state.durable.recording.is_some() {
                    "true"
                } else {
                    "false"
                },
            )],
        );
        self.state = decide(&self.state, Event::HelperLost, now())?.state;
        self.call_signal_lost().await;
        self.voice_trigger_unknown();
        self.features.question.helper_listens = false;
        self.clear_audio_sources();
        for source in ["mic_local", "remote_app"] {
            self.note_bound_audio_gap(source)?;
        }
        let text = match cause.cause {
            Some(detail)
                if detail.starts_with("native_audio_invalid event=audio field=")
                    && detail.len() <= MAX_NATIVE_AUDIO_CAUSE_BYTES =>
            {
                format!(
                    "Orbit cortó el enlace con su helper nativo en este Mac porque el audio \
                     llegaba mal formado ({detail})."
                )
            }
            Some(_) | None => "Se perdió el helper nativo de Orbit en este Mac.".to_owned(),
        };
        self.contain(orbit_core::privacy_stop::StopCause::HelperLost, &text)
            .await;
        Ok(())
    }

    async fn on_native_hotkey(
        &mut self,
        event: &Value,
        now_ms: i64,
        utc_offset_ms: Option<i64>,
    ) -> Result<()> {
        let hotkey = NativeHotkeyFields::deserialize(event)?;
        let pressed = hotkey.phase == "pressed";
        let result: Result<()> = async {
            match hotkey.action {
                "ptt" => {
                    self.apply(if pressed {
                        Event::VoiceStart
                    } else {
                        Event::VoiceEnd
                    })
                    .await?
                }
                "privacy_stop" if pressed => {
                    self.privacy_stop(orbit_core::privacy_stop::MarsStop::Hotkey)
                        .await
                }
                "call_toggle" if pressed => {
                    self.apply(if self.state.durable.call.is_some() {
                        Event::CallStop
                    } else {
                        Event::CallStart {
                            id: id(),
                            duration_ms: 15 * 60_000,
                        }
                    })
                    .await?
                }
                "meeting_toggle" if pressed => {
                    if self.recording_active() {
                        self.command(Command::MeetingFinish).await?
                    } else {
                        // Started from the keyboard this used to pass no
                        // application at all, so the recording held one
                        // voice and nobody learned that until reading the
                        // transcript. The Mac now sends what is running
                        // and the choice is made here.
                        let choice =
                            orbit_core::meeting_audio::choose(&meeting_applications(event));
                        let audio = match &choice {
                            orbit_core::meeting_audio::AudioChoice::Application {
                                application_pid,
                                ..
                            } => orbit_core::protocol::MeetingAudio::Application {
                                application_pid: *application_pid,
                            },
                            orbit_core::meeting_audio::AudioChoice::NoCandidate
                            | orbit_core::meeting_audio::AudioChoice::Ambiguous { .. } => {
                                orbit_core::protocol::MeetingAudio::MicrophoneOnly
                            }
                        };
                        // Said before the meeting rather than after it:
                        // a recording with a single voice is only worth
                        // knowing about while it can still be fixed.
                        self.say(orbit_core::meeting_audio::notice(&choice));
                        self.command(Command::MeetingStart {
                            title: orbit_core::protocol::recording_title(
                                String::new(),
                                now_ms,
                                utc_offset_ms,
                            ),
                            audio,
                            duration_minutes: 60,
                            event: None,
                        })
                        .await?
                    }
                }
                _ => {}
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            if error
                .downcast_ref::<orbit_core::refusal::Refusal>()
                .is_none()
            {
                return Err(error);
            }
            // Expected interaction refusal is not audio corruption.
            // Persistence/native failures still propagate fail-closed.
            let notice = super::refusal_notice(&error, NoticeCode::ShortcutRefused);
            self.say_failure(notice, &error);
        }
        Ok(())
    }

    async fn on_native_audio(&mut self, event: &mut Value) -> Result<()> {
        if !self.accepts_audio(event) {
            return Ok(());
        }
        // A producer's lifetime is stable for a shared PTT turn. The
        // recorder separately assigns its common start/resume epoch.
        let stream_id = NativeAudioFrameRef::from_value(event)?.stream_id.to_owned();
        event
            .as_object_mut()
            .context("Evento audio sin objeto")?
            .insert("capture_epoch_id".into(), Value::String(stream_id));
        let received_at_ms = now();
        let turn_action = self.prepare_call_frame(event, received_at_ms).await?;
        let ingested = match self.capture.buffer.ingest_observed(event, received_at_ms) {
            Ok(ingested) => ingested,
            Err(error) => {
                let source = NativeAudioFrameRef::from_value(event)?.source;
                if let Err(gap_error) = self.note_bound_audio_gap(source) {
                    self.log(
                        "native_audio_gap_failed",
                        &[("cause", &gap_error.to_string())],
                    );
                }
                return Err(error);
            }
        };
        self.observe_recording_signal(ingested.signal, std::time::Instant::now());
        self.seal(ingested.segments)?;
        self.finish_call_frame(turn_action).await?;
        Ok(())
    }

    async fn on_native_gap(&mut self, event: &Value) -> Result<()> {
        if self.accepts_audio(event) {
            // A hole in a live or interrupted recording is declared:
            // the gaps counter moves and an additive notice lands,
            // the recording keeps going and no privacy stop is
            // requested. A gap that arrives for a paused meeting or
            // for a stream this Mac no longer owns cannot be
            // attributed to a recording the helper is still feeding,
            // so it bails closed unless the Mac is already tearing
            // the audio graph down (in which case the gap is part of
            // the teardown and is dropped without re-triggering a
            // stop).
            match orbit_core::failure::classify_gap(&self.state) {
                Outcome::Degraded { kind, .. } => {
                    let recording_attributable =
                        self.state.durable.recording.as_ref().is_some_and(|r| {
                            matches!(
                                r.phase,
                                CapturePhase::Recording
                                    | CapturePhase::Starting
                                    | CapturePhase::Interrupted
                            )
                        });
                    if recording_attributable {
                        self.apply(Event::Degraded(kind)).await?;
                        let gap = NativeGapSource::deserialize(event)?;
                        if gap.source == "mic_local" && gap.reason == Some("audio_device_changed") {
                            let segments = self.capture.buffer.restart_source(gap.source)?;
                            self.seal(segments)?;
                            let mut notice = Notice::new(NoticeCode::AudioRouteRecovered);
                            if let Some(duration_ns) = gap.duration_ns {
                                self.log(
                                    "audio_route_gap",
                                    &[("duration_ns", &duration_ns.to_string())],
                                );
                                notice = notice.with(NoticeKey::ElapsedMs, duration_ns / 1_000_000);
                            } else {
                                self.log("audio_route_gap", &[("duration_unknown", "true")]);
                            }
                            if !self.state.privacy_stopped()
                                && !self.audio_is_draining()
                                && self
                                    .state
                                    .durable
                                    .recording
                                    .as_ref()
                                    .is_some_and(|recording| {
                                        recording.phase == CapturePhase::Recording
                                    })
                            {
                                self.say(notice);
                            }
                        }
                        self.note_bound_track_gap(gap.source)?;
                    } else if !self.state.privacy_stopped() {
                        bail!(
                            "La fuente de audio notificó una discontinuidad sin grabación activa"
                        );
                    }
                }
                Outcome::Refused { .. } | Outcome::IntegrityFailure { .. } => {
                    // classify_gap always returns Degraded(AudioGap).
                }
            }
        }
        Ok(())
    }

    /// SIM-23: every order the helper answered with an error is recorded,
    /// whatever else the daemon does with the answer, so a failure on the
    /// Mac is never lost for want of a notice.
    fn record_failed_order(&self, response: &NativeResponseFields, named: Option<&str>) {
        let step = named
            .or_else(|| self.capture_response_command(&response.id))
            .or_else(|| {
                self.wake_order_pending(&response.id)
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

    /// SIM-23: a step the helper reports that answers no order. A code this
    /// daemon does not know is the helper's error, never silence.
    fn on_native_notice(&mut self, event: &Value) -> Result<()> {
        let orbit_core::wire::native::NativeEvent::Notice {
            code, abandonment, ..
        } = serde_json::from_value(event.clone())?
        else {
            bail!("Evento notice incoherente");
        };
        match code.as_str() {
            "wake_phrase_heard" => self.record(NoticeCode::WakePhraseHeard),
            "wake_phrase_abandoned" => {
                let mut notice = Notice::new(NoticeCode::WakePhraseAbandoned);
                if let Some(abandonment) = abandonment {
                    notice = notice
                        .with_token(NoticeKey::Cause, abandonment.cause.as_str())
                        .with(NoticeKey::ElapsedMs, abandonment.elapsed_ms)
                        .with(NoticeKey::SilentMs, abandonment.silent_ms);
                }
                self.record(notice);
            }
            "wake_phrase_question_dropped" => self.record(NoticeCode::WakePhraseQuestionDropped),
            other => {
                self.say(Notice::new(NoticeCode::HelperError).with_token(NoticeKey::Code, other))
            }
        }
        Ok(())
    }

    async fn on_native_error(&mut self, event: &Value) -> Result<()> {
        let error = NativeErrorCode::deserialize(event)?;
        let mut notice =
            Notice::new(NoticeCode::HelperError).with_token(NoticeKey::Code, error.code);
        if let Some(domain) = error.os_domain {
            notice = notice.with_token(NoticeKey::Cause, &domain.to_ascii_lowercase());
        }
        if let Some(status) = error.os_status {
            notice = notice.with(NoticeKey::Status, status);
        }
        self.say(notice);
        Ok(())
    }
}

#[cfg(test)]
mod result_fixture_tests {
    use super::*;

    #[tokio::test]
    async fn meeting_shortcut_uses_the_local_time_or_an_unknown_zone_title() -> Result<()> {
        let mut titles = Vec::new();
        for offset in [Some(7_200_000), None] {
            let directory = tempfile::tempdir()?;
            let (mut runtime, mut native) = super::super::audio::tests::fixture(directory.path())?;
            runtime
                .on_native_hotkey(
                    &json!({"action":"meeting_toggle","phase":"pressed"}),
                    1_800_000,
                    offset,
                )
                .await?;
            titles.push(
                runtime
                    .state
                    .durable
                    .recording
                    .as_ref()
                    .context("shortcut recording")?
                    .title
                    .clone(),
            );
            assert_eq!(native.try_recv()?["command"], "audio_start");
        }
        assert_eq!(titles, ["Reunión 02:30", "Reunión"]);
        Ok(())
    }

    #[test]
    fn retired_audio_receipts_use_the_same_bodies_as_the_helper_fixtures() -> Result<()> {
        let start: Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/native/results/audio_start.json"
        ))?;
        let receipt: RetiredAudioStartResult = decode_result_body("audio_start", &start)?;
        assert_eq!(receipt.stream_id, "stream-1");
        let mut stop: Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/native/results/audio_stop.json"
        ))?;
        let receipt: RetiredAudioStopResult = decode_result_body("audio_stop", &stop)?;
        assert_eq!(receipt.stream_id, "stream-1");
        assert!(receipt.drain_complete);
        stop.as_object_mut()
            .context("stop fixture body")?
            .remove("drain_complete");
        let error = decode_result_body::<RetiredAudioStopResult>("audio_stop", &stop)
            .err()
            .context("stop without a drain receipt was accepted")?;
        assert_eq!(error.field, "drain_complete");
        assert_eq!(error.observed_type, "absent");
        Ok(())
    }
}
