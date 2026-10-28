#![allow(clippy::expect_used, clippy::type_complexity, clippy::unwrap_used)]

use orbit_app_core::{
    AppCommand, BriefingSourceState, CalendarResponse, CalendarViewKind, CalendarWeekStart,
    DaemonClient, DaemonConnection, LocalZone, MeetingPhase, PermissionState, ReceiptDelivery,
    calendar::CalendarClock, features::FeatureFrame,
};
use orbit_core::calendar::CalendarAuthorization;
use orbit_core::topics::{Knowledge, TopicData, TopicId};
use orbit_ipc::Paths;
use serde_json::{Value, json};
use std::{
    error::Error,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender, SyncSender, channel, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

#[path = "support/embedded_daemon.rs"]
mod embedded_daemon;
#[path = "support/native_state.rs"]
mod native_state;
#[path = "support/websocket.rs"]
mod websocket;
use websocket::{read_websocket_frame, send_hub_event, websocket_handshake, write_websocket_frame};

// The wait budgets in this file, and where their numbers come from.
//
// The original measurement spawned real daemon binaries, all starting
// at once. The fixtures now use threads of this process; the measured guards stay.
// Every wait here is really "how long does a cold daemon take to
// get this far". That is a measurement, not a matter of taste, so it was
// measured: every wait was instrumented with its budget lifted to 120 s and
// the suite run 60 times under the heaviest load it was reproduced failing at
// (96 busy processes on 32 cores). In milliseconds:
//
//     wait                        n     p50    p95    p99   worst
//     helper command            540    1005   1801   2077    3969
//     snapshot condition       1671     327   1197   1580    2579
//     daemon exit on shutdown    60     538    580    583     597
//     calendar authorization     60     265    397    417     522
//     audio start requested      60      29     60     73      87
//     native command counted     60      20     23     25      27
//
// Two of these had 2 s. The helper command wait sat *below its own p99*: it
// was not a deadline, it was a coin toss, and it reported the loss as a bare
// `Timeout`. The rest were already generous; they stay where they are and now
// carry the measurement that says why.
//
// The calendar selection was retired, and with it the tests that listed, selected
// and restored it. The three waits only they made — calendar authorization,
// native command counted and daemon exit on shutdown — went with them, so
// their budgets did too: a budget nothing waits on bounds nothing. The rows
// stay in the table above as the record of what was measured.
const HELPER_COMMAND_BUDGET: Duration = Duration::from_secs(30);
const SNAPSHOT_BUDGET: Duration = Duration::from_secs(30);
const AUDIO_START_BUDGET: Duration = Duration::from_secs(10);

const NATIVE_INSTANCE: &str = "22222222-2222-4222-8222-222222222222";

fn helper_state_result(permissions: Option<Value>, authorization: CalendarAuthorization) -> Value {
    let mut result = native_state::result(permissions);
    // Every full state carries the same current calendar authorization, even
    // when it carries no permissions listing (hello, heartbeat, capture_stop).
    result["calendar"] = calendar_status(authorization);
    result["native_instance_id"] = json!(NATIVE_INSTANCE);
    result["privacy_generation"] = json!(2);
    result
}
const INVENTORY_ID: &str = "11111111-1111-4111-8111-111111111111";

struct AudioStartGate {
    release: Receiver<()>,
    requested: SyncSender<()>,
}

struct NativeHelperConfig {
    status_reports_permissions: bool,
    calendar_authorization: CalendarAuthorization,
    /// When set, every `audio_start` is answered with this refusal
    /// code instead of an acknowledgement, so a test can stand where the Mac
    /// says no to the capture.
    audio_start_refusal: Option<&'static str>,
}

struct TestDaemon {
    child: embedded_daemon::TestDaemon,
    helper: Option<thread::JoinHandle<()>>,
    native_input: Arc<Mutex<File>>,
    native_commands: Arc<Mutex<Vec<String>>>,
    calendar_authorization: CalendarAuthorization,
}

impl TestDaemon {
    fn start(data_dir: &Path) -> Result<Self, Box<dyn Error>> {
        let (daemon, _) = Self::start_inner(
            data_dir,
            None,
            false,
            true,
            CalendarAuthorization::NotDetermined,
            None,
        )?;
        Ok(daemon)
    }

    fn start_calendar(data_dir: &Path) -> Result<Self, Box<dyn Error>> {
        let (daemon, _) = Self::start_inner(
            data_dir,
            None,
            false,
            true,
            CalendarAuthorization::FullAccess,
            None,
        )?;
        Ok(daemon)
    }

    /// The calendar backend with full access, and every command the
    /// helper receives handed to the test.
    fn start_calendar_observed(data_dir: &Path) -> Result<(Self, Receiver<Value>), Box<dyn Error>> {
        let (daemon, commands) = Self::start_inner(
            data_dir,
            None,
            true,
            true,
            CalendarAuthorization::FullAccess,
            None,
        )?;
        commands.map_or_else(
            || Err("native helper observation channel missing".into()),
            |commands| Ok((daemon, commands)),
        )
    }

    fn start_calendar_denied(data_dir: &Path) -> Result<Self, Box<dyn Error>> {
        let (daemon, _) = Self::start_inner(
            data_dir,
            None,
            false,
            true,
            CalendarAuthorization::Denied,
            None,
        )?;
        Ok(daemon)
    }

    fn start_observed(data_dir: &Path) -> Result<(Self, Receiver<Value>), Box<dyn Error>> {
        let (daemon, commands) = Self::start_inner(
            data_dir,
            None,
            true,
            true,
            CalendarAuthorization::NotDetermined,
            None,
        )?;
        commands.map_or_else(
            || Err("native helper observation channel missing".into()),
            |commands| Ok((daemon, commands)),
        )
    }

    fn start_with_audio_gate(
        data_dir: &Path,
    ) -> Result<(Self, Receiver<()>, SyncSender<()>), Box<dyn Error>> {
        let (requested_tx, requested_rx) = sync_channel(0);
        let (release_tx, release_rx) = sync_channel(0);
        let (daemon, _) = Self::start_inner(
            data_dir,
            Some(AudioStartGate {
                release: release_rx,
                requested: requested_tx,
            }),
            false,
            true,
            CalendarAuthorization::NotDetermined,
            None,
        )?;
        Ok((daemon, requested_rx, release_tx))
    }

    /// The gate holds the first `audio_start`, and the release answers it with
    /// a refusal carrying `code`, as the Mac does when a permission is missing.
    fn start_with_audio_refusal(
        data_dir: &Path,
        code: &'static str,
    ) -> Result<(Self, Receiver<()>, SyncSender<()>), Box<dyn Error>> {
        let (requested_tx, requested_rx) = sync_channel(0);
        let (release_tx, release_rx) = sync_channel(0);
        let (daemon, _) = Self::start_inner(
            data_dir,
            Some(AudioStartGate {
                release: release_rx,
                requested: requested_tx,
            }),
            false,
            true,
            CalendarAuthorization::NotDetermined,
            Some(code),
        )?;
        Ok((daemon, requested_rx, release_tx))
    }

    fn start_inner(
        data_dir: &Path,
        audio_start_gate: Option<AudioStartGate>,
        observe_commands: bool,
        status_reports_permissions: bool,
        calendar_authorization: CalendarAuthorization,
        audio_start_refusal: Option<&'static str>,
    ) -> Result<(Self, Option<Receiver<Value>>), Box<dyn Error>> {
        let relay = HelperRelay::create(data_dir)?;
        let mut child = embedded_daemon::TestDaemon::start(data_dir, Some(relay.script.clone()))?;
        let (output, events) = match relay.connect(HELPER_COMMAND_BUDGET) {
            Ok(ends) => ends,
            Err(error) => {
                let _ = child.cancel();
                return Err(error);
            }
        };
        let input = Arc::new(Mutex::new(events));
        let (commands_tx, commands_rx) = if observe_commands {
            let (sender, receiver) = channel();
            (Some(sender), Some(receiver))
        } else {
            (None, None)
        };
        let native_commands = Arc::new(Mutex::new(Vec::new()));
        let helper_input = Arc::clone(&input);
        let helper_commands = Arc::clone(&native_commands);
        let helper = thread::spawn(move || {
            emulate_native_helper(
                helper_input,
                output,
                audio_start_gate,
                commands_tx,
                helper_commands,
                NativeHelperConfig {
                    status_reports_permissions,
                    calendar_authorization,
                    audio_start_refusal,
                },
            )
        });
        let mut daemon = Self {
            child,
            helper: Some(helper),
            native_input: input,
            native_commands,
            calendar_authorization,
        };
        wait_helper_declared(
            &mut daemon.child,
            status_reports_permissions,
            HELPER_COMMAND_BUDGET,
        )?;
        Ok((daemon, commands_rx))
    }

    fn calendar_invalidated(&self) -> Result<(), Box<dyn Error>> {
        write_shared_frame(
            &self.native_input,
            &json!({"v":1,"event":"calendar_invalidated","code":"calendar_invalidated"}),
        )
    }

    /// Repeat the full state a heartbeat reports, after a completed read or
    /// write. The notice follows it on the same native pipe; observing that
    /// sequence proves the daemon consumed the state before the test looks.
    async fn repeat_helper_state(&self) -> Result<(), Box<dyn Error>> {
        use orbit_core::notice::{DataValue, NoticeCode, NoticeKey};
        let mut notices = self
            .child
            .client
            .topics()
            .remove(&TopicId::Notices)
            .ok_or("notices watch missing")?;
        let prior_sequence = match &notices.borrow().data {
            TopicData::Notices(Knowledge::Present { value, .. }) => value
                .notices
                .last()
                .and_then(|notice| notice.sequence)
                .unwrap_or(0),
            other => return Err(format!("notices not present: {other:?}").into()),
        };
        let mut state = helper_state_result(None, self.calendar_authorization);
        state["v"] = json!(1);
        state["event"] = json!("state");
        write_shared_frame(&self.native_input, &state)?;
        // A synthetic helper error is an observable barrier, with no device effect.
        write_shared_frame(
            &self.native_input,
            &json!({"v":1,"event":"notice","code":"fixture_state_barrier"}),
        )?;
        tokio::time::timeout(SNAPSHOT_BUDGET, async {
            loop {
                if matches!(&notices.borrow_and_update().data,
                    TopicData::Notices(Knowledge::Present { value, .. })
                    if value.notices.iter().any(|notice| notice.code == NoticeCode::HelperError
                        && matches!(notice.data.get(&NoticeKey::Code), Some(DataValue::Token(code))
                            if code.as_str() == "fixture_state_barrier")
                        && notice.sequence.is_some_and(|sequence| sequence > prior_sequence)))
                {
                    return Ok::<_, Box<dyn Error>>(());
                }
                notices.changed().await?;
            }
        })
        .await
        .map_err(|_| {
            format!(
                "el daemon no confirmó el estado nativo en {SNAPSHOT_BUDGET:?}; \
                 se esperaba fixture_state_barrier después de la secuencia {prior_sequence}; último: {:?}",
                notices.borrow().data
            )
        })??;
        // Publish every dirty head after the native barrier. The test then
        // connects a late observer, which can only see what remains.
        self.child
            .client
            .request_command(orbit_core::protocol::Command::Status)
            .await?;
        Ok(())
    }

    fn native_command_count(&self, name: &str) -> Result<usize, Box<dyn Error>> {
        let commands = self
            .native_commands
            .lock()
            .map_err(|_| "native command probe poisoned")?;
        Ok(commands
            .iter()
            .filter(|command| command.as_str() == name)
            .count())
    }

    fn voice_audio_frame(&self, stream_id: &str) -> Result<(), Box<dyn Error>> {
        write_shared_frame(
            &self.native_input,
            &json!({
                "v": 1,
                "event": "audio",
                "source": "mic_local",
                "stream_id": stream_id,
                "sample_rate": 8_000,
                "channels": 1,
                // 225 ms at 8 kHz: above the daemon's speech minimum
                // (`orbit_core::voice::MINIMUM_SPEECH_MS`), so it is sent.
                "frames": 1_800,
                "sequence": 0,
                "monotonic_ns": 0,
                "clock_domain": "mach_absolute_time",
                "pcm_s16le_base64": "AQEB".repeat(1_200),
                "levels": {"rms_dbfs": -12.0, "peak_dbfs": -9.0,
                    "clipped_frames": 0},
                "processing": "unprocessed"
            }),
        )
    }

    fn kill(&mut self) -> Result<(), Box<dyn Error>> {
        self.child.cancel()?;
        if let Some(helper) = self.helper.take() {
            helper.join().map_err(|_| "native helper thread panicked")?;
        }
        Ok(())
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.cancel();
        if let Some(helper) = self.helper.take() {
            let _ = helper.join();
        }
    }
}

fn write_frame(input: &mut File, frame: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *input, frame)?;
    input.write_all(b"\n")?;
    input.flush()
}

