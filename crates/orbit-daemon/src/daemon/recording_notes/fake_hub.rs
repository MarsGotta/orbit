//! A synthetic hub for the device's meeting routes: local loopback only, no
//! provider, no vault. Tests place the documents and turns the hub computes;
//! this keeps the rules of the real routes (`server/orbit_hub/meetings/
//! {routes,views}.py`, `notes/routes.py`) the Mac relies on — revisions and
//! the 409 with its numbers, a speaker's default name, the list's order and
//! its tie-break, fixture pages, the strict note request — and
//! records every request it received.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone)]
pub(crate) struct Received {
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: Value,
}

#[derive(Default)]
pub(crate) struct Hub {
    pub received: Vec<Received>,
    /// `GET /v1/recordings/{id}` as the hub computes it.
    pub documents: BTreeMap<String, Value>,
    pub turns: BTreeMap<String, Vec<Value>>,
    pub templates: Vec<Value>,
    pub links: Vec<Value>,
    pub owner_name: Option<String>,
    /// Answer every request with this status and the hub's envelope.
    pub failing: Option<u16>,
    pub missing_turn: Option<(u64, u64)>,
    /// Hold every answer until this is notified.
    pub hold: Option<Arc<tokio::sync::Notify>>,
    /// The meeting's own title, kept when the user first renames it.
    pub own_titles: BTreeMap<String, Value>,
    /// Groups whose audio the hub cannot read, of how many: a change
    /// of language is refused as the hub refuses it.
    pub unreadable_audio: Option<(u64, u64)>,
    /// `GET /v1/series/previous-meeting`: the earlier meeting of each
    /// series as the hub finds it; a series not here has none.
    pub series_previous: BTreeMap<String, Value>,
    /// `PUT /v1/recordings/{id}/series` refused with this status, code and
    /// numbers, as `meetings/series.py` refuses.
    pub series_refusal: Option<(u16, &'static str, Value)>,
    /// A hub of 0.10 or earlier: the series routes are not there, and
    /// FastAPI answers them as it answered then (`{"detail":"Not Found"}`,
    /// measured against 0.10.1's code).
    pub without_series_routes: bool,
}

pub(crate) type Shared = Arc<Mutex<Hub>>;

pub(crate) fn lock(hub: &Shared) -> std::sync::MutexGuard<'_, Hub> {
    match hub.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn refusal(status: u16, code: &str, message: &str) -> (u16, Value) {
    numbered(status, code, message, json!({}))
}

/// The hub's envelope (`errors.render`): the refusal carries its numbers.
fn numbered(status: u16, code: &str, message: &str, numbers: Value) -> (u16, Value) {
    let mut refusal = json!({"code": code, "retry": {"kind": "never"}});
    if let (Some(refusal), Some(numbers)) = (refusal.as_object_mut(), numbers.as_object()) {
        refusal.extend(numbers.clone());
    }
    (
        status,
        json!({"message": message, "code": code, "refusal": refusal, "op": "synthetic-op"}),
    )
}

fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.to_owned())
    })
}

/// A query value as the hub reads it: `%XX` escapes decoded, `None` when
/// they do not make UTF-8.
fn urlencoding_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = value.get(index + 1..index + 3)?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// A meeting the hub has, closed and transcribed, as its document says it.
pub(crate) fn document(recording: &str, title: &str, started_at_ms: i64) -> Value {
    json!({
        "recording_id": recording, "title": title, "started_at_ms": started_at_ms,
        "duration_ms": 120_000, "capture": "closed", "language": "es",
        "speakers": [
            {"key": "mic_local", "track": "mic_local", "name": "Nerea"},
            {"key": "remote_app", "track": "remote_app", "name": "Otra persona"}
        ],
        "transcript": {"state": "ready", "pending_chunks": 0, "failed_groups": []},
        "note": null,
        "private_notes": {"text": "", "revision": 0}
    })
}

pub(crate) fn note(revision: u64) -> Value {
    json!({
        "state": "ready", "revision": revision, "attempts": 1, "next_attempt_ms": null,
        "error": null, "template_id": "general",
        "note": {
            "title": "Comité", "short_summary": "Resumen corto.",
            "sections": [{"name": "Resumen", "markdown": "- punto"}],
            "action_items": [{"id": "a0123456789ab", "owner": "Nerea", "text": "Enviar el acta",
                "due": null, "done": false}],
            "decisions": ["Aprobado"], "open_topics": ["Presupuesto"],
            "template_id": "general", "template_name": "General",
            "model": "qwen", "generated_at_ms": 1_800_000_000_000_i64,
            "segments": 4, "duration_ms": 120_000, "parts": 1
        },
        "edited": {"sections": false}, "superseded_available": false,
        "updated_at_ms": 1_800_000_000_000_i64
    })
}

