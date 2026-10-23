//! The composer's files, through the daemon's own paths: a
//! synthetic hub channel for `ready` and `chat.send`, and a loopback HTTP hub
//! that records each attachment request. The files are invented.
use super::*;
use crate::{daemon::audio::tests::fixture, hub::HubEvent};
use base64::Engine;
use orbit_core::attachments::{Attachment, AttachmentKind, AttachmentSource};
use orbit_core::protocol::Command;
use orbit_core::refusal::{AttachmentCause, AttachmentCeiling, Refusal};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

const DEVICE: &str = "device-attachments";
const PICTURE: &str = "40000000-0000-4000-8000-000000000001";
const NOTES: &str = "40000000-0000-4000-8000-000000000002";

fn limits(max_file_bytes: u64, max_message_files: u64) -> Value {
    json!({"max_file_bytes": max_file_bytes, "max_message_files": max_message_files,
        "max_message_bytes": 1_000_000, "max_session_bytes": 10_000_000})
}

/// A hub's `ready`: with its attachment ceilings, or without them as 0.9.0.
fn ready(attachments: Option<Value>) -> Value {
    let mut data = json!({"device_id": DEVICE, "hub_protocol": 1, "assistant_name": "Deimos"});
    if let Some(attachments) = attachments {
        data["attachments"] = attachments;
    }
    json!({"event": "ready", "data": data})
}

async fn connected(
    attachments: Option<Value>,
) -> Result<(
    tempfile::TempDir,
    Runtime,
    mpsc::Receiver<Value>,
    mpsc::Receiver<Value>,
)> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, helper) = fixture(directory.path())?;
    let (hub, frames) = mpsc::channel(16);
    runtime.hub.install(hub);
    runtime.settings.device_id = Some(DEVICE.into());
    runtime.token = Some("synthetic-device-token".into());
    runtime
        .hub_event(HubEvent::Frame(ready(attachments)))
        .await?;
    Ok((directory, runtime, frames, helper))
}

fn file(id: &str, name: &str, bytes: u64) -> Attachment {
    Attachment {
        id: id.into(),
        name: name.into(),
        kind: AttachmentKind::Image,
        mime: "image/png".into(),
        bytes,
        pages: None,
    }
}

fn chat(text: &str, attachments: Vec<Attachment>) -> Command {
    Command::Chat {
        text: text.into(),
        attachments,
        primary: false,
    }
}

fn refusal_of(error: &anyhow::Error) -> Option<Refusal> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<Refusal>().cloned())
}

#[tokio::test]
async fn chat_send_names_the_files_of_a_hub_that_takes_them() -> Result<()> {
    let (_directory, mut runtime, mut frames, _helper) = connected(Some(limits(1_000, 10))).await?;
    let sent = vec![file(PICTURE, "mapa.png", 120), file(NOTES, "ruta.txt", 30)];
    runtime
        .command(chat("¿Qué ruta marca?", sent.clone()))
        .await?;
    let frame = frames.try_recv().context("no chat.send")?;
    assert_eq!(frame["method"], "chat.send");
    assert_eq!(frame["params"]["attachments"], json!([PICTURE, NOTES]));
    // The open turn shows the files before the hub's page is read again.
    let turn = runtime.state.durable.turn.as_ref().context("no turn")?;
    assert_eq!(turn.user.attachments, sent);
    Ok(())
}

#[tokio::test]
async fn a_file_alone_is_a_turn() -> Result<()> {
    let (_directory, mut runtime, mut frames, _helper) = connected(Some(limits(1_000, 10))).await?;
    runtime
        .command(chat("", vec![file(PICTURE, "mapa.png", 120)]))
        .await?;
    let frame = frames.try_recv().context("no chat.send")?;
    assert_eq!(frame["params"]["text"], "");
    assert_eq!(frame["params"]["attachments"], json!([PICTURE]));
    Ok(())
}

/// A 0.9.0 hub ignores fields it does not know: a turn with files would
/// reach it without them. The Mac refuses before anything leaves, and a turn
/// without files carries no `attachments` field at all.
#[tokio::test]
async fn a_0_9_0_hub_is_never_sent_attachments() -> Result<()> {
    let (_directory, mut runtime, mut frames, _helper) = connected(None).await?;
    let refused = runtime
        .command(chat("Mira", vec![file(PICTURE, "mapa.png", 120)]))
        .await
        .err()
        .context("a turn with files went to a 0.9.0 hub")?;
    assert_eq!(
        refusal_of(&refused),
        Some(Refusal::Attachment {
            cause: AttachmentCause::Unsupported
        })
    );
    assert!(
        frames.try_recv().is_err(),
        "a frame left for a refused turn"
    );
    runtime.command(chat("Hola", Vec::new())).await?;
    let frame = frames.try_recv().context("no chat.send")?;
    assert!(frame["params"].get("attachments").is_none(), "{frame}");
    Ok(())
}

#[tokio::test]
async fn more_files_than_the_hub_takes_are_refused_before_sending() -> Result<()> {
    let (_directory, mut runtime, mut frames, _helper) = connected(Some(limits(1_000, 1))).await?;
    let refused = runtime
        .command(chat(
            "Las dos",
            vec![file(PICTURE, "mapa.png", 1), file(NOTES, "ruta.txt", 1)],
        ))
        .await
        .err()
        .context("two files went past a ceiling of one")?;
    assert_eq!(
        refusal_of(&refused),
        Some(Refusal::Attachment {
            cause: AttachmentCause::TooMany {
                files: 2,
                max_files: 1
            }
        })
    );
    assert!(frames.try_recv().is_err());
    Ok(())
}

// The loopback hub ------------------------------------------------------------

#[derive(Debug, Clone)]
pub(in crate::daemon) struct Request {
    pub(in crate::daemon) method: String,
    pub(in crate::daemon) path: String,
    pub(in crate::daemon) headers: std::collections::BTreeMap<String, String>,
    pub(in crate::daemon) body: Vec<u8>,
}

