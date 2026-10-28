//! Synthetic kernel pipes; no native host effects or providers.
use super::*;
use crate::ipc::read_frame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::ErrorKind,
    time::Duration,
};
use tokio::{net::unix::pipe, time::timeout};

fn pipes() -> Result<(pipe::Sender, pipe::Receiver, pipe::Receiver, pipe::Sender)> {
    let (input, reader) = pipe::pipe()?;
    let (writer, output) = pipe::pipe()?;
    Ok((input, output, reader, writer))
}

#[tokio::test]
async fn bridge_pairs_replies_and_forwards_an_id_it_never_sent_unnamed() -> Result<()> {
    let (mut input, output, reader, writer) = pipes()?;
    let (send, commands) = mpsc::channel(4);
    let (events, mut received) = mpsc::channel(4);
    let task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    let mut output = BufReader::new(output);
    let hello: Value =
        serde_json::from_slice(&timeout(Duration::from_secs(2), read_frame(&mut output)).await??)?;
    assert_eq!(hello["command"], "hello");
    // The helper cannot name its own reply: a `_bridge_command` it sends is
    // stripped, so an unpaired reply reaches the daemon unnamed.
    let unknown = json!({"v":1,"event":"response","id":"never-sent","_bridge_command":"permissions",
        "ok":true,"result":{"microphone":"authorized"}});
    input
        .write_all(format!("{unknown}\n{unknown}\n").as_bytes())
        .await?;

    let unknown1 = timeout(Duration::from_secs(2), received.recv())
        .await?
        .context("first unknown response absent")?;
    assert_eq!(unknown1["id"], "never-sent");
    assert!(unknown1.get("_bridge_command").is_none());

    let unknown2 = timeout(Duration::from_secs(2), received.recv())
        .await?
        .context("second unknown response absent")?;
    assert_eq!(unknown2["id"], "never-sent");
    assert!(unknown2.get("_bridge_command").is_none());

    for name in ["hello", "status", "permissions"] {
        let command = if name == "hello" {
            hello.clone()
        } else {
            let command = command(name, json!({}))?;
            send.send(command.clone()).await?;
            loop {
                let written: Value = serde_json::from_slice(
                    &timeout(Duration::from_secs(2), read_frame(&mut output)).await??,
                )?;
                if written["id"] == command["id"] {
                    break written;
                }
            }
        };
        let fixture: Value = serde_json::from_str(match name {
            "hello" => include_str!("../../../contracts/fixtures/native/results/hello.json"),
            "status" => include_str!("../../../contracts/fixtures/native/results/status.json"),
            "permissions" => {
                include_str!("../../../contracts/fixtures/native/results/permissions.json")
            }
            _ => unreachable!(),
        })?;
        input
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "v":1,"event":"response","id":command["id"],"ok":true,
                        "result":fixture
                    })
                )
                .as_bytes(),
            )
            .await?;
        let reply = timeout(Duration::from_secs(2), received.recv())
            .await?
            .context("paired response absent")?;
        assert_eq!(reply["id"], command["id"]);
        assert_eq!(reply["_bridge_command"], name);
        assert_eq!(reply["result"], fixture);
    }
    assert!(!task.is_finished(), "unknown response ended the bridge");
    drop(send);
    timeout(Duration::from_secs(2), task).await???;
    Ok(())
}

#[test]
fn unanswered_heartbeats_do_not_evict_another_command() {
    let mut pending = BTreeMap::new();
    retain_pending_command(&mut pending, "0-status".into(), "status".into());
    for index in 0..63 {
        retain_pending_command(&mut pending, format!("hb-{index:02}"), "heartbeat".into());
    }
    assert_eq!(pending.len(), 64);
    retain_pending_command(&mut pending, "hb-new".into(), "heartbeat".into());
    assert_eq!(pending.len(), 64);
    assert_eq!(pending.get("0-status").map(String::as_str), Some("status"));
    assert!(pending.contains_key("hb-new"));
    assert_eq!(
        pending
            .values()
            .filter(|name| name.as_str() == "heartbeat")
            .count(),
        63
    );

    let mut full = BTreeMap::new();
    for index in 0..64 {
        retain_pending_command(&mut full, format!("cmd-{index:02}"), "status".into());
    }
    retain_pending_command(&mut full, "cmd-new".into(), "status".into());
    assert_eq!(full.len(), 64);
    assert!(full.contains_key("cmd-new"));
    assert!(!full.contains_key("cmd-00"));
}