impl Hub {
    fn row(document: &Value) -> Value {
        json!({
            "recording_id": document["recording_id"], "title": document["title"],
            "started_at_ms": document["started_at_ms"], "duration_ms": document["duration_ms"],
            "capture": document["capture"], "transcript": document["transcript"]["state"],
            "note": document["note"].as_object().map(|note| json!({
                "state": note["state"],
                "short_summary": note["note"]["short_summary"],
            })),
        })
    }

    /// Newest first, equal starts by id; a page continues after
    /// (`before_ms`, `before_id`) — rows that start earlier, or at the same
    /// instant with a later id (`views.before`).
    fn list(&self, query: &str) -> (u16, Value) {
        let before = query_value(query, "before_ms").and_then(|value| value.parse::<i64>().ok());
        let before_id = query_value(query, "before_id");
        let limit = query_value(query, "limit")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(50);
        if limit > 50 {
            return numbered(
                400,
                "invalid_parameters",
                "limit fuera de rango: entre 1 y 50",
                json!({"field": "limit", "min": 1, "max": 50}),
            );
        }
        let mut rows = self
            .documents
            .values()
            .filter(|document| {
                let Some(before) = before else { return true };
                let started = document["started_at_ms"].as_i64().unwrap_or_default();
                started < before
                    || (started == before
                        && before_id.as_deref().is_some_and(|id| {
                            document["recording_id"]
                                .as_str()
                                .is_some_and(|mine| mine > id)
                        }))
            })
            .map(Self::row)
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| {
            (
                std::cmp::Reverse(row["started_at_ms"].as_i64()),
                row["recording_id"].as_str().map(str::to_owned),
            )
        });
        let more = rows.len() > limit;
        rows.truncate(limit);
        (200, json!({"meetings": rows, "more": more}))
    }

    fn default_name(&self, key: &str) -> String {
        match key {
            "mic_local" => self
                .owner_name
                .clone()
                .unwrap_or_else(|| "Micrófono local".into()),
            _ => "Otra persona".into(),
        }
    }

    fn answer(&mut self, method: &str, path: &str, query: &str, body: Value) -> (u16, Value) {
        self.received.push(Received {
            method: method.into(),
            path: path.into(),
            query: query.into(),
            body: body.clone(),
        });
        if let Some(status) = self.failing {
            return refusal(status, "synthetic_failure", "Fallo sintético");
        }
        let segments = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
        let series_route = matches!(
            (method, segments.as_slice()),
            ("GET", ["v1", "series", "previous-meeting"])
                | ("PUT", ["v1", "recordings", _, "series"])
        );
        if series_route && self.without_series_routes {
            return (404, json!({"detail": "Not Found"}));
        }
        match (method, segments.as_slice()) {
            ("GET", ["v1", "series", "previous-meeting"]) => {
                let series = query_value(query, "calendar_item_id")
                    .map(|value| value.replace('+', " "))
                    .and_then(|value| urlencoding_decode(&value))
                    .unwrap_or_default();
                let before_ms = query_value(query, "before_ms")
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or_default();
                let previous = self
                    .series_previous
                    .get(&series)
                    .cloned()
                    .unwrap_or(Value::Null);
                (
                    200,
                    json!({"calendar_item_id": series, "before_ms": before_ms, "previous": previous}),
                )
            }
            ("PUT", ["v1", "recordings", id, "series"]) => {
                if let Some((status, code, numbers)) = self.series_refusal.clone() {
                    return numbered(status, code, "No se guardó nada", numbers);
                }
                if !self.documents.contains_key(*id) {
                    return refusal(404, "recording_not_found", "Grabación no disponible");
                }
                (
                    200,
                    json!({"recording_id": id, "calendar_item_id": "serie-comite",
                        "template_id": body["template_id"], "dictionary_id": body["dictionary_id"]}),
                )
            }
            ("PUT", ["v1", "device", "profile"]) => {
                let name = body["owner_name"].as_str().unwrap_or_default().to_owned();
                self.owner_name = Some(name.clone());
                (200, json!({"owner_name": name}))
            }
            ("GET", ["v1", "recordings"]) => self.list(query),
            ("GET", ["v1", "recordings", id]) => match self.documents.get(*id) {
                Some(document) => (200, document.clone()),
                None => refusal(404, "recording_not_found", "Grabación no disponible"),
            },
            ("GET", ["v1", "recordings", id, "turns"]) => {
                let Some(turns) = self.turns.get(*id) else {
                    return refusal(404, "recording_not_found", "Grabación no disponible");
                };
                let offset = query_value(query, "offset")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                // Two synthetic turns per response exercise the Mac's cursor;
                // this fixture does not reproduce the hub's byte/page rules.
                let page: Vec<Value> = turns.iter().skip(offset).take(2).cloned().collect();
                let next = (offset + page.len() < turns.len()).then_some(offset + page.len());
                (
                    200,
                    json!({"turns": page, "next_offset": next, "total": turns.len()}),
                )
            }
            ("PUT", ["v1", "recordings", id, "speakers"]) => {
                if let Some((at_ms, turns)) = self.missing_turn {
                    return numbered(
                        409,
                        "turn_no_longer_exists",
                        "El turno ya no existe",
                        json!({"at_ms":at_ms,"turns":turns}),
                    );
                }
                let defaults = BTreeMap::from([
                    ("mic_local".to_owned(), self.default_name("mic_local")),
                    ("remote_app".to_owned(), self.default_name("remote_app")),
                ]);
                let Some(document) = self.documents.get_mut(*id) else {
                    return refusal(404, "recording_not_found", "Grabación no disponible");
                };
                if let (Some(names), Some(speakers)) = (
                    body["names"].as_object(),
                    document["speakers"].as_array_mut(),
                ) {
                    for speaker in speakers.iter_mut() {
                        let Some(key) = speaker["key"].as_str().map(str::to_owned) else {
                            continue;
                        };
                        match names.get(&key) {
                            Some(Value::String(name)) => speaker["name"] = json!(name),
                            Some(Value::Null) => {
                                speaker["name"] =
                                    json!(defaults.get(&key).cloned().unwrap_or_default());
                            }
                            _ => {}
                        }
                    }
                }
                (200, json!({"speakers": document["speakers"]}))
            }
            // As `meetings/routes.py` does: the same language repeats
            // nothing; another repeats the meeting's four groups.
            ("PUT", ["v1", "recordings", id, "language"]) => {
                if let Some((groups, total)) = self.unreadable_audio {
                    return numbered(
                        409,
                        "meeting_audio_unreadable",
                        "No se cambió el idioma",
                        json!({"groups": groups, "total": total, "cause": "invalid_group_audio"}),
                    );
                }
                let Some(document) = self.documents.get_mut(*id) else {
                    return refusal(404, "recording_not_found", "Grabación no disponible");
                };
                let requeued = if document["language"] == body["language"] {
                    0
                } else {
                    4
                };
                document["language"] = body["language"].clone();
                if requeued > 0 {
                    document["transcript"]["state"] = json!("pending");
                    document["transcript"]["retranscribing"] =
                        json!({"language": body["language"], "done": 0, "total": requeued});
                }
                (
                    200,
                    json!({"language": body["language"], "requeued": requeued}),
                )
            }
            // The user's name wins; an empty one gives the meeting back its own.
            ("PUT", ["v1", "recordings", id, "title"]) => {
                let Some(document) = self.documents.get_mut(*id) else {
                    return refusal(404, "recording_not_found", "Grabación no disponible");
                };
                let own = self
                    .own_titles
                    .entry((*id).to_owned())
                    .or_insert_with(|| document["title"].clone())
                    .clone();
                let title = body["title"].as_str().unwrap_or_default().trim().to_owned();
                document["title"] = if title.is_empty() { own } else { json!(title) };
                (200, json!({"title": document["title"]}))
            }
            ("PUT", ["v1", "recordings", id, "private-notes"]) => {
                let document = self
                    .documents
                    .entry((*id).to_owned())
                    .or_insert_with(|| document(id, "Grabación", 0));
                let current = document["private_notes"]["revision"].as_u64().unwrap_or(0);
                if body["base_revision"].as_u64() != Some(current) {
                    return numbered(
                        409,
                        "stale_revision",
                        "Las notas privadas cambiaron",
                        json!({"revision": current, "base_revision": body["base_revision"]}),
                    );
                }
                document["private_notes"] = json!({"text": body["text"], "revision": current + 1});
                (200, json!({"revision": current + 1}))
            }
            ("POST", ["v1", "recordings", id, "note"]) => {
                // `GenerateRequest` is strict: `{template_id?}` and nothing else.
                let strict = match &body {
                    Value::Null => true,
                    Value::Object(fields) => fields.keys().all(|key| key == "template_id"),
                    _ => false,
                };
                if !strict {
                    return refusal(400, "invalid_parameters", "Petición no válida");
                }
                let Some(document) = self.documents.get_mut(*id) else {
                    return refusal(404, "recording_not_found", "Grabación no disponible");
                };
                let revision = document["note"]["revision"].as_u64().unwrap_or(0) + 1;
                let mut queued = note(revision);
                queued["state"] = json!("queued");
                queued["template_id"] = body["template_id"].clone();
                queued["note"] = document["note"]["note"].clone();
                document["note"] = queued;
                (202, document["note"].clone())
            }
            ("PUT", ["v1", "recordings", id, "note", "edits"]) => {
                let Some(document) = self.documents.get_mut(*id) else {
                    return refusal(404, "recording_not_found", "Grabación no disponible");
                };
                let current = document["note"]["revision"].as_u64().unwrap_or(0);
                if body["base_revision"].as_u64() != Some(current) {
                    return refusal(409, "note_revision_changed", "La nota cambió");
                }
                if body.get("sections").is_some() {
                    document["note"]["note"]["sections"] = body["sections"].clone();
                    document["note"]["edited"]["sections"] = json!(true);
                }
                if let (Some(done), Some(items)) = (
                    body["done"].as_object(),
                    document["note"]["note"]["action_items"].as_array_mut(),
                ) {
                    for item in items.iter_mut() {
                        if let Some(mark) = item["id"].as_str().and_then(|id| done.get(id)) {
                            item["done"] = mark.clone();
                        }
                    }
                }
                document["note"]["revision"] = json!(current + 1);
                (200, document["note"].clone())
            }
            ("GET", ["v1", "note-templates"]) => (200, json!({"templates": self.templates})),
            ("PUT", ["v1", "note-templates", id]) => {
                let mut template = body.clone();
                template["id"] = json!(id);
                template["builtin"] = json!(false);
                self.templates
                    .retain(|existing| existing["id"] != json!(id));
                self.templates.push(template.clone());
                (200, template)
            }
            ("DELETE", ["v1", "note-templates", id]) => {
                let before = self.templates.len();
                self.templates
                    .retain(|existing| existing["id"] != json!(id));
                (
                    200,
                    json!({"template_id": id, "deleted": before != self.templates.len()}),
                )
            }
            ("GET", ["v1", "note-template-links"]) => (200, json!({"links": self.links})),
            _ => refusal(404, "not_found", "Ruta desconocida"),
        }
    }
}