pub(in crate::daemon) type Recorded = Arc<Mutex<Vec<Request>>>;

/// A hub on a loopback port that records each request and answers each with
/// `(status, content type, extra headers, body)`.
pub(in crate::daemon) async fn http_hub(
    answer: impl Fn(&Request) -> (u16, &'static str, Vec<(&'static str, String)>, Vec<u8>)
    + Send
    + Sync
    + 'static,
) -> Result<(String, Recorded)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let recorded: Recorded = Arc::default();
    let seen = recorded.clone();
    let answer = Arc::new(answer);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let seen = seen.clone();
            let answer = answer.clone();
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0_u8; 8192];
                let head_end = loop {
                    let Ok(read) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
                let mut lines = head.split("\r\n");
                let mut start = lines.next().unwrap_or_default().split(' ');
                let method = start.next().unwrap_or_default().to_owned();
                let path = start.next().unwrap_or_default().to_owned();
                let headers: std::collections::BTreeMap<String, String> = lines
                    .filter_map(|line| line.split_once(": "))
                    .map(|(key, value)| (key.to_ascii_lowercase(), value.to_owned()))
                    .collect();
                let length: usize = headers
                    .get("content-length")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                let mut body = buffer[head_end..].to_vec();
                while body.len() < length {
                    let Ok(read) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        break;
                    }
                    body.extend_from_slice(&chunk[..read]);
                }
                let request = Request {
                    method,
                    path,
                    headers,
                    body,
                };
                let (status, content_type, extra, payload) = answer(&request);
                if let Ok(mut seen) = seen.lock() {
                    seen.push(request);
                }
                let mut response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n",
                    payload.len()
                );
                for (key, value) in extra {
                    response.push_str(&format!("{key}: {value}\r\n"));
                }
                response.push_str("\r\n");
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.write_all(&payload).await;
            });
        }
    });
    Ok((url, recorded))
}

fn hub_attachment(name: &str, bytes: usize) -> Vec<u8> {
    json!({"attachment": {"id": PICTURE, "name": name, "kind": "text",
        "mime": "text/plain", "bytes": bytes}})
    .to_string()
    .into_bytes()
}

async fn order(runtime: &Runtime, command: Command) -> orbit_core::protocol::AttachmentResponse {
    runtime
        .attachment_context()
        .answer(command, "order-1".into())
        .await
}

#[tokio::test]
async fn a_file_is_read_by_its_path_and_uploaded_with_its_name_and_digest() -> Result<()> {
    let (directory, mut runtime, _frames, _helper) = connected(Some(limits(1_000, 10))).await?;
    let content = "Lista para la excursión: botas y agua.\n";
    let path = directory.path().join("lista de la excursión.txt");
    std::fs::write(&path, content)?;
    let (url, recorded) =
        http_hub(move |_| (200, "application/json", Vec::new(), hub_attachment("x", 1))).await?;
    runtime.settings.hub_url = Some(url);
    let answered = order(
        &runtime,
        Command::AttachmentAdd {
            source: AttachmentSource::Path {
                path: path.to_string_lossy().into_owned(),
            },
        },
    )
    .await;
    assert!(answered.ok, "{:?}", answered.refusal);
    assert_eq!(
        answered.attachment.map(|attachment| attachment.id),
        Some(PICTURE.to_owned())
    );
    let requests = recorded.lock().map(|seen| seen.clone()).unwrap_or_default();
    let [request] = requests.as_slice() else {
        bail!("expected one upload, got {requests:?}");
    };
    assert_eq!(
        (request.method.as_str(), request.path.as_str()),
        ("POST", "/v1/attachments")
    );
    assert_eq!(request.body, content.as_bytes());
    assert_eq!(
        request
            .headers
            .get("x-orbit-attachment-name")
            .map(String::as_str),
        Some("lista%20de%20la%20excursi%C3%B3n.txt")
    );
    assert_eq!(
        request.headers.get("x-orbit-sha256"),
        Some(&format!(
            "{:x}",
            <sha2::Sha256 as sha2::Digest>::digest(content)
        ))
    );
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer synthetic-device-token")
    );
    Ok(())
}

#[tokio::test]
async fn a_pasted_image_is_uploaded_from_its_bytes() -> Result<()> {
    let (_directory, mut runtime, _frames, _helper) = connected(Some(limits(1_000, 10))).await?;
    let (url, recorded) =
        http_hub(move |_| (200, "application/json", Vec::new(), hub_attachment("x", 1))).await?;
    runtime.settings.hub_url = Some(url);
    let pixel = b"\x89PNG\r\n\x1a\nsynthetic";
    let answered = order(
        &runtime,
        Command::AttachmentAdd {
            source: AttachmentSource::Bytes {
                name: "Imagen pegada.png".into(),
                data_base64: base64::engine::general_purpose::STANDARD.encode(pixel),
            },
        },
    )
    .await;
    assert!(answered.ok, "{:?}", answered.refusal);
    let requests = recorded.lock().map(|seen| seen.clone()).unwrap_or_default();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, pixel);
    Ok(())
}

#[tokio::test]
async fn a_file_over_the_ceiling_is_refused_before_it_is_read() -> Result<()> {
    let (directory, mut runtime, _frames, _helper) = connected(Some(limits(4, 10))).await?;
    let path = directory.path().join("grande.txt");
    std::fs::write(&path, "cinco")?;
    let (url, recorded) =
        http_hub(move |_| (200, "application/json", Vec::new(), hub_attachment("x", 1))).await?;
    runtime.settings.hub_url = Some(url);
    let answered = order(
        &runtime,
        Command::AttachmentAdd {
            source: AttachmentSource::Path {
                path: path.to_string_lossy().into_owned(),
            },
        },
    )
    .await;
    assert_eq!(
        answered.refusal,
        Some(Refusal::Attachment {
            cause: AttachmentCause::TooLarge {
                ceiling: AttachmentCeiling::File,
                name: Some("grande.txt".into()),
                files: None,
                bytes: 5,
                session_bytes: None,
                max_bytes: 4,
            }
        })
    );
    assert!(recorded.lock().map(|seen| seen.is_empty()).unwrap_or(false));
    Ok(())
}

