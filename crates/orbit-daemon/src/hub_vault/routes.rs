//! URL construction is confined to fixed routes and canonical hub identities.
use super::*;
use orbit_core::vault::DistillationAction;

pub(super) struct Request {
    pub(super) method: reqwest::Method,
    pub(super) url: reqwest::Url,
    pub(super) body: Option<serde_json::Value>,
}
fn id(value: &str) -> Result<String, Box<Refusal>> {
    if !orbit_core::wire::canonical_uuid(value) {
        return Err(Box::new(Refusal::InvalidParameter {
            parameter: orbit_core::refusal::Parameter::Identifier,
            expected: orbit_core::refusal::Expected::WireType {
                name: "canonical_uuid".into(),
            },
        }));
    }
    Ok(value.into())
}

pub(super) fn request(base: &reqwest::Url, call: &VaultCall) -> Result<Request, Box<Refusal>> {
    use reqwest::Method;
    let prefix = vec!["v1".into(), "vault".into()];
    let proposal = |identity: &str| -> Result<Vec<String>, Box<Refusal>> {
        Ok([prefix.clone(), vec!["distillations".into(), id(identity)?]].concat())
    };
    let (method, path, body) = match call {
        VaultCall::ListDistillations { meeting_id, page } => {
            return list(base, meeting_id.as_deref(), *page);
        }
        VaultCall::Distillation { id } => (Method::GET, proposal(id)?, None),
        VaultCall::PrepareDistillation { meeting_id } => (
            Method::POST,
            vec![
                "v1".into(),
                "recordings".into(),
                id(meeting_id)?,
                "distillations".into(),
            ],
            None,
        ),
        VaultCall::ReviewDistillation { id, review } => (
            Method::PATCH,
            proposal(id)?,
            Some(serde_json::json!(review)),
        ),
        VaultCall::DistillationAction { id, action } => {
            let mut path = proposal(id)?;
            path.push(
                match action {
                    DistillationAction::Apply => "apply",
                    DistillationAction::Discard => "discard",
                    DistillationAction::Undo => "undo",
                    DistillationAction::Prepare => "prepare",
                }
                .into(),
            );
            (Method::POST, path, None)
        }
        VaultCall::Config => (Method::GET, [prefix, vec!["config".into()]].concat(), None),
        VaultCall::Configure { configuration } => (
            Method::PUT,
            [prefix, vec!["config".into()]].concat(),
            Some(serde_json::json!(configuration)),
        ),
        VaultCall::Disconnect => (
            Method::DELETE,
            [prefix, vec!["config".into()]].concat(),
            None,
        ),
        VaultCall::Read { request } => (
            Method::POST,
            [prefix, vec!["read".into()]].concat(),
            Some(serde_json::json!(request)),
        ),
    };
    build(method, base, path, body)
}

fn list(base: &reqwest::Url, meeting: Option<&str>, page: u32) -> Result<Request, Box<Refusal>> {
    if let Some(meeting) = meeting {
        id(meeting)?;
    }
    let mut request = build(
        reqwest::Method::GET,
        base,
        vec!["v1".into(), "vault".into(), "distillations".into()],
        None,
    )?;
    request
        .url
        .query_pairs_mut()
        .append_pair("page", &page.to_string());
    if let Some(meeting) = meeting {
        request
            .url
            .query_pairs_mut()
            .append_pair("meeting_id", meeting);
    }
    Ok(request)
}

fn build(
    method: reqwest::Method,
    base: &reqwest::Url,
    path: Vec<String>,
    body: Option<serde_json::Value>,
) -> Result<Request, Box<Refusal>> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|()| invalid())?
        .pop_if_empty()
        .extend(path);
    Ok(Request { method, url, body })
}
