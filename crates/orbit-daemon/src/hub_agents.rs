//! HTTP adapter for an agent's configuration on the hub (hub-v1
//! «Agents»). One request per call, with the device's credential; the route
//! and the reading of a refusal are `orbit_core::agent_config`'s. No retries,
//! and no personality, skill or refusal text in logs.
use orbit_core::{
    agent_config::{AgentCall, AgentOutcome, AgentRefusal, HubMethod},
    limits::MAX_HUB_HTTP_BODY_BYTES,
    refusal::{Refusal, Resource, Service},
};

#[derive(Clone)]
pub struct AgentHub {
    pub base: reqwest::Url,
    pub token: String,
    /// The op of the order this call belongs to.
    pub op: orbit_core::notice::Op,
}

impl AgentHub {
    pub async fn call(&self, call: &AgentCall) -> AgentOutcome {
        match self.exchange(call).await {
            Ok(outcome) => outcome,
            Err(outcome) => *outcome,
        }
    }

    /// `Err` is every outcome that ends the call early, boxed: a refusal is
    /// as large as the answer it replaces.
    async fn exchange(&self, call: &AgentCall) -> Result<AgentOutcome, Box<AgentOutcome>> {
        let response = self.send(call).await?;
        if !response.status().is_success() {
            return Err(refusal(response).await);
        }
        let Some(answer) = Answer::of(call) else {
            return Ok(AgentOutcome::Done);
        };
        let bytes = crate::hub::bounded_body(response, MAX_HUB_HTTP_BODY_BYTES)
            .await
            .map_err(|error| {
                failed(match error {
                    crate::hub::BodyError::TooLarge(size) => Refusal::TooLarge {
                        resource: Resource::HubResponse,
                        bytes: size.bytes,
                        max_bytes: size.max_bytes,
                    },
                    crate::hub::BodyError::Cut { received_bytes, .. } => Refusal::HubReplyInvalid {
                        field: "body".into(),
                        received_bytes: Some(received_bytes as u64),
                    },
                })
            })?;
        answer.read(&bytes)
    }

    /// The call's request, sent to the hub on its route.
    async fn send(&self, call: &AgentCall) -> Result<reqwest::Response, Box<AgentOutcome>> {
        let route = call.route();
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|()| internal("The hub URL cannot contain paths"))?
            .pop_if_empty()
            // Each segment is percent-encoded: a name is never a path.
            .extend(&route.segments);
        let method = match route.method {
            HubMethod::Get => reqwest::Method::GET,
            HubMethod::Put => reqwest::Method::PUT,
            HubMethod::Post => reqwest::Method::POST,
            HubMethod::Delete => reqwest::Method::DELETE,
        };
        let mut request = crate::hub::request(method, url, Some(&self.token), &self.op)
            .map_err(|_| internal("Could not prepare the HTTP client"))?;
        if let Some(body) = &route.body {
            request = request.json(body);
        }
        request.send().await.map_err(|error| {
            failed(match crate::hub::TransportCause::of(&error) {
                // Nothing reached the hub.
                crate::hub::TransportCause::ConnectionTimeout
                | crate::hub::TransportCause::Connection => {
                    Refusal::HubUnreachable { since_ms: None }
                }
                // Sent, and no answer: an order may have been applied, so
                // it is not called unreachable.
                crate::hub::TransportCause::Timeout | crate::hub::TransportCause::Cut => {
                    Refusal::UpstreamFailed {
                        service: Service::Hub,
                        status: None,
                    }
                }
            })
        })
    }
}

/// A hub refusal is carried whole whatever its status: a Hermes refusal
/// arrives as a 5xx with its route and status. Without a code (an older hub
/// without these routes, a proxy) only the status is known.
async fn refusal(response: reqwest::Response) -> Box<AgentOutcome> {
    let status = response.status().as_u16();
    let envelope = crate::hub::json_body(response, crate::hub_notes::MAX_ERROR_BYTES).await;
    match envelope
        .ok()
        .and_then(|envelope| AgentRefusal::from_envelope(status, &envelope))
    {
        Some(refusal) => Box::new(AgentOutcome::Refused { refusal }),
        None => failed(Refusal::UpstreamFailed {
            service: Service::Hub,
            status: Some(status),
        }),
    }
}

/// What an accepted call answers that the Mac reads.
enum Answer {
    AiSettings,
    TranscriptionSettings,
    Config,
    Skill,
    Added,
    SignIn,
}

impl Answer {
    /// What the Mac reads from an accepted call; `None` for an order that
    /// answers with nothing it reads: it is done, and the screen reads again.
    fn of(call: &AgentCall) -> Option<Self> {
        match call {
            AgentCall::AiSettings
            | AgentCall::AiProviderSave { .. }
            | AgentCall::AiFunctionSave { .. } => Some(Self::AiSettings),
            AgentCall::TranscriptionSettings | AgentCall::TranscriptionSave { .. } => {
                Some(Self::TranscriptionSettings)
            }
            AgentCall::Config { .. } => Some(Self::Config),
            AgentCall::Skill { .. } => Some(Self::Skill),
            AgentCall::Add { .. } => Some(Self::Added),
            AgentCall::SignIn { .. }
            | AgentCall::SignInStatus { .. }
            | AgentCall::SignInCode { .. }
            | AgentCall::SignInCancel { .. } => Some(Self::SignIn),
            AgentCall::WorkingFolder { .. }
            | AgentCall::Personality { .. }
            | AgentCall::Access { .. }
            | AgentCall::SkillSave { .. }
            | AgentCall::SkillEnabled { .. }
            | AgentCall::SkillDelete { .. }
            | AgentCall::Profile { .. }
            | AgentCall::Remove { .. }
            | AgentCall::SignOut { .. } => None,
        }
    }

    fn read(self, bytes: &[u8]) -> Result<AgentOutcome, Box<AgentOutcome>> {
        let outcome = match self {
            Self::AiSettings => {
                serde_json::from_slice(bytes).map(|settings| AgentOutcome::AiSettings { settings })
            }
            Self::TranscriptionSettings => serde_json::from_slice(bytes)
                .map(|settings| AgentOutcome::TranscriptionSettings { settings }),
            Self::Config => {
                serde_json::from_slice(bytes).map(|config| AgentOutcome::Config { config })
            }
            Self::Skill => serde_json::from_slice(bytes).map(|skill| AgentOutcome::Skill { skill }),
            Self::Added => {
                serde_json::from_slice::<Added>(bytes).map(|added| AgentOutcome::Added {
                    agent_id: added.agent_id,
                })
            }
            Self::SignIn => {
                serde_json::from_slice(bytes).map(|attempt| AgentOutcome::SignIn { attempt })
            }
        };
        outcome.map_err(|_| invalid("body"))
    }
}

/// What `POST /v1/agents` answers that the Mac reads: the id the hub made.
/// The whole list beside it arrives as `agents.changed`.
#[derive(serde::Deserialize)]
struct Added {
    agent_id: String,
}

fn failed(refusal: Refusal) -> Box<AgentOutcome> {
    Box::new(AgentOutcome::Failed { refusal })
}

/// The Mac could not even make the request; never the hub's fault.
fn internal(error: &str) -> Box<AgentOutcome> {
    failed(Refusal::Internal {
        cause: orbit_core::refusal::InternalCause::DaemonOperation {
            error: error.into(),
        },
    })
}

fn invalid(field: &str) -> Box<AgentOutcome> {
    failed(Refusal::HubReplyInvalid {
        field: field.into(),
        received_bytes: None,
    })
}