#[tokio::test]
async fn a_path_that_is_not_there_says_so_by_name() -> Result<()> {
    let (directory, runtime, _frames, _helper) = connected(Some(limits(1_000, 10))).await?;
    let answered = order(
        &runtime,
        Command::AttachmentAdd {
            source: AttachmentSource::Path {
                path: directory
                    .path()
                    .join("no-existe.pdf")
                    .to_string_lossy()
                    .into_owned(),
            },
        },
    )
    .await;
    assert_eq!(
        answered.refusal,
        Some(Refusal::Attachment {
            cause: AttachmentCause::Unreadable {
                name: Some("no-existe.pdf".into()),
                reason: "missing".into(),
            }
        })
    );
    Ok(())
}

#[tokio::test]
async fn a_hub_without_attachments_is_not_asked() -> Result<()> {
    let (directory, runtime, _frames, _helper) = connected(None).await?;
    let path = directory.path().join("nota.txt");
    std::fs::write(&path, "nota")?;
    let answered = order(
        &runtime,
        Command::AttachmentAdd {
            source: AttachmentSource::Path {
                path: path.to_string_lossy().into_owned(),
            },
        },
    )
    .await;
    assert_eq!(
        answered.refusal,
        Some(Refusal::Attachment {
            cause: AttachmentCause::Unsupported
        })
    );
    Ok(())
}

#[tokio::test]
async fn the_hub_s_refusal_keeps_the_file_s_name_and_type() -> Result<()> {
    let (directory, mut runtime, _frames, _helper) = connected(Some(limits(1_000, 10))).await?;
    let path = directory.path().join("fotos.zip");
    std::fs::write(&path, b"PK\x03\x04synthetic")?;
    let (url, _recorded) = http_hub(move |_| {
        let body = json!({"code": "attachment_type_unsupported", "message": "«fotos.zip» es application/zip",
            "refusal": {"code": "attachment_type_unsupported", "name": "fotos.zip",
                "mime": "application/zip", "retry": {"kind": "after_fix"}}});
        (415, "application/json", Vec::new(), body.to_string().into_bytes())
    })
    .await?;
    runtime.settings.hub_url = Some(url);
    let answered = order(
        &runtime,
        Command::AttachmentAdd {
            source: AttachmentSource::Path {
                path: path.to_string_lossy().into_owned(),
            },
        },
    )
    .await;
    assert_eq!(
        answered.refusal,
        Some(Refusal::Attachment {
            cause: AttachmentCause::TypeUnsupported {
                name: "fotos.zip".into(),
                mime: "application/zip".into(),
            }
        })
    );
    Ok(())
}

#[tokio::test]
async fn an_attachment_reads_back_and_is_written_for_opening() -> Result<()> {
    let (directory, mut runtime, _frames, _helper) = connected(Some(limits(1_000, 10))).await?;
    let pdf = b"%PDF-1.4 synthetic".to_vec();
    let served = pdf.clone();
    let (url, recorded) = http_hub(move |_| {
        (
            200,
            "application/pdf",
            vec![("x-orbit-attachment-name", "ruta%20del%20lago.pdf".into())],
            served.clone(),
        )
    })
    .await?;
    runtime.settings.hub_url = Some(url);
    let fetched = order(
        &runtime,
        Command::AttachmentFetch {
            attachment_id: PICTURE.into(),
        },
    )
    .await;
    let content = fetched.content.context("no content")?;
    assert_eq!(
        (content.name.as_str(), content.mime.as_str()),
        ("ruta del lago.pdf", "application/pdf")
    );
    assert_eq!(content.bytes, pdf);
    let saved = order(
        &runtime,
        Command::AttachmentSave {
            attachment_id: PICTURE.into(),
        },
    )
    .await;
    let path = PathBuf::from(saved.path.context("no path")?);
    assert_eq!(
        path,
        directory
            .path()
            .join("attachments")
            .join(PICTURE)
            .join("ruta del lago.pdf")
    );
    assert_eq!(std::fs::read(&path)?, pdf);
    let requests = recorded.lock().map(|seen| seen.clone()).unwrap_or_default();
    assert!(
        requests.iter().all(|request| request.method == "GET"
            && request.path == format!("/v1/attachments/{PICTURE}"))
    );
    Ok(())
}

// «Adjuntar ventana» -----------------------------------------------------------

const ORBIT_TOKEN: &str = "40000000-0000-4000-8000-0000000000a1";
const EDITOR_TOKEN: &str = "40000000-0000-4000-8000-0000000000a2";

fn window(token: &str, window_id: u64, application: &str, bundle: &str, title: &str) -> Value {
    json!({"binding":{"native_instance_id":"00000000-0000-4000-8000-000000000001",
        "privacy_generation":2,"inventory_id":"00000000-0000-4000-8000-000000000002",
        "selection_token":token,"window_id":window_id,"application_pid":window_id + 1000,
        "application_instance_id":"00000000-0000-4000-8000-000000000004",
        "binding_quality":"observed_owner_and_window","descriptor_hash":"a".repeat(64)},
        "window_id":window_id,"application_pid":window_id + 1000,"application_name":application,
        "bundle_id":bundle,"title":title,"title_truncated":false,"unavailable_reason":null})
}

/// The windows in front to back order, as the helper lists them: Orbit's own
/// first, because the user just pressed its button.
fn front_to_back() -> Value {
    json!({"native_instance_id":"00000000-0000-4000-8000-000000000001",
    "privacy_generation":2,"inventory_id":"00000000-0000-4000-8000-000000000002",
    "clock_domain":"mach_absolute_time","observed_monotonic_ns":1000000u64,
    "expires_monotonic_ns":30001000000u64,"selected_text_available":false,"omitted_count":0,
    "windows":[
        window(ORBIT_TOKEN, 11, "Orbit", "app.orbit.companion", "Orbit"),
        window(EDITOR_TOKEN, 22, "Editor", "example.editor", "Nota sintética"),
    ]})
}