fn write_shared_frame(input: &Arc<Mutex<File>>, frame: &Value) -> Result<(), Box<dyn Error>> {
    let mut input = input.lock().map_err(|_| "native input poisoned")?;
    write_frame(&mut input, frame)?;
    Ok(())
}

/// The daemon starts its helper with `--helper`, as in production, and
/// a shell relay stands where `OrbitNative` stands, copying the daemon's pipes
/// to two FIFOs this test owns. The helper side of the protocol is played over
/// them exactly as it was over the daemon's own stdio before.
struct HelperRelay {
    script: PathBuf,
    commands: PathBuf,
    events: PathBuf,
}

impl HelperRelay {
    fn create(directory: &Path) -> Result<Self, Box<dyn Error>> {
        use std::os::unix::fs::PermissionsExt;
        // A fresh relay per launch: a restarted daemon launches its own helper.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let name = format!(
            "helper-{}",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        );
        let relay = Self {
            script: directory.join(&name),
            commands: directory.join(format!("{name}.commands")),
            events: directory.join(format!("{name}.events")),
        };
        for fifo in [&relay.commands, &relay.events] {
            let status = Command::new("mkfifo")
                .args(["-m", "600"])
                .arg(fifo)
                .status()?;
            if !status.success() {
                return Err(format!("mkfifo ended with {status}").into());
            }
        }
        let source = directory.join(format!("{name}.txt"));
        std::fs::write(
            &source,
            format!(
                "#!/bin/sh\n/bin/cat '{}' &\nexec /bin/cat > '{}'\n",
                relay.events.display(),
                relay.commands.display()
            ),
        )?;
        // Written by `cp`, never held open for writing by this process: the
        // suite forks constantly, and executing a file someone holds open for
        // writing fails with ETXTBSY, which the daemon reads as a fallen helper.
        let copied = Command::new("/bin/cp")
            .arg(&source)
            .arg(&relay.script)
            .status()?;
        if !copied.success() {
            return Err(format!("cp ended with {copied}").into());
        }
        std::fs::set_permissions(&relay.script, std::fs::Permissions::from_mode(0o700))?;
        Ok(relay)
    }

    /// The daemon's commands to read and the helper's events to write, once
    /// the daemon has launched the relay. Opening a FIFO waits for its other
    /// end, so it happens on a thread and a daemon that never launches its
    /// helper fails the test by name.
    fn connect(&self, budget: Duration) -> Result<(File, File), Box<dyn Error>> {
        let (commands, events) = (self.commands.clone(), self.events.clone());
        let (sender, receiver) = channel();
        thread::spawn(move || {
            let opened = (|| -> std::io::Result<(File, File)> {
                let events = OpenOptions::new().write(true).open(&events)?;
                Ok((File::open(&commands)?, events))
            })();
            let _ = sender.send(opened);
        });
        match receiver.recv_timeout(budget) {
            Ok(opened) => Ok(opened?),
            Err(_) => {
                Err(format!("the daemon did not launch the test helper within {budget:?}").into())
            }
        }
    }
}