async fn serve_one(hub: Shared, mut socket: tokio::net::TcpStream) -> Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    let header_end = loop {
        if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        let count = socket.read(&mut buffer).await?;
        if count == 0 {
            bail!("closed before the request arrived");
        }
        request.extend_from_slice(&buffer[..count]);
    };
    let head = String::from_utf8_lossy(&request[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().context("request line")?.split(' ');
    let method = first.next().context("method")?.to_owned();
    let target = first.next().context("target")?.to_owned();
    let length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while request.len() < header_end + length {
        let count = socket.read(&mut buffer).await?;
        if count == 0 {
            bail!("closed before the body arrived");
        }
        request.extend_from_slice(&buffer[..count]);
    }
    let body = if length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&request[header_end..header_end + length])?
    };
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    let hold = lock(&hub).hold.clone();
    if let Some(hold) = hold {
        hold.notified().await;
    }
    let (status, answer) = lock(&hub).answer(&method, path, query, body);
    let bytes = serde_json::to_vec(&answer)?;
    socket
        .write_all(
            format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .as_bytes(),
        )
        .await?;
    socket.write_all(&bytes).await?;
    socket.shutdown().await?;
    Ok(())
}

/// Serves until the test ends. Returns the hub URL.
pub(crate) async fn start(hub: Shared) -> Result<String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let hub = hub.clone();
            tokio::spawn(async move {
                let _ = serve_one(hub, socket).await;
            });
        }
    });
    Ok(url)
}