fn png() -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder
            .write_header()?
            .write_image_data(&[0, 0, 0, 255, 255, 255, 255, 255])?;
    }
    Ok(bytes)
}

async fn helper_command(helper: &mut mpsc::Receiver<Value>, name: &str) -> Result<Value> {
    loop {
        let command = tokio::time::timeout(std::time::Duration::from_secs(10), helper.recv())
            .await
            .ok()
            .flatten()
            .with_context(|| format!("no {name} for the helper"))?;
        if command["command"] == name {
            return Ok(command);
        }
    }
}

/// «Adjuntar ventana» through the helper, to its end: the listing, the
/// capture of the window behind Orbit's, its upload, and the window's text,
/// which the helper answers with `text_reply` (`ok` and `result` or `error`),
/// after the person latched privacy when `stop_first`.
struct WindowRun {
    runtime: Runtime,
    frames: mpsc::Receiver<Value>,
    answered: Result<orbit_core::protocol::AttachmentResponse>,
    requests: Vec<Request>,
    text_order: Value,
}

async fn attach_the_editor_window(text_reply: Value, stop_first: bool) -> Result<WindowRun> {
    let (_directory, mut runtime, frames, mut helper) =
        connected(Some(limits(10_000_000, 10))).await?;
    let (work, mut work_events) = mpsc::channel(16);
    runtime.work.work_events = work;
    let (url, recorded) = http_hub(|request| {
        let body = if request.path == "/v1/attachments/capture" {
            json!({"attachment": {"id": PICTURE, "name": "Editor — Nota sintética.png",
                "kind": "image", "mime": "image/png", "bytes": 70}})
        } else if request.path == "/v1/attachments" {
            json!({"attachment": {"id": NOTES, "name": "Editor — Nota sintética.txt",
                "kind": "text", "mime": "text/plain", "bytes": request.body.len()}})
        } else {
            json!({"capture_id": "stored", "stored": true})
        };
        (
            200,
            "application/json",
            Vec::new(),
            body.to_string().into_bytes(),
        )
    })
    .await?;
    runtime.settings.hub_url = Some(url);
    // The app's own client, so the order carries its real reply channel.
    let (calls, mut received) = mpsc::channel(4);
    let (_ended, ended) = tokio::sync::watch::channel(None);
    let client = orbit_ipc::LocalClient::new(calls, &std::collections::BTreeMap::new(), ended);
    let asked = tokio::spawn(async move {
        client
            .attachment(Command::AttachmentAdd {
                source: AttachmentSource::Window,
            })
            .await
    });
    let call = received.recv().await.context("no order")?;
    runtime.attach_window(call.request.id, call.reply).await;

    let listing = helper_command(&mut helper, "screen_sources").await?;
    runtime
        .native_event(
            json!({"event":"response","id":listing["id"],"ok":true,"result":front_to_back()}),
        )
        .await?;
    let shot = helper_command(&mut helper, "screenshot").await?;
    // Orbit's window was in front; the one behind it is taken.
    assert_eq!(shot["window_binding"]["selection_token"], EDITOR_TOKEN);
    runtime
        .native_event(json!({"event":"response","id":shot["id"],"ok":true,
            "result":{"binding":shot["window_binding"],"clock_domain":"mach_absolute_time",
            "capture_started_monotonic_ns":2000000,"capture_completed_monotonic_ns":3000000,
            "mime_type":"image/png","width":2,"height":1,
            "data_base64":base64::engine::general_purpose::STANDARD.encode(png()?),
            "source":"native_window"}}))
        .await?;
    let uploaded = tokio::time::timeout(std::time::Duration::from_secs(10), work_events.recv())
        .await
        .ok()
        .flatten()
        .context("the capture upload never ended")?;
    runtime.work_event(uploaded).await?;
    let text_order = helper_command(&mut helper, "window_text").await?;
    if stop_first {
        runtime
            .reads
            .retire(orbit_core::authority::AuthorityChange::UserStop);
    }
    let mut reply = text_reply;
    reply["event"] = json!("response");
    reply["id"] = text_order["id"].clone();
    runtime.native_event(reply).await?;
    let answered = tokio::time::timeout(std::time::Duration::from_secs(10), asked)
        .await
        .context("no answer")??;
    let requests = recorded.lock().map(|seen| seen.clone()).unwrap_or_default();
    Ok(WindowRun {
        runtime,
        frames,
        answered,
        requests,
        text_order,
    })
}

#[tokio::test]
async fn a_window_is_captured_as_the_assistant_s_and_kept_as_an_attachment() -> Result<()> {
    let WindowRun {
        runtime,
        mut frames,
        answered,
        requests,
        text_order,
    } = attach_the_editor_window(json!({"ok": true, "result": {"text": " \n "}}), false).await?;
    let answered = answered?;
    assert_eq!(
        answered.attachment.map(|attachment| attachment.name),
        Some("Editor — Nota sintética.png".to_owned())
    );
    // The window just captured is the one whose text is read.
    assert_eq!(
        (&text_order["application_pid"], &text_order["window_id"]),
        (&json!(1022), &json!(22))
    );
    assert_eq!(
        text_order["limits"],
        json!({"max_bytes": 131_072, "max_nodes": 2_000, "deadline_ms": 3_000})
    );
    // A window without readable text sends the capture alone, and says so.
    assert_eq!(
        answered.window_text,
        Some(orbit_core::attachments::WindowText::Missing {
            cause: "empty".into()
        })
    );
    let [capture, kept] = requests.as_slice() else {
        bail!("expected the capture and its attachment, got {requests:?}");
    };
    assert_eq!(capture.method, "PUT");
    assert!(
        capture.path.starts_with("/v1/captures/"),
        "{}",
        capture.path
    );
    assert_eq!(capture.body, png()?);
    let request_id = capture
        .headers
        .get("x-orbit-request")
        .context("capture without its request")?
        .clone();
    assert_eq!(
        (kept.method.as_str(), kept.path.as_str()),
        ("POST", "/v1/attachments/capture")
    );
    let asked_for: Value = serde_json::from_slice(&kept.body)?;
    assert_eq!(
        asked_for,
        json!({"capture_id": capture.path.trim_start_matches("/v1/captures/"),
            "request_id": request_id, "name": "Editor — Nota sintética.png"})
    );
    // Nothing of it went to the hub as an answer to the assistant.
    while let Ok(frame) = frames.try_recv() {
        assert_ne!(frame["method"], "device.reply", "{frame}");
    }
    assert!(runtime.conversation.bridge_request.is_none());
    assert!(runtime.conversation.window_attachment.is_none());
    Ok(())
}