/// Waits until the daemon has declared the relayed helper and, when the helper
/// reports permissions, has taken them in. The helper is launched once the
/// daemon's startup is over, so a client that connects at once can
/// otherwise act before `ready`; the old stdio peer won that race by accident.
fn wait_helper_declared(
    child: &mut embedded_daemon::TestDaemon,
    permissions_reported: bool,
    budget: Duration,
) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(status) = child.outcome() {
            return Err(
                format!("the daemon exited ({status:?}) before declaring its helper").into(),
            );
        }
        let topics = child.client.topics();
        let helper_connected = topics.get(&TopicId::Status).is_some_and(|head|
            matches!(&head.borrow().data, TopicData::Status(Knowledge::Present { value, .. }) if value.helper_connected));
        let microphone_observed = topics.get(&TopicId::Permissions).is_some_and(|head|
            matches!(&head.borrow().data, TopicData::Permissions(Knowledge::Present { value, .. }) if value.microphone != orbit_core::PermissionState::Unknown));
        if helper_connected && (!permissions_reported || microphone_observed) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(
                format!("the daemon did not declare the test helper within {budget:?}").into(),
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn calendar_status(authorization: CalendarAuthorization) -> Value {
    json!({"authorization":authorization,"physical_read_pending":false,"read_only":true})
}

/// Every calendar on the synthetic Mac, in one page of its listing (the
/// daemon lists them by itself; nothing is chosen from them).
fn calendar_inventory() -> Value {
    json!({"inventory_id":INVENTORY_ID,"native_instance_id":NATIVE_INSTANCE,
    "privacy_generation":2,"clock_domain":"mach_absolute_time",
    "observed_monotonic_ns":9_000_000_000_u64,
    "completed_monotonic_ns":9_000_000_100_u64,
    "calendars":[
        {"reference":{"calendar_id":"work","source_id":"fixture"},
         "title":"Trabajo","source_title":"Fixture","calendar_type":"caldav"},
        {"reference":{"calendar_id":"team","source_id":"fixture"},
         "title":"Equipo","source_title":"Fixture","calendar_type":"caldav"}
    ],"after_index":0,"total":2})
}

/// One page holding the whole window asked for: every calendar, one event.
/// How the user answered the fixture's daily, by day, so one week holds
/// every state the card draws. The day is counted from 1970-01-01 (UTC).
const DAILY_ANSWERS: [&str; 4] = ["accepted", "tentative", "pending", "declined"];
const DAILY_MEET: &str = "https://meet.google.com/abc-defg-hij";
const FIXTURE_DAY_MS: i64 = 24 * 60 * 60 * 1_000;

fn calendar_read_page(command: &Value) -> Value {
    // Each completed native read has a fresh identity. Reusing one ID hid a
    // client that returned after seeing the preceding week's topic frame.
    static NEXT_READ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let read_id = format!(
        "44444444-4444-4444-8444-{:012x}",
        NEXT_READ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let query = &command["calendar_query"];
    let start = query["start_unix_ms"].as_i64().unwrap_or_default();
    let end = query["end_unix_ms"].as_i64().unwrap_or(start);
    // The daily on every day of the window at 10:45 UTC, so any view
    // the app asks for has events in it, wherever it starts.
    let events = (start.div_euclid(FIXTURE_DAY_MS)..=(end - 1).div_euclid(FIXTURE_DAY_MS))
        .map(|day| (day, day * FIXTURE_DAY_MS + 38_700_000))
        .filter(|(_, at)| (start..end).contains(at))
        .map(|(day, at)| daily_occurrence(day, at))
        .collect::<Vec<_>>();
    json!({"read_id":read_id,"native_instance_id":NATIVE_INSTANCE,"privacy_generation":2,
        "window":{"start_unix_ms":query["start_unix_ms"],"end_unix_ms":query["end_unix_ms"]},
        "clock_domain":"mach_absolute_time","capture_started_monotonic_ns":9_000_000_200_u64,
        "capture_completed_monotonic_ns":9_000_000_300_u64,
        "observed_at_unix_ms":start + 1_000,"system_timezone":"UTC",
        "provider_sync_status":"unknown","enumeration":"bounded_local_read",
        "after_index":0,"total":events.len(),"events":events})
}

fn daily_occurrence(day: i64, at: i64) -> Value {
    let answer = DAILY_ANSWERS[usize::try_from(day.rem_euclid(4)).unwrap_or(0)];
    json!({"observation_id":format!("55555555-5555-4555-8555-{day:012}"),
        "calendar":{"calendar_id":"work","source_id":"fixture"},
        "calendar_item_id":"fixture-daily","event_id":"fixture-daily",
        "start_unix_ms":at,"end_unix_ms":at + 900_000,
        "event_timezone":"Europe/Madrid",
        "interpreted_timezone":"Europe/Madrid","is_floating":false,"is_all_day":false,
        "is_detached":false,"title":"[LedgerBox - Inst.] Daily","status":"confirmed",
        // Bruno first: the user's answer must come from the guest marked as the user,
        // not from whoever is listed first.
        "invitees":[
            {"name":"Bruno","participant_url":"mailto:bruno@example.test",
             "email":"bruno@example.test","role":"required","response_status":"accepted",
             "participant_type":"person","is_current_user":false},
            {"name":"Alina","participant_url":"mailto:alina@example.test",
             "email":"alina@example.test","role":"required","response_status":answer,
             "participant_type":"person","is_current_user":true}],
        "invitees_status":"provided",
        "event_url":DAILY_MEET,"event_url_status":"provided"})
}

/// The helper's answer to a `calendar_save`: one occurrence, as a read gives it.
fn calendar_saved() -> Value {
    json!({"write_id":"66666666-6666-4666-8666-666666666666",
        "native_instance_id":NATIVE_INSTANCE,"privacy_generation":2,"system_timezone":"UTC",
        "occurrence":{"observation_id":"55555555-5555-4555-8555-555555555556",
            "calendar":{"calendar_id":"work","source_id":"fixture"},
            "calendar_item_id":"fixture-daily","event_id":"fixture-daily",
            "original_occurrence_unix_ms":1_792_058_400_000_i64,
            "start_unix_ms":1_792_062_000_000_i64,"end_unix_ms":1_792_065_600_000_i64,
            "event_timezone":"Europe/Madrid","interpreted_timezone":"Europe/Madrid",
            "is_floating":false,"is_all_day":false,"is_detached":true,
            "title":"[LedgerBox - Inst.] Daily","status":"confirmed",
            "invitees_status":"not_provided","event_url_status":"not_provided",
            "availability":"busy","can_edit":true}})
}

fn emulate_native_helper(
    input: Arc<Mutex<File>>,
    output: File,
    mut audio_start_gate: Option<AudioStartGate>,
    commands: Option<Sender<Value>>,
    native_commands: Arc<Mutex<Vec<String>>>,
    config: NativeHelperConfig,
) {
    let ready: Value = match serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/ready.json"
    )) {
        Ok(ready) => ready,
        Err(_) => return,
    };
    let mut state: Value = match serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/state.json"
    )) {
        Ok(state) => state,
        Err(_) => return,
    };
    state["native_instance_id"] = json!(NATIVE_INSTANCE);
    state["privacy_generation"] = json!(2);
    state["calendar"] = calendar_status(config.calendar_authorization);
    if write_shared_frame(&input, &ready).is_err() || write_shared_frame(&input, &state).is_err() {
        return;
    }
    for line in BufReader::new(output).lines() {
        let Ok(line) = line else { return };
        let Ok(command) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(name) = command["command"].as_str() else {
            continue;
        };
        if let Ok(mut seen) = native_commands.lock() {
            seen.push(name.to_owned());
        } else {
            return;
        }
        // Only the first audio_start is held back: a system recording starts one
        // stream per track and the test observes the starting phase once.
        if name == "audio_start"
            && let Some(gate) = audio_start_gate.take()
            && (gate.requested.send(()).is_err() || gate.release.recv().is_err())
        {
            return;
        }
        let frame = match name {
            "status" if config.status_reports_permissions => json!({"v":1,"event":"response",
                "id":command["id"],"ok":true,"result":helper_state_result(Some(json!({
                "microphone":"authorized","screen":true,"accessibility":true,
                "calendars":config.calendar_authorization})), config.calendar_authorization)}),
            "status" => json!({"v":1,"event":"response","id":command["id"],"ok":true,
                "result":helper_state_result(None, config.calendar_authorization)}),
            "hello" | "heartbeat" => json!({"v":1,"event":"response",
                "id":command["id"],"ok":true,"result":helper_state_result(None, config.calendar_authorization)}),
            "permissions" => {
                let requests_calendar = command["request"]
                    .as_array()
                    .is_some_and(|request| request.iter().any(|item| item == "calendars"));
                let calendars = if config.calendar_authorization == CalendarAuthorization::Denied {
                    CalendarAuthorization::Denied
                } else if requests_calendar
                    || config.calendar_authorization == CalendarAuthorization::FullAccess
                {
                    CalendarAuthorization::FullAccess
                } else {
                    config.calendar_authorization
                };
                json!({"v":1,"event":"response","id":command["id"],"ok":true,
                    "result":{"microphone":"authorized","screen":false,
                    "accessibility":true,"calendars":calendars}})
            }
            "calendar_sources" => json!({"v":1,"event":"response","id":command["id"],"ok":true,
                "result":calendar_inventory()}),
            "calendar_snapshot"
                if config.calendar_authorization != CalendarAuthorization::FullAccess =>
            {
                json!({
                    "v":1,"event":"response","id":command["id"],"ok":false,
                    "error":{"code":"calendar_permission_required",
                        "message":"fixture: el permiso de calendarios no está concedido"}
                })
            }
            "calendar_snapshot" => json!({"v":1,"event":"response","id":command["id"],"ok":true,
                "result":calendar_read_page(&command)}),
            "calendar_disconnect" => json!({"v":1,"event":"response","id":command["id"],"ok":true,
                "result":calendar_status(config.calendar_authorization)}),
            // What the helper answers a write with — the occurrence as
            // it is now, or what it removed.
            "calendar_save" => json!({"v":1,"event":"response","id":command["id"],"ok":true,
                "result":calendar_saved()}),
            "calendar_delete" => json!({"v":1,"event":"response","id":command["id"],"ok":true,
                "result":{"write_id":"66666666-6666-4666-8666-666666666666",
                    "native_instance_id":NATIVE_INSTANCE,"privacy_generation":2,
                    "calendar_item_id":command["calendar_delete"]["calendar_item_id"],
                    "occurrence_start_ms":command["calendar_delete"]["occurrence_start_ms"],
                    "span":command["calendar_delete"]["span"]}}),
            "audio_start" => match config.audio_start_refusal {
                // The shape the Mac answers with when it will not capture: the
                // daemon reads `error.code` and refuses the recording by it.
                Some(code) => json!({"v":1,"event":"response","id":command["id"],"ok":false,
                    "error":{"code":code,"message":"fixture: el Mac rechaza la captura"}}),
                None => json!({"v":1,"event":"response","id":command["id"],"ok":true,
                    "result":{"stream_id":command["stream_id"]}}),
            },
            "audio_stop" => json!({"v":1,"event":"response","id":command["id"],"ok":true,
                "result":{"stream_id":command["stream_id"],"drain_complete":true}}),
            "privacy_stop" => {
                let mut result = helper_state_result(None, config.calendar_authorization);
                result["privacy_stopped"] = json!(true);
                result["drain_complete"] = json!(true);
                json!({"v":1,"event":"response","id":command["id"],"ok":true,"result":result})
            }
            "capture_stop" => {
                let mut result = helper_state_result(None, config.calendar_authorization);
                result["drain_complete"] = json!(true);
                json!({"v":1,"event":"response","id":command["id"],"ok":true,"result":result})
            }
            "rearm" => json!({"v":1,"event":"local_rearm"}),
            _ => json!({"v":1,"event":"response","id":command["id"],"ok":true,"result":{}}),
        };
        if write_shared_frame(&input, &frame).is_err() {
            return;
        }
        if let Some(commands) = commands.as_ref() {
            let _ = commands.send(command);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_status_supplies_permissions_to_app() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-permissions-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (_daemon, helper_commands) = TestDaemon::start_calendar_observed(directory.path())?;
    let client = DaemonClient::connect(paths, _daemon.child.client.clone());

    let command = wait_for_helper_command(&helper_commands, "status")?;
    assert_eq!(command["command"], "status");
    wait_for(&client, |snapshot| {
        snapshot.permissions.microphone == PermissionState::Granted
            && snapshot.permissions.screen == PermissionState::Granted
            && snapshot.permissions.accessibility == PermissionState::Granted
            && snapshot.permissions.calendars == PermissionState::Granted
    })
    .await;
    Ok(())
}

/// With the permission, the app shows every calendar and the
/// week without anyone listing or choosing anything, and Settings counts them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_app_shows_every_calendar_without_being_asked() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-calendar-list-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let daemon = TestDaemon::start_calendar(directory.path())?;
    let client = DaemonClient::connect(paths, daemon.child.client.clone());
    wait_for(&client, |snapshot| {
        snapshot.calendar.calendars.len() == 2
            && snapshot.settings.calendars.total == Some(2)
            && !snapshot.calendar.events.is_empty()
    })
    .await;
    assert!(daemon.native_command_count("calendar_sources")? >= 1);
    Ok(())
}