#[test]
fn unpaired_response_is_reported_once_per_id_within_its_own_ceiling() {
    let mut reported = BTreeSet::new();
    assert_eq!(
        report_unpaired_response_once(&mut reported, "never-sent"),
        Some("never-sent".into())
    );
    assert!(report_unpaired_response_once(&mut reported, "never-sent").is_none());
    for index in 1..MAX_REPORTED_UNPAIRED_RESPONSES {
        assert!(report_unpaired_response_once(&mut reported, &format!("id-{index}")).is_some());
    }
    assert_eq!(reported.len(), MAX_REPORTED_UNPAIRED_RESPONSES);
    assert!(
        report_unpaired_response_once(&mut reported, "one-too-many").is_none(),
        "the ceiling bounds the diagnostic set"
    );
}

#[test]
fn unsupported_native_command_reports_its_name_instead_of_making_a_frame() {
    let observed = format!("{:?}", command("unsupported_fixture", json!({})));
    assert!(
        observed.contains("Invalid native command unsupported_fixture"),
        "{observed}"
    );
}

/// The gap the helper writes when its own room for audio the daemon has
/// not read is full crosses the bridge and decodes as a native event.
#[tokio::test]
async fn daemon_backpressure_gap_decodes_as_a_native_event() -> Result<()> {
    use orbit_core::wire::native::NativeEvent;

    let (mut input, _output, reader, writer) = pipes()?;
    let (_send, commands) = mpsc::channel(1);
    let (events, mut receive) = mpsc::channel(1);
    let task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    let gap = json!({"v":1,"event":"gap","source":"mic_local","stream_id":"stream-1",
        "reason":"daemon_backpressure","frames":7,"duration_ns":70_000_000u64});
    input.write_all(format!("{gap}\n").as_bytes()).await?;
    let frame = timeout(Duration::from_secs(1), receive.recv())
        .await?
        .context("the helper's gap must cross the bridge")?;
    task.abort();
    let event: NativeEvent = serde_json::from_value(frame)?;
    assert!(matches!(
        event,
        NativeEvent::Gap {
            v: 1,
            frames: 7,
            duration_ns: Some(70_000_000),
            reason,
            ..
        } if reason == "daemon_backpressure"
    ));
    Ok(())
}

#[tokio::test]
async fn known_event_with_wrong_field_type_is_not_delivered_and_link_survives() -> Result<()> {
    let (mut input, _output, reader, writer) = pipes()?;
    let (send, commands) = mpsc::channel(1);
    let (events, mut received) = mpsc::channel(2);
    let task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    let mut ready: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/ready.json"
    ))?;
    ready["selected_text_available"] = json!("yes");
    input.write_all(format!("{ready}\n").as_bytes()).await?;
    let valid = json!({"v":1,"event":"error","code":"fixture_error"});
    input.write_all(format!("{valid}\n").as_bytes()).await?;
    let delivered = timeout(Duration::from_secs(1), received.recv())
        .await?
        .context("valid event after malformed ready absent")?;
    assert_eq!(delivered, valid);
    assert!(
        received.try_recv().is_err(),
        "malformed ready was delivered"
    );
    assert!(
        !task.is_finished(),
        "one malformed known event ended the link"
    );
    drop(send);
    timeout(Duration::from_secs(1), task).await???;
    Ok(())
}

