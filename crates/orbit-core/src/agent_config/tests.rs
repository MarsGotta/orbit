#![allow(clippy::expect_used)] // Fixture assertions only.
//! The configuration of an agent as hub-v1 «Agents» fixes it:
//! what the Mac reads, where each order goes and what a refusal carries.
//! Every name and value is invented.
use super::*;
use serde_json::json;

fn config_json() -> serde_json::Value {
    json!({
        "agent_id": "vega",
        "kind": "hermes",
        "personality": {"text": "# Vega\n\nBreve.", "editable": true, "max_bytes": 65536},
        "access": {
            "groups": {"screen": true, "control": false, "calendar": true,
                       "meetings": true, "files": false},
            "editable": false,
            "cause": "Hermes decide las herramientas de este perfil"
        },
        "skills": {
            "editable": true, "max_skills": 64, "max_skill_bytes": 32768,
            "items": [{"name": "resumen-diario", "description": "Resume el día.", "enabled": true}]
        },
        "future_field": 1
    })
}

#[test]
fn the_hub_config_reads_with_each_field_and_its_cause() {
    let config: AgentConfig = serde_json::from_value(config_json()).expect("contract shape");
    assert_eq!(config.agent_id, "vega");
    assert_eq!(config.kind, "hermes");
    assert!(config.personality.editable);
    assert_eq!(config.personality.max_bytes, 65_536);
    assert_eq!(config.personality.cause, None);
    assert!(!config.access.editable);
    assert_eq!(
        config.access.cause.as_deref(),
        Some("Hermes decide las herramientas de este perfil")
    );
    assert_eq!(
        config.access.groups,
        AccessGroups {
            screen: true,
            control: false,
            calendar: true,
            meetings: true,
            files: false,
        }
    );
    assert_eq!(config.skills.max_skills, 64);
    assert_eq!(config.skills.max_skill_bytes, 32_768);
    assert_eq!(config.skills.items[0].name, "resumen-diario");
    assert!(config.skills.items[0].enabled);
}

#[test]
fn a_config_without_a_group_is_not_read_as_that_group_off() {
    // Absence is not a negative fact: a missing group fails the read.
    let mut value = config_json();
    value["access"]["groups"]
        .as_object_mut()
        .expect("groups")
        .remove("files");
    assert!(serde_json::from_value::<AgentConfig>(value).is_err());
}

#[test]
fn each_call_names_its_route_method_and_body() {
    let groups = AccessGroups {
        screen: false,
        control: true,
        calendar: true,
        meetings: false,
        files: true,
    };
    let cases = [
        (
            AgentCall::Config {
                agent_id: "vega".into(),
            },
            HubMethod::Get,
            vec!["v1", "agents", "vega", "config"],
            None,
        ),
        (
            AgentCall::Personality {
                agent_id: "vega".into(),
                text: "Sé breve.".into(),
            },
            HubMethod::Put,
            vec!["v1", "agents", "vega", "personality"],
            Some(json!({"text": "Sé breve."})),
        ),
        (
            AgentCall::Access {
                agent_id: "vega".into(),
                groups,
            },
            HubMethod::Put,
            vec!["v1", "agents", "vega", "access"],
            Some(
                json!({"groups": {"screen": false, "control": true, "calendar": true,
                                   "meetings": false, "files": true}}),
            ),
        ),
        (
            AgentCall::Skill {
                agent_id: "vega".into(),
                name: "resumen-diario".into(),
            },
            HubMethod::Get,
            vec!["v1", "agents", "vega", "skills", "resumen-diario"],
            None,
        ),
        (
            AgentCall::SkillSave {
                agent_id: "vega".into(),
                name: "resumen-diario".into(),
                content: "---\nname: resumen-diario\n---\n".into(),
            },
            HubMethod::Put,
            vec!["v1", "agents", "vega", "skills", "resumen-diario"],
            Some(json!({"content": "---\nname: resumen-diario\n---\n"})),
        ),
        (
            AgentCall::SkillEnabled {
                agent_id: "vega".into(),
                name: "resumen-diario".into(),
                enabled: false,
            },
            HubMethod::Put,
            vec![
                "v1",
                "agents",
                "vega",
                "skills",
                "resumen-diario",
                "enabled",
            ],
            Some(json!({"enabled": false})),
        ),
        (
            AgentCall::SkillDelete {
                agent_id: "vega".into(),
                name: "resumen-diario".into(),
            },
            HubMethod::Delete,
            vec!["v1", "agents", "vega", "skills", "resumen-diario"],
            None,
        ),
    ];
    for (call, method, segments, body) in cases {
        let route = call.route();
        assert_eq!(route.method, method, "{call:?}");
        assert_eq!(route.segments, segments, "{call:?}");
        assert_eq!(route.body, body, "{call:?}");
    }
}