/// The window's text goes as a second attachment, a `.txt` named like the
/// capture, with the text the helper read.
#[tokio::test]
async fn the_window_s_text_is_attached_next_to_its_capture() -> Result<()> {
    let text = "Lista de la compra\nPan\nAgua";
    let run =
        attach_the_editor_window(json!({"ok": true, "result": {"text": text}}), false).await?;
    let answered = run.answered?;
    assert_eq!(
        answered.attachment.map(|attachment| attachment.id),
        Some(PICTURE.to_owned())
    );
    let [_capture, _kept, file] = run.requests.as_slice() else {
        bail!(
            "expected the capture, its attachment and the text, got {:?}",
            run.requests
        );
    };
    assert_eq!(
        (file.method.as_str(), file.path.as_str()),
        ("POST", "/v1/attachments")
    );
    assert_eq!(file.body, text.as_bytes());
    assert_eq!(
        file.headers
            .get("x-orbit-attachment-name")
            .map(String::as_str),
        Some("Editor%20%E2%80%94%20Nota%20sint%C3%A9tica.txt")
    );
    let Some(orbit_core::attachments::WindowText::Attached { attachment }) = answered.window_text
    else {
        bail!("the text was not attached: {:?}", answered.window_text);
    };
    assert_eq!(
        (attachment.id.as_str(), attachment.name.as_str()),
        (NOTES, "Editor — Nota sintética.txt")
    );
    Ok(())
}

/// Without Accessibility the helper reads nothing: the capture goes alone
/// and the answer names that permission.
#[tokio::test]
async fn a_window_whose_text_cannot_be_read_sends_the_capture_and_its_cause() -> Result<()> {
    let run = attach_the_editor_window(
        json!({"ok": false,
        "error": {"code": "accessibility_permission_required", "message": "untrusted text"}}),
        false,
    )
    .await?;
    let answered = run.answered?;
    assert!(answered.ok, "{:?}", answered.refusal);
    assert_eq!(
        answered.window_text,
        Some(orbit_core::attachments::WindowText::Missing {
            cause: "accessibility_permission_required".into()
        })
    );
    assert_eq!(run.requests.len(), 2, "{:?}", run.requests);
    Ok(())
}

/// A privacy stop while the window's text is on its way: the text never
/// leaves, and the app hears the stop rather than an attachment.
#[tokio::test]
async fn a_privacy_stop_keeps_the_window_s_text_on_the_mac() -> Result<()> {
    let run = attach_the_editor_window(
        json!({"ok": true, "result": {"text": "Lista de la compra"}}),
        true,
    )
    .await?;
    let refused = run
        .answered
        .err()
        .context("a window was attached after the privacy stop")?;
    assert_eq!(
        refusal_of(&refused),
        Some(Refusal::PrivacyStopped { since_ms: None })
    );
    assert!(
        run.requests
            .iter()
            .all(|request| request.path != "/v1/attachments"),
        "{:?}",
        run.requests
    );
    Ok(())
}

#[tokio::test]
async fn a_window_the_helper_cannot_take_answers_the_app_with_its_cause() -> Result<()> {
    let (_directory, mut runtime, mut frames, mut helper) =
        connected(Some(limits(10_000_000, 10))).await?;
    let (calls, mut received) = mpsc::channel(4);
    let (_ended, ended) = tokio::sync::watch::channel(None);
    let client = orbit_ipc::LocalClient::new(calls, &std::collections::BTreeMap::new(), ended);
    let asked = tokio::spawn(async move {
        client
            .attachment(Command::AttachmentAdd {
                source: AttachmentSource::Window,
            })
            .await
    });
    let call = received.recv().await.context("no order")?;
    runtime.attach_window(call.request.id, call.reply).await;
    let listing = helper_command(&mut helper, "screen_sources").await?;
    runtime
        .native_event(json!({"event":"response","id":listing["id"],"ok":false,
            "error":{"code":"screen_permission_required","message":"untrusted text"}}))
        .await?;
    let refused = tokio::time::timeout(std::time::Duration::from_secs(10), asked)
        .await
        .context("no answer")??
        .err()
        .context("a window the helper refused was attached")?;
    assert!(
        refusal_of(&refused).is_some(),
        "the cause was not typed: {refused:#}"
    );
    while let Ok(frame) = frames.try_recv() {
        assert_ne!(frame["method"], "device.reply", "{frame}");
    }
    assert!(runtime.conversation.window_attachment.is_none());
    Ok(())
}

/// The app's «Adjuntar ventana» order, waiting for its answer.
type WindowOrder = tokio::task::JoinHandle<Result<orbit_core::protocol::AttachmentResponse>>;

/// «Adjuntar ventana» from the app's own client, started up to the helper's
/// window listing, which is returned: the capture is under way.
async fn attach_window_started(
    runtime: &mut Runtime,
    client: &orbit_ipc::LocalClient,
    received: &mut mpsc::Receiver<orbit_ipc::Call>,
    helper: &mut mpsc::Receiver<Value>,
) -> Result<(WindowOrder, Value)> {
    let client = client.clone();
    let asked = tokio::spawn(async move {
        client
            .attachment(Command::AttachmentAdd {
                source: AttachmentSource::Window,
            })
            .await
    });
    let call = received.recv().await.context("no order")?;
    runtime.attach_window(call.request.id, call.reply).await;
    let listing = helper_command(helper, "screen_sources").await?;
    Ok((asked, listing))
}

