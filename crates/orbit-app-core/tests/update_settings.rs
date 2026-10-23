#![allow(clippy::expect_used, clippy::unwrap_used)]

use orbit_app_core::{AppCommand, UpdateService, UpdateState};
use orbit_core::refusal::{InstallLogIssue, Refusal, UpdateCause};
use std::{
    error::Error,
    fs,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[path = "support/fake_daemon.rs"]
mod fake_daemon;

fn idle_update_facts() -> (serde_json::Value, serde_json::Value) {
    use serde_json::json;
    let recording = json!({"topic":"recording", "knowledge":{"state":"present","value":{
        "recording":null,"recording_signal":null,"pending_jobs":0,"offline":false,
        "applications":{"state":"present","value":[]}
    }}});
    let assistant = json!({"topic":"assistant", "knowledge":{"state":"present","value":{
        "session_id":null,"turn":null,"pending":null,"voice_active":false,"turn_in_flight":false,
        "activity":null,"voice":{"state":"present"},"call":{"state":"present","value":null}
    }}});
    (recording, assistant)
}

fn reply(mut stream: TcpStream, latest: &str, installer: &str) -> Result<(), Box<dyn Error>> {
    let mut first_line = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut first_line)?;
    let body = if first_line.contains("/latest.txt") {
        latest
    } else if first_line.contains("/install.sh") {
        installer
    } else {
        return Err(format!("unexpected update request: {first_line}").into());
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )?;
    stream.flush()?;
    Ok(())
}

fn update_server(
    latest: String,
    installer: String,
    requests: usize,
) -> Result<(String, thread::JoinHandle<()>), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let handle = thread::spawn(move || {
        for stream in listener.incoming().take(requests) {
            let stream = stream.expect("update fixture connection");
            reply(stream, &latest, &installer).expect("update fixture response");
        }
    });
    Ok((format!("http://{address}"), handle))
}

fn wait_for_file(path: &Path) -> Result<String, Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match fs::read_to_string(path) {
            Ok(value) => return Ok(value),
            Err(error) if Instant::now() < deadline => {
                let _ = error;
                thread::yield_now();
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[tokio::test]
async fn checks_a_bounded_manifest_and_installs_with_relaunch_before_quitting()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-app-update-")
        .tempdir_in("/tmp")?;
    let invocation = directory.path().join("installer-arguments.txt");
    let major = env!("CARGO_PKG_VERSION")
        .split('.')
        .next()
        .ok_or("packaged version has no major component")?
        .parse::<u64>()?;
    let version = format!("{}.0.0", major + 1);
    let latest = format!(
        "version={version}\narm64_sha256={digest}\nx86_64_sha256={digest}\n",
        digest = "b".repeat(64)
    );
    let installer = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$ORBIT_INSTALL_READY_SIGNAL\"\nexec >/dev/null\nprintf '%70000s' ''\nprintf '%s' \"$1\" > '{}.tmp'\nmv '{}.tmp' '{}'\n",
        invocation.display(),
        invocation.display(),
        invocation.display()
    );
    let (origin, server) = update_server(latest, installer, 3)?;
    let service = UpdateService::new(origin);

    let available = service.check_configured().await;
    assert_eq!(available, UpdateState::Available { version });

    let (recording, assistant) = idle_update_facts();
    let mut channels = orbit_ipc::TopicChannels::new();
    for data in [recording, assistant] {
        let frame: orbit_core::topics::TopicFrame = serde_json::from_value(serde_json::json!({
            "epoch":"update-install-fixture", "revision":1, "data":data
        }))?;
        let (sender, _) = tokio::sync::watch::channel(frame.clone());
        channels.insert(frame.id(), sender);
    }
    let (calls, receiver) = tokio::sync::mpsc::channel(1);
    drop(receiver);
    let endpoint = fake_daemon::channel(calls, channels);
    let client = orbit_app_core::DaemonClient::connect(
        orbit_ipc::Paths::resolve(Some(directory.path().join("data")))?,
        endpoint.client.clone(),
    );
    let mut frames = client.watch_frames();
    while client.update_installation_blocker().await.is_some() {
        frames.changed().await?;
    }
    let outcome = service
        .install_when_idle(&client)
        .await?
        .ok_or("idle installation did not start")?;
    assert_eq!(wait_for_file(&invocation)?, "--relaunch");
    assert_eq!(
        outcome.next_command,
        AppCommand::Status(orbit_app_core::features::status::StatusCommand::Quit)
    );
    server.join().map_err(|_| "update fixture panicked")?;
    Ok(())
}

