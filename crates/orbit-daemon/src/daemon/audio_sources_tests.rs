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
        anyhow::bail!("The verified inventory did not reach the topic");
    };
    let Knowledge::Present { value: apps, .. } = value.applications else {
        anyhow::bail!("The 40 verified applications did not arrive as present");
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
        anyhow::bail!("The unverifiable inventory hid the recording");
    };
    let Knowledge::Absent { refusal } = value.applications else {
        anyhow::bail!("The unverifiable inventory did not arrive as Absent");
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
        anyhow::bail!("The topic did not publish the pending inventory");
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
        let TopicData::Recording(Knowledge::Present { value, .. }) = receiver.borrow().data.clone()
        else {
            anyhow::bail!("The topic lost the pending inventory");
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
    assert!(!runtime.audio.inventory.audio_source_observed);
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
        anyhow::bail!("The recording topic disappeared");
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
async fn a_partial_inventory_request_is_absent_instead_of_a_false_complete_list() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    for _ in 0..63 {
        runtime
            .helper
            .sender
            .try_send(json!({"synthetic": "filler"}))?;
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
    assert_eq!(runtime.audio.inventory.audio_source_requests.len(), 1);
    let command_id = runtime
        .audio
        .inventory
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