/// The hub link drops while the window is captured: the app's order is
/// answered with that cause, and nothing is left holding the capture.
#[tokio::test]
async fn a_hub_drop_during_attach_window_answers_the_app_and_frees_the_slot() -> Result<()> {
    let (_directory, mut runtime, _frames, mut helper) =
        connected(Some(limits(10_000_000, 10))).await?;
    let (calls, mut received) = mpsc::channel(4);
    let (_ended, ended) = tokio::sync::watch::channel(None);
    let client = orbit_ipc::LocalClient::new(calls, &std::collections::BTreeMap::new(), ended);
    let (asked, _listing) =
        attach_window_started(&mut runtime, &client, &mut received, &mut helper).await?;
    assert!(runtime.conversation.bridge_request.is_some());

    runtime.hub_event(HubEvent::Disconnected).await?;
    assert!(runtime.conversation.bridge_request.is_none());
    assert!(runtime.conversation.window_attachment.is_none());
    let refused = tokio::time::timeout(std::time::Duration::from_secs(5), asked)
        .await
        .context("the app's «Adjuntar ventana» was never answered after the hub dropped")??
        .err()
        .context("a window was attached without a hub")?;
    assert_eq!(
        refusal_of(&refused),
        Some(Refusal::HubUnreachable { since_ms: None })
    );
    Ok(())
}

/// Once the hub is back, «Adjuntar ventana» works again without restarting.
/// The helper's listing for the first one still ends as it always does, by
/// its answer (or its own deadline); the hub does not wait on it.
#[tokio::test]
async fn attach_window_works_again_after_the_hub_comes_back() -> Result<()> {
    let (_directory, mut runtime, _frames, mut helper) =
        connected(Some(limits(10_000_000, 10))).await?;
    let (calls, mut received) = mpsc::channel(4);
    let (_ended, ended) = tokio::sync::watch::channel(None);
    let client = orbit_ipc::LocalClient::new(calls, &std::collections::BTreeMap::new(), ended);
    let (_first, listing) =
        attach_window_started(&mut runtime, &client, &mut received, &mut helper).await?;
    runtime.hub_event(HubEvent::Disconnected).await?;
    runtime
        .native_event(
            json!({"event":"response","id":listing["id"],"ok":true,"result":front_to_back()}),
        )
        .await?;

    let (hub, _frames) = mpsc::channel(16);
    runtime.hub.install(hub);
    runtime.hub_event(HubEvent::Connected).await?;
    runtime
        .hub_event(HubEvent::Frame(ready(Some(limits(10_000_000, 10)))))
        .await?;

    // The second capture starts: the helper is asked for the windows again.
    let (second, _listing) =
        attach_window_started(&mut runtime, &client, &mut received, &mut helper).await?;
    assert!(runtime.conversation.window_attachment.is_some());
    assert!(!second.is_finished());
    Ok(())
}

// HEIF photos ---------------------------------------------------------------

/// An order for a file, answered on the app's real reply channel.
async fn order_through_the_app(
    runtime: &mut Runtime,
    command: Command,
) -> Result<tokio::task::JoinHandle<Result<orbit_core::protocol::AttachmentResponse, anyhow::Error>>>
{
    let (calls, mut received) = mpsc::channel(4);
    let (_ended, ended) = tokio::sync::watch::channel(None);
    let client = orbit_ipc::LocalClient::new(calls, &std::collections::BTreeMap::new(), ended);
    let sent = command.clone();
    let asked = tokio::spawn(async move { client.attachment(sent).await });
    let call = received.recv().await.context("no order")?;
    runtime.serve_attachment(command, call.request.id, call.reply);
    Ok(asked)
}

/// A synthetic HEIF photo: an `ftyp` box of an iPhone's brands, then a
/// box that holds nothing real.
const HEIF_PHOTO: &[u8] =
    b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00mif1heic\x00\x00\x00\x0cmeta0000";

fn heic_photo(directory: &tempfile::TempDir, name: &str) -> Result<PathBuf> {
    let photo = directory.path().join(name);
    std::fs::write(&photo, HEIF_PHOTO)?;
    Ok(photo)
}

fn by_path(path: &std::path::Path) -> Command {
    Command::AttachmentAdd {
        source: AttachmentSource::Path {
            path: path.to_string_lossy().into_owned(),
        },
    }
}

/// A hub that keeps each upload as a JPEG under the name it was sent with.
async fn photo_hub() -> Result<(String, Recorded)> {
    http_hub(|request| {
        let name = request
            .headers
            .get("x-orbit-attachment-name")
            .cloned()
            .unwrap_or_default();
        let body = json!({"attachment": {"id": PICTURE, "name": name,
            "kind": "image", "mime": "image/jpeg", "bytes": request.body.len()}});
        (
            200,
            "application/json",
            Vec::new(),
            body.to_string().into_bytes(),
        )
    })
    .await
}

/// A connected runtime whose work queue the test reads, as the loop does.
async fn photo_runtime() -> Result<(
    tempfile::TempDir,
    Runtime,
    mpsc::Receiver<Value>,
    mpsc::Receiver<WorkResult>,
    Recorded,
)> {
    let (directory, mut runtime, _frames, helper) = connected(Some(limits(1_000_000, 10))).await?;
    let (work, work_events) = mpsc::channel(16);
    runtime.work.work_events = work;
    let (url, recorded) = photo_hub().await?;
    runtime.settings.hub_url = Some(url);
    Ok((directory, runtime, helper, work_events, recorded))
}