fn wait_for_helper_command(
    commands: &Receiver<Value>,
    name: &str,
) -> Result<Value, Box<dyn Error>> {
    wait_for_helper_command_within(commands, name, HELPER_COMMAND_BUDGET)
}

/// The budget is a parameter only so that the guard below can watch one expire
/// without standing there for the real budget; every caller uses the constant.
fn wait_for_helper_command_within(
    commands: &Receiver<Value>,
    name: &str,
    budget: Duration,
) -> Result<Value, Box<dyn Error>> {
    let deadline = Instant::now() + budget;
    let mut arrived: Vec<String> = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let command = match commands.recv_timeout(remaining) {
            Ok(command) => command,
            // This used to be a bare `?`, so a lost wait reached the
            // build log as `Error: Timeout` and said nothing about what the
            // test had been waiting for. It costs an hour to read that.
            Err(reason) => {
                return Err(format!(
                    "el daemon nunca pidió `{name}` al helper nativo en {budget:?} \
                     ({reason}); órdenes que sí llegaron: {}",
                    if arrived.is_empty() {
                        "ninguna".to_owned()
                    } else {
                        arrived.join(", ")
                    }
                )
                .into());
            }
        };
        if command["command"] == name {
            return Ok(command);
        }
        if let Some(other) = command["command"].as_str() {
            arrived.push(other.to_owned());
        }
    }
}

async fn wait_for<F>(client: &DaemonClient, condition: F) -> orbit_app_core::AppSnapshot
where
    F: Fn(&orbit_app_core::AppSnapshot) -> bool,
{
    let mut updates = client.watch_frames();
    tokio::time::timeout(SNAPSHOT_BUDGET, async {
        loop {
            let snapshot = client.composed_snapshot();
            if condition(&snapshot) {
                return snapshot;
            }
            updates.changed().await.expect("client stopped publishing");
        }
    })
    .await
    .unwrap_or_else(|_| {
        // `type_name` of the closure carries the test it was written in, so an
        // expiry names the wait without every call site having to label itself.
        panic!(
            "the snapshot never met the condition of {} within {SNAPSHOT_BUDGET:?}; last: {:?}",
            std::any::type_name::<F>(),
            client.composed_snapshot()
        )
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_end_leaves_the_last_recording_stale() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-client-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let mut first = TestDaemon::start(directory.path())?;
    let client = DaemonClient::connect(paths, first.child.client.clone());

    wait_for(&client, |snapshot| {
        snapshot.connection.daemon == DaemonConnection::Ready
            && snapshot.permissions.microphone == PermissionState::Granted
    })
    .await;
    client
        .send(AppCommand::Recording(
            orbit_app_core::features::recording::RecordingCommand::MeetingStart {
                title: "Fixture de integración".into(),
            },
        ))
        .await?;
    wait_for(&client, |snapshot| {
        snapshot.meeting.phase == MeetingPhase::Recording
    })
    .await;

    first.kill()?;
    wait_for(&client, |snapshot| {
        snapshot.connection.daemon == DaemonConnection::Lost
    })
    .await;
    assert_eq!(
        client.composed_snapshot().meeting.phase,
        MeetingPhase::Recording
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meeting_start_and_finish_round_trip_through_the_real_daemon() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-meeting-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (_daemon, audio_requested, release_audio) =
        TestDaemon::start_with_audio_gate(directory.path())?;
    let client = DaemonClient::connect(paths, _daemon.child.client.clone());

    wait_for(&client, |snapshot| {
        snapshot.connection.daemon == DaemonConnection::Ready
            && snapshot.permissions.microphone == PermissionState::Granted
    })
    .await;

    let start = serde_json::from_value::<AppCommand>(json!({
        "command": "meeting_start",
        "title": "Fixture de integración",
        "minutes": 1,
        "audio": "system"
    }))?;
    let sending_client = client.clone();
    let send_task = tokio::spawn(async move { sending_client.send(start).await });
    tokio::task::spawn_blocking(move || audio_requested.recv_timeout(AUDIO_START_BUDGET))
        .await?
        .map_err(|reason| {
            format!("the daemon never asked to start the audio capture within {AUDIO_START_BUDGET:?} ({reason})")
        })?;
    // While the gate holds the helper's acknowledgement, `starting` is
    // the only phase anything can publish, so this wait and the read below it
    // are a fact, not a race against the answer to meeting_start.
    wait_for(&client, |snapshot| {
        snapshot.meeting.phase == MeetingPhase::Starting
    })
    .await;
    let starting = serde_json::to_value(client.composed_snapshot())?;
    release_audio.send(())?;
    let started = send_task.await??;
    assert!(matches!(started.delivery, Some(ReceiptDelivery::Reflected)));
    let recording = client
        .bootstrap()
        .frames
        .into_iter()
        .find_map(|frame| {
            if let FeatureFrame::Recording { view, .. } = frame {
                Some(view)
            } else {
                None
            }
        })
        .ok_or("recording frame missing after start receipt")?;
    assert_eq!(recording.meeting.phase, MeetingPhase::Starting);
    assert_eq!(
        serde_json::to_value(&recording)?["availability"]["state"],
        "current"
    );
    assert_eq!(
        starting["meeting"]["phase"], "starting",
        "the app must expose the daemon's starting phase before audio capture is acknowledged"
    );
    // The event, not a deadline: recording appears when the daemon says both
    // tracks are alive, never because the start was asked.
    wait_for(&client, |snapshot| {
        snapshot.meeting.phase == MeetingPhase::Recording
    })
    .await;

    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::MeetingFinish,
        )))
        .await?;
    wait_for(&client, |snapshot| {
        snapshot.meeting.phase == MeetingPhase::Idle
    })
    .await;
    let finished = serde_json::to_value(client.composed_snapshot())?;
    assert!(
        finished["meeting"]["recent"][0]["recording_id"]
            .as_str()
            .is_some_and(|recording_id| !recording_id.is_empty()),
        "the finished recording is absent from meeting.recent"
    );
    assert_eq!(
        finished["meeting"]["recent"][0]["title"],
        "Fixture de integración"
    );
    Ok(())
}

