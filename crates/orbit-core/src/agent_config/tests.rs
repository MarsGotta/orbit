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
            403,
            "skill_delete_unsupported",
            json!({"agent_id": "vega", "kind": "hermes"}),
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

#[test]
fn each_assistant_call_names_its_route_method_and_body() {
    let groups = AccessGroups {
        screen: true,
        control: false,
        calendar: true,
        meetings: true,
        files: true,
    };
    let cases = [
        (
            AgentCall::Profile {
                agent_id: "vega".into(),
                name: "Nova".into(),
                description: "Programa conmigo".into(),
                color: "c2".into(),
                primary: true,
            },
            HubMethod::Put,
            vec!["v1", "agents", "vega", "profile"],
            Some(
                json!({"name": "Nova", "description": "Programa conmigo", "color": "c2",
                        "primary": true}),
            ),
        ),
        (
            AgentCall::Add {
                service: "codex".into(),
                name: "Nova".into(),
                description: String::new(),
                color: "c3".into(),
                access: groups,
            },
            HubMethod::Post,
            vec!["v1", "agents"],
            Some(
                json!({"service": "codex", "name": "Nova", "description": "", "color": "c3",
                        "access": {"screen": true, "control": false, "calendar": true,
                                   "meetings": true, "files": true}}),
            ),
        ),
        (
            AgentCall::Remove {
                agent_id: "codex-1a2b3c4d".into(),
            },
            HubMethod::Delete,
            vec!["v1", "agents", "codex-1a2b3c4d"],
            None,
        ),
        (
            AgentCall::SignIn {
                agent_id: "vega".into(),
            },
            HubMethod::Post,
            vec!["v1", "agents", "vega", "sign-in"],
            None,
        ),
        (
            AgentCall::SignInStatus {
                agent_id: "vega".into(),
            },
            HubMethod::Get,
            vec!["v1", "agents", "vega", "sign-in"],
            None,
        ),
        (
            AgentCall::SignInCode {
                agent_id: "vega".into(),
                code: "codigo#estado".into(),
            },
            HubMethod::Post,
            vec!["v1", "agents", "vega", "sign-in", "code"],
            Some(json!({"code": "codigo#estado"})),
        ),
        (
            AgentCall::SignInCancel {
                agent_id: "vega".into(),
            },
            HubMethod::Delete,
            vec!["v1", "agents", "vega", "sign-in"],
            None,
        ),
        (
            AgentCall::SignOut {
                agent_id: "vega".into(),
            },
            HubMethod::Post,
            vec!["v1", "agents", "vega", "sign-out"],
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
fn a_sign_in_reads_with_its_address_code_and_outcome() {
    let waiting: SignInAttempt = serde_json::from_value(json!({
        "agent_id": "codex-1a2b3c4d", "state": "waiting", "expires_at_ms": 1_790_000_000_000_i64,
        "expects_code": false, "url": "https://auth.example.test/codex/device",
        "code": "ABCD-1234", "future_field": true
    }))
    .expect("contract shape");
    assert_eq!(waiting.state, SignInState::Waiting);
    assert_eq!(waiting.code.as_deref(), Some("ABCD-1234"));
    assert_eq!(waiting.expires_at_ms, Some(1_790_000_000_000));
    let failed: SignInAttempt = serde_json::from_value(json!({
        "agent_id": "codex-1a2b3c4d", "state": "failed", "cause": "device_code_disabled",
        "exit_status": 1
    }))
    .expect("contract shape");
    assert_eq!(failed.state, SignInState::Failed);
    assert_eq!(failed.cause.as_deref(), Some("device_code_disabled"));
    assert_eq!(failed.exit_status, Some(1));
    assert!(!failed.expects_code);
    // A state a newer hub adds is not read as any of these.
    let newer: SignInAttempt =
        serde_json::from_value(json!({"agent_id": "vega", "state": "paused"})).expect("tolerant");
    assert_eq!(newer.state, SignInState::Unknown);
}

#[test]
fn workspace_is_an_optional_hub_path_and_sent_unchanged() {
    let mut value = config_json();
    let old: AgentConfig = serde_json::from_value(value.clone()).expect("older hub");
    assert_eq!(old.working_folder, None);
    value["working_folder"] = json!("/home/example/project");
    let new: AgentConfig = serde_json::from_value(value).expect("workspace");
    assert_eq!(new.working_folder.as_deref(), Some("/home/example/project"));
    let call = AgentCall::WorkingFolder {
        agent_id: "vega".into(),
        path: "/home/example/project".into(),
    };
    let route = call.route();
    assert_eq!(route.method, HubMethod::Put);
    assert_eq!(route.segments, ["v1", "agents", "vega", "working-folder"]);
    assert_eq!(route.body, Some(json!({"path": "/home/example/project"})));
}