#[tokio::test]
async fn malformed_audio_reports_its_track_and_field_as_a_gap_then_ends_after_eight() -> Result<()>
{
    let (mut input, _output, reader, writer) = pipes()?;
    let (_send, commands) = mpsc::channel(1);
    let (events, mut received) = mpsc::channel(32);
    let mut task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    let mut bad: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/audio.json"
    ))?;
    bad["sample_rate"] = json!("32000");
    input.write_all(format!("{bad}\n").as_bytes()).await?;
    let gap = timeout(Duration::from_secs(1), received.recv())
        .await?
        .context("malformed audio gap absent")?;
    assert_eq!(gap["event"], "gap");
    assert_eq!(gap["source"], "mic_local");
    assert_eq!(gap["stream_id"], "stream-1");
    assert_eq!(gap["frames"], 1);
    let reason = gap["reason"].as_str().context("gap reason absent")?;
    assert!(
        reason.contains("sample_rate") && reason.contains("string"),
        "{reason}"
    );
    assert!(
        !task.is_finished(),
        "one malformed audio frame ended the bridge"
    );

    let valid: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/audio.json"
    ))?;
    input.write_all(format!("{valid}\n").as_bytes()).await?;
    let delivered = timeout(Duration::from_secs(1), received.recv())
        .await?
        .context("valid audio after malformed frame absent")?;
    assert_eq!(delivered["event"], "audio");
    for index in 0..8 {
        input.write_all(format!("{bad}\n").as_bytes()).await?;
        let gap = timeout(Duration::from_secs(1), received.recv())
            .await?
            .context("consecutive malformed audio gap absent")?;
        assert_eq!(gap["event"], "gap", "malformed frame {index}");
    }
    let error = timeout(Duration::from_secs(2), &mut task)
        .await?
        .context("bridge task panicked")?
        .err()
        .context("eight malformed audio frames must end the bridge")?;
    let cause = error.to_string();
    assert!(
        cause.contains("sample_rate") && cause.contains("string"),
        "{cause}"
    );
    Ok(())
}

#[tokio::test]
async fn native_audio_sample_rate_above_u8_still_decodes_as_audio() -> Result<()> {
    let (mut input, _output, reader, writer) = pipes()?;
    let (send, commands) = mpsc::channel(1);
    let (events, mut received) = mpsc::channel(2);
    let task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    let valid: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/audio.json"
    ))?;
    assert_eq!(valid["sample_rate"], 32_000);
    input.write_all(format!("{valid}\n").as_bytes()).await?;
    let delivered = timeout(Duration::from_secs(1), received.recv())
        .await?
        .context("32 kHz audio absent")?;
    assert_eq!(delivered, valid);
    drop(send);
    timeout(Duration::from_secs(1), task).await???;
    Ok(())
}

#[tokio::test]
async fn unknown_event_is_skipped_without_ending_the_link() -> Result<()> {
    let (mut input, _output, reader, writer) = pipes()?;
    let (send, commands) = mpsc::channel(1);
    let (events, mut received) = mpsc::channel(2);
    let task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    input
        .write_all(b"{\"v\":1,\"event\":\"future_event\"}\n")
        .await?;
    let valid = json!({"v":1,"event":"error","code":"fixture_error"});
    input.write_all(format!("{valid}\n").as_bytes()).await?;
    let delivered = timeout(Duration::from_secs(1), received.recv())
        .await?
        .context("valid event after future event absent")?;
    assert_eq!(delivered, valid);
    assert!(received.try_recv().is_err(), "unknown event was delivered");
    assert!(!task.is_finished(), "unknown event ended the link");
    drop(send);
    timeout(Duration::from_secs(1), task).await???;
    Ok(())
}

#[test]
fn malformed_notice_names_event_field_and_observed_type() -> Result<()> {
    let mut ready: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/ready.json"
    ))?;
    ready["selected_text_available"] = json!("yes");
    let bytes = serde_json::to_vec(&ready)?;
    let mut decoder = serde_json::Deserializer::from_slice(&bytes);
    let error =
        serde_path_to_error::deserialize::<_, orbit_core::wire::native::NativeEvent>(&mut decoder)
            .err()
            .context("malformed event unexpectedly decoded")?;
    assert_eq!(
        native_error_field(&ready, &error),
        ("selected_text_available".into(), "string")
    );
    Ok(())
}

#[test]
fn numeric_malformed_notice_names_the_actual_field() -> Result<()> {
    for number in [42, 1] {
        let mut ready: Value = serde_json::from_str(include_str!(
            "../../../contracts/fixtures/native/events/ready.json"
        ))?;
        ready["selected_text_available"] = json!(number);
        let bytes = serde_json::to_vec(&ready)?;
        let mut decoder = serde_json::Deserializer::from_slice(&bytes);
        let error = serde_path_to_error::deserialize::<_, orbit_core::wire::native::NativeEvent>(
            &mut decoder,
        )
        .err()
        .context("malformed event unexpectedly decoded")?;
        assert_eq!(
            native_error_field(&ready, &error),
            ("selected_text_available".into(), "number"),
            "{error:?}"
        );
    }
    Ok(())
}

