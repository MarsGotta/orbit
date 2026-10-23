//! HTTP adapter for the hub's meeting, note and template routes. It performs
//! one request and names what came back; no retries here, no note content in
//! logs.
use orbit_core::{
    limits::MAX_HUB_HTTP_BODY_BYTES,
    notes::{
        MeetingDocument, MeetingList, Section, Template, TemplateDraft, TemplateLinks,
        TemplateList, TurnsPage,
    },
    refusal::{Refusal, Resource, Service},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The hub's error envelope (`message`, `code` and its typed `refusal`).
/// Its longest note message names two revisions (under 100 bytes); 4 KiB
/// leaves room for the refusal object, also for an agent's refusal
/// (`hub_agents`), whose numbers are a few scalars: a skill name, a rule, a
/// Hermes route. Past it the code is not read and the status alone is
/// reported.
pub(crate) const MAX_ERROR_BYTES: usize = 4 * 1024;

mod reminders;

#[derive(Clone)]
pub struct NoteHub {
    pub base: reqwest::Url,
    pub token: String,
    /// The op every request of this worker carries.
    pub op: orbit_core::notice::Op,
}

/// What one request came back with.
#[derive(Debug)]
pub enum Answer<T> {
    Value(T),
    /// The hub answered with an error of its own: its status and code.
    Refused {
        status: u16,
        code: Option<String>,
        message: Option<String>,
        /// The current revision a 409 `stale_revision` names
        /// (`refusal.revision`).
        revision: Option<u64>,
        /// The refusal's whole numbers (`refusal.groups`, `refusal.total`…),
        /// so whoever asked is told them; at most what fits the 4 KiB
        /// envelope.
        numbers: BTreeMap<String, u64>,
    },
    /// The request did not reach the hub or its answer was unusable, with
    /// the cause and its numbers.
    Unreachable(Unreached),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreached {
    Transport(&'static str),
    TooLarge {
        bytes: u64,
        max_bytes: usize,
    },
    Cut {
        received_bytes: usize,
    },
    Shape,
    /// 408, 429 and 5xx: the hub asked for time or failed.
    Status {
        status: u16,
        code: Option<String>,
    },
}

impl std::fmt::Display for Unreached {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(cause) => formatter.write_str(cause),
            Self::TooLarge { bytes, max_bytes } => write!(
                formatter,
                "la respuesta del hub mide al menos {bytes} bytes; el techo de lectura es {max_bytes}"
            ),
            Self::Cut { received_bytes } => write!(
                formatter,
                "la respuesta del hub se cortó tras {received_bytes} bytes"
            ),
            Self::Shape => {
                formatter.write_str("la respuesta del hub no tiene la forma del contrato")
            }
            Self::Status {
                status,
                code: Some(code),
            } => write!(formatter, "el hub respondió HTTP {status} (código {code})"),
            Self::Status { status, code: None } => {
                write!(formatter, "el hub respondió HTTP {status}")
            }
        }
    }
}

impl<T> Answer<T> {
    /// The refusal a read answers with when the hub did not give `T`.
    pub fn into_result(self) -> Result<T, Refusal> {
        match self {
            Self::Value(value) => Ok(value),
            Self::Refused { status: 404, .. } => Err(Refusal::NotFound {
                resource: Resource::Recording,
            }),
            Self::Refused { status, .. } => Err(Refusal::UpstreamFailed {
                service: Service::Hub,
                status: Some(status),
            }),
            Self::Unreachable(Unreached::Transport(_)) => {
                Err(Refusal::HubUnreachable { since_ms: None })
            }
            Self::Unreachable(Unreached::TooLarge { bytes, max_bytes }) => Err(Refusal::TooLarge {
                resource: Resource::HubResponse,
                bytes,
                max_bytes: max_bytes as u64,
            }),
            Self::Unreachable(Unreached::Status { status, .. }) => Err(Refusal::UpstreamFailed {
                service: Service::Hub,
                status: Some(status),
            }),
            Self::Unreachable(Unreached::Cut { received_bytes }) => Err(Refusal::HubReplyInvalid {
                field: "body".into(),
                received_bytes: Some(received_bytes as u64),
            }),
            Self::Unreachable(Unreached::Shape) => Err(Refusal::HubReplyInvalid {
                field: "body".into(),
                received_bytes: None,
            }),
        }
    }
}

fn transport_cause(error: &reqwest::Error) -> &'static str {
    use crate::hub::TransportCause;
    match TransportCause::of(error) {
        TransportCause::ConnectionTimeout | TransportCause::Timeout => {
            "el hub no respondió a tiempo"
        }
        TransportCause::Connection => "no se pudo conectar con el hub",
        TransportCause::Cut => "la conexión con el hub se cortó",
    }
}

