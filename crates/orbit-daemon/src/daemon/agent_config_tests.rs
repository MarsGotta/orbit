//! Ajustes › Agentes: each call reaches its hub route with the
//! device's credential, and what the hub answers or refuses comes back whole.
//! Every hub is a one-shot loopback fixture; every value is invented.
use super::pairing_session_tests::{json_response, one_shot_http, online_with_hub, recorded};
use super::*;
use crate::daemon::audio::tests::fixture;
use orbit_core::agent_config::{
    AccessGroups, AgentCall, AgentOutcome, AgentRefusal, RefusalNumber,
};
use orbit_core::refusal::{Refusal, Service};
use serde_json::json;

const CONFIG: &str = r#"{"agent_id":"vega","kind":"acp",
 "personality":{"text":"Sé breve.","editable":true,"max_bytes":65536},
 "access":{"groups":{"screen":true,"control":true,"calendar":true,"meetings":true,"files":true},
   "editable":true},
 "skills":{"editable":false,"cause":"Las skills de este agente viven en su carpeta",
   "max_skills":64,"max_skill_bytes":32768,
   "items":[{"name":"resumen-diario","description":"Resume el día.","enabled":false}]}}"#;

const SKILL: &str = r#"{"name":"resumen-diario","description":"Resume el día.","enabled":true,
 "content":"---\nname: resumen-diario\ndescription: Resume el día.\n---\n\nPasos."}"#;

/// One call against a hub that answers `response`, and what reached it.
async fn exchange(
    call: AgentCall,
    response: Vec<u8>,
) -> Result<(AgentOutcome, super::pairing_session_tests::HttpRequest)> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let (base, seen) = one_shot_http(response).await?;
    online_with_hub(&mut runtime, &base);
    let outcome = runtime.agent_call(call).await;
    Ok((outcome, recorded(&seen)?))
}

#[tokio::test]
async fn each_call_reaches_its_route_with_the_device_credential() -> Result<()> {
    let groups = AccessGroups {
        screen: false,
        control: false,
        calendar: true,
        meetings: true,
        files: false,
    };
    let orders = [
        (
            AgentCall::Personality {
                agent_id: "vega".into(),
                text: "Sé breve.".into(),
            },
            "PUT /v1/agents/vega/personality HTTP/1.1",
            Some(json!({"text": "Sé breve."})),
        ),
        (
            AgentCall::Access {
                agent_id: "vega".into(),
                groups,
            },
            "PUT /v1/agents/vega/access HTTP/1.1",
            Some(
                json!({"groups": {"screen": false, "control": false, "calendar": true,
                                   "meetings": true, "files": false}}),
            ),
        ),
        (
            AgentCall::SkillSave {
                agent_id: "vega".into(),
                name: "resumen-diario".into(),
                content: "---\nname: resumen-diario\n---\n".into(),
            },
            "PUT /v1/agents/vega/skills/resumen-diario HTTP/1.1",
            Some(json!({"content": "---\nname: resumen-diario\n---\n"})),
        ),
        (
            AgentCall::SkillEnabled {
                agent_id: "vega".into(),
                name: "resumen-diario".into(),
                enabled: true,
            },
            "PUT /v1/agents/vega/skills/resumen-diario/enabled HTTP/1.1",
            Some(json!({"enabled": true})),
        ),
        (
            AgentCall::SkillDelete {
                agent_id: "vega".into(),
                name: "resumen-diario".into(),
            },
            "DELETE /v1/agents/vega/skills/resumen-diario HTTP/1.1",
            None,
        ),
    ];
    for (call, start_line, body) in orders {
        let (outcome, wire) =
            exchange(call.clone(), json_response("200 OK", r#"{"ok":true}"#)).await?;
        assert_eq!(outcome, AgentOutcome::Done, "{call:?}");
        assert_eq!(wire.start_line, start_line);
        assert_eq!(
            wire.authorization.as_deref(),
            Some("bearer synthetic-not-a-credential")
        );
        match body {
            Some(body) => assert_eq!(serde_json::from_str::<Value>(&wire.body)?, body),
            None => assert_eq!(wire.body, ""),
        }
    }
    Ok(())
}

#[tokio::test]
async fn the_config_and_a_skill_come_back_as_the_hub_wrote_them() -> Result<()> {
    let (outcome, wire) = exchange(
        AgentCall::Config {
            agent_id: "vega".into(),
        },
        json_response("200 OK", CONFIG),
    )
    .await?;
    assert_eq!(wire.start_line, "GET /v1/agents/vega/config HTTP/1.1");
    let AgentOutcome::Config { config } = outcome else {
        panic!("expected the config, got {outcome:?}")
    };
    assert_eq!(config.kind, "acp");
    assert!(!config.skills.editable);
    assert_eq!(
        config.skills.cause.as_deref(),
        Some("Las skills de este agente viven en su carpeta")
    );
    let (outcome, wire) = exchange(
        AgentCall::Skill {
            agent_id: "vega".into(),
            name: "resumen-diario".into(),
        },
        json_response("200 OK", SKILL),
    )
    .await?;
    assert_eq!(
        wire.start_line,
        "GET /v1/agents/vega/skills/resumen-diario HTTP/1.1"
    );
    let AgentOutcome::Skill { skill } = outcome else {
        panic!("expected the skill, got {outcome:?}")
    };
    assert!(skill.content.ends_with("Pasos."));
    Ok(())
}

#[tokio::test]
async fn a_name_is_one_path_segment_never_a_path() -> Result<()> {
    let (_, wire) = exchange(
        AgentCall::SkillDelete {
            agent_id: "vega".into(),
            name: "../config".into(),
        },
        json_response("200 OK", "{}"),
    )
    .await?;
    assert_eq!(
        wire.start_line,
        "DELETE /v1/agents/vega/skills/..%2Fconfig HTTP/1.1"
    );
    Ok(())
}

#[tokio::test]
async fn a_hub_refusal_reaches_the_screen_with_its_numbers() -> Result<()> {
    let (outcome, _) = exchange(
        AgentCall::SkillSave {
            agent_id: "vega".into(),
            name: "resumen-diario".into(),
            content: "x".into(),
        },
        json_response(
            "413 Payload Too Large",
            r#"{"code":"skill_too_large","message":"La skill ocupa 40000 bytes",
                "refusal":{"code":"skill_too_large","name":"resumen-diario","bytes":40000,
                           "max_bytes":32768,"retry":{"kind":"after_fix"}}}"#,
        ),
    )
    .await?;
    assert_eq!(
        outcome,
        AgentOutcome::Refused {
            refusal: AgentRefusal {
                code: "skill_too_large".into(),
                status: 413,
                numbers: [
                    ("bytes".to_owned(), RefusalNumber::Integer(40_000)),
                    ("max_bytes".to_owned(), RefusalNumber::Integer(32_768)),
                    (
                        "name".to_owned(),
                        RefusalNumber::Text("resumen-diario".into())
                    ),
                ]
                .into_iter()
                .collect(),
            }
        }
    );
    Ok(())
}

#[tokio::test]
async fn hermes_refusing_its_panel_keeps_route_and_status_even_on_a_5xx() -> Result<()> {
    let (outcome, _) = exchange(
        AgentCall::Personality {
            agent_id: "atlas".into(),
            text: "Hola".into(),
        },
        json_response(
            "502 Bad Gateway",
            r#"{"code":"assistant_admin_refused","message":"Hermes respondió 403",
                "refusal":{"code":"assistant_admin_refused",
                           "route":"/api/profiles/atlas/soul","status":403}}"#,
        ),
    )
    .await?;
    let AgentOutcome::Refused { refusal } = outcome else {
        panic!("expected the hub's refusal, got {outcome:?}")
    };
    assert_eq!(refusal.code, "assistant_admin_refused");
    assert_eq!(refusal.status, 502);
    assert_eq!(
        refusal.numbers.get("route"),
        Some(&RefusalNumber::Text("/api/profiles/atlas/soul".into()))
    );
    assert_eq!(
        refusal.numbers.get("status"),
        Some(&RefusalNumber::Integer(403))
    );
    Ok(())
}