#[test]
fn native_diagnostic_names_stop_growing_at_128_without_repeating_a_name() -> Result<()> {
    let mut reported = BTreeSet::new();
    for index in 0..MAX_REPORTED_NATIVE_EVENTS {
        assert!(
            report_native_event_once(&mut reported, &format!("future_{index}"), "", "unknown",)
                .is_some()
        );
    }
    assert!(report_native_event_once(&mut reported, "future_0", "", "unknown").is_none());
    assert!(report_native_event_once(&mut reported, "future_129", "", "unknown").is_none());
    assert_eq!(reported.len(), MAX_REPORTED_NATIVE_EVENTS);

    let mut long_name = BTreeSet::new();
    let (stored, _) = report_native_event_once(&mut long_name, &"é".repeat(128), "", "unknown")
        .context("first diagnostic is retained")?;
    assert_eq!(stored.chars().count(), 64);
    assert!(stored.len() <= 256);
    Ok(())
}

#[tokio::test]
async fn bridge_sends_hello_before_any_heartbeat_or_queued_command() -> Result<()> {
    let (_input, output, reader, writer) = pipes()?;
    let (send, commands) = mpsc::channel(1);
    let (events, _receive) = mpsc::channel(1);
    let task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    let mut output = BufReader::new(output);
    let first = timeout(Duration::from_secs(1), read_frame(&mut output)).await??;
    assert_eq!(serde_json::from_slice::<Value>(&first)?["command"], "hello");
    drop(send);
    timeout(Duration::from_secs(1), task).await???;
    Ok(())
}

#[tokio::test]
async fn closing_command_channel_cancels_an_idle_reader_with_host_input_open() -> Result<()> {
    let (_host_input, _host_output, reader, writer) = pipes()?;
    let (send, commands) = mpsc::channel(1);
    let (events, _receive) = mpsc::channel(1);
    let task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    tokio::task::yield_now().await;
    drop(send);
    timeout(Duration::from_secs(1), task).await???;
    // No blocking pool work exists to keep the test runtime alive at drop.
    Ok(())
}

#[tokio::test]
async fn partial_frame_survives_outbound_commands_and_coalesced_next_frame() -> Result<()> {
    let (mut input, output, reader, writer) = pipes()?;
    let (send, commands) = mpsc::channel(2);
    let (events, mut received) = mpsc::channel(2);
    let task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    input.write_all(b"{\"v\":1,\"event\":\"err").await?;
    tokio::task::yield_now().await;
    send.send(command("status", json!({}))?).await?;
    let mut output = BufReader::new(output);
    loop {
        let bytes = timeout(Duration::from_secs(1), read_frame(&mut output)).await??;
        if serde_json::from_slice::<Value>(&bytes)?["command"] == "status" {
            break;
        }
    }
    assert!(
        received.try_recv().is_err(),
        "an incomplete frame must not be delivered"
    );
    input
        .write_all(b"or\",\"code\":\"fixture_one\"}\n{\"v\":1,\"event\":\"error\",\"code\":\"fixture_two\"}\n")
        .await?;
    assert_eq!(
        timeout(Duration::from_secs(1), received.recv())
            .await?
            .context("first event")?["code"],
        "fixture_one"
    );
    assert_eq!(
        timeout(Duration::from_secs(1), received.recv())
            .await?
            .context("second event")?["code"],
        "fixture_two"
    );
    drop(send);
    timeout(Duration::from_secs(1), task).await???;
    Ok(())
}

