//! Executes the core's authority plan. Each slot has one adapter here.
use super::*;
use orbit_core::authority::{AuthorityChange, RetirePlan, RetireSlot};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RetirePhase {
    ReadEpoch,
    DeviceAdmission,
    BeforeDecision,
    Volatile,
    PreSnapshot,
    Link,
    Durable,
}

pub(super) fn computer_reason(change: AuthorityChange) -> &'static str {
    match change {
        AuthorityChange::UserStop | AuthorityChange::Revoked => {
            "Todo está parado: conexión, observaciones y autorización de control revocadas."
        }
        AuthorityChange::Closing => {
            "Orbit se está cerrando: conexión, observaciones y autorización de control retiradas."
        }
        AuthorityChange::Rearm => {
            "Orbit se reactivó: el control requiere una conexión y observación nuevas."
        }
        AuthorityChange::RearmWithoutStop => {
            "La autoridad local se renovó sin parada; el control requiere observación nueva."
        }
        AuthorityChange::Interrupt => "Control interrumpido; no se repetirá un intento incierto.",
        AuthorityChange::HubLink => "Cambió el enlace con el hub; el control anterior se retiró.",
        AuthorityChange::HubTransportReady => {
            "Llegó un transporte nuevo del hub; el control anterior se retiró."
        }
        AuthorityChange::HubLinkOffline => {
            "El enlace del hub quedó sin conexión; el control se retiró."
        }
        AuthorityChange::HubDisconnected => "El hub se desconectó; el control se retiró.",
        AuthorityChange::Conversation => "Cambió la conversación; su control anterior se retiró.",
        AuthorityChange::Device => "Cambió el dispositivo; el control anterior se retiró.",
    }
}

impl Runtime {
    /// Pair and Setup historically evict old-device reads even when their
    /// parameters are rejected. Only the read slot is retired at admission;
    /// a successful device change runs the complete plan afterward.
    pub(super) async fn retire_device_request_reads(&mut self) -> Result<()> {
        let change = AuthorityChange::Device;
        self.execute_retire_plan(
            orbit_core::authority::retire(change),
            change,
            RetirePhase::DeviceAdmission,
            None,
        )
        .await
    }

    async fn retire_read_slot(&mut self, change: AuthorityChange) {
        self.reads.retire(change);
        if matches!(
            change,
            AuthorityChange::Rearm | AuthorityChange::RearmWithoutStop
        ) {
            // The stop already evicted old reads. Rearm only resets the epoch
            // marker; it must not evict a new local note cache entry.
            return;
        }
        if change == AuthorityChange::Device {
            for task in std::mem::take(&mut self.reads.tasks) {
                task.abort();
                match task.await {
                    Ok(()) => {}
                    Err(error) if error.is_cancelled() => {}
                    Err(error) => {
                        self.log("read_worker_join_failed", &[("error", &error.to_string())])
                    }
                }
            }
        }
    }