async fn answer<T: DeserializeOwned>(sent: Result<reqwest::Response, reqwest::Error>) -> Answer<T> {
    let response = match sent {
        Ok(response) => response,
        Err(error) => return Answer::Unreachable(Unreached::Transport(transport_cause(&error))),
    };
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let envelope = crate::hub::read_refusal(response, MAX_ERROR_BYTES)
            .await
            .unwrap_or_default();
        let code = envelope
            .code
            .as_deref()
            .and_then(crate::hub::hub_code)
            .map(str::to_owned);
        if status >= 500 || matches!(status, 408 | 429) {
            return Answer::Unreachable(Unreached::Status { status, code });
        }
        let numbers = envelope.numbers;
        return Answer::Refused {
            status,
            code,
            message: envelope.message,
            revision: numbers.get("revision").copied(),
            numbers,
        };
    }
    match crate::hub::bounded_body(response, MAX_HUB_HTTP_BODY_BYTES).await {
        Ok(bytes) => match serde_json::from_slice::<T>(&bytes) {
            Ok(value) => Answer::Value(value),
            Err(_) => Answer::Unreachable(Unreached::Shape),
        },
        Err(crate::hub::BodyError::TooLarge(size)) => Answer::Unreachable(Unreached::TooLarge {
            bytes: size.bytes,
            max_bytes: size.max_bytes as usize,
        }),
        Err(crate::hub::BodyError::Cut { received_bytes, .. }) => {
            Answer::Unreachable(Unreached::Cut { received_bytes })
        }
    }
}

impl NoteHub {
    pub async fn dictionaries(&self) -> Answer<orbit_core::notes::DictionaryList> {
        self.send(
            reqwest::Method::GET,
            self.endpoint(&["v1", "meeting-dictionaries"]),
            None,
        )
        .await
    }

    pub async fn dictionary_order(
        &self,
        request: &orbit_core::notes::DictionaryRequest,
    ) -> Answer<Value> {
        self.send(
            reqwest::Method::POST,
            self.endpoint(&["v1", "meeting-dictionaries", "orders"]),
            Some(json!(request)),
        )
        .await
    }

    fn endpoint(&self, segments: &[&str]) -> Result<reqwest::Url, &'static str> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| "la URL del hub no admite rutas")?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    fn recording(&self, recording: &str, tail: &[&str]) -> Result<reqwest::Url, &'static str> {
        let mut segments = vec!["v1", "recordings", recording];
        segments.extend_from_slice(tail);
        self.endpoint(&segments)
    }