#[tokio::test]
async fn check_explains_an_invalid_manifest() -> Result<(), Box<dyn Error>> {
    let (origin, server) = update_server("version=not-semver\n".into(), "exit 0\n".into(), 2)?;
    let service = UpdateService::new(origin);
    let state = service.check_configured().await;
    assert!(
        matches!(state, UpdateState::Refused { refusal } if refusal.to_string().contains("not-semver")),
        "invalid version lost its value"
    );
    // The fixture serves the second manifest request before joining.
    assert!(service.install().await.is_err());
    server.join().map_err(|_| "update fixture panicked")?;
    Ok(())
}

#[tokio::test]
async fn installer_ceiling_names_file_and_numbers() -> Result<(), Box<dyn Error>> {
    let (origin, server) = update_server("version=9.0.0\n".into(), "#".repeat(16_757), 3)?;
    let service = UpdateService::new(origin);
    assert!(matches!(
        service.check_configured().await,
        UpdateState::Available { .. }
    ));
    let error = service
        .install()
        .await
        .expect_err("oversized installer must fail");
    assert!(error.message.contains("install.sh"), "{}", error.message);
    assert!(error.message.contains("16757"), "{}", error.message);
    assert!(error.message.contains("16756"), "{}", error.message);
    server.join().map_err(|_| "update fixture panicked")?;
    Ok(())
}

#[tokio::test]
async fn late_installer_failure_keeps_app_running() -> Result<(), Box<dyn Error>> {
    let (origin, server) = update_server("version=9.0.0\n".into(), "sleep 1\nexit 7\n".into(), 3)?;
    let service = UpdateService::new(origin);
    assert!(matches!(
        service.check_configured().await,
        UpdateState::Available { .. }
    ));
    let error = service
        .install()
        .await
        .expect_err("installer failed before announcing readiness");
    assert!(error.message.contains('7'), "{}", error.message);
    server.join().map_err(|_| "update fixture panicked")?;
    Ok(())
}

#[tokio::test]
async fn app_refuses_a_different_installer_before_running_it() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let receipt = directory.path().join("ran");
    let expected = "a".repeat(64);
    let latest = format!("version=9.0.0\ninstaller_sha256={expected}\n");
    let installer = format!("#!/bin/sh\nprintf ran > '{}'\n", receipt.display());
    let (origin, server) = update_server(latest, installer, 2)?;
    let error = UpdateService::new(origin)
        .install()
        .await
        .expect_err("mismatched installer must be refused");
    assert!(error.message.contains(&expected), "{}", error.message);
    let wire: serde_json::Value = serde_json::from_str(error.message.split_once(": ").unwrap().1)?;
    assert_eq!(wire["cause"]["kind"], "installer_digest_mismatch");
    let actual = wire["cause"]["actual"]
        .as_str()
        .ok_or("missing observed digest")?;
    assert_eq!(actual.len(), 64);
    assert!(actual.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_ne!(actual, expected);
    assert!(!receipt.exists(), "mismatched installer ran");
    server.join().map_err(|_| "update fixture panicked")?;
    Ok(())
}

