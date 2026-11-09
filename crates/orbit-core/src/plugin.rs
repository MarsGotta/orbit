//! A plugin call as the device carries it (`contracts/plugin-v1.md`): the
//! call, the shape of the plugin's `id`, and what came of it. The device
//! never interprets a call: `input` and `value` pass through untouched.
use crate::refusal::{Expected, Parameter, Refusal};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub use crate::limits::{PLUGIN_CALL_MAX_BYTES, PLUGIN_ID_MAX_CHARS, PLUGIN_RESPONSE_MAX_BYTES};

/// The request body of `POST /v1/plugins/{id}/call`, exactly two fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCall {
    pub name: String,
    pub input: Map<String, Value>,
}

/// What came of one call: the hub's value, the hub's refusal as it wrote
/// it, or the device's own refusal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PluginOutcome {
    /// The `value` of the hub's `200 {"value": …}`.
    Answered { value: Value },
    /// The hub's status (400 or above) and its body, `op` included.
    Refused {
        status: u16,
        body: Map<String, Value>,
    },
    /// The device's refusal, the `refusal` object alone.
    Failed { refusal: Refusal },
}

/// The local channel's answer to `Command::Plugin`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginResponse {
    pub v: u32,
    pub id: String,
    pub outcome: PluginOutcome,
}

/// `[a-z0-9_]{1,PLUGIN_ID_MAX_CHARS}`, the manifest's `id`. The reserved
/// names are the hub's to refuse (`plugin_not_installed`).
pub fn plugin_id_refusal(id: &str) -> Option<Refusal> {
    let well_formed = (1..=PLUGIN_ID_MAX_CHARS).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    (!well_formed).then(|| Refusal::InvalidParameter {
        parameter: Parameter::Identifier,
        expected: Expected::WireType {
            name: "plugin_id".into(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_lowercase_digits_and_underscores_up_to_the_limit_form_an_id() {
        for id in [
            "vault",
            "a",
            "reading_list_2",
            &"z".repeat(PLUGIN_ID_MAX_CHARS),
        ] {
            assert!(plugin_id_refusal(id).is_none(), "{id:?}");
        }
        let too_long = "a".repeat(PLUGIN_ID_MAX_CHARS + 1);
        for id in ["", too_long.as_str(), "Vault", "a-b", "a/b", "é", " a"] {
            assert!(plugin_id_refusal(id).is_some(), "{id:?}");
        }
    }

    #[test]
    fn an_outcome_says_its_kind_beside_its_fields() -> serde_json::Result<()> {
        let outcome = PluginOutcome::Answered { value: Value::Null };
        assert_eq!(
            serde_json::to_value(&outcome)?,
            serde_json::json!({"outcome": "answered", "value": null})
        );
        let failed = PluginOutcome::Failed {
            refusal: Refusal::NotPaired,
        };
        assert_eq!(
            serde_json::to_value(&failed)?,
            serde_json::json!({"outcome": "failed", "refusal": {"code": "not_paired", "retry": {"kind": "after_fix"}}})
        );
        Ok(())
    }
}
