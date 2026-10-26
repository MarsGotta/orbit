//! Recording and capture: a recording's muted stretches and the calls the
//! Mac detects.
use super::Limit;

pub(super) fn limits() -> Vec<Limit> {
    vec![
        Limit {
            name: "CALL_SILENCE_MS",
            value: crate::silence::CALL_SILENCE_MS as u64,
            unit: "milliseconds",
            resource: "call silence before a presence question",
        },
        Limit {
            name: "CALL_ANSWER_MS",
            value: crate::silence::CALL_ANSWER_MS as u64,
            unit: "milliseconds",
            resource: "call presence answer window",
        },
        Limit {
            name: "RECORDING_SILENCE_MS",
            value: crate::silence::RECORDING_SILENCE_MS,
            unit: "milliseconds",
            resource: "recording silence before a presence question",
        },
        Limit {
            name: "RECORDING_ANSWER_MS",
            value: crate::silence::RECORDING_ANSWER_MS as u64,
            unit: "milliseconds",
            resource: "recording presence answer window",
        },
        // The stretches one recording's microphone may be muted, which its
        // closing manifest declares; the hub admits as many
        // (server/orbit_hub/protocol.py).
        Limit {
            name: "MAX_MUTED_SPANS",
            value: crate::microphone_mute::MAX_MUTED_SPANS as u64,
            unit: "spans",
            resource: "muted microphone stretches of one recording",
        },
        // The call detection's times. The app shows the countdown
        // (from the deadline the daemon publishes, never a copy of these).
        Limit {
            name: "CALL_CONFIRM_MS",
            value: crate::call_detection::CALL_CONFIRM_MS as u64,
            unit: "milliseconds",
            resource: "microphone hold before a call counts",
        },
        Limit {
            name: "CALL_RELEASE_MS",
            value: crate::call_detection::CALL_RELEASE_MS as u64,
            unit: "milliseconds",
            resource: "microphone release before a call is over",
        },
        Limit {
            name: "AUTO_STOP_COUNTDOWN_MS",
            value: crate::call_detection::AUTO_STOP_COUNTDOWN_MS as u64,
            unit: "milliseconds",
            resource: "countdown before a call's recording ends",
        },
        Limit {
            name: "CALL_EVENT_LOOKAHEAD_MS",
            value: crate::call_detection::EVENT_LOOKAHEAD_MS as u64,
            unit: "milliseconds",
            resource: "how soon an event starts to be a call's event",
        },
    ]
}
