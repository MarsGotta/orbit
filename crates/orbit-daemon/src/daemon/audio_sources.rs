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
        self.features.meeting.audio_source_requests.clear();
        self.features.meeting.audio_source_observed = false;
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
            if self.native.try_send(command.clone()).is_err() {
                let cause = Refusal::ReadInterrupted {
                    cause: ReadInterruption::InventoryDispatch {
                        sent_reads: sent_reads as u64,
                        total_reads: total_reads as u64,
                    },
                };
                self.state.views.audio_sources_unavailable = Some(cause.clone());
                return Err(cause.into());
            }
            self.record_pending_native(&command);
            self.features
                .meeting
                .audio_source_requests
                .insert(command_id, requested);
        }
        self.features.meeting.audio_source_observed =
            self.features.meeting.audio_source_requests.is_empty();
        Ok(())
    }

    pub(super) async fn audio_sources_response(&mut self, event: &Value) -> Result<bool> {
        let header: NativeResponseId = decode_result_body("application_sources", event)?;
        let command_id = header.id.as_str();
        let Some(requested) = self
            .features
            .meeting
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
        self.features.meeting.audio_source_observed =
            self.features.meeting.audio_source_requests.is_empty()
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
        let host = self.features.native_host.identity();
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
        if reply.clock_domain != "mach_absolute_time" {
            return Err(Refusal::HelperReplyInvalid {
                field: "clock_domain".into(),
                received_type: "string".into(),
                expected: Expected::ClockDomain {
                    received: reply.clock_domain,
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
            let invalid =
                |field: &str, expected: &str, received_type: &str| Refusal::HelperReplyInvalid {
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
            rows.push(AudioSource {
                application_pid: row.application_pid,
                application_name: row.application_name,
                bundle_id: row.bundle_id,
            });
        }
        Ok(rows)
    }

    pub(super) fn clear_audio_sources(&mut self) {
        self.features.meeting.audio_source_requests.clear();
        self.features.meeting.audio_source_observed = false;
        self.state.views.audio_sources.clear();
        self.state.views.audio_sources_unavailable = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::audio::tests::fixture;
    use crate::daemon::topics::{TopicBoard, TopicSource};
    use anyhow::Context;
    use orbit_core::refusal::{ReadInterruption, Refusal};
    use orbit_core::topics::{Knowledge, TopicData, TopicId};

    #[tokio::test]
    async fn oversized_inventory_reply_preserves_bytes_and_ceiling_in_topic() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        runtime.refresh_audio_sources()?;
        let request = helper.try_recv()?;
        let event = json!({
            "event": "response", "id": request["id"], "ok": true,
            "result": {"padding": "x".repeat(MAX_RESPONSE_BYTES)}
        });
        let bytes = serde_json::to_vec(&event["result"])?.len();
        assert!(runtime.audio_sources_response(&event).await?);
        let mut board = TopicBoard::new("synthetic-epoch".into());
        let channels = board.channels();
        let receiver = channels[&TopicId::Recording].subscribe();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        let frame = receiver.borrow().clone();
        let TopicData::Recording(Knowledge::Present { value, .. }) = frame.data else {
            anyhow::bail!("the inventory failure hid the recording topic");
        };
        let Knowledge::Absent { refusal } = value.applications else {
            anyhow::bail!("the invalid inventory must be absent with its cause");
        };
        assert_eq!(
            serde_json::to_value(refusal)?,
            json!({
                "code": "too_large", "resource": "source", "bytes": bytes,
                "max_bytes": MAX_RESPONSE_BYTES, "retry": {"kind": "after_fix"}
            })
        );
        assert!(!runtime.state.privacy_stopped());
        Ok(())
    }

    #[test]
    fn application_inventory_fixture_is_read_by_the_production_verifier() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (runtime, _helper) = fixture(directory.path())?;
        let mut body: Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/native/results/application_sources.json"
        ))?;
        let requested = vec!["com.example.meeting".to_owned()];
        let rows = runtime
            .verified_inventory(&body, &requested)
            .map_err(anyhow::Error::msg)?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].application_pid, 4242);
        assert_eq!(rows[0].application_name, "Fixture");
        assert_eq!(rows[0].bundle_id, "com.example.meeting");
        body["applications"][0]["binding"] = Value::Null;
        let refusal = runtime
            .verified_inventory(&body, &requested)
            .err()
            .context("an inventory row without binding or cause was accepted")?;
        let refusal = serde_json::to_value(refusal)?;
        assert_eq!(refusal["code"], "helper_reply_invalid");
        assert_eq!(refusal["field"], "applications[0].binding");
        assert_eq!(
            refusal["expected"]["name"],
            "binding_or_application_identity_unavailable"
        );
        Ok(())
    }

    #[tokio::test]
    async fn wrong_inventory_generation_names_response_field_and_type() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        runtime.refresh_audio_sources()?;
        let request = helper.try_recv()?;
        let event = json!({
            "event": "response", "id": request["id"], "ok": true,
            "result": {
                "native_instance_id": "synthetic-host", "privacy_generation": "zero",
                "clock_domain": "mach_absolute_time", "observed_monotonic_ns": 1_000,
                "completed_monotonic_ns": 2_000, "omitted_count": 0,
                "applications": []
            }
        });
        assert!(runtime.audio_sources_response(&event).await?);
        let detail = runtime
            .state
            .views
            .audio_sources_unavailable
            .as_ref()
            .context("cause")?;
        let detail = serde_json::to_value(detail)?;
        assert_eq!(detail["code"], "helper_reply_invalid");
        assert_eq!(detail["field"], "privacy_generation");
        assert_eq!(detail["received_type"], "string");
        Ok(())
    }

    #[tokio::test]
    async fn forty_applications_across_helper_reads_are_all_visible() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        runtime.refresh_audio_sources()?;
        let first = helper.try_recv()?;
        let second = helper.try_recv()?;
        for (request, start, count) in [(first, 0, 30), (second, 30, 10)] {
            let bundle_id = request["bundle_ids"][0].as_str().context("bundle id")?;
            let applications: Vec<_> = (start..start + count)
                .map(|index| {
                    json!({
                        "application_pid": 1000 + index,
                        "application_name": format!("App {index}"),
                        "bundle_id": bundle_id,
                        "binding": null,
                        "unavailable_reason": "application_identity_unavailable"
                    })
                })
                .collect();
            let event = json!({
                "event": "response", "id": request["id"], "ok": true,
                "result": {
                    "native_instance_id": "synthetic-host", "privacy_generation": 0,
                    "clock_domain": "mach_absolute_time",
                    "observed_monotonic_ns": 1_000,
                    "completed_monotonic_ns": 2_000,
                    "omitted_count": 0, "applications": applications
                }
            });
            assert!(runtime.audio_sources_response(&event).await?);
        }
        assert_eq!(runtime.state.views.audio_sources.len(), 40);
        assert!(runtime.state.views.audio_sources_unavailable.is_none());
        assert!(!runtime.state.privacy_stopped());
        let mut board = TopicBoard::new("synthetic-epoch".into());
        let channels = board.channels();
        let receiver = channels[&TopicId::Recording].subscribe();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        let frame = receiver.borrow().clone();
        let payload_bytes = serde_json::to_vec(&frame.data)?.len();
        assert!(
            payload_bytes <= TopicId::Recording.max_bytes(),
            "40 aplicaciones ocupan {payload_bytes} bytes"
        );
        let TopicData::Recording(Knowledge::Present { value, .. }) = frame.data else {
            anyhow::bail!("el inventario verificado no llegó al tema");
        };
        let Knowledge::Present { value: apps, .. } = value.applications else {
            anyhow::bail!("las 40 aplicaciones verificadas no llegaron como presentes");
        };
        assert_eq!(apps.len(), 40);
        Ok(())
    }

    #[tokio::test]
    async fn unverifiable_inventory_publishes_absent_without_latching_privacy() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        runtime.refresh_audio_sources()?;
        let request = helper.try_recv()?;
        let bundle_id = request["bundle_ids"][0].as_str().context("bundle id")?;
        let applications: Vec<_> = (0..33)
            .map(|index| {
                json!({
                    "application_pid": 1000 + index,
                    "application_name": format!("App {index}"),
                    "bundle_id": bundle_id,
                    "binding": null,
                    "unavailable_reason": "application_identity_unavailable"
                })
            })
            .collect();
        let event = json!({
            "event": "response", "id": request["id"], "ok": true,
            "result": {
                "native_instance_id": "synthetic-host", "privacy_generation": 0,
                "clock_domain": "mach_absolute_time",
                "observed_monotonic_ns": 1_000,
                "completed_monotonic_ns": 2_000,
                "omitted_count": 0, "applications": applications
            }
        });
        assert!(runtime.audio_sources_response(&event).await?);
        assert!(!runtime.state.privacy_stopped());

        let mut board = TopicBoard::new("synthetic-epoch".into());
        let channels = board.channels();
        let receiver = channels[&TopicId::Recording].subscribe();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        let frame = receiver.borrow().clone();
        let TopicData::Recording(Knowledge::Present { value, .. }) = frame.data else {
            anyhow::bail!("el inventario no verificable ocultó la grabación");
        };
        let Knowledge::Absent { refusal } = value.applications else {
            anyhow::bail!("el inventario no verificable no llegó como Absent");
        };
        assert!(matches!(
            refusal,
            orbit_core::refusal::Refusal::TooMany {
                count: 33,
                max: 32,
                ..
            }
        ));
        Ok(())
    }

    #[tokio::test]
    async fn only_a_completed_empty_inventory_means_no_application_has_audio() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        runtime.refresh_audio_sources()?;
        let first = helper.try_recv()?;
        let second = helper.try_recv()?;
        let mut board = TopicBoard::new("synthetic-epoch".into());
        let channels = board.channels();
        let receiver = channels[&TopicId::Recording].subscribe();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        let TopicData::Recording(Knowledge::Present { value, .. }) = receiver.borrow().data.clone()
        else {
            anyhow::bail!("el tema no publicó el inventario pendiente");
        };
        assert!(matches!(value.applications, Knowledge::Unknown { .. }));

        for (index, request) in [first, second].into_iter().enumerate() {
            let event = json!({
                "event": "response", "id": request["id"], "ok": true,
                "result": {
                    "native_instance_id": "synthetic-host", "privacy_generation": 0,
                    "clock_domain": "mach_absolute_time",
                    "observed_monotonic_ns": 1_000,
                    "completed_monotonic_ns": 2_000,
                    "omitted_count": 0, "applications": []
                }
            });
            assert!(runtime.audio_sources_response(&event).await?);
            board.state_changed();
            board.publish(&TopicSource {
                state: &runtime.state,
                runtime: Some(&runtime),
                now_ms: (index as u64 + 1) * TopicId::Recording.min_interval_ms(),
            });
            let TopicData::Recording(Knowledge::Present { value, .. }) =
                receiver.borrow().data.clone()
            else {
                anyhow::bail!("el tema perdió el inventario pendiente");
            };
            if index == 0 {
                assert!(matches!(value.applications, Knowledge::Unknown { .. }));
            } else {
                assert!(
                    matches!(value.applications, Knowledge::Present { value, .. } if value.is_empty())
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn one_of_two_inventory_replies_cannot_publish_a_partial_list() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        runtime.refresh_audio_sources()?;
        let first = helper.try_recv()?;
        let _second = helper.try_recv()?;
        let bundle_id = first["bundle_ids"][0].as_str().context("bundle id")?;
        let reply = json!({
            "event": "response", "id": first["id"], "ok": true,
            "result": {
                "native_instance_id": "synthetic-host", "privacy_generation": 0,
                "clock_domain": "mach_absolute_time", "observed_monotonic_ns": 1_000,
                "completed_monotonic_ns": 2_000, "omitted_count": 0,
                "applications": [{
                    "application_pid": 123, "application_name": "Prueba",
                    "bundle_id": bundle_id, "binding": null,
                    "unavailable_reason": "application_identity_unavailable"
                }]
            }
        });
        assert!(runtime.audio_sources_response(&reply).await?);
        assert_eq!(runtime.state.views.audio_sources.len(), 1);
        assert!(!runtime.features.meeting.audio_source_observed);
        let mut board = TopicBoard::new("synthetic-epoch".into());
        let channels = board.channels();
        let receiver = channels[&TopicId::Recording].subscribe();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        let projected = receiver.borrow().data.clone();
        let TopicData::Recording(Knowledge::Present { value, .. }) = projected else {
            anyhow::bail!("el tema de grabación desapareció");
        };
        assert!(matches!(value.applications, Knowledge::Unknown { .. }));
        Ok(())
    }

    #[tokio::test]
    async fn a_second_failed_inventory_read_records_its_own_cause() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        runtime.refresh_audio_sources()?;
        let first = helper.try_recv()?;
        let second = helper.try_recv()?;
        let failed = |request: &serde_json::Value, code: &str| {
            json!({
                "event": "response", "id": request["id"], "ok": false,
                "error": {"code": code, "message": "Fallo sintético de inventario"}
            })
        };
        assert!(
            runtime
                .audio_sources_response(&failed(&first, "primero"))
                .await?
        );
        let count = runtime.state.durable.notice_log.records().len();
        assert!(
            runtime
                .audio_sources_response(&failed(&second, "segundo"))
                .await?
        );
        assert_eq!(runtime.state.durable.notice_log.records().len(), count + 1);
        assert!(
            runtime
                .state
                .durable
                .notice_log
                .records()
                .iter()
                .any(|notice| format!("{notice:?}").contains("segundo"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_partial_inventory_request_is_absent_instead_of_a_false_complete_list() -> Result<()>
    {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _helper) = fixture(directory.path())?;
        for _ in 0..63 {
            runtime.native.try_send(json!({"synthetic": "filler"}))?;
        }
        let error = runtime
            .refresh_audio_sources()
            .err()
            .context("partial request must fail")?;
        let detail = runtime
            .state
            .views
            .audio_sources_unavailable
            .as_ref()
            .context("partial inventory cause")?;
        let expected = Refusal::ReadInterrupted {
            cause: ReadInterruption::InventoryDispatch {
                sent_reads: 1,
                total_reads: 2,
            },
        };
        assert_eq!(error.downcast_ref::<Refusal>(), Some(&expected));
        assert_eq!(detail, &expected);
        assert_eq!(runtime.features.meeting.audio_source_requests.len(), 1);
        let command_id = runtime
            .features
            .meeting
            .audio_source_requests
            .keys()
            .next()
            .cloned()
            .context("sent inventory request")?;
        let late_reply = json!({
            "event": "response", "id": command_id, "ok": true,
            "result": {
                "native_instance_id": "synthetic-host", "privacy_generation": 0,
                "clock_domain": "mach_absolute_time",
                "observed_monotonic_ns": 1_000,
                "completed_monotonic_ns": 2_000,
                "omitted_count": 0, "applications": []
            }
        });
        assert!(runtime.audio_sources_response(&late_reply).await?);
        assert!(runtime.state.views.audio_sources.is_empty());
        assert!(
            runtime
                .state
                .views
                .audio_sources_unavailable
                .as_ref()
                .is_some_and(|refusal| refusal == &expected)
        );
        assert!(!runtime.state.privacy_stopped());
        Ok(())
    }
}