#[test]
fn the_app_sends_each_call_tagged_by_its_name() {
    let call: AgentCall = serde_json::from_value(json!({
        "call": "skill_enabled", "agent_id": "vega", "name": "resumen-diario", "enabled": true
    }))
    .expect("tagged call");
    assert_eq!(
        call,
        AgentCall::SkillEnabled {
            agent_id: "vega".into(),
            name: "resumen-diario".into(),
            enabled: true,
        }
    );
    assert!(
        serde_json::from_value::<AgentCall>(json!({
            "call": "config", "agent_id": "vega", "extra": 1
        }))
        .is_err(),
        "an order with a field the Mac does not know is refused"
    );
}

#[test]
fn a_refusal_keeps_its_code_status_and_scalar_numbers() {
    // The hub's envelope: `code`, a message and the typed `refusal` with
    // its numbers, `retry` beside them.
    let envelope = json!({
        "code": "skill_too_large",
        "message": "La skill ocupa 40000 bytes; el máximo es 32768",
        "refusal": {"code": "skill_too_large", "name": "resumen-diario", "bytes": 40000,
                    "max_bytes": 32768, "retry": {"kind": "after_fix"}}
    });
    let refusal = AgentRefusal::from_envelope(413, &envelope).expect("coded refusal");
    assert_eq!(refusal.code, "skill_too_large");
    assert_eq!(refusal.status, 413);
    assert_eq!(
        refusal.numbers,
        [
            ("bytes".to_owned(), RefusalNumber::Integer(40_000)),
            ("max_bytes".to_owned(), RefusalNumber::Integer(32_768)),
            (
                "name".to_owned(),
                RefusalNumber::Text("resumen-diario".into())
            ),
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn each_refusal_of_the_contract_carries_its_numbers() {
    let cases = [
        (
            404,
            "agent_not_found",
            json!({"agent_id": "orion", "agent_ids": "atlas,vega"}),
        ),
        (
            409,
            "field_not_editable",
            json!({"agent_id": "vega", "field": "access", "kind": "hermes"}),
        ),
        (
            400,
            "skill_invalid",
            json!({"name": "Resumen", "rule": "name_lowercase"}),
        ),
        (404, "skill_not_found", json!({"name": "resumen-diario"})),
        (409, "too_many_skills", json!({"count": 64, "max": 64})),
        (
            413,
            "personality_too_large",
            json!({"bytes": 70000, "max_bytes": 65536}),
        ),
        (
            502,
            "assistant_admin_refused",
            json!({"route": "/api/profiles/vega/soul", "status": 403}),
        ),
    ];
    for (status, code, numbers) in cases {
        let mut refusal = numbers.clone();
        refusal["code"] = json!(code);
        refusal["retry"] = json!({"kind": "never"});
        let envelope = json!({"code": code, "message": "texto", "refusal": refusal});
        let parsed = AgentRefusal::from_envelope(status, &envelope).expect(code);
        assert_eq!(parsed.code, code);
        assert_eq!(parsed.status, status);
        let expected = numbers
            .as_object()
            .expect("numbers")
            .iter()
            .map(|(key, value)| {
                let number = match value {
                    serde_json::Value::String(text) => RefusalNumber::Text(text.clone()),
                    other => RefusalNumber::Integer(other.as_i64().expect("integer")),
                };
                (key.clone(), number)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(parsed.numbers, expected, "{code}");
    }
}

#[test]
fn a_reply_without_a_code_is_not_an_agent_refusal() {
    // An older hub answers an unknown route with FastAPI's `detail`.
    assert_eq!(
        AgentRefusal::from_envelope(404, &json!({"detail": "Not Found"})),
        None
    );
    // A code that is not a code is not echoed.
    assert_eq!(
        AgentRefusal::from_envelope(400, &json!({"code": "Texto libre <b>"})),
        None
    );
}

#[test]
fn lists_and_objects_are_not_numbers() {
    let envelope = json!({
        "code": "skill_invalid",
        "refusal": {"code": "skill_invalid", "name": "x", "rule": "description_empty",
                    "seen": ["a"], "nested": {"a": 1}, "flag": true}
    });
    let refusal = AgentRefusal::from_envelope(400, &envelope).expect("refusal");
    assert_eq!(
        refusal.numbers.get("flag"),
        Some(&RefusalNumber::Flag(true))
    );
    assert!(!refusal.numbers.contains_key("seen"));
    assert!(!refusal.numbers.contains_key("nested"));
}