    async fn send<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: Result<reqwest::Url, &'static str>,
        body: Option<Value>,
    ) -> Answer<T> {
        let url = match url {
            Ok(url) => url,
            Err(cause) => return Answer::Unreachable(Unreached::Transport(cause)),
        };
        let Ok(mut request) = crate::hub::request(method, url, Some(&self.token), &self.op) else {
            return Answer::Unreachable(Unreached::Transport(
                "no se pudo preparar el cliente HTTP",
            ));
        };
        if let Some(body) = body {
            request = request.json(&body);
        }
        answer(request.send().await).await
    }

    /// `GET /v1/recordings?before_ms=&before_id=&limit=`: the page after
    /// the row that started at `before_ms` with id `before_id`.
    pub async fn list(&self, before: Option<&(i64, String)>, limit: usize) -> Answer<MeetingList> {
        let mut url = self.endpoint(&["v1", "recordings"]);
        if let Ok(url) = url.as_mut() {
            let mut query = url.query_pairs_mut();
            if let Some((before_ms, before_id)) = before {
                query.append_pair("before_ms", &before_ms.to_string());
                query.append_pair("before_id", before_id);
            }
            query.append_pair("limit", &limit.to_string());
        }
        self.send(reqwest::Method::GET, url, None).await
    }

    /// `GET /v1/recordings/{id}`.
    pub async fn document(&self, recording: &str) -> Answer<MeetingDocument> {
        self.send(reqwest::Method::GET, self.recording(recording, &[]), None)
            .await
    }

    /// `GET /v1/recordings/{id}/turns?offset=`.
    pub async fn turns(&self, recording: &str, offset: u64) -> Answer<TurnsPage> {
        let mut url = self.recording(recording, &["turns"]);
        if let Ok(url) = url.as_mut() {
            url.query_pairs_mut()
                .append_pair("offset", &offset.to_string());
        }
        self.send(reqwest::Method::GET, url, None).await
    }

    /// `PUT /v1/recordings/{id}/speakers`; `None` gives the default back.
    pub async fn rename(
        &self,
        recording: &str,
        key: &str,
        name: Option<&str>,
        scope: orbit_core::notes::SpeakerRenameScope,
    ) -> Answer<Value> {
        let mut body = json!({"names": {key: name}});
        match scope {
            orbit_core::notes::SpeakerRenameScope::Voice => {}
            orbit_core::notes::SpeakerRenameScope::Turn { at_ms } => {
                body["turn"] = json!({"at_ms": at_ms, "speaker_key": key});
            }
        }
        self.send(
            reqwest::Method::PUT,
            self.recording(recording, &["speakers"]),
            Some(body),
        )
        .await
    }

    /// `PUT /v1/recordings/{id}/language`.
    pub async fn language(&self, recording: &str, language: &str) -> Answer<Value> {
        self.send(
            reqwest::Method::PUT,
            self.recording(recording, &["language"]),
            Some(json!({"language": language})),
        )
        .await
    }

    /// `PUT /v1/recordings/{id}/title`; empty gives the meeting back
    /// its own.
    pub async fn title(&self, recording: &str, title: &str) -> Answer<Value> {
        self.send(
            reqwest::Method::PUT,
            self.recording(recording, &["title"]),
            Some(json!({"title": title})),
        )
        .await
    }

    /// `GET /v1/series/previous-meeting?calendar_item_id=&before_ms=`, and
    /// `&title_digest=` for the title's fallback (a hub up to 0.11 ignores it).
    pub async fn previous_meeting(
        &self,
        calendar_item_id: &str,
        before_ms: i64,
        title_digest: Option<&str>,
    ) -> Answer<orbit_core::notes::SeriesPrevious> {
        let mut url = self.endpoint(&["v1", "series", "previous-meeting"]);
        if let Ok(url) = url.as_mut() {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("calendar_item_id", calendar_item_id)
                .append_pair("before_ms", &before_ms.to_string());
            if let Some(digest) = title_digest {
                query.append_pair("title_digest", digest);
            }
        }
        self.send(reqwest::Method::GET, url, None).await
    }

    /// `PUT /v1/recordings/{id}/series`: the series' template and
    /// dictionary, saved together. With `dictionary_ids` (a hub that takes
    /// several), the whole list instead of the one dictionary.
    pub async fn use_for_series(
        &self,
        recording: &str,
        template_id: &str,
        dictionary_id: Option<&str>,
        dictionary_ids: Option<&[String]>,
    ) -> Answer<Value> {
        let body = match dictionary_ids {
            Some(ids) => json!({"template_id": template_id, "dictionary_ids": ids}),
            None => json!({"template_id": template_id, "dictionary_id": dictionary_id}),
        };
        self.send(
            reqwest::Method::PUT,
            self.recording(recording, &["series"]),
            Some(body),
        )
        .await
    }

    /// `PUT /v1/recordings/{id}/private-notes`.
    pub async fn private_notes(
        &self,
        recording: &str,
        text: &str,
        base_revision: u64,
    ) -> Answer<Value> {
        self.send(
            reqwest::Method::PUT,
            self.recording(recording, &["private-notes"]),
            Some(json!({"text": text, "base_revision": base_revision})),
        )
        .await
    }

    /// `POST /v1/recordings/{id}/note`: write it (again) with this template.
    pub async fn write_note(&self, recording: &str, template_id: &str) -> Answer<Value> {
        self.send(
            reqwest::Method::POST,
            self.recording(recording, &["note"]),
            Some(json!({"template_id": template_id})),
        )
        .await
    }

    /// `PUT /v1/recordings/{id}/note/edits`.
    pub async fn edit_note(
        &self,
        recording: &str,
        sections: Option<&[Section]>,
        done: &BTreeMap<String, bool>,
        base_revision: u64,
    ) -> Answer<Value> {
        let mut body = json!({"base_revision": base_revision});
        if let Some(sections) = sections {
            body["sections"] = json!(sections);
        }
        if !done.is_empty() {
            body["done"] = json!(done);
        }
        self.send(
            reqwest::Method::PUT,
            self.recording(recording, &["note", "edits"]),
            Some(body),
        )
        .await
    }

    /// `GET /v1/recordings/{id}/note`: the edits a regeneration set aside
    /// (`superseded_edits`), which the meeting's document does not carry.
    pub async fn note_with_superseded(&self, recording: &str) -> Answer<Value> {
        self.send(
            reqwest::Method::GET,
            self.recording(recording, &["note"]),
            None,
        )
        .await
    }

    /// `POST /v1/recordings/{id}/final-pass`.
    pub async fn final_pass(&self, recording: &str) -> Answer<Value> {
        self.send(
            reqwest::Method::POST,
            self.recording(recording, &["final-pass"]),
            None,
        )
        .await
    }

    /// `POST /v1/recordings/{id}/transcript/retry`.
    pub async fn retry_transcript(&self, recording: &str) -> Answer<Value> {
        self.send(
            reqwest::Method::POST,
            self.recording(recording, &["transcript", "retry"]),
            None,
        )
        .await
    }

    /// `PUT /v1/device/profile`.
    pub async fn profile(&self, owner_name: &str) -> Answer<Value> {
        self.send(
            reqwest::Method::PUT,
            self.endpoint(&["v1", "device", "profile"]),
            Some(json!({"owner_name": owner_name})),
        )
        .await
    }

    /// `GET /v1/note-templates`.
    pub async fn templates(&self) -> Answer<TemplateList> {
        self.send(
            reqwest::Method::GET,
            self.endpoint(&["v1", "note-templates"]),
            None,
        )
        .await
    }

    /// `GET /v1/note-template-links`.
    pub async fn links(&self) -> Answer<TemplateLinks> {
        self.send(
            reqwest::Method::GET,
            self.endpoint(&["v1", "note-template-links"]),
            None,
        )
        .await
    }

    /// `PUT /v1/note-templates/{id}`.
    pub async fn save_template(
        &self,
        template_id: &str,
        draft: &TemplateDraft,
    ) -> Answer<Template> {
        self.send(
            reqwest::Method::PUT,
            self.endpoint(&["v1", "note-templates", template_id]),
            Some(json!(draft)),
        )
        .await
    }

    /// `DELETE /v1/note-templates/{id}`: already gone is done too.
    pub async fn delete_template(&self, template_id: &str) -> Answer<Value> {
        self.send(
            reqwest::Method::DELETE,
            self.endpoint(&["v1", "note-templates", template_id]),
            None,
        )
        .await
    }

    /// `PUT /v1/note-template-links`.
    pub async fn link(&self, calendar_item_id: &str, template_id: &str) -> Answer<Value> {
        self.send(
            reqwest::Method::PUT,
            self.endpoint(&["v1", "note-template-links"]),
            Some(json!({"calendar_item_id": calendar_item_id, "template_id": template_id})),
        )
        .await
    }

    /// `DELETE /v1/note-template-links`: already gone is done too.
    pub async fn unlink(&self, calendar_item_id: &str) -> Answer<Value> {
        self.send(
            reqwest::Method::DELETE,
            self.endpoint(&["v1", "note-template-links"]),
            Some(json!({"calendar_item_id": calendar_item_id})),
        )
        .await
    }
}