#[test]
fn reads_last_result_and_legacy_0312_record() -> Result<(), Box<dyn Error>> {
    let home = tempfile::tempdir()?;
    let applications = home.path().join("Applications");
    fs::create_dir_all(&applications)?;
    fs::write(
        applications.join(".orbit-instalado"),
        "version=0.3.12\nsha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\narchitecture=arm64\ninstalled=2026-09-25T07:36:53Z\norigin=fixture\n",
    )?;
    let legacy =
        UpdateService::last_installation_at(home.path())?.ok_or("missing legacy record")?;
    assert_eq!(legacy.version.as_deref(), Some("0.3.12"));
    assert_eq!(legacy.when, "2026-09-25T07:36:53Z");
    assert_eq!(legacy.failure, None);

    let logs = home.path().join("Library/Logs/Orbit");
    fs::create_dir_all(&logs)?;
    fs::write(
        logs.join("instalador.log"),
        "Inicio 2026-10-01T09:00:00Z\nresultado|0.3.14|2026-10-01T09:01:00Z|fallo|checksum distinto\n",
    )?;
    let result = UpdateService::last_installation_at(home.path())?.ok_or("missing new result")?;
    assert_eq!(result.version.as_deref(), Some("0.3.14"));
    assert_eq!(result.failure.as_deref(), Some("checksum distinto"));
    assert!(
        result
            .log_path
            .as_deref()
            .is_some_and(|path| path.ends_with("instalador.log"))
    );
    fs::write(
        logs.join("instalador.log"),
        "resultado|0.3.14|2026-10-01T09:01:00Z|bien|instalado; el manifiesto no traía installer_sha256\n",
    )?;
    let unverified =
        UpdateService::last_installation_at(home.path())?.ok_or("missing unverified result")?;
    assert_eq!(
        unverified.note.as_deref(),
        Some("instalado; el manifiesto no traía installer_sha256")
    );
    let incomplete = "Inicio 2026-10-01T09:00:00Z\n";
    fs::write(logs.join("instalador.log"), incomplete)?;
    let error = UpdateService::last_installation_at(home.path()).unwrap_err();
    assert!(
        matches!(
            &error,
            Refusal::Update {
                cause: UpdateCause::InstallLog {
                    bytes: Some(bytes),
                    max_bytes: 131_072,
                    reason: InstallLogIssue::MissingResult {
                        started_at: Some(started_at),
                    },
                }
            } if *bytes == incomplete.len() as u64 && started_at == "2026-10-01T09:00:00Z"
        ),
        "{error:?}"
    );
    fs::write(logs.join("instalador.log"), vec![b'x'; 131_073])?;
    let error = UpdateService::last_installation_at(home.path()).unwrap_err();
    assert!(
        matches!(
            &error,
            Refusal::Update {
                cause: UpdateCause::InstallLog {
                    bytes: Some(131_073),
                    max_bytes: 131_072,
                    reason: InstallLogIssue::TooLarge,
                }
            }
        ),
        "{error:?}"
    );
    Ok(())
}

#[test]
fn latest_start_without_result_does_not_reuse_an_older_success() -> Result<(), Box<dyn Error>> {
    let home = tempfile::tempdir()?;
    let logs = home.path().join("Library/Logs/Orbit");
    fs::create_dir_all(&logs)?;
    fs::write(
        logs.join("instalador.log"),
        "Inicio 2026-10-01T09:00:00Z\nresultado|0.3.14|2026-10-01T09:01:00Z|bien|instalado\nInicio 2026-10-01T10:00:00Z\n",
    )?;
    let error = UpdateService::last_installation_at(home.path()).unwrap_err();
    assert!(
        error.to_string().contains("2026-10-01T10:00:00Z"),
        "{error}"
    );
    assert!(error.to_string().contains("missing_result"), "{error}");
    Ok(())
}