/// When the helper refuses the capture, the app shows the refusal
/// with its cause and never «recording». The gate holds the refusal until the
/// test has seen the app in `starting`, so both halves are facts: what the
/// answer to `meeting_start` may claim, and what the refusal leaves on screen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_audio_start_shows_the_cause_and_not_recording() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-refused-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (_daemon, audio_requested, release_refusal) =
        TestDaemon::start_with_audio_refusal(directory.path(), "microphone_permission_required")?;
    let client = DaemonClient::connect(paths, _daemon.child.client.clone());

    wait_for(&client, |snapshot| {
        snapshot.connection.daemon == DaemonConnection::Ready
            && snapshot.permissions.microphone == PermissionState::Granted
    })
    .await;

    let sending_client = client.clone();
    let send_task = tokio::spawn(async move {
        sending_client
            .send(AppCommand::Recording(
                orbit_app_core::features::recording::RecordingCommand::MeetingStart {
                    title: "Fixture rechazada".into(),
                },
            ))
            .await
    });
    tokio::task::spawn_blocking(move || audio_requested.recv_timeout(AUDIO_START_BUDGET))
        .await?
        .map_err(|reason| {
            format!("the daemon never asked to start the audio capture within {AUDIO_START_BUDGET:?} ({reason})")
        })?;
    wait_for(&client, |snapshot| {
        snapshot.meeting.phase == MeetingPhase::Starting
    })
    .await;
    let prior_sequence = client
        .composed_snapshot()
        .notice
        .and_then(|notice| notice.sequence)
        .unwrap_or(0);
    release_refusal.send(())?;
    let started = send_task.await??;
    assert!(matches!(started.delivery, Some(ReceiptDelivery::Reflected)));
    let recording = client
        .bootstrap()
        .frames
        .into_iter()
        .find_map(|frame| {
            if let FeatureFrame::Recording { view, .. } = frame {
                Some(view)
            } else {
                None
            }
        })
        .ok_or("recording frame missing after refused helper")?;
    // Starting was observed before releasing the helper refusal above. With
    // revision-bound receipts, the refusal's idle topic can arrive before
    // send returns, so Starting is no longer the only valid phase here. This
    // assertion still forbids a false Recording claim after the helper said no.
    assert_ne!(
        recording.meeting.phase,
        MeetingPhase::Recording,
        "the helper refused the capture, so the receipt cannot claim recording"
    );
    // The observer deliberately arrives after the following notice. The
    // permission cause must still reach the interface's ordered notice list.
    let cut = wait_for(&client, |snapshot| {
        snapshot.notice.as_ref().is_some_and(|notice| {
            notice.code == orbit_core::notice::NoticeCode::CaptureCutConfirmed
        })
    })
    .await;
    let cut_sequence = cut
        .notice
        .and_then(|notice| notice.sequence)
        .ok_or("capture cut confirmation has no sequence")?;
    // The refusal itself: back to idle, with the helper's cause on screen and
    // its code, so whoever reads it —the user or Deimos— can act on the numbers.
    wait_for(&client, |snapshot| {
        snapshot.meeting.phase == MeetingPhase::Idle
            && client.bootstrap().frames.into_iter().any(|frame| {
                matches!(frame, FeatureFrame::Status { view, .. }
                    if view.notices.iter().any(|notice|
                        notice.code == orbit_core::notice::NoticeCode::MicrophonePermissionMissing
                            && notice.sequence.is_some_and(|sequence| sequence > prior_sequence
                                && sequence < cut_sequence)))
            })
    })
    .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_requests_reach_the_real_helper() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-settings-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (_daemon, helper_commands) = TestDaemon::start_observed(directory.path())?;
    let client = DaemonClient::connect(paths, _daemon.child.client.clone());

    wait_for(&client, |snapshot| {
        snapshot.connection.daemon == DaemonConnection::Ready
    })
    .await;
    // There is no switch to send before; the macOS permission is the
    // whole decision, and asking for it is what reaches the helper.
    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Permissions {
                request: vec!["microphone".into()],
            },
        )))
        .await
        .map_err(|error| format!("permission request failed: {error}"))?;
    let permission_command = wait_for_helper_command(&helper_commands, "permissions")?;
    assert_eq!(permission_command["request"], json!(["microphone"]));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reads_a_voice_note_wav_once_through_the_real_daemon() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-voice-note-")
        .tempdir_in("/tmp")?;
    let hub = SyntheticAssistantHub::start()?;
    configure_assistant_hub(directory.path(), hub.address)?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (daemon, helper_commands) = TestDaemon::start_observed(directory.path())?;
    let client = DaemonClient::connect(paths, daemon.child.client.clone());

    wait_for(&client, |snapshot| {
        snapshot.connection.daemon == DaemonConnection::Ready
            && snapshot.connection.hub == orbit_app_core::HubConnection::Paired
            && snapshot.permissions.microphone == PermissionState::Granted
    })
    .await;
    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::VoiceStart,
        )))
        .await?;
    let start = wait_for_helper_command(&helper_commands, "audio_start")?;
    let stream_id = start["stream_id"]
        .as_str()
        .ok_or("audio_start stream id missing")?;
    daemon.voice_audio_frame(stream_id)?;
    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::VoiceEnd,
        )))
        .await?;

    wait_for(&client, |snapshot| {
        snapshot.chat.turn.iter().any(|message| {
            message
                .voice
                .as_ref()
                .is_some_and(|voice| voice.audio_available)
        })
    })
    .await;
    let message_id = client
        .composed_snapshot()
        .chat
        .turn
        .iter()
        .find(|message| message.voice.is_some())
        .map(|message| message.id.clone())
        .ok_or("voice note missing")?;
    client
        .send(AppCommand::Assistant(
            orbit_app_core::features::assistant::AssistantCommand::VoiceNoteAudio {
                message_id: message_id.clone(),
            },
        ))
        .await?;
    let with_audio = client.composed_snapshot();
    let audio = with_audio
        .chat
        .voice_audio
        .as_ref()
        .ok_or("voice note audio missing")?;
    assert_eq!(audio.message_id, message_id);
    assert_eq!(audio.format, "wav");
    assert!(audio.bytes_base64.starts_with("UklGR"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn briefing_section_stays_total_against_the_real_daemon() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-briefing-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let _daemon = TestDaemon::start(directory.path())?;
    let client = DaemonClient::connect(paths, _daemon.child.client.clone());

    wait_for(&client, |snapshot| {
        snapshot.connection.daemon == DaemonConnection::Ready
            && snapshot.permissions.microphone == PermissionState::Granted
    })
    .await;

    let briefing = client
        .bootstrap()
        .frames
        .into_iter()
        .find_map(|frame| {
            if let FeatureFrame::Briefing { view, .. } = frame {
                *view
            } else {
                None
            }
        })
        .ok_or("the daemon must publish a briefing frame")?;
    assert_eq!(
        briefing.next, None,
        "without calendar events there is no next meeting"
    );
    assert!(briefing.agenda.is_empty());
    assert!(matches!(
        briefing.sources.as_ref().map(|sources| &sources.agenda),
        Some(BriefingSourceState::Absent { cause } | BriefingSourceState::Unknown { cause })
            if !cause.is_empty()
    ));
    Ok(())
}

struct FixedUtcCalendarClock;

impl CalendarClock for FixedUtcCalendarClock {
    fn now_ms(&self) -> i64 {
        // A Friday before the Sunday sent below; the test's zone is explicit.
        1_789_862_400_000 - 2 * 24 * 60 * 60 * 1_000
    }

    fn local_zone(&self, _now_ms: i64) -> LocalZone {
        LocalZone::utc()
    }

    fn timezone_name(&self) -> Result<String, String> {
        Ok("UTC".into())
    }
}

// Keep the app's synchronous bootstrap/snapshot reads on one observer turn.
// The real daemon and helper still run on their own threads; app publications
// can advance between awaits, but not between those two synchronous reads.
#[tokio::test(flavor = "current_thread")]
async fn loads_and_refreshes_the_visible_week_through_the_real_daemon() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-calendar-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let daemon = TestDaemon::start_calendar(directory.path())?;
    let client = DaemonClient::connect_with_clock(
        paths.clone(),
        Arc::new(FixedUtcCalendarClock),
        daemon.child.client.clone(),
    );

    // The permission is enough; the week is read with nothing chosen.
    let loaded_week = |snapshot: &orbit_app_core::AppSnapshot| {
        !snapshot.calendar.events.is_empty() && !snapshot.calendar.stale
    };
    wait_for(&client, loaded_week).await;
    daemon.repeat_helper_state().await?;
    drop(client);
    let client = DaemonClient::connect_with_clock(
        paths.clone(),
        Arc::new(FixedUtcCalendarClock),
        daemon.child.client.clone(),
    );
    wait_for(&client, loaded_week).await;
    let bootstrap = client.bootstrap();
    assert_eq!(
        client.composed_snapshot().revision,
        bootstrap.revision.counter
    );

    let first_reads = daemon.native_command_count("calendar_snapshot")?;
    assert_eq!(first_reads, 1);
    let first_head = observed_agenda(&daemon.child.client).await?;
    let next_sunday = 1_789_862_400_000_i64;
    client
        .send(set_range(
            CalendarViewKind::Week,
            next_sunday,
            7,
            CalendarWeekStart::Sunday,
        ))
        .await?;
    // The week on screen, and the read widened back to today (the
    // Friday before) so the Briefing keeps its day.
    let today = FixedUtcCalendarClock.now_ms();
    let next_head = observed_agenda(&daemon.child.client).await?;
    assert_eq!(
        next_head
            .read_window
            .map(|window| (window.start_unix_ms, window.end_unix_ms)),
        Some((today, next_sunday + 7 * FIXTURE_DAY_MS))
    );
    assert_ne!(
        next_head.read_id, first_head.read_id,
        "next week returned before its new read_id"
    );
    wait_for(&client, |snapshot| {
        snapshot.calendar.range.start_ms == next_sunday
            && snapshot.calendar.window.start_ms == today
            && !snapshot.calendar.events.is_empty()
    })
    .await;
    assert_eq!(daemon.native_command_count("calendar_snapshot")?, 2);

    client
        .send(AppCommand::Agenda(
            orbit_app_core::features::agenda::AgendaCommand::CalendarRefresh,
        ))
        .await?;
    let refreshed_head = observed_agenda(&daemon.child.client).await?;
    assert_eq!(refreshed_head.read_window, next_head.read_window);
    assert_ne!(
        refreshed_head.read_id, next_head.read_id,
        "refresh returned before its new read_id"
    );
    wait_for(&client, |snapshot| {
        snapshot.revision > 0 && !snapshot.calendar.stale
    })
    .await;
    assert_eq!(daemon.native_command_count("calendar_snapshot")?, 3);

    daemon.calendar_invalidated()?;
    wait_for(&client, |_| {
        daemon
            .native_command_count("calendar_snapshot")
            .is_ok_and(|count| count == 4)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(daemon.native_command_count("calendar_snapshot")?, 4);
    Ok(())
}

fn set_range(
    view: CalendarViewKind,
    anchor_ms: i64,
    days: u8,
    week_start: CalendarWeekStart,
) -> AppCommand {
    AppCommand::Agenda(
        orbit_app_core::features::agenda::AgendaCommand::CalendarSetRange {
            view,
            anchor_ms,
            days,
            week_start,
        },
    )
}

/// How the user answered and the video link come from the real read —
/// helper, daemon, topic pages and app-core — not only from the projection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn answers_and_video_links_arrive_through_the_real_daemon() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-answers-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let _daemon = TestDaemon::start_calendar(directory.path())?;
    let client = DaemonClient::connect_with_clock(
        paths.clone(),
        Arc::new(FixedUtcCalendarClock),
        _daemon.child.client.clone(),
    );
    let seven_events = |snapshot: &orbit_app_core::AppSnapshot| {
        snapshot.calendar.events.len() == 7 && !snapshot.calendar.stale
    };
    wait_for(&client, seven_events).await;
    _daemon.repeat_helper_state().await?;
    drop(client);
    let client = DaemonClient::connect_with_clock(
        paths,
        Arc::new(FixedUtcCalendarClock),
        _daemon.child.client.clone(),
    );
    let snapshot = wait_for(&client, seven_events).await;
    let mut answers = snapshot
        .calendar
        .events
        .iter()
        .map(|event| {
            let day = event.start_ms.div_euclid(FIXTURE_DAY_MS);
            (
                DAILY_ANSWERS[usize::try_from(day.rem_euclid(4)).unwrap_or(0)],
                event.self_response,
            )
        })
        .collect::<Vec<_>>();
    answers.sort_by_key(|(written, _)| *written);
    answers.dedup();
    assert_eq!(
        answers,
        vec![
            ("accepted", Some(CalendarResponse::Accepted)),
            ("declined", Some(CalendarResponse::Declined)),
            ("pending", Some(CalendarResponse::NeedsAction)),
            ("tentative", Some(CalendarResponse::Tentative)),
        ]
    );
    for event in &snapshot.calendar.events {
        assert_eq!(event.conference_url.as_deref(), Some(DAILY_MEET));
        let guests = event.attendees.as_deref().unwrap_or_default();
        assert_eq!(guests.len(), 2);
        assert_eq!(guests[0].response, Some(CalendarResponse::Accepted));
        assert_eq!(guests[1].response, event.self_response);
    }
    Ok(())
}

/// Each view asks the daemon for its own window — the month and
/// the agenda are wider than one daemon window (`READ_WINDOW_MS`) and are
/// read in several — and the Briefing keeps today while the calendar shows
/// another month.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_view_reads_its_window_and_the_briefing_keeps_today() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-windows-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let daemon = TestDaemon::start_calendar(directory.path())?;
    let client = DaemonClient::connect_with_clock(
        paths.clone(),
        Arc::new(FixedUtcCalendarClock),
        daemon.child.client.clone(),
    );
    wait_for(&client, |snapshot| {
        !snapshot.calendar.events.is_empty() && !snapshot.calendar.stale
    })
    .await;
    // Friday 18 September 2026, 00:00 UTC.
    let friday = FixedUtcCalendarClock.now_ms();
    let day = FIXTURE_DAY_MS;
    let monday = friday - 4 * day;
    let cases = [
        // (view, anchor, days, visible range, read window, native reads)
        (
            CalendarViewKind::Day,
            friday,
            1,
            (friday, friday + day),
            (friday, friday + day),
            1,
        ),
        (
            CalendarViewKind::Week,
            friday,
            7,
            (monday, monday + 7 * day),
            (monday, monday + 7 * day),
            1,
        ),
        // October: Monday 28 September to Sunday 1 November, 35 days, read
        // from today (18 September), 45 days: two daemon windows.
        (
            CalendarViewKind::Month,
            friday + 27 * day,
            0,
            (friday + 10 * day, friday + 45 * day),
            (friday, friday + 45 * day),
            2,
        ),
        // ±31 days around Friday: 63 days, three daemon windows.
        (
            CalendarViewKind::Agenda,
            friday,
            0,
            (friday - 31 * day, friday + 32 * day),
            (friday - 31 * day, friday + 32 * day),
            3,
        ),
    ];
    for (view, anchor, days, visible, read, native_reads) in cases {
        let before = daemon.native_command_count("calendar_snapshot")?;
        client
            .send(set_range(view, anchor, days, CalendarWeekStart::Monday))
            .await?;
        let head = observed_agenda(&daemon.child.client).await?;
        assert_eq!(
            head.read_window
                .map(|window| (window.start_unix_ms, window.end_unix_ms)),
            Some(read),
            "{view:?}: window read"
        );
        assert_eq!(
            daemon.native_command_count("calendar_snapshot")? - before,
            native_reads,
            "{view:?}: daemon windows"
        );
        wait_for(&client, |snapshot| {
            snapshot.calendar.window.start_ms == read.0
                && snapshot.calendar.window.end_ms == read.1
                && !snapshot.calendar.stale
        })
        .await;
        let snapshot = client.composed_snapshot();
        assert_eq!(
            (
                snapshot.calendar.range.start_ms,
                snapshot.calendar.range.end_ms
            ),
            visible,
            "{view:?}: range on screen"
        );
        assert!(
            snapshot
                .calendar
                .events
                .iter()
                .all(|event| event.start_ms >= visible.0 && event.start_ms < visible.1),
            "{view:?}: the view holds only its own events"
        );
        let briefing = snapshot.briefing.as_ref().ok_or("briefing missing")?;
        assert_eq!(
            briefing.sources.as_ref().map(|sources| &sources.agenda),
            Some(&BriefingSourceState::Present),
            "{view:?}: the Briefing keeps today"
        );
        assert_eq!(briefing.agenda.len(), 1, "{view:?}: today's daily");
    }
    Ok(())
}