#[tokio::test]
async fn an_older_hub_without_the_route_says_its_status_not_a_refusal() -> Result<()> {
    let (outcome, _) = exchange(
        AgentCall::Config {
            agent_id: "vega".into(),
        },
        json_response("404 Not Found", r#"{"detail":"Not Found"}"#),
    )
    .await?;
    assert_eq!(
        outcome,
        AgentOutcome::Failed {
            refusal: Refusal::UpstreamFailed {
                service: Service::Hub,
                status: Some(404),
            }
        }
    );
    Ok(())
}

#[tokio::test]
async fn a_config_out_of_contract_is_an_invalid_reply_not_an_empty_one() -> Result<()> {
    let (outcome, _) = exchange(
        AgentCall::Config {
            agent_id: "vega".into(),
        },
        json_response("200 OK", r#"{"agent_id":"vega","kind":"acp"}"#),
    )
    .await?;
    assert!(
        matches!(
            outcome,
            AgentOutcome::Failed {
                refusal: Refusal::HubReplyInvalid { .. }
            }
        ),
        "{outcome:?}"
    );
    Ok(())
}

#[tokio::test]
async fn without_pairing_or_with_the_privacy_stop_nothing_is_asked() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let call = AgentCall::Config {
        agent_id: "vega".into(),
    };
    assert_eq!(
        runtime.agent_call(call.clone()).await,
        AgentOutcome::Failed {
            refusal: Refusal::NotPaired
        }
    );
    let base = reqwest::Url::parse("http://127.0.0.1:9")?;
    online_with_hub(&mut runtime, &base);
    runtime.apply(Event::PrivacyStop).await?;
    assert!(matches!(
        runtime.agent_call(call).await,
        AgentOutcome::Failed {
            refusal: Refusal::PrivacyStopped { .. }
        }
    ));
    Ok(())
}

#[tokio::test]
async fn a_hub_that_hangs_up_after_the_order_is_not_called_unreachable() -> Result<()> {
    // The order may have been applied: the cause says the answer is missing,
    // not that the hub could not be reached.
    let (outcome, wire) = exchange(
        AgentCall::Access {
            agent_id: "vega".into(),
            groups: AccessGroups {
                screen: true,
                control: true,
                calendar: true,
                meetings: true,
                files: true,
            },
        },
        Vec::new(),
    )
    .await?;
    assert_eq!(wire.start_line, "PUT /v1/agents/vega/access HTTP/1.1");
    assert_eq!(
        outcome,
        AgentOutcome::Failed {
            refusal: Refusal::UpstreamFailed {
                service: Service::Hub,
                status: None,
            }
        }
    );
    Ok(())
}