/// The loop's part of a photo: the order read it, found a HEIF photo and
/// asked for its JPEG; the loop sends the helper `photo_jpeg`.
async fn photo_jpeg_order(
    runtime: &mut Runtime,
    work_events: &mut mpsc::Receiver<WorkResult>,
    helper: &mut mpsc::Receiver<Value>,
) -> Result<Value> {
    let asked = tokio::time::timeout(std::time::Duration::from_secs(10), work_events.recv())
        .await
        .ok()
        .flatten()
        .context("the order never asked for the photo's JPEG")?;
    runtime.work_event(asked).await?;
    helper_command(helper, "photo_jpeg").await
}

/// The helper's work: the JPEG where it was asked to write it, and its answer.
async fn helper_writes_the_jpeg(runtime: &mut Runtime, convert: &Value) -> Result<Vec<u8>> {
    let destination = convert["destination_path"]
        .as_str()
        .context("no destination")?;
    let jpeg = b"\xff\xd8\xff synthetic jpeg".to_vec();
    std::fs::write(destination, &jpeg)?;
    runtime
        .native_event(json!({"event":"response","id":convert["id"],"ok":true,
            "result":{"width":4032,"height":3024}}))
        .await?;
    Ok(jpeg)
}

fn upload_name(request: &Request) -> Option<&str> {
    request
        .headers
        .get("x-orbit-attachment-name")
        .map(String::as_str)
}

/// No agent reads a HEIC photo, so the helper writes it as JPEG in a folder
/// of the daemon's; that JPEG is uploaded under the photo's name with
/// `.jpg`, and the folder is gone once it is.
#[tokio::test]
async fn a_heic_photo_is_uploaded_as_the_jpeg_the_helper_wrote() -> Result<()> {
    let (directory, mut runtime, mut helper, mut work_events, recorded) = photo_runtime().await?;
    let photo = heic_photo(&directory, "IMG_0001.HEIC")?;
    let asked = order_through_the_app(&mut runtime, by_path(&photo)).await?;
    let convert = photo_jpeg_order(&mut runtime, &mut work_events, &mut helper).await?;
    assert_eq!(convert["source_path"], photo.to_string_lossy().as_ref());
    let destination = PathBuf::from(
        convert["destination_path"]
            .as_str()
            .context("no destination")?,
    );
    assert!(
        destination.starts_with(runtime.paths.root.join("attachments")),
        "{}",
        destination.display()
    );
    assert_eq!(
        destination.file_name().and_then(|name| name.to_str()),
        Some("IMG_0001.jpg")
    );
    let jpeg = helper_writes_the_jpeg(&mut runtime, &convert).await?;
    let answered = tokio::time::timeout(std::time::Duration::from_secs(10), asked)
        .await
        .context("no answer")???;
    assert!(answered.ok, "{:?}", answered.refusal);
    let requests = recorded.lock().map(|seen| seen.clone()).unwrap_or_default();
    let [upload] = requests.as_slice() else {
        bail!("expected one upload, got {requests:?}");
    };
    assert_eq!(upload.body, jpeg);
    assert_eq!(upload_name(upload), Some("IMG_0001.jpg"));
    let folder = destination.parent().context("no folder")?;
    assert!(!folder.exists(), "{} was left behind", folder.display());
    Ok(())
}

/// The photo is known by its content, not its name: a HEIC called `.jpg`
/// goes as JPEG too.
#[tokio::test]
async fn a_heic_photo_named_jpg_is_known_by_its_content() -> Result<()> {
    let (directory, mut runtime, mut helper, mut work_events, recorded) = photo_runtime().await?;
    let photo = heic_photo(&directory, "playa.jpg")?;
    let asked = order_through_the_app(&mut runtime, by_path(&photo)).await?;
    let convert = photo_jpeg_order(&mut runtime, &mut work_events, &mut helper).await?;
    assert_eq!(convert["source_path"], photo.to_string_lossy().as_ref());
    let jpeg = helper_writes_the_jpeg(&mut runtime, &convert).await?;
    let answered = tokio::time::timeout(std::time::Duration::from_secs(10), asked)
        .await
        .context("no answer")???;
    assert!(answered.ok, "{:?}", answered.refusal);
    let requests = recorded.lock().map(|seen| seen.clone()).unwrap_or_default();
    let [upload] = requests.as_slice() else {
        bail!("expected one upload, got {requests:?}");
    };
    assert_eq!(upload.body, jpeg);
    assert_eq!(upload_name(upload), Some("playa.jpg"));
    Ok(())
}

/// A pasted HEIC photo is written in the daemon's private folder for the
/// helper to read, then uploaded as JPEG under its name with `.jpg`; both
/// copies go with the folder.
#[tokio::test]
async fn a_pasted_heic_photo_is_written_privately_and_uploaded_as_jpeg() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let (_directory, mut runtime, mut helper, mut work_events, recorded) = photo_runtime().await?;
    let pasted = Command::AttachmentAdd {
        source: AttachmentSource::Bytes {
            name: "Imagen pegada.png".into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(HEIF_PHOTO),
        },
    };
    let asked = order_through_the_app(&mut runtime, pasted).await?;
    let convert = photo_jpeg_order(&mut runtime, &mut work_events, &mut helper).await?;
    let source = PathBuf::from(convert["source_path"].as_str().context("no source")?);
    let destination = PathBuf::from(
        convert["destination_path"]
            .as_str()
            .context("no destination")?,
    );
    let folder = source.parent().context("no folder")?.to_owned();
    assert!(
        folder.starts_with(runtime.paths.root.join("attachments")),
        "{}",
        folder.display()
    );
    assert_eq!(destination.parent(), Some(folder.as_path()));
    assert_ne!(source, destination);
    assert_eq!(std::fs::read(&source)?, HEIF_PHOTO);
    assert_eq!(
        std::fs::metadata(&source)?.permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(&folder)?.permissions().mode() & 0o777,
        0o700
    );
    let jpeg = helper_writes_the_jpeg(&mut runtime, &convert).await?;
    let answered = tokio::time::timeout(std::time::Duration::from_secs(10), asked)
        .await
        .context("no answer")???;
    assert!(answered.ok, "{:?}", answered.refusal);
    let requests = recorded.lock().map(|seen| seen.clone()).unwrap_or_default();
    let [upload] = requests.as_slice() else {
        bail!("expected one upload, got {requests:?}");
    };
    assert_eq!(upload.body, jpeg);
    assert_eq!(upload_name(upload), Some("Imagen%20pegada.jpg"));
    assert!(!folder.exists(), "{} was left behind", folder.display());
    Ok(())
}