async fn observed_agenda(
    daemon: &orbit_ipc::LocalClient,
) -> Result<orbit_core::topics::agenda::Value, Box<dyn Error>> {
    let (_reply, mut subscription) =
        embedded_daemon::subscribe_topics(daemon, vec![TopicId::Agenda]).await?;
    let head = tokio::time::timeout(Duration::from_secs(10), subscription.next_topic()).await??;
    match head.data {
        TopicData::Agenda(Knowledge::Present { value, .. }) => Ok(value),
        other => Err(format!("agenda head was not a completed read: {other:?}").into()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_calendar_read_returns_the_daemon_cause() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-calendar-refused-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let _daemon = TestDaemon::start_calendar_denied(directory.path())?;
    let client = DaemonClient::connect_with_clock(
        paths.clone(),
        Arc::new(FixedUtcCalendarClock),
        _daemon.child.client.clone(),
    );
    wait_for(&client, |snapshot| {
        snapshot.connection.daemon == DaemonConnection::Ready
    })
    .await;

    _daemon.repeat_helper_state().await?;
    drop(client);
    let client = DaemonClient::connect_with_clock(
        paths,
        Arc::new(FixedUtcCalendarClock),
        _daemon.child.client.clone(),
    );
    for _ in 0..2 {
        let result = client
            .send(AppCommand::Agenda(
                orbit_app_core::features::agenda::AgendaCommand::CalendarRefresh,
            ))
            .await;
        assert!(
            result.as_ref().err().is_some_and(|error| {
                error.refusal.as_deref()
                    == Some(&orbit_core::refusal::Refusal::PermissionDenied {
                        permission: orbit_core::refusal::Permission::Calendar,
                    })
            }),
            "the Mac permission refusal must be returned by the read: {result:?}"
        );
    }
    Ok(())
}

const ASSISTANT_DEVICE_ID: &str = "00000000-0000-4000-8000-000000000006";
const ASSISTANT_SESSION_ID: &str = "session-app-06";

struct SyntheticAssistantHub {
    address: SocketAddr,
    worker: Option<thread::JoinHandle<()>>,
}

impl SyntheticAssistantHub {
    fn start() -> Result<Self, Box<dyn Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let worker = thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            if websocket_handshake(&mut stream).is_err() {
                return;
            }
            let websocket = thread::spawn(move || serve_assistant_websocket(stream));
            for _ in 0..2 {
                let Ok((mut http, _)) = listener.accept() else {
                    let _ = websocket.join();
                    return;
                };
                let _ = answer_assistant_http(&mut http);
            }
            let _ = websocket.join();
        });
        Ok(Self {
            address,
            worker: Some(worker),
        })
    }
}