#[test]
fn parser_reads_a_log_written_by_the_real_installer() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    let home = tempfile::tempdir()?;
    let bin = home.path().join("bin");
    fs::create_dir_all(&bin)?;
    let shims = [
        (
            "uname",
            "case \"$1\" in -m) echo arm64;; *) echo Darwin;; esac\n",
        ),
        ("ps", "printf '%s %s %s\\n' \"$$\" \"$PPID\" /bin/bash\n"),
        (
            "curl",
            "out=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = -o ]; then shift; out=$1; fi\n  shift\ndone\ncase \"$out\" in */latest.txt) printf 'version=0.1.0\\narm64_file=Orbit.zip\\narm64_sha256=%064d\\n' 0 > \"$out\";; *) printf archive > \"$out\";; esac\n",
        ),
        ("shasum", "printf '%064d  Orbit.zip\\n' 0\n"),
        (
            "unzip",
            "while [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = -d ]; then shift; out=$1; fi\n  shift\ndone\nmkdir -p \"$out/Orbit.app/Contents/MacOS\"\nprintf '#!/bin/sh\\nexit 0\\n' > \"$out/Orbit.app/Contents/MacOS/Orbit\"\nchmod +x \"$out/Orbit.app/Contents/MacOS/Orbit\"\n",
        ),
    ];
    for (name, body) in shims {
        let path = bin.join(name);
        fs::write(&path, format!("#!/bin/bash\n{body}"))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    let path = format!("{}:{}", bin.display(), std::env::var("PATH")?);
    let output = Command::new("bash")
        .arg("--posix")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/release/install.sh"
        ))
        .env("HOME", home.path())
        .env(
            "ORBIT_APPLICATIONS_DIR",
            home.path().join("system-applications"),
        )
        .env("PATH", path)
        .env("ORBIT_INSTALL_BASE", "fixture.invalid")
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log = fs::read_to_string(home.path().join("Library/Logs/Orbit/instalador.log"))?;
    assert!(log.contains("Inicio "));
    assert!(log.contains("resultado|0.1.0|"));
    let parsed = UpdateService::last_installation_at(home.path())?.ok_or("missing result")?;
    assert_eq!(parsed.version.as_deref(), Some("0.1.0"));
    assert_eq!(parsed.failure, None);
    assert!(
        parsed
            .log_path
            .as_deref()
            .is_some_and(|path| path.ends_with("instalador.log"))
    );
    Ok(())
}

#[tokio::test]
async fn packaged_update_version_is_up_to_date() -> Result<(), Box<dyn Error>> {
    let latest = format!(
        "version={}\narm64_sha256={digest}\nx86_64_sha256={digest}\n",
        env!("CARGO_PKG_VERSION"),
        digest = "b".repeat(64)
    );
    let (origin, server) = update_server(latest, String::new(), 1)?;
    assert_eq!(UpdateService::check(&origin).await, UpdateState::UpToDate);
    server.join().map_err(|_| "update fixture panicked")?;
    Ok(())
}

#[test]
fn reads_and_migrates_the_receipt_to_orbit_data() -> Result<(), Box<dyn Error>> {
    let home = tempfile::tempdir()?;
    let legacy = home.path().join("Applications/.orbit-instalado");
    let record = home
        .path()
        .join(orbit_core::orbit_bundle::INSTALL_DATA_DIRECTORY)
        .join(orbit_core::orbit_bundle::INSTALL_RECORD);
    fs::create_dir_all(legacy.parent().ok_or("legacy parent")?)?;
    fs::write(&legacy, "version=0.7.0\ninstalled=2026-10-20T10:00:00Z\n")?;
    assert_eq!(
        UpdateService::last_installation_at(home.path())?
            .ok_or("legacy receipt")?
            .version
            .as_deref(),
        Some("0.7.0")
    );
    assert!(
        record.is_file(),
        "legacy receipt was not moved to the data directory"
    );
    assert!(!legacy.exists(), "legacy receipt must be consumed once");
    fs::write(&legacy, "version=0.1.0\ninstalled=old\n")?;
    assert_eq!(
        UpdateService::last_installation_at(home.path())?
            .ok_or("current receipt")?
            .version
            .as_deref(),
        Some("0.7.0")
    );
    Ok(())
}

