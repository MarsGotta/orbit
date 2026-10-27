//! Hub-owned providers. Credentials are write-only and redact Debug output.
use crate::transcription_config::TranscriptionKey;
use serde::{Deserialize, Serialize};

macro_rules! wire_type {
    ($item:item) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[cfg_attr(
            feature = "ts",
            ts(export_to = "../../apps/desktop/src/bridge/generated/")
        )]
        $item
    };
}

wire_type! {
    pub struct AiModel {
        pub id: String,
        pub capabilities: Vec<String>,
    }
}
wire_type! {
    pub struct AiProviderInput {
        pub name: String,
        pub kind: String,
        pub base_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub api_key: Option<TranscriptionKey>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub models: Option<Vec<AiModel>>,
    }
}
wire_type! {
    pub struct AiProvider {
        pub id: String,
        pub name: String,
        pub kind: String,
        pub base_url: Option<String>,
        pub key_configured: bool,
        pub models: Vec<AiModel>,
    }
}
wire_type! {
    pub struct AiChoice {
        pub provider_id: String,
        pub model: String,
    }
}
wire_type! {
    pub struct AiFunction {
        pub id: String,
        pub provider_id: String,
        pub model: String,
        pub capability: String,
        pub privacy: String,
    }
}
wire_type! {
    pub struct AiSettings {
        pub providers: Vec<AiProvider>,
        pub functions: Vec<AiFunction>,
    }
}

type Route = (
    crate::agent_config::HubMethod,
    Vec<String>,
    Option<serde_json::Value>,
);
pub(crate) fn read_route() -> Route {
    (
        crate::agent_config::HubMethod::Get,
        vec!["v1".into(), "settings".into(), "ai".into()],
        None,
    )
}
pub(crate) fn provider_route(id: &str, provider: &AiProviderInput) -> Route {
    let body = serde_json::json!({"name": provider.name, "kind": provider.kind, "base_url": provider.base_url, "api_key": provider.api_key, "models": provider.models});
    save_route("providers", id, body)
}
pub(crate) fn function_route(id: &str, choice: &AiChoice) -> Route {
    save_route(
        "functions",
        id,
        serde_json::json!({"provider_id": choice.provider_id, "model": choice.model}),
    )
}
fn save_route(kind: &str, id: &str, body: serde_json::Value) -> Route {
    (
        crate::agent_config::HubMethod::Put,
        vec![
            "v1".into(),
            "settings".into(),
            "ai".into(),
            kind.into(),
            id.into(),
        ],
        Some(body),
    )
}