#[tokio::test]
async fn oversized_native_frame_is_rejected_without_event_delivery() -> Result<()> {
    let (mut input, _output, reader, writer) = pipes()?;
    let (_send, commands) = mpsc::channel(1);
    let (events, mut received) = mpsc::channel(1);
    let task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    let producer = tokio::spawn(async move {
        input
            .write_all(&vec![b'x'; MAX_HELPER_EVENT_LINE_BYTES + 1])
            .await
    });
    assert!(timeout(Duration::from_secs(2), task).await??.is_err());
    let _ = timeout(Duration::from_secs(1), producer).await??;
    assert!(received.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn saturated_stdout_times_out_without_leaving_a_blocking_writer() -> Result<()> {
    let (_input, _output, reader, writer) = pipes()?;
    // Fill the kernel pipe while deliberately keeping the remote reader open.
    writer.writable().await?;
    let mut filled = 0;
    loop {
        match writer.try_write(&[0; 4096]) {
            Ok(count) => {
                filled += count;
                assert!(
                    filled <= 16 * 1024 * 1024,
                    "fixture pipe did not become saturated"
                );
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) => return Err(error.into()),
        }
    }
    assert!(filled > 0);
    let (send, commands) = mpsc::channel(1);
    let (events, _received) = mpsc::channel(1);
    send.send(command("status", json!({}))?).await?;
    let task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    assert!(timeout(Duration::from_secs(3), task).await??.is_err());
    Ok(())
}

/// The bundled CUA driver is resolved through the app executable's Contents,
/// including old bundles left by an update, never from a development path.
#[test]
fn the_packaged_driver_is_resolved_through_the_bundle_contents() -> Result<()> {
    for bundle in [
        "/Applications/Orbit.app",
        "/Applications/Orbit.app.anterior-20260928",
        "/Volumes/Externo/Orbit.app",
    ] {
        let daemon = std::path::Path::new(bundle).join("Contents/MacOS/Orbit");
        assert_eq!(
            crate::cua::resolve_driver_path("", &daemon)?,
            std::path::Path::new(bundle)
                .join("Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver")
        );
    }
    // A development executable must not invent a packaged path.
    let development = std::path::Path::new("/build/target/debug/orbit");
    let error = crate::cua::resolve_driver_path("", development)
        .err()
        .context("development executable must not resolve a packaged driver")?;
    assert!(error.to_string().contains("/build/target/debug/orbit"));
    assert!(
        error
            .to_string()
            .contains("no es la app de un bundle de Orbit")
    );
    Ok(())
}

#[tokio::test]
async fn dropping_host_writer_reports_eof_without_treating_partial_data_as_a_frame() -> Result<()> {
    let (mut input, _output, reader, _writer) = pipes()?;
    input.write_all(b"{\"v\":1").await?;
    drop(input);
    assert!(
        timeout(Duration::from_secs(1), read_frame(BufReader::new(reader)))
            .await?
            .is_err()
    );
    Ok(())
}

fn stalled_audio_line(sequence: u64) -> Result<String> {
    let mut frame: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/audio.json"
    ))?;
    frame["stream_id"] = json!("stalled");
    frame["sequence"] = json!(sequence);
    Ok(format!("{frame}\n"))
}

fn fixture_ready_line() -> Result<String> {
    let ready: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/ready.json"
    ))?;
    Ok(ready.to_string())
}

/// A daemon that stops reading for longer than the bridge ever allowed (ten
/// seconds for a non-audio event) keeps its helper and its audio: the bridge
/// waits for the consumer instead of discarding frames or declaring the link
/// dead, and every frame arrives in order, without a gap, once it reads again.
/// While it waits it stops reading the pipe, so the backlog stays in the
/// helper, which keeps up to five minutes and counts what does not fit.
/// The two-slot channel models the stall compressed.
#[tokio::test(start_paused = true)]
async fn a_stalled_consumer_receives_the_whole_backlog_in_order_and_keeps_the_helper() -> Result<()>
{
    let (mut input, _output, reader, writer) = pipes()?;
    let (send, commands) = mpsc::channel(1);
    let (events, mut received) = mpsc::channel(2);
    let task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    for sequence in 0..10u64 {
        input
            .write_all(stalled_audio_line(sequence)?.as_bytes())
            .await?;
    }
    input
        .write_all(format!("{}\n", json!({"v":1,"event":"error","code":"fixture"})).as_bytes())
        .await?;
    // Thirty seconds without reading: three times the old non-audio bound.
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(
        !task.is_finished(),
        "a consumer that stops reading must not end the bridge or the helper"
    );
    for sequence in 0..10u64 {
        let event = timeout(Duration::from_secs(1), received.recv())
            .await?
            .with_context(|| format!("audio frame {sequence} absent"))?;
        assert_eq!(
            event["event"], "audio",
            "the backlog is audio, never a gap: {event}"
        );
        assert_eq!(event["sequence"], sequence, "the backlog arrives in order");
    }
    let control = timeout(Duration::from_secs(1), received.recv())
        .await?
        .context("the event behind the audio must arrive after it")?;
    assert_eq!(control["event"], "error");
    input.write_all(stalled_audio_line(10)?.as_bytes()).await?;
    let resumed = timeout(Duration::from_secs(1), received.recv())
        .await?
        .context("audio after the backlog must be delivered")?;
    assert_eq!(resumed["event"], "audio");
    assert_eq!(resumed["sequence"], 10);
    assert!(
        !task.is_finished(),
        "the bridge must stay alive after the episode"
    );
    drop(send);
    timeout(Duration::from_secs(1), task).await???;
    Ok(())
}