#[tokio::test]
async fn automatic_check_reports_new_versions_but_keeps_failures_quiet()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let log = orbit_ipc::report::LogFile::new(
        directory.path().join("updates.log"),
        orbit_ipc::report::MAX_BYTES,
    );
    let (origin, server) = update_server("version=9.0.0\n".into(), String::new(), 1)?;
    assert_eq!(
        UpdateService::new(origin).check_automatically(&log).await,
        Some(UpdateState::Available {
            version: "9.0.0".into()
        })
    );
    server.join().map_err(|_| "server panicked")?;
    let (origin, server) = update_server("version=not-a-version\n".into(), String::new(), 2)?;
    let service = UpdateService::new(origin);
    assert_eq!(service.check_automatically(&log).await, None);
    assert_eq!(service.check_automatically(&log).await, None);
    server.join().map_err(|_| "server panicked")?;
    assert!(fs::read_to_string(log.path())?.contains("not-a-version"));
    Ok(())
}

#[test]
fn installation_waits_for_recording_call_and_known_state() -> Result<(), Box<dyn Error>> {
    use orbit_app_core::topics::TopicCache;
    use serde_json::json;
    let mut topics = TopicCache::default();
    assert!(
        UpdateService::installation_blocker(&topics).is_some(),
        "unknown is not idle"
    );
    let (mut recording, mut assistant) = idle_update_facts();
    let observe = |topics: &mut TopicCache,
                   data: &serde_json::Value,
                   revision|
     -> Result<(), Box<dyn Error>> {
        topics.observe_at(
            serde_json::from_value(json!({"epoch":"update","revision":revision,"data":data}))?,
            Some(1),
        );
        Ok(())
    };
    observe(&mut topics, &recording, 1)?;
    observe(&mut topics, &assistant, 1)?;
    assert_eq!(UpdateService::installation_blocker(&topics), None);
    recording["knowledge"]["value"]["recording"] = serde_json::to_value(orbit_core::Recording {
        id: "fixture".into(),
        title: "Prueba".into(),
        started_at_ms: 1,
        ended_at_ms: None,
        expires_at_ms: 2,
        application_pid: None,
        audio: orbit_core::protocol::MeetingAudio::System,
        phase: orbit_core::CapturePhase::Recording,
        sealed_chunks: 0,
        gaps: 0,
        tracks: vec![],
        finalizing_since_ms: None,
        ceiling_notice_given: false,
        muted_spans: Vec::new(),
    })?;
    observe(&mut topics, &recording, 2)?;
    assert!(
        UpdateService::installation_blocker(&topics)
            .is_some_and(|reason| reason.contains("grabación"))
    );
    recording["knowledge"]["value"]["recording"] = json!(null);
    observe(&mut topics, &recording, 3)?;
    assistant["knowledge"]["value"]["call"]["value"] =
        json!({"started_at_ms":1,"phase":"listening","id":"fixture","expires_at_ms":2,"turns":0});
    observe(&mut topics, &assistant, 2)?;
    assert!(
        UpdateService::installation_blocker(&topics)
            .is_some_and(|reason| reason.contains("llamada"))
    );
    assistant["knowledge"]["value"]["call"]["value"] = json!(null);
    observe(&mut topics, &assistant, 3)?;
    assert_eq!(UpdateService::installation_blocker(&topics), None);
    assistant["knowledge"]["value"]["call"] =
        json!({"state":"unknown","why":{"reading":{"since_ms":123}}});
    observe(&mut topics, &assistant, 4)?;
    let reason = UpdateService::installation_blocker(&topics).ok_or("missing pending cause")?;
    assert!(reason.contains("leyendo"), "{reason}");
    assert!(reason.contains("123"), "{reason}");
    topics.ended("fixture disconnected".into());
    assert!(
        UpdateService::installation_blocker(&topics).is_some(),
        "stale is not idle"
    );
    Ok(())
}