fn serve_assistant_websocket(mut stream: TcpStream) {
    if send_hub_event(
        &mut stream,
        "ready",
        "ready",
        json!({"device_id": ASSISTANT_DEVICE_ID}),
    )
    .is_err()
    {
        return;
    }
    let mut chats = 0_u8;
    while let Ok(Some((opcode, payload))) = read_websocket_frame(&mut stream) {
        if opcode == 0x8 {
            return;
        }
        if opcode == 0x9 {
            if write_websocket_frame(&mut stream, 0xA, &payload).is_err() {
                return;
            }
            continue;
        }
        if opcode != 0x1 {
            continue;
        }
        let Ok(frame) = serde_json::from_slice::<Value>(&payload) else {
            continue;
        };
        let Some(id) = frame["id"].as_str() else {
            continue;
        };
        match frame["method"].as_str() {
            Some("chat.send") => {
                chats = chats.saturating_add(1);
                let sent = match chats {
                    1 => send_hub_event(
                        &mut stream,
                        id,
                        "chat.session",
                        json!({"session_id": ASSISTANT_SESSION_ID}),
                    )
                    .and_then(|()| {
                        send_hub_event(
                            &mut stream,
                            id,
                            "chat.delta",
                            json!({
                                "session_id": ASSISTANT_SESSION_ID,
                                "text": "Respuesta "
                            }),
                        )
                    })
                    .and_then(|()| {
                        send_hub_event(
                            &mut stream,
                            id,
                            "chat.delta",
                            json!({
                                "session_id": ASSISTANT_SESSION_ID,
                                "text": "parcial"
                            }),
                        )
                    }),
                    2 => send_hub_event(
                        &mut stream,
                        id,
                        "approval.required",
                        json!({
                            "session_id": ASSISTANT_SESSION_ID,
                            "request_id": id,
                            "description": "Actualizar la agenda"
                        }),
                    ),
                    _ => send_hub_event(
                        &mut stream,
                        id,
                        "clarify.required",
                        json!({
                            "session_id": ASSISTANT_SESSION_ID,
                            "request_id": id,
                            "question": "¿En qué calendario?"
                        }),
                    ),
                };
                if sent.is_err() {
                    return;
                }
            }
            Some("approval.respond" | "clarify.respond")
                if send_hub_event(
                    &mut stream,
                    id,
                    "interaction.resolved",
                    json!({
                        "session_id": ASSISTANT_SESSION_ID,
                        "request_id": frame["params"]["request_id"],
                        "status": "ok",
                        "applied": true,
                        "resolved": 1,
                        "stale_interaction": false
                    }),
                )
                .is_err() =>
            {
                return;
            }
            Some("approval.respond" | "clarify.respond") => {}
            _ => {}
        }
    }
}

fn answer_assistant_http(stream: &mut TcpStream) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        let count = stream.read(&mut buffer)?;
        if count == 0 || request.len() + count > 16 * 1024 {
            return Err(std::io::Error::other("invalid meetings-sharing request"));
        }
        request.extend_from_slice(&buffer[..count]);
    };
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
        })
        .unwrap_or_default();
    let request_line = headers.lines().next().unwrap_or_default().to_owned();
    while request.len() < header_end + content_length {
        let count = stream.read(&mut buffer)?;
        if count == 0 || request.len() + count > 16 * 1024 {
            return Err(std::io::Error::other("truncated meetings-sharing request"));
        }
        request.extend_from_slice(&buffer[..count]);
    }
    let body = match request_line.as_str() {
        "PUT /v1/meetings-sharing HTTP/1.1" => br#"{"meetings":false}"#.as_slice(),
        "GET /v1/sessions HTTP/1.1" => br#"{"sessions":[]}"#.as_slice(),
        _ => return Err(std::io::Error::other("unexpected assistant request")),
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

impl Drop for SyntheticAssistantHub {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            // A failed daemon launch may leave the fixture waiting for its
            // first websocket or two optional HTTP requests. Wake those local
            // accepts so the real startup error can reach the test runner.
            for _ in 0..3 {
                if let Err(error) = TcpStream::connect(self.address) {
                    if error.kind() != std::io::ErrorKind::ConnectionRefused {
                        eprintln!("fixture hub wake failed: {error}");
                    }
                    break;
                }
            }
            if worker.join().is_err() {
                eprintln!("fixture hub worker panicked");
            }
        }
    }
}

fn configure_assistant_hub(data_dir: &Path, address: SocketAddr) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(
        data_dir.join("config.toml"),
        format!(
            "version = 1\ndevice_name = \"Fixture assistant\"\nhub_url = \"http://{address}\"\ndevice_id = \"{ASSISTANT_DEVICE_ID}\"\nspeak_responses = false\n"
        ),
    )?;
    std::fs::write(data_dir.join("development-device.key"), "fixture-token")
}

fn pending_kind(snapshot: &orbit_app_core::AppSnapshot) -> Option<String> {
    serde_json::to_value(snapshot)
        .ok()?
        .pointer("/chat/pending/kind")?
        .as_str()
        .map(str::to_owned)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_daemon_publishes_chat_deltas_and_resolves_pending_interactions()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-assistant-")
        .tempdir_in("/tmp")?;
    let hub = SyntheticAssistantHub::start()?;
    configure_assistant_hub(directory.path(), hub.address)?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let _daemon = TestDaemon::start(directory.path())?;
    let client = DaemonClient::connect(paths, _daemon.child.client.clone());

    wait_for(&client, |snapshot| {
        snapshot.connection.daemon == DaemonConnection::Ready
            && snapshot.connection.hub == orbit_app_core::HubConnection::Paired
    })
    .await;

    let admitted = client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Chat {
                attachments: Vec::new(),
                text: "Prepárame la daily".into(),
                primary: false,
            },
        )))
        .await?;
    assert!(matches!(
        admitted.delivery,
        Some(ReceiptDelivery::Reflected)
    ));
    let admitted_view = client
        .bootstrap()
        .frames
        .into_iter()
        .find_map(|frame| {
            if let FeatureFrame::Assistant { view, .. } = frame {
                Some(view)
            } else {
                None
            }
        })
        .ok_or("assistant frame missing after chat receipt")?;
    let admitted_json = serde_json::to_value(&admitted_view)?;
    assert_eq!(admitted_json["turn"][0]["text"], "Prepárame la daily");
    assert_eq!(admitted_json["turn"][0]["role"], "user");
    assert!(admitted_json["turn"][0]["id"].is_string());

    wait_for(&client, |snapshot| {
        snapshot
            .chat
            .turn
            .iter()
            .any(|message| message.text == "Respuesta parcial")
    })
    .await;
    let streaming = serde_json::to_value(client.composed_snapshot())?;
    assert_eq!(streaming["chat"]["turn"][1]["streaming"], true);
    let interrupted = client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Interrupt,
        )))
        .await?;
    assert!(
        matches!(interrupted.delivery, Some(ReceiptDelivery::Reflected)),
        "interrupt must be reflected in the topic cache before send returns: {interrupted:?}"
    );
    let interrupted_view = client
        .bootstrap()
        .frames
        .into_iter()
        .find_map(|frame| {
            if let FeatureFrame::Assistant { view, .. } = frame {
                Some(view)
            } else {
                None
            }
        })
        .ok_or("assistant frame missing after interrupt")?;
    assert!(
        serde_json::to_value(interrupted_view)?["turn"]
            .as_array()
            .is_some_and(|messages| messages.iter().all(|message| message["streaming"] == false)),
        "when interrupt returns, the assistant topic already shows every message finished"
    );

    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Chat {
                attachments: Vec::new(),
                text: "Actualiza la agenda".into(),
                primary: false,
            },
        )))
        .await?;
    wait_for(&client, |snapshot| {
        pending_kind(snapshot).as_deref() == Some("approval")
    })
    .await;
    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Approval {
                approved: true,
                choice: None,
            },
        )))
        .await?;
    wait_for(&client, |snapshot| snapshot.chat.pending_contract.is_none()).await;
    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Interrupt,
        )))
        .await?;

    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Chat {
                attachments: Vec::new(),
                text: "Añade el bloque".into(),
                primary: false,
            },
        )))
        .await?;
    wait_for(&client, |snapshot| {
        pending_kind(snapshot).as_deref() == Some("clarify")
    })
    .await;
    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Clarify {
                answer: "En el calendario de trabajo".into(),
                question_id: None,
            },
        )))
        .await?;
    wait_for(&client, |snapshot| snapshot.chat.pending_contract.is_none()).await;
    Ok(())
}

/// The app's three calendar writes reach the helper
/// through the real daemon, each with its shape — create without a calendar
/// (macOS's default), a move of one occurrence, a delete of it and the ones
/// after it. Nothing is confirmed on the way: the permission is the decision.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_three_calendar_writes_reach_the_helper_with_their_shapes() -> Result<(), Box<dyn Error>>
{
    use orbit_app_core::{CalendarEventChanges, CalendarEventDraft, CalendarSpan};
    const OCCURRENCE: i64 = 1_792_058_400_000;
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-calendar-writes-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (_daemon, helper_commands) = TestDaemon::start_calendar_observed(directory.path())?;
    let client = DaemonClient::connect(paths, _daemon.child.client.clone());
    wait_for(&client, |snapshot| {
        snapshot.connection.daemon == DaemonConnection::Ready
    })
    .await;

    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Calendar {
                request: orbit_core::calendar_control::CalendarCommand::Save {
                    request_id: "create-1".into(),
                    save: orbit_core::calendar::CalendarSave::Create(Box::new(
                        CalendarEventDraft {
                            calendar_id: None,
                            title: "Café con Ada".into(),
                            start_ms: OCCURRENCE,
                            end_ms: OCCURRENCE + 1_800_000,
                            all_day: false,
                            time_zone: Some("Europe/Madrid".into()),
                            location: Some("Sala 3".into()),
                            notes: None,
                            url: None,
                            availability: None,
                            recurrence: None,
                            alarms_minutes: Some(vec![10]),
                        },
                    )),
                },
            },
        )))
        .await?;
    let create = wait_for_helper_command(&helper_commands, "calendar_save")?;
    assert_eq!(
        create["calendar_save"],
        json!({"create": {"title": "Café con Ada", "start_ms": OCCURRENCE,
            "end_ms": OCCURRENCE + 1_800_000, "all_day": false, "time_zone": "Europe/Madrid",
            "location": "Sala 3", "alarms_minutes": [10]}})
    );

    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Calendar {
                request: orbit_core::calendar_control::CalendarCommand::Save {
                    request_id: "move-1".into(),
                    save: orbit_core::calendar::CalendarSave::Change(Box::new(
                        orbit_core::calendar::CalendarEventChange {
                            calendar_item_id: "fixture-daily".into(),
                            occurrence_start_ms: OCCURRENCE,
                            span: CalendarSpan::This,
                            changes: CalendarEventChanges {
                                calendar_id: None,
                                title: None,
                                start_ms: Some(OCCURRENCE + 3_600_000),
                                end_ms: Some(OCCURRENCE + 7_200_000),
                                all_day: None,
                                time_zone: None,
                                location: None,
                                notes: None,
                                url: None,
                                availability: None,
                                recurrence: None,
                                remove_recurrence: None,
                                alarms_minutes: None,
                            },
                        },
                    )),
                },
            },
        )))
        .await?;
    let update = wait_for_helper_command(&helper_commands, "calendar_save")?;
    assert_eq!(
        update["calendar_save"],
        json!({"change": {"calendar_item_id": "fixture-daily",
            "occurrence_start_ms": OCCURRENCE, "span": "this",
            "changes": {"start_ms": OCCURRENCE + 3_600_000, "end_ms": OCCURRENCE + 7_200_000}}})
    );

    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Calendar {
                request: orbit_core::calendar_control::CalendarCommand::Delete {
                    request_id: "delete-1".into(),
                    delete: orbit_core::calendar::CalendarDelete {
                        calendar_item_id: "fixture-daily".into(),
                        occurrence_start_ms: OCCURRENCE,
                        span: CalendarSpan::Future,
                    },
                },
            },
        )))
        .await?;
    let delete = wait_for_helper_command(&helper_commands, "calendar_delete")?;
    assert_eq!(
        delete["calendar_delete"],
        json!({"calendar_item_id": "fixture-daily", "occurrence_start_ms": OCCURRENCE,
            "span": "future"})
    );
    Ok(())
}

