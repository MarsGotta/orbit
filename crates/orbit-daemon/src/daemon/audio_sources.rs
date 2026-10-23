//! Volatile helper inventory for the meeting source picker.
//!
//! An inventory that cannot be verified is data, not a danger: it is shown as
//! unavailable with its cause and its numbers, and nothing else in Orbit
//! stops or waits for it.
use super::{Runtime, id};
use anyhow::Result;
use orbit_core::refusal::{Expected, ReadInterruption, Refusal, Resource};
use orbit_core::{
    AudioSource,
    wire::native::{NativeResponseFields, NativeResponseId, decode_result_body},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// How many applications one inventory reply may list.
///
/// What it protects: the parse and the IPC state from a helper that lists more
/// than it can identify. Why 32: it is the helper's own ceiling
/// (`ApplicationIdentityPolicy.maximumApplications`, the ceiling of one
/// inventory), so a longer reply is a helper breaking its own rule. The two are
/// compared by the guard of contracts/constants.json.
/// What it is not: a limit on how many applications the user may run.
const MAX_APPLICATIONS: usize = 32;
/// The largest inventory reply read, in bytes. The helper refuses to build a
/// larger one (`ApplicationIdentityPolicy.maximumResponseBytes`); 192 KiB
/// accommodates 32 bindings with their bounded paths. The registry guard
/// compares both copies. It bounds one IPC read, not the list shown.
const MAX_RESPONSE_BYTES: usize = 196_608;
/// The helper accepts at most eight bundle identifiers per read.
const MAX_BUNDLE_IDS: usize = 8;
/// Longest inventory read, from its first to its last observation, in
/// nanoseconds: the helper's identity lifetime (15 s). An older list no
/// longer describes what is open.
const MAX_READ_NS: u64 = 15_000_000_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryReply {
    native_instance_id: String,
    privacy_generation: u64,
    clock_domain: String,
    observed_monotonic_ns: u64,
    completed_monotonic_ns: u64,
    omitted_count: u64,
    applications: Vec<ApplicationRow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplicationRow {
    application_pid: i32,
    application_name: String,
    bundle_id: String,
    binding: Option<Value>,
    unavailable_reason: Option<String>,
}

impl Runtime {
    pub(super) fn refresh_audio_sources(&mut self) -> Result<()> {
        self.state.views.audio_sources.clear();
        self.state.views.audio_sources_unavailable = None;
        self.audio.inventory.audio_source_requests.clear();
        self.audio.inventory.audio_source_observed = false;
        if self.state.privacy_stopped() || !self.state.capabilities().native_helper() {
            return Err(if self.state.privacy_stopped() {
                Refusal::PrivacyStopped { since_ms: None }
            } else {
                Refusal::HelperNotRunning
            }
            .into());
        }
        let bundle_ids = orbit_core::meeting_audio::candidate_bundle_ids();
        let total_reads = bundle_ids.chunks(MAX_BUNDLE_IDS).len();
        // The helper accepts at most eight bundle IDs per read. An empty known
        // set deliberately means an empty list and performs no unbounded scan.
        for (sent_reads, bundle_ids) in bundle_ids.chunks(MAX_BUNDLE_IDS).enumerate() {
            let command_id = format!("audio-sources-{}", id());
            let requested = bundle_ids.to_vec();
            let command = crate::native::command(
                "application_sources",
                json!({"id":command_id,"bundle_ids":requested}),
            )?;
            if self.helper.sender.try_send(command.clone()).is_err() {
                let cause = Refusal::ReadInterrupted {
                    cause: ReadInterruption::InventoryDispatch {
                        sent_reads: sent_reads as u64,
                        total_reads: total_reads as u64,
                    },
                };
                self.state.views.audio_sources_unavailable = Some(cause.clone());
                return Err(cause.into());
            }
            self.helper.record_pending(&command);
            self.audio
                .inventory
                .audio_source_requests
                .insert(command_id, requested);
        }
        self.audio.inventory.audio_source_observed =
            self.audio.inventory.audio_source_requests.is_empty();
        Ok(())
    }

    pub(super) async fn audio_sources_response(&mut self, event: &Value) -> Result<bool> {
        let header: NativeResponseId = decode_result_body("application_sources", event)?;
        let command_id = header.id.as_str();
        let Some(requested) = self
            .audio
            .inventory
            .audio_source_requests
            .remove(command_id)
        else {
            return Ok(false);
        };
        let response: NativeResponseFields = decode_result_body("application_sources", event)?;
        // The helper's own code, so a second failure says its own cause.
        let code = response.error.as_ref().map_or_else(
            || "audio_inventory_unverifiable".to_owned(),
            |error| error.code.clone(),
        );
        let verified = if !response.ok {
            Err(Self::bridge_refusal_code(&code))
        } else {
            match response.result.as_ref() {
                Some(result) => self.verified_inventory(result, &requested),
                None => Err(Refusal::HelperReplyInvalid {
                    field: "application_sources.result".into(),
                    expected: "object".into(),
                    received_type: "absent".into(),
                }),
            }
        };
        match verified {
            // One part that could not be verified makes the whole list
            // unavailable: the later parts are read and not shown, so a
            // partial list never passes for the complete one.
            Ok(_) if self.state.views.audio_sources_unavailable.is_some() => {}
            Ok(rows) => {
                self.state.views.audio_sources.extend(rows);
                self.state.views.audio_sources.sort_by(|left, right| {
                    left.application_name
                        .cmp(&right.application_name)
                        .then_with(|| left.application_pid.cmp(&right.application_pid))
                });
            }
            Err(cause) if self.state.views.audio_sources_unavailable.is_some() => {
                let text = cause.to_string();
                self.contain_coded(
                    orbit_core::privacy_stop::StopCause::AudioInventoryUnverifiable,
                    &code,
                    &text,
                )
                .await;
            }
            Err(cause) => {
                let text = cause.to_string();
                self.state.views.audio_sources.clear();
                self.state.views.audio_sources_unavailable = Some(cause);
                self.contain_coded(
                    orbit_core::privacy_stop::StopCause::AudioInventoryUnverifiable,
                    &code,
                    &text,
                )
                .await;
            }
        }
        self.audio.inventory.audio_source_observed =
            self.audio.inventory.audio_source_requests.is_empty()
                && self.state.views.audio_sources_unavailable.is_none();
        Ok(true)
    }

    /// The rows of one inventory reply, or why it cannot be trusted, with the
    /// numbers that decided it.
    fn verified_inventory(
        &self,
        result: &Value,
        requested: &[String],
    ) -> std::result::Result<Vec<AudioSource>, Refusal> {
        let bytes = serde_json::to_vec(result)
            .map_err(|_| Refusal::EncodeFailed {
                resource: Resource::Source,
            })?
            .len();
        if bytes > MAX_RESPONSE_BYTES {
            return Err(Refusal::TooLarge {
                resource: Resource::Source,
                bytes: bytes as u64,
                max_bytes: MAX_RESPONSE_BYTES as u64,
            });
        }
        let reply: InventoryReply = decode_result_body("application_sources", result)
            .map_err(Refusal::helper_reply_invalid)?;
        self.helper.check_inventory_host(&reply)?;
        check_inventory_reply(&reply)?;
        let requested = requested.iter().collect::<BTreeSet<_>>();
        let mut seen = self
            .state
            .views
            .audio_sources
            .iter()
            .map(|source| source.application_pid)
            .collect::<BTreeSet<_>>();
        let mut rows = Vec::with_capacity(reply.applications.len());
        for (position, row) in reply.applications.into_iter().enumerate() {
            rows.push(verified_row(position, row, &requested, &mut seen)?);
        }
        Ok(rows)
    }

    pub(super) fn clear_audio_sources(&mut self) {
        self.audio.inventory.audio_source_requests.clear();
        self.audio.inventory.audio_source_observed = false;
        self.state.views.audio_sources.clear();
        self.state.views.audio_sources_unavailable = None;
    }
}

/// The reply's clock, read times, omissions and count are what the
/// contract allows.
fn check_inventory_reply(reply: &InventoryReply) -> std::result::Result<(), Refusal> {
    if reply.clock_domain != "mach_absolute_time" {
        return Err(Refusal::HelperReplyInvalid {
            field: "clock_domain".into(),
            received_type: "string".into(),
            expected: Expected::ClockDomain {
                received: reply.clock_domain.clone(),
            },
        });
    }
    let Some(read_ns) = reply
        .completed_monotonic_ns
        .checked_sub(reply.observed_monotonic_ns)
        .filter(|_| reply.observed_monotonic_ns > 0)
    else {
        return Err(Refusal::HelperReplyInvalid {
            field: "read_times".into(),
            received_type: "number".into(),
            expected: Expected::ReadTimes {
                started_ns: reply.observed_monotonic_ns,
                completed_ns: reply.completed_monotonic_ns,
            },
        });
    };
    if read_ns >= MAX_READ_NS {
        return Err(Refusal::HelperReplyInvalid {
            field: "read_duration_ns".into(),
            received_type: "number".into(),
            expected: Expected::ReadDuration {
                read_ns,
                max_exclusive_ns: MAX_READ_NS,
            },
        });
    }
    if reply.omitted_count != 0 {
        return Err(Refusal::HelperReplyInvalid {
            field: "omitted_count".into(),
            received_type: "number".into(),
            expected: Expected::UnsignedRange {
                received: reply.omitted_count,
                min: 0,
                max: 0,
            },
        });
    }
    if reply.applications.len() > MAX_APPLICATIONS {
        return Err(Refusal::TooMany {
            resource: Resource::Source,
            count: reply.applications.len() as u64,
            max: MAX_APPLICATIONS as u64,
        });
    }
    Ok(())
}

/// One application row, checked as the contract and the request say: a
/// real and unique pid, a name and bundle within their bounds, a bundle
/// that was asked for, and a binding or the cause of its absence.
fn verified_row(
    position: usize,
    row: ApplicationRow,
    requested: &BTreeSet<&String>,
    seen: &mut BTreeSet<i32>,
) -> std::result::Result<AudioSource, Refusal> {
    let invalid = |field: &str, expected: &str, received_type: &str| Refusal::HelperReplyInvalid {
        field: format!("applications[{position}].{field}"),
        expected: expected.into(),
        received_type: received_type.into(),
    };
    if row.application_pid <= 0 {
        return Err(Refusal::HelperReplyInvalid {
            field: format!("applications[{position}].application_pid"),
            received_type: "number".into(),
            expected: Expected::SignedRange {
                received: i64::from(row.application_pid),
                min: 1,
                max: i64::from(i32::MAX),
            },
        });
    }
    if !seen.insert(row.application_pid) {
        return Err(invalid("application_pid", "unique_pid", "duplicate_pid"));
    }
    if row.application_name.is_empty() || row.application_name.len() > 128 {
        return Err(Refusal::HelperReplyInvalid {
            field: format!("applications[{position}].application_name"),
            received_type: "string".into(),
            expected: Expected::TextLength {
                bytes: row.application_name.len() as u64,
                max_bytes: 128,
            },
        });
    }
    if row.bundle_id.is_empty() || row.bundle_id.len() > 256 {
        return Err(Refusal::HelperReplyInvalid {
            field: format!("applications[{position}].bundle_id"),
            received_type: "string".into(),
            expected: Expected::TextLength {
                bytes: row.bundle_id.len() as u64,
                max_bytes: 256,
            },
        });
    }
    if !requested.contains(&row.bundle_id) {
        return Err(invalid(
            "bundle_id",
            "requested_bundle",
            "unrequested_bundle",
        ));
    }
    if row.binding.is_none()
        && row.unavailable_reason.as_deref() != Some("application_identity_unavailable")
    {
        return Err(invalid(
            "binding",
            "binding_or_application_identity_unavailable",
            "null",
        ));
    }
    Ok(AudioSource {
        application_pid: row.application_pid,
        application_name: row.application_name,
        bundle_id: row.bundle_id,
    })
}

impl crate::daemon::helper::HelperHandle {
    /// The reply comes from the helper this daemon knows, in its current
    /// privacy generation.
    fn check_inventory_host(&self, reply: &InventoryReply) -> std::result::Result<(), Refusal> {
        let host = self.host.identity();
        if reply.native_instance_id.is_empty()
            || host.is_some_and(|host| {
                host.id != reply.native_instance_id
                    || host.generation != reply.privacy_generation
                    || host.stopped
            })
        {
            return Err(Refusal::HelperReplyInvalid {
                field: "privacy_generation".into(),
                received_type: "different_generation_or_instance".into(),
                expected: Expected::CurrentGeneration {
                    received: reply.privacy_generation,
                    required: host.map(|host| host.generation),
                    instance_matches: host.is_some_and(|host| host.id == reply.native_instance_id),
                },
            });
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "audio_sources_tests.rs"]
mod tests;