/// Only a live capture holds an update back: one that is being recorded,
/// starting or still draining. A recording a cut left interrupted, a paused
/// one or a closed one is not lost by restarting Orbit, and once blocked the
/// update for as long as it stayed in the state.
#[test]
fn only_a_live_capture_holds_the_update() -> Result<(), Box<dyn Error>> {
    use orbit_app_core::topics::TopicCache;
    use orbit_core::CapturePhase;
    use serde_json::json;
    let (mut recording, assistant) = idle_update_facts();
    let mut topics = TopicCache::default();
    let mut revision = 0;
    let mut observe =
        |topics: &mut TopicCache, data: &serde_json::Value| -> Result<(), Box<dyn Error>> {
            revision += 1;
            topics.observe_at(
                serde_json::from_value(json!({"epoch":"update","revision":revision,"data":data}))?,
                Some(1),
            );
            Ok(())
        };
    observe(&mut topics, &assistant)?;
    for (phase, ended_at_ms, blocks) in [
        (CapturePhase::Starting, None, true),
        (CapturePhase::Recording, None, true),
        (CapturePhase::Finalizing, None, true),
        (CapturePhase::Interrupted, None, false),
        (CapturePhase::Paused, None, false),
        (CapturePhase::Idle, Some(2), false),
        (CapturePhase::Interrupted, Some(2), false),
    ] {
        recording["knowledge"]["value"]["recording"] =
            serde_json::to_value(orbit_core::Recording {
                id: "reunion-inventada".into(),
                title: "Prueba".into(),
                started_at_ms: 1,
                ended_at_ms,
                expires_at_ms: 2,
                application_pid: None,
                audio: orbit_core::protocol::MeetingAudio::System,
                phase: phase.clone(),
                sealed_chunks: 3,
                gaps: 0,
                tracks: vec![],
                finalizing_since_ms: None,
                ceiling_notice_given: false,
                muted_spans: Vec::new(),
            })?;
        observe(&mut topics, &recording)?;
        let blocker = UpdateService::installation_blocker(&topics);
        assert_eq!(blocker.is_some(), blocks, "{phase:?}: {blocker:?}");
        if blocks {
            assert!(blocker.is_some_and(|reason| reason.contains("grabación")));
        }
    }
    Ok(())
}

#[tokio::test]
async fn no_update_origin_disables_checks_without_writing_failure_logs()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let log = orbit_ipc::report::LogFile::new(
        directory.path().join("updates.log"),
        orbit_ipc::report::MAX_BYTES,
    );
    let service = UpdateService::new("");
    let state = service.check_configured().await;
    assert!(
        !matches!(state, UpdateState::Refused { .. }),
        "missing build configuration is not a failed check"
    );
    let (calls, receiver) = tokio::sync::mpsc::channel(1);
    drop(receiver);
    let endpoint = fake_daemon::channel(calls, orbit_ipc::TopicChannels::new());
    let client = orbit_app_core::DaemonClient::connect(
        orbit_ipc::Paths::resolve(Some(directory.path().join("data")))?,
        endpoint.client.clone(),
    );
    let snapshot = client.set_update_result(state);
    assert_eq!(
        serde_json::to_value(snapshot.settings.update)?["state"],
        "disabled"
    );
    assert!(service.check_automatically(&log).await.is_some());
    assert!(!log.path().exists(), "disabled checks are not failures");
    Ok(())
}

#[test]
fn update_origin_is_fixed_by_the_build_environment() {
    assert_eq!(
        orbit_app_core::DEFAULT_UPDATE_BASE_URL,
        option_env!("ORBIT_UPDATE_BASE_URL").unwrap_or("")
    );
}