/// Malformed frames are counted and skipped while the stream stays
/// usable; only eight consecutive ones declare the bridge down. The valid
/// frame in between proves the counter is really consecutive.
#[tokio::test]
async fn malformed_frames_are_skipped_until_eight_consecutive_ones_end_the_bridge() -> Result<()> {
    let (mut input, _output, reader, writer) = pipes()?;
    let (_send, commands) = mpsc::channel(1);
    let (events, mut received) = mpsc::channel(4);
    let mut task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    for _ in 0..7 {
        input.write_all(b"not-json\n").await?;
    }
    input
        .write_all(format!("{}\n", json!({"v":1,"event":"error","code":"fixture"})).as_bytes())
        .await?;
    let delivered = timeout(Duration::from_secs(1), received.recv())
        .await?
        .context("the valid frame after seven malformed ones must be delivered")?;
    assert_eq!(delivered["event"], "error");
    assert!(
        !task.is_finished(),
        "seven malformed frames must not end the bridge"
    );
    for _ in 0..8 {
        input.write_all(b"not-json\n").await?;
    }
    let finished = timeout(Duration::from_secs(2), &mut task)
        .await
        .context("eight consecutive malformed frames must end the bridge")?
        .context("the bridge task must not panic")?;
    assert!(
        finished.is_err(),
        "the eighth consecutive malformed frame must declare the bridge down"
    );
    Ok(())
}

/// The cause of a bridge that malformed audio took down reaches
/// the daemon in `helper_disconnected`, not only the log. Without this step
/// the user would see the generic notice again.
#[tokio::test]
async fn malformed_audio_bridge_loss_carries_its_cause_to_the_daemon() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut bad: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/audio.json"
    ))?;
    bad["sample_rate"] = json!("32000");
    let mut body = format!(
        "/bin/echo '{}'
",
        fixture_ready_line()?
    );
    for _ in 0..8 {
        body.push_str(&format!(
            "/bin/echo '{bad}'
"
        ));
    }
    body.push_str("/bin/sleep 5");
    let script = helper_script(directory.path(), &body)?;
    let (_send, commands) = mpsc::channel(1);
    let (events, mut received) = mpsc::channel(32);
    let (_stop, never) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(run(Some(script), commands, events, never));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let disconnected = loop {
        let event = timeout(
            deadline.saturating_duration_since(std::time::Instant::now()),
            received.recv(),
        )
        .await?
        .context("the bridge never reported the helper as lost")?;
        if event["event"] == "helper_disconnected" {
            break event;
        }
    };
    let cause = disconnected["cause"]
        .as_str()
        .context("helper_disconnected without the malformed-audio cause")?;
    assert!(
        cause.contains("sample_rate") && cause.contains("string"),
        "{cause}"
    );
    task.abort();
    Ok(())
}

/// The daemon launches its helper as a managed child. The launch
/// arguments are the observable contract — `--stdio --managed`, never plain
/// `--stdio` — while the existing supervision keeps announcing the fall and
/// relaunching with backoff.
#[tokio::test]
async fn the_managed_helper_runs_with_stdio_managed_under_supervision() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let args = directory.path().join("args");
    let counter = directory.path().join("launches");
    let script = helper_script(
        directory.path(),
        &format!(
            "/bin/echo \"$@\" > '{}'\n/bin/echo '{}'\n\
             /bin/echo launch >> '{}'\nexit 1",
            args.display(),
            fixture_ready_line()?,
            counter.display()
        ),
    )?;
    let (send, commands) = mpsc::channel(1);
    let (events, mut received) = mpsc::channel(4);
    let (_stop, never) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(run(Some(script), commands, events, never));
    let first = timeout(Duration::from_secs(2), received.recv())
        .await?
        .context("the managed helper must report ready")?;
    assert_eq!(first["event"], "ready");
    let observed = std::fs::read_to_string(&args)
        .context("the helper must record the arguments it was launched with")?;
    assert_eq!(
        observed.trim(),
        "--stdio --managed",
        "the daemon must launch the helper in managed mode, not development stdio"
    );
    let disconnected = timeout(Duration::from_secs(2), received.recv())
        .await?
        .context("the fall of a ready managed helper must be announced")?;
    assert_eq!(
        disconnected["event"], "helper_disconnected",
        "the daemon must learn the managed helper is gone"
    );
    let launches = || {
        std::fs::read_to_string(&counter)
            .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
            .unwrap_or(0)
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while launches() < 2 && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        launches() >= 2,
        "the managed helper must stay under supervision (launches: {})",
        launches()
    );
    let second = timeout(Duration::from_secs(2), received.recv())
        .await?
        .context("the relaunched managed helper must report ready again")?;
    assert_eq!(second["event"], "ready");
    assert!(
        !task.is_finished(),
        "the supervision ended after relaunching the helper"
    );
    drop(send);
    task.abort();
    Ok(())
}