/// A file called `.heic` that is not a photo goes as it is: the hub judges
/// it by its content, and the helper is never asked.
#[tokio::test]
async fn a_file_named_heic_that_is_not_a_photo_goes_as_it_is() -> Result<()> {
    let (directory, mut runtime, mut helper, mut work_events, recorded) = photo_runtime().await?;
    let notes = directory.path().join("notas.heic");
    std::fs::write(&notes, "Ideas para el viaje\n")?;
    let asked = order_through_the_app(&mut runtime, by_path(&notes)).await?;
    let answered = tokio::time::timeout(std::time::Duration::from_secs(10), asked)
        .await
        .context("no answer")???;
    assert!(answered.ok, "{:?}", answered.refusal);
    let requests = recorded.lock().map(|seen| seen.clone()).unwrap_or_default();
    let [upload] = requests.as_slice() else {
        bail!("expected one upload, got {requests:?}");
    };
    assert_eq!(upload.body, b"Ideas para el viaje\n");
    assert_eq!(upload_name(upload), Some("notas.heic"));
    assert!(work_events.try_recv().is_err());
    while let Ok(command) = helper.try_recv() {
        assert_ne!(command["command"], "photo_jpeg", "{command}");
    }
    Ok(())
}

/// A photo the helper cannot turn into JPEG is refused with its name and
/// the helper's cause; nothing is uploaded.
#[tokio::test]
async fn a_heic_photo_the_helper_cannot_convert_is_refused_with_its_cause() -> Result<()> {
    let (directory, mut runtime, mut helper, mut work_events, recorded) = photo_runtime().await?;
    let photo = heic_photo(&directory, "IMG_0001.HEIC")?;
    let asked = order_through_the_app(&mut runtime, by_path(&photo)).await?;
    let convert = photo_jpeg_order(&mut runtime, &mut work_events, &mut helper).await?;
    runtime
        .native_event(json!({"event":"response","id":convert["id"],"ok":false,
            "error":{"code":"photo_unreadable","message":"untrusted text"}}))
        .await?;
    let refused = tokio::time::timeout(std::time::Duration::from_secs(10), asked)
        .await
        .context("no answer")??
        .err()
        .context("a photo the helper could not convert was attached")?;
    assert_eq!(
        refusal_of(&refused),
        Some(Refusal::Attachment {
            cause: AttachmentCause::Unreadable {
                name: Some("IMG_0001.HEIC".into()),
                reason: "photo_unreadable".into(),
            }
        })
    );
    assert!(recorded.lock().map(|seen| seen.is_empty()).unwrap_or(false));
    Ok(())
}

/// A turn the hub refused for its files (here, the conversation's ceiling)
/// tells the person the sizes, not only the code.
#[tokio::test]
async fn a_turn_refused_for_its_files_keeps_the_numbers() -> Result<()> {
    let (_directory, mut runtime, mut frames, _helper) = connected(Some(limits(1_000, 10))).await?;
    runtime
        .command(chat("Otra", vec![file(PICTURE, "mapa.png", 120)]))
        .await?;
    let sent = frames.try_recv().context("no chat.send")?;
    runtime
        .hub_event(HubEvent::Frame(
            json!({"event": "error", "id": sent["id"], "data": {
            "code": "attachments_too_large", "message": "Esta conversación ya guarda…",
            "refusal": {"code": "attachments_too_large", "ceiling": "session", "bytes": 120,
                "session_bytes": 268_435_400_u64, "max_bytes": 268_435_456_u64,
                "retry": {"kind": "after_fix"}}}}),
        ))
        .await?;
    let notice = serde_json::to_value(runtime.state.durable.notice.as_ref().context("no notice")?)?;
    assert_eq!(notice["code"], "assistant_failed");
    assert_eq!(notice["data"]["code"], "attachments_too_large");
    assert_eq!(notice["data"]["bytes"], "120");
    assert_eq!(notice["data"]["max_bytes"], "268435456");
    assert_eq!(notice["data"]["stored"], "268435400");
    assert!(runtime.state.durable.active_request.is_none());
    Ok(())
}

/// The hub stops between its `ready` and an upload, «Ver» or «Abrir»: each
/// is refused as a hub out of reach, never as an internal failure carrying
/// the request library's text and the hub's address.
#[tokio::test]
async fn a_stopped_hub_is_named_when_uploading_viewing_or_opening() -> Result<()> {
    let (_directory, mut runtime, _frames, _helper) =
        connected(Some(limits(10_000_000, 10))).await?;
    // A port nobody listens on: bind one, then let it go.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?;
    runtime.settings.hub_url = Some(format!("http://{closed}"));
    let orders = [
        Command::AttachmentAdd {
            source: AttachmentSource::Bytes {
                name: "Texto pegado.txt".into(),
                data_base64: base64::engine::general_purpose::STANDARD
                    .encode(b"Lista de la compra"),
            },
        },
        Command::AttachmentFetch {
            attachment_id: PICTURE.into(),
        },
        Command::AttachmentSave {
            attachment_id: PICTURE.into(),
        },
    ];
    for (index, order) in orders.into_iter().enumerate() {
        let answered = runtime
            .attachment_context()
            .answer(order, format!("order-{index}"))
            .await;
        assert!(!answered.ok);
        let refusal = answered.refusal.context("no refusal")?;
        let wire = serde_json::to_string(&refusal)?;
        assert!(
            matches!(refusal, Refusal::HubUnreachable { .. }),
            "order {index}: a stopped hub reads as {wire}"
        );
        let said = format!("{wire} {}", answered.error.unwrap_or_default());
        assert!(!said.contains(&closed.to_string()), "order {index}: {said}");
    }
    Ok(())
}