    pub(super) fn execute_retire_plan<'a>(
        &'a mut self,
        plan: RetirePlan,
        change: AuthorityChange,
        phase: RetirePhase,
        interrupted_turn: Option<&'a str>,
    ) -> impl std::future::Future<Output = Result<()>> + 'a {
        Box::pin(async move {
            let mut first_error = None;
            let slots = if phase == RetirePhase::Durable {
                plan.durable_slots
            } else {
                plan.slots
            };
            for slot in slots {
                if let Err(error) = self
                    .execute_retire_slot(*slot, change, phase, interrupted_turn)
                    .await
                {
                    self.log(
                        "retire_slot_failed",
                        &[
                            ("slot", &format!("{slot:?}")),
                            ("error", &error.to_string()),
                        ],
                    );
                    if phase == RetirePhase::BeforeDecision {
                        // Source retirement is the fallible gate for these
                        // commands. The base did not run the later slots or
                        // adopt the decision after that gate failed.
                        return Err(error);
                    }
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
            first_error.map_or(Ok(()), Err)
        })
    }

    async fn execute_retire_slot(
        &mut self,
        slot: RetireSlot,
        change: AuthorityChange,
        phase: RetirePhase,
        interrupted_turn: Option<&str>,
    ) -> Result<()> {
        match phase {
            RetirePhase::ReadEpoch => {
                if slot == RetireSlot::InFlightReads {
                    self.reads.retire(change);
                }
                Ok(())
            }
            RetirePhase::DeviceAdmission => {
                if slot == RetireSlot::InFlightReads {
                    self.retire_read_slot(change).await;
                }
                Ok(())
            }
            RetirePhase::BeforeDecision => self.retire_slot_before_decision(slot, change).await,
            RetirePhase::Volatile => {
                self.retire_slot_volatile(slot, change).await;
                Ok(())
            }
            RetirePhase::PreSnapshot => {
                if slot == RetireSlot::Computer && change != AuthorityChange::Interrupt {
                    self.revoke_computer_for(change).await;
                }
                Ok(())
            }
            RetirePhase::Link => {
                self.retire_slot_link(slot, change, interrupted_turn).await;
                Ok(())
            }
            RetirePhase::Durable => self.retire_slot_durable(slot, change).await,
        }
    }

    async fn revoke_computer_for(&mut self, change: AuthorityChange) {
        self.revoke_computer_with_cause(
            computer_reason(change),
            super::computer_control::ComputerRevocation::Authority(change),
        )
        .await;
    }

    /// In the base, these commands withdrew control and cancelled sources
    /// before adopting the new conversation or turn state. A failed source
    /// receipt must leave that decision unapplied.
    async fn retire_slot_before_decision(
        &mut self,
        slot: RetireSlot,
        change: AuthorityChange,
    ) -> Result<()> {
        use RetireSlot::*;
        if slot == Computer && change == AuthorityChange::Interrupt {
            self.revoke_computer_for(change).await;
        }
        // Each change closes the sources with its own cause:
        // a new conversation is not a local cancellation.
        if slot == Sources && change == AuthorityChange::Conversation {
            self.cancel_sources_with_cause(
                "conversation_changed",
                "La conversación cambió; la respuesta anterior no se aceptará.",
            )
            .await?;
        }
        if slot == Sources && change == AuthorityChange::Interrupt {
            self.cancel_sources().await?;
        }
        if slot == CallTurn && change == AuthorityChange::Interrupt {
            self.interrupt_call_turn();
        }
        Ok(())
    }

    async fn retire_slot_volatile(&mut self, slot: RetireSlot, change: AuthorityChange) {
        use RetireSlot::*;
        match slot {
            InFlightReads => self.retire_read_slot(change).await,
            Bridge => {
                // A lost hub released the bridge after applying Offline
                // in the base. Its Link phase below preserves that order.
                if !matches!(
                    change,
                    AuthorityChange::HubDisconnected | AuthorityChange::Rearm
                ) {
                    let computer = self.assistant.bridge_request.as_ref().is_some_and(|ask| {
                        matches!(
                            ask.capability,
                            orbit_core::device_bridge::Capability::PerformAction
                                | orbit_core::device_bridge::Capability::ControlTools
                        )
                    });
                    self.release_bridge_request(bridge_release_reason(change, computer));
                }
            }
            RecordingNotes => self.retire_recording_notes(),
            Calendar => {
                let (code, refusal) = calendar_retirement(change);
                self.revoke_calendar(code, refusal);
            }
            CallDetection => self.calls.retire_call_detection(),
            Sources => {
                if !matches!(
                    change,
                    AuthorityChange::Conversation | AuthorityChange::Interrupt
                ) {
                    self.cancel_sources_volatile();
                }
            }
            VoiceTranscription => {
                if self.retire_voice_transcription(change) {
                    // Messages are outside DurableSnapshot's fingerprint.
                    // The kernel consumes this before its decision snapshot.
                    self.persisted.force_once = true;
                }
            }
            CallTurn => {
                if change != AuthorityChange::Interrupt {
                    self.interrupt_call_turn();
                }
            }
            Playback => {
                self.assistant.speech_task.cancel();
                self.assistant.speak_next = false;
            }
            Upload => {
                self.transfers.worker.cancel();
            }
            Computer | HubLink => {}
        }
    }

    async fn retire_slot_link(
        &mut self,
        slot: RetireSlot,
        change: AuthorityChange,
        interrupted_turn: Option<&str>,
    ) {
        if slot == RetireSlot::HubLink {
            let reason = match change {
                AuthorityChange::UserStop | AuthorityChange::Revoked => {
                    hub_link::Retirement::PrivacyStop
                }
                AuthorityChange::Device => {
                    // Pair/Setup aborted the old hub task immediately in
                    // the base; no five-second drain precedes Offline.
                    self.link
                        .outbound
                        .retire(hub_link::Retirement::NetworkCancelled);
                    if let Some(task) = self.link.task.take() {
                        task.abort();
                    }
                    return;
                }
                AuthorityChange::Rearm
                | AuthorityChange::RearmWithoutStop
                | AuthorityChange::Closing
                | AuthorityChange::HubLink
                | AuthorityChange::HubTransportReady
                | AuthorityChange::HubLinkOffline
                | AuthorityChange::HubDisconnected
                | AuthorityChange::Conversation
                | AuthorityChange::Interrupt => hub_link::Retirement::AuthorityRetired,
            };
            self.retire_hub(interrupted_turn, reason).await;
        }
        if slot == RetireSlot::Bridge
            && matches!(
                change,
                AuthorityChange::HubDisconnected | AuthorityChange::Rearm
            )
        {
            self.release_bridge_request(BridgeReleaseReason::Cancelled);
        }
    }

    async fn retire_slot_durable(
        &mut self,
        slot: RetireSlot,
        change: AuthorityChange,
    ) -> Result<()> {
        use RetireSlot::*;
        match slot {
            Computer => {
                if matches!(change, AuthorityChange::UserStop | AuthorityChange::Closing) {
                    self.revoke_computer_for(change).await;
                }
            }
            Sources => self.cancel_sources().await?,
            Bridge | CallDetection | Calendar | VoiceTranscription | CallTurn | Playback
            | HubLink | Upload | InFlightReads | RecordingNotes => {}
        }
        Ok(())
    }
}