/// Writes a fixture helper without this process ever holding it open for
/// writing: the suite forks all the time, a fork inherits an open descriptor
/// until its `exec`, and executing a file someone holds open for writing fails
/// with `ETXTBSY` — which the supervisor reads, correctly, as a helper that
/// fell. `cp` writes it from a process of its own instead.
fn helper_script(directory: &std::path::Path, body: &str) -> Result<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let source = directory.join("helper.txt");
    let script = directory.join("helper");
    std::fs::write(&source, format!("#!/bin/sh\n{body}\n"))?;
    let copied = std::process::Command::new("/bin/cp")
        .arg(&source)
        .arg(&script)
        .status()?;
    anyhow::ensure!(
        copied.success(),
        "cp of the fixture helper failed: {copied}"
    );
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    Ok(script)
}

/// Waits for the helper's `ready`, skipping nothing else: the bridge forwards
/// every frame, and a fixture helper here sends only that one.
async fn helper_ready(events: &mut mpsc::Receiver<Value>) -> Result<()> {
    let ready = timeout(Duration::from_secs(10), events.recv())
        .await?
        .context("the helper fixture never reported ready")?;
    anyhow::ensure!(ready["event"] == "ready", "unexpected first frame: {ready}");
    Ok(())
}

/// Stopping the supervised helper sends it `shutdown`, then closes
/// its input; a helper that then leaves by itself is reported as such and is
/// not killed.
#[tokio::test]
async fn stop_sends_shutdown_closes_the_input_and_lets_the_helper_leave() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let received = directory.path().join("received");
    let script = helper_script(
        directory.path(),
        &format!(
            "printf '%s\\n' '{}'\n/bin/cat > '{}'\nexit 0",
            fixture_ready_line()?,
            received.display()
        ),
    )?;
    let (_send, commands) = mpsc::channel(1);
    let (events, mut observed) = mpsc::channel(8);
    let helper = Supervisor::spawn(Some(script), commands, events, Default::default());
    helper_ready(&mut observed).await?;
    let end = helper.stop(&mut observed).await;
    assert_eq!(
        end,
        HelperEnd::Exited {
            code: Some(0),
            signal: None,
            asked: true
        },
        "a helper that leaves at the end of its input is not killed"
    );
    let sent = std::fs::read_to_string(&received)?;
    let last = sent
        .lines()
        .last()
        .context("the helper received nothing before its input closed")?;
    assert_eq!(
        serde_json::from_str::<Value>(last)?["command"],
        "shutdown",
        "`shutdown` is the last thing the helper reads: {sent}"
    );
    Ok(())
}

/// A helper that ignores `shutdown`, its input and SIGTERM is
/// killed, and only once its grace is over.
#[tokio::test]
async fn a_helper_that_does_not_leave_is_killed_after_its_grace() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let script = helper_script(
        directory.path(),
        &format!(
            "trap '' TERM\nprintf '%s\\n' '{}'\nexec /bin/sleep 3600",
            fixture_ready_line()?
        ),
    )?;
    let (_send, commands) = mpsc::channel(1);
    let (events, mut observed) = mpsc::channel(8);
    let helper = Supervisor::spawn(Some(script), commands, events, Default::default());
    helper_ready(&mut observed).await?;
    let asked = std::time::Instant::now();
    let end = helper.stop(&mut observed).await;
    let took = asked.elapsed();
    assert_eq!(end, HelperEnd::Killed { asked: true });
    // A lower bound, so no clock decides it: it cannot be killed sooner.
    assert!(
        took >= HELPER_EXIT_GRACE,
        "killed after {took:?}, before its {HELPER_EXIT_GRACE:?} grace"
    );
    Ok(())
}