/// A write's outcome comes back to the calendar view through the
/// real daemon and the agenda head — here a save, with the occurrence it
/// left, which is what an undo names later. Nothing is polled by hand.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_saved_write_reaches_the_calendar_view() -> Result<(), Box<dyn Error>> {
    use orbit_app_core::{CalendarEventChanges, CalendarSpan, CalendarWriteStage};
    const OCCURRENCE: i64 = 1_792_058_400_000;
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-calendar-outcome-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (_daemon, helper_commands) = TestDaemon::start_calendar_observed(directory.path())?;
    let client = DaemonClient::connect(paths.clone(), _daemon.child.client.clone());
    wait_for(&client, |snapshot| {
        snapshot.connection.daemon == DaemonConnection::Ready
    })
    .await;
    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Calendar {
                request: orbit_core::calendar_control::CalendarCommand::Save {
                    request_id: "move-outcome".into(),
                    save: orbit_core::calendar::CalendarSave::Change(Box::new(
                        orbit_core::calendar::CalendarEventChange {
                            calendar_item_id: "fixture-daily".into(),
                            occurrence_start_ms: OCCURRENCE,
                            span: CalendarSpan::This,
                            changes: CalendarEventChanges {
                                calendar_id: None,
                                title: None,
                                start_ms: Some(OCCURRENCE + 3_600_000),
                                end_ms: Some(OCCURRENCE + 7_200_000),
                                all_day: None,
                                time_zone: None,
                                location: None,
                                notes: None,
                                url: None,
                                availability: None,
                                recurrence: None,
                                remove_recurrence: None,
                                alarms_minutes: None,
                            },
                        },
                    )),
                },
            },
        )))
        .await?;
    wait_for_helper_command(&helper_commands, "calendar_save")?;
    let saved_write = |snapshot: &orbit_app_core::AppSnapshot| {
        snapshot.calendar.writes.iter().any(|write| {
            write.request_id == "move-outcome"
                && write.stage == CalendarWriteStage::Saved
                && write.saved.as_ref().is_some_and(|saved| {
                    saved.calendar_item_id == "fixture-daily"
                        && saved.occurrence_start_ms == OCCURRENCE
                        && saved.start_ms == 1_792_062_000_000
                })
        })
    };
    wait_for(&client, saved_write).await;
    _daemon.repeat_helper_state().await?;
    drop(client);
    let client = DaemonClient::connect(paths, _daemon.child.client.clone());
    wait_for(&client, saved_write).await;
    Ok(())
}

fn notes_of(snapshot: &orbit_app_core::AppSnapshot) -> Value {
    serde_json::to_value(snapshot)
        .expect("the app snapshot must serialize")
        .pointer("/notes")
        .cloned()
        .expect("the snapshot must carry the notes section")
}

/// Through the real daemon: a recording this Mac made opens and is
/// listed without the hub (unpaired here), as what the Mac knows of it; the
/// list says the hub's part is missing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recording_of_this_mac_opens_and_is_listed_without_the_hub() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-notes-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (_daemon, audio_requested, release_audio) =
        TestDaemon::start_with_audio_gate(directory.path())?;
    let client = DaemonClient::connect(paths, _daemon.child.client.clone());

    wait_for(&client, |snapshot| {
        snapshot.connection.daemon == DaemonConnection::Ready
            && snapshot.permissions.microphone == PermissionState::Granted
    })
    .await;

    let start = serde_json::from_value::<AppCommand>(json!({
        "command": "meeting_start",
        "title": "Fixture de notas",
        "minutes": 1,
        "audio": "system"
    }))?;
    let sending_client = client.clone();
    let send_task = tokio::spawn(async move { sending_client.send(start).await });
    tokio::task::spawn_blocking(move || audio_requested.recv_timeout(AUDIO_START_BUDGET)).await??;
    wait_for(&client, |snapshot| {
        snapshot.meeting.phase != MeetingPhase::Idle
    })
    .await;
    release_audio.send(())?;
    send_task.await??;
    let active = wait_for(&client, |snapshot| {
        snapshot.notes.as_ref().is_some_and(|notes| {
            notes
                .index
                .as_ref()
                .is_some_and(|index| index.state == orbit_app_core::NotesIndexState::NeedsHub)
                && notes
                    .list
                    .first()
                    .is_some_and(|row| row.state == orbit_app_core::NoteListState::Recording)
        })
    })
    .await;
    let notes = active.notes.ok_or("notes absent while recording")?;
    assert!(notes.index.ok_or("notes index absent")?.detail.is_none());
    assert_eq!(notes.list.len(), 1);
    client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::MeetingFinish,
        )))
        .await?;
    wait_for(&client, |snapshot| !snapshot.meeting.recent.is_empty()).await;
    let finished = serde_json::to_value(client.composed_snapshot())?;
    let recording_id = finished["meeting"]["recent"][0]["recording_id"]
        .as_str()
        .expect("the finished recording must have an id")
        .to_owned();

    let opened_result = client
        .send(AppCommand::Note(
            orbit_app_core::features::note::NoteCommand::NotesOpen {
                recording_id: recording_id.clone(),
            },
        ))
        .await;
    assert!(
        opened_result.is_ok(),
        "transcript failure hid the note: {opened_result:?}"
    );

    // The list arrives with the notes topic and the open note may be read
    // again when its revision moves (NOTE-STALE): wait for both, never an
    // instant right after `NotesOpen`. The row may first say «recording»: the
    // notes topic keeps its 1 s cadence, so the head of the finish
    // comes up to a second later. Wait for the state the row must reach, not
    // for whichever list came first (Mac, 2026-10-20: 3 of 20 runs read it).
    let opened = notes_of(
        &wait_for(&client, |snapshot| {
            let notes = notes_of(snapshot);
            notes.pointer("/list/0/recording_id") == Some(&json!(recording_id))
                && notes.pointer("/list/0/state") == Some(&json!("uploading"))
                && notes.pointer("/open_read/recording_id") == Some(&json!(recording_id))
                && notes.pointer("/open_read/state") == Some(&json!("available"))
        })
        .await,
    );
    assert_eq!(
        opened.pointer("/list/0/recording_id"),
        Some(&Value::String(recording_id.clone())),
        "the opened recording leads the notes list"
    );
    assert_eq!(
        opened.pointer("/list/0/state"),
        Some(&Value::String("uploading".into())),
        "without the hub its audio is still on its way"
    );
    // Not paired: the hub cannot be asked, which is not a failed read
    // (seen in a run of the real test on the Mac).
    assert_eq!(opened.pointer("/index/state"), Some(&json!("needs_hub")));
    assert_eq!(opened.pointer("/index/detail"), Some(&Value::Null));
    assert_eq!(
        opened.pointer("/open/recording_id"),
        Some(&json!(recording_id))
    );
    assert_eq!(
        opened.pointer("/open/title"),
        Some(&json!("Fixture de notas"))
    );
    assert_eq!(
        opened.pointer("/open/progress/upload/cause/kind"),
        Some(&json!("uploading"))
    );
    assert!(
        opened
            .pointer("/open/transcript/error")
            .and_then(Value::as_str)
            .is_some_and(|cause| serde_json::from_str::<Value>(cause)
                .is_ok_and(|refusal| refusal["code"] == "not_paired"))
    );

    client
        .send(AppCommand::Notes(
            orbit_app_core::features::notes::NotesCommand::NotesRefresh,
        ))
        .await?;
    let refreshed = notes_of(&client.composed_snapshot());
    assert_eq!(
        refreshed.pointer("/list/0/state"),
        Some(&Value::String("uploading".into())),
        "refreshing never loses the note"
    );
    Ok(())
}