/// Why a retired change releases the bridge's request: a control request
/// names the change unless the user stopped; otherwise the user's stop is
/// said as such and anything else is a cancellation.
fn bridge_release_reason(change: AuthorityChange, computer: bool) -> BridgeReleaseReason<'static> {
    if computer && !matches!(change, AuthorityChange::UserStop | AuthorityChange::Revoked) {
        return BridgeReleaseReason::Authority(change);
    }
    match change {
        AuthorityChange::UserStop | AuthorityChange::Revoked => BridgeReleaseReason::PrivacyStopped,
        AuthorityChange::Rearm
        | AuthorityChange::RearmWithoutStop
        | AuthorityChange::Closing
        | AuthorityChange::HubLink
        | AuthorityChange::HubTransportReady
        | AuthorityChange::HubLinkOffline
        | AuthorityChange::HubDisconnected
        | AuthorityChange::Conversation
        | AuthorityChange::Interrupt
        | AuthorityChange::Device => BridgeReleaseReason::Cancelled,
    }
}

/// The code and refusal a retired calendar request answers with.
fn calendar_retirement(change: AuthorityChange) -> (&'static str, orbit_core::refusal::Refusal) {
    let code = match change {
        AuthorityChange::UserStop | AuthorityChange::Revoked => "privacy_stopped",
        AuthorityChange::Closing
        | AuthorityChange::Device
        | AuthorityChange::Rearm
        | AuthorityChange::RearmWithoutStop
        | AuthorityChange::HubLink
        | AuthorityChange::HubTransportReady
        | AuthorityChange::HubLinkOffline
        | AuthorityChange::HubDisconnected
        | AuthorityChange::Conversation
        | AuthorityChange::Interrupt => "mac_disconnected",
    };
    let refusal = if code == "privacy_stopped" || change == AuthorityChange::Closing {
        Runtime::bridge_refusal_code(code)
    } else {
        orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::Authority { change },
        }
    };
    (code, refusal)
}

#[cfg(test)]
mod authority_cause_tests {
    use super::*;

    #[test]
    fn each_computer_authority_retirement_names_its_actual_cause() {
        for (change, cause) in [
            (AuthorityChange::UserStop, "Todo está parado"),
            (AuthorityChange::Revoked, "Todo está parado"),
            (AuthorityChange::Closing, "Orbit se está cerrando"),
            (AuthorityChange::Rearm, "Orbit se reactivó"),
            (AuthorityChange::RearmWithoutStop, "sin parada"),
            (AuthorityChange::Interrupt, "Control interrumpido"),
            (AuthorityChange::HubLink, "enlace con el hub"),
            (
                AuthorityChange::HubTransportReady,
                "transporte nuevo del hub",
            ),
            (
                AuthorityChange::HubLinkOffline,
                "enlace del hub quedó sin conexión",
            ),
            (AuthorityChange::HubDisconnected, "hub se desconectó"),
            (AuthorityChange::Conversation, "Cambió la conversación"),
            (AuthorityChange::Device, "Cambió el dispositivo"),
        ] {
            let text = computer_reason(change);
            assert!(text.contains(cause), "{change:?}: {text}");
            if !matches!(change, AuthorityChange::UserStop | AuthorityChange::Revoked) {
                assert!(!text.contains("Todo está parado"), "{change:?}: {text}");
            }
        }
    }
}