/// The helper's heartbeat does not run on the daemon's runtime. With every
/// thread of that runtime held (here the test's only one, as a blocking call
/// would hold it), beats keep reaching the helper, which would otherwise
/// read the daemon as silent.
#[tokio::test]
async fn heartbeats_reach_the_helper_while_the_daemon_runtime_is_held() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let received = directory.path().join("received");
    let script = helper_script(
        directory.path(),
        &format!(
            "printf '%s\\n' '{}'\n/bin/cat > '{}'\nexit 0",
            fixture_ready_line()?,
            received.display()
        ),
    )?;
    let (_send, commands) = mpsc::channel(1);
    let (events, mut observed) = mpsc::channel(8);
    let helper = Supervisor::spawn(Some(script), commands, events, Default::default());
    helper_ready(&mut observed).await?;
    let beats = || {
        std::fs::read_to_string(&received)
            .map(|text| {
                text.lines()
                    .filter(|line| line.contains("\"heartbeat\""))
                    .count()
            })
            .unwrap_or(0)
    };
    let before = beats();
    // Two and a half beat periods with the daemon's runtime held.
    std::thread::sleep(Duration::from_secs(5));
    let during = beats().saturating_sub(before);
    let end = helper.stop(&mut observed).await;
    assert!(
        during >= 1,
        "no heartbeat reached the helper in 5 s with the daemon's runtime held"
    );
    assert_eq!(
        end,
        HelperEnd::Exited {
            code: Some(0),
            signal: None,
            asked: true
        }
    );
    Ok(())
}

/// A stop that arrives while the supervisor waits to relaunch a
/// fallen helper ends the supervision instead of launching another one.
#[tokio::test]
async fn a_stop_during_the_relaunch_wait_launches_nothing_more() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let launches = directory.path().join("launches");
    let script = helper_script(
        directory.path(),
        &format!("/bin/echo launch >> '{}'\nexit 1", launches.display()),
    )?;
    let (_send, commands) = mpsc::channel(1);
    let (events, mut observed) = mpsc::channel(8);
    let helper = Supervisor::spawn(Some(script), commands, events, Default::default());
    // The fall is announced before the supervisor starts its wait: the event,
    // not a sleep, says the supervisor is now between launches.
    let fell = timeout(Duration::from_secs(10), observed.recv())
        .await?
        .context("the fall of the fixture helper was never announced")?;
    assert_eq!(fell["event"], "helper_disconnected");
    let end = timeout(Duration::from_secs(2), helper.stop(&mut observed)).await?;
    assert_eq!(end, HelperEnd::NotRunning);
    let count = std::fs::read_to_string(&launches)?.lines().count();
    assert_eq!(count, 1, "no helper is launched after the stop");
    Ok(())
}

#[tokio::test]
async fn evicted_live_capture_ack_still_crosses_the_bridge() -> Result<()> {
    let (mut input, output, reader, writer) = pipes()?;
    let (send, commands) = mpsc::channel(128);
    let (events, mut received) = mpsc::channel(8);
    let task = tokio::spawn(bridge(BufReader::new(reader), writer, commands, events));
    let mut output = BufReader::new(output);
    let _hello = timeout(Duration::from_secs(2), read_frame(&mut output)).await??;
    send.send(command(
        "audio_stop",
        json!({"id":"0000-stop","stream_id":"s1"}),
    )?)
    .await?;
    for index in 0..64 {
        send.send(command(
            "status",
            json!({"id": format!("zz-status-{index:02}")}),
        )?)
        .await?;
    }
    loop {
        let written: Value = serde_json::from_slice(
            &timeout(Duration::from_secs(5), read_frame(&mut output)).await??,
        )?;
        if written["id"] == "zz-status-63" {
            break;
        }
    }
    input
        .write_all(
            format!(
                "{}\n",
                json!({"v":1,"event":"response","id":"0000-stop","ok":true,
            "result":{"stream_id":"s1","drain_complete":true}})
            )
            .as_bytes(),
        )
        .await?;
    let reply = timeout(Duration::from_secs(2), received.recv()).await;
    assert!(
        matches!(reply, Ok(Some(_))),
        "the helper's ACK of a live audio_stop was dropped by the bridge: {reply:?}"
    );
    drop(send);
    let _ = timeout(Duration::from_secs(2), task).await;
    Ok(())
}
