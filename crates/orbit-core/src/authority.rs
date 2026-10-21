//! Authority changes and the work each change retires.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityChange {
    MarsStop,
    Revoked,
    Rearm,
    RearmWithoutStop,
    Closing,
    HubLink,
    /// Transport ready while Connection was already Connecting.
    HubTransportReady,
    HubLinkOffline,
    HubDisconnected,
    Conversation,
    Interrupt,
    Device,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetireSlot {
    Bridge,
    Computer,
    /// P9-11: the call detection's offer and the recording it owns. Only
    /// Mars's stop, a revocation and closing retire it: it is local, and the
    /// hub coming and going says nothing about a call on this Mac.
    CallDetection,
    Calendar,
    Sources,
    VoiceTranscription,
    CallTurn,
    Playback,
    HubLink,
    Upload,
    InFlightReads,
    /// SIM-41: the private notes and the note and template orders on their
    /// way to the hub.
    RecordingNotes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetirePlan {
    /// Volatile order. ReadEpoch is run separately before the first await.
    pub slots: &'static [RetireSlot],
    /// The old stop retired computer, then sources after the hub drain. This
    /// order differs from the volatile abort order.
    pub durable_slots: &'static [RetireSlot],
}

pub fn retire(change: AuthorityChange) -> RetirePlan {
    use RetireSlot::*;
    // Slot order follows the base's volatile retirement. The calendar refusal
    // precedes bridge release (REF-303b A5), while ReadEpoch retires reads
    // before any of these slots can await. The cache lock is reached only
    // after all the old volatile aborts.
    let slots: &'static [RetireSlot] = match change {
        AuthorityChange::MarsStop | AuthorityChange::Revoked | AuthorityChange::Closing => &[
            Calendar,
            Bridge,
            CallDetection,
            Computer,
            Sources,
            VoiceTranscription,
            CallTurn,
            Playback,
            Upload,
            RecordingNotes,
            InFlightReads,
            HubLink,
        ],
        AuthorityChange::Rearm => &[InFlightReads, Computer, Bridge],
        AuthorityChange::RearmWithoutStop => &[InFlightReads, Computer],
        AuthorityChange::HubLink => &[],
        // P9-15: this plan held only the context query, which is retired.
        // The kernel, audio and computer control still react to the change
        // on their own, outside the plan.
        AuthorityChange::HubTransportReady => &[],
        AuthorityChange::HubLinkOffline => &[],
        AuthorityChange::HubDisconnected => &[Bridge],
        AuthorityChange::Conversation => &[VoiceTranscription, Sources],
        AuthorityChange::Interrupt => &[Computer, Sources, VoiceTranscription, CallTurn],
        AuthorityChange::Device => &[InFlightReads, VoiceTranscription, RecordingNotes, HubLink],
    };
    let durable_slots: &'static [RetireSlot] = match change {
        AuthorityChange::MarsStop | AuthorityChange::Revoked | AuthorityChange::Closing => {
            &[Computer, Sources]
        }
        AuthorityChange::Rearm
        | AuthorityChange::RearmWithoutStop
        | AuthorityChange::HubLink
        | AuthorityChange::HubTransportReady
        | AuthorityChange::HubLinkOffline
        | AuthorityChange::HubDisconnected
        | AuthorityChange::Conversation
        | AuthorityChange::Interrupt
        | AuthorityChange::Device => &[],
    };
    RetirePlan {
        slots,
        durable_slots,
    }
}

/// Preserve the old stopped-state fence on later decisions. These are the
/// slots `execute_boxed_decision` retired again on every stopped decision;
/// the hub, storage cache and durable jobs were never repeated by that fence.
pub fn retire_stopped_decision() -> RetirePlan {
    use RetireSlot::*;
    RetirePlan {
        slots: &[CallDetection, Computer],
        durable_slots: &[],
    }
}
