use super::*;
use orbit_app_core::{
    AppSnapshot, CallPanelSnapshot, DaemonConnection,
    features::{FeatureFrame, FeatureInput, FrameRevision, project_all},
    topics::TopicCache,
};

fn frames() -> Vec<FeatureFrame> {
    let snapshot = AppSnapshot::empty(1, DaemonConnection::Ready);
    let topics = TopicCache::default();
    project_all(
        &FeatureInput::new(&snapshot, &topics),
        FrameRevision {
            epoch: 1,
            counter: 1,
        },
    )
}

fn recording(panel: Option<CallPanelSnapshot>) -> Option<FeatureFrame> {
    frames().into_iter().find_map(|frame| match frame {
        FeatureFrame::Recording {
            revision,
            mut view,
            availability,
        } => {
            view.meeting.call_panel = panel.clone();
            Some(FeatureFrame::Recording {
                revision,
                view,
                availability,
            })
        }
        _ => None,
    })
}

#[test]
fn the_recording_frame_shows_the_panel_only_with_something_to_say() {
    let offer = recording(Some(CallPanelSnapshot::Offer {
        call_id: "call-1".into(),
        application: "zoom.us".into(),
        event_title: None,
    }));
    assert_eq!(offer.as_ref().and_then(panel_visible), Some(true));
    let quiet = recording(None);
    assert_eq!(quiet.as_ref().and_then(panel_visible), Some(false));
    for frame in frames() {
        if !matches!(frame, FeatureFrame::Recording { .. }) {
            assert_eq!(panel_visible(&frame), None, "{:?}", frame.feature());
        }
    }
}

#[test]
fn the_panel_sits_at_the_top_right_of_its_monitor() {
    // A 1512-point-wide Retina display at 2x, the panel 360 points wide and
    // 16 points in from the top and right edges.
    assert_eq!(
        top_right(0, 0, 3024, 2.0, 360.0, 16.0),
        Placement {
            x: 3024 - 720 - 32,
            y: 32
        }
    );
    // A second monitor to the right of the first, at 1x.
    assert_eq!(
        top_right(1920, 0, 2560, 1.0, 360.0, 16.0),
        Placement {
            x: 1920 + 2560 - 360 - 16,
            y: 16
        }
    );
}

#[test]
fn window_capabilities_keep_the_panel_command_boundary() -> Result<(), Box<dyn std::error::Error>> {
    let capability: serde_json::Value =
        serde_json::from_str(include_str!("../capabilities/panel.json"))?;
    assert_eq!(capability["identifier"], PANEL_LABEL);
    assert_eq!(capability["windows"], serde_json::json!([PANEL_LABEL]));
    assert_eq!(
        capability["permissions"],
        serde_json::json!([
            "core:event:allow-listen",
            "core:event:allow-unlisten",
            "allow-orbit-snapshot",
            "allow-orbit-panel-send",
            "allow-orbit-report"
        ]),
        "the panel listens to its frames, answers a call and records its own failures; \
         it emits nothing"
    );
    let main: serde_json::Value =
        serde_json::from_str(include_str!("../capabilities/default.json"))?;
    assert_eq!(main["windows"], serde_json::json!(["main"]));
    let permissions = main["permissions"].as_array().ok_or("main permissions")?;
    assert!(
        !permissions.iter().any(|permission| {
            permission
                .as_str()
                .or_else(|| permission["identifier"].as_str())
                == Some("allow-orbit-panel-send")
        }),
        "the main window must not invoke panel-only orders: {permissions:?}"
    );
    Ok(())
}

#[test]
fn panel_accepts_its_recording_orders_and_explicit_focus_only()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::commands::PanelOrder;
    for value in [
        serde_json::json!({"command":"call_record","call_id":"c"}),
        serde_json::json!({"command":"call_dismiss","call_id":"c"}),
        serde_json::json!({"command":"recording_keep","recording_id":"r"}),
        serde_json::json!({"command":"meeting_finish"}),
        serde_json::json!({"command":"meeting_ask","question":"¿Qué dijo?","minutes":5}),
        serde_json::json!({"command":"meeting_question_close","id":"q"}),
    ] {
        let command: orbit_core::protocol::Command = serde_json::from_value(value.clone())?;
        let actual: PanelOrder = serde_json::from_value(value)?;
        assert_eq!(
            serde_json::to_value(actual)?,
            serde_json::to_value(command)?
        );
    }
    for value in [
        serde_json::json!({"command":"meeting_start","event_id":"e"}),
        serde_json::json!({"command":"meeting_pause"}),
        serde_json::json!({"command":"meeting_resume"}),
        serde_json::json!({"command":"call_preference","preference":"nothing"}),
        serde_json::json!({"command":"audio_sources_refresh"}),
        serde_json::json!({"command":"privacy_stop"}),
        serde_json::json!({"command":"chat","text":"hola"}),
        serde_json::json!({"command":"call_record","call_id":"c","extra":true}),
    ] {
        assert!(
            serde_json::from_value::<PanelOrder>(value.clone()).is_err(),
            "{value}"
        );
    }
    let ask: crate::commands::PanelOrder = serde_json::from_value(serde_json::json!({
        "command": "meeting_ask", "question": "¿Qué dijo Lucía?", "minutes": 5
    }))?;
    assert!(matches!(
        ask,
        crate::commands::PanelOrder::MeetingAsk { ref question, minutes: 5 } if question == "¿Qué dijo Lucía?"
    ));
    let close: crate::commands::PanelOrder = serde_json::from_value(
        serde_json::json!({"command": "meeting_question_close", "id": "turn-1"}),
    )?;
    assert!(matches!(
        close,
        crate::commands::PanelOrder::MeetingQuestionClose { ref id } if id == "turn-1"
    ));
    let focus: crate::commands::PanelOrder =
        serde_json::from_value(serde_json::json!({"command": "panel_focus", "focus": true}))?;
    assert!(matches!(
        focus,
        crate::commands::PanelOrder::PanelFocus { focus: true }
    ));
    for refused in [
        serde_json::json!({"command": "chat", "text": "hola"}),
        serde_json::json!({"command": "meeting_ask", "question": "x", "minutes": 2, "extra": 1}),
        serde_json::json!({"command": "panel_focus"}),
    ] {
        assert!(
            serde_json::from_value::<crate::commands::PanelOrder>(refused.clone()).is_err(),
            "{refused}"
        );
    }
    Ok(())
}

fn recording_with(
    panel: Option<CallPanelSnapshot>,
    question: Option<orbit_app_core::MeetingQuestionSnapshot>,
) -> Option<FeatureFrame> {
    recording(panel).map(|frame| match frame {
        FeatureFrame::Recording {
            revision,
            mut view,
            availability,
        } => {
            view.meeting.question = question.clone();
            FeatureFrame::Recording {
                revision,
                view,
                availability,
            }
        }
        other => other,
    })
}

fn thinking() -> orbit_app_core::MeetingQuestionSnapshot {
    orbit_app_core::MeetingQuestionSnapshot {
        id: "turn-1".into(),
        question: "¿Qué dijo Lucía?".into(),
        minutes: 2,
        origin: orbit_app_core::MeetingQuestionOrigin::Voice,
        stage: orbit_app_core::MeetingQuestionStage::Thinking,
        answer: None,
        cause: None,
        shown_until_ms: None,
        sent: true,
    }
}

/// A question to Deimos shows the panel, taller, until the daemon
/// lets it go (45 s after the answer, 8 s after a failure) or the user closes it.
#[test]
fn a_question_to_deimos_shows_the_panel_with_room_for_the_answer() {
    let asked = recording_with(None, Some(thinking()));
    assert_eq!(asked.as_ref().and_then(panel_visible), Some(true));
    assert_eq!(
        asked.as_ref().and_then(panel_height),
        Some(PANEL_ANSWER_HEIGHT)
    );
    let call = recording_with(
        Some(CallPanelSnapshot::Offer {
            call_id: "call-1".into(),
            application: "zoom.us".into(),
            event_title: None,
        }),
        None,
    );
    assert_eq!(call.as_ref().and_then(panel_height), Some(PANEL_HEIGHT));
    let quiet = recording_with(None, None);
    assert_eq!(quiet.as_ref().and_then(panel_visible), Some(false));
}

/// The answer's links open through the panel's own closed order, only
/// for http(s), with the same check as the main window's `open_url`.
#[test]
fn the_panel_opens_only_web_links_from_an_answer() -> Result<(), Box<dyn std::error::Error>> {
    let open: crate::commands::PanelOrder = serde_json::from_value(
        serde_json::json!({"command": "open_url", "url": "https://acme.example/acta"}),
    )?;
    assert!(matches!(
        open,
        crate::commands::PanelOrder::OpenUrl { ref url }
            if url == "https://acme.example/acta"
    ));
    use crate::commands::{WebUrlRefusal, web_url};
    for accepted in [
        "https://acme.example/acta",
        "http://intranet.local/x",
        "https://docs.google.com/document/d/1?usp=sharing#heading=h.1",
        "https://acme.example:8443/acta",
    ] {
        assert!(
            web_url(accepted).is_ok(),
            "{accepted}: {:?}",
            web_url(accepted)
        );
    }
    for (refused, expected) in [
        (
            "file:///etc/passwd",
            WebUrlRefusal::Scheme {
                scheme: "file".into(),
            },
        ),
        (
            "javascript:alert(1)",
            WebUrlRefusal::Scheme {
                scheme: "javascript".into(),
            },
        ),
        (
            "obsidian://open?vault=x",
            WebUrlRefusal::Scheme {
                scheme: "obsidian".into(),
            },
        ),
        (
            "mailto:lucia@example.com",
            WebUrlRefusal::Scheme {
                scheme: "mailto".into(),
            },
        ),
        (
            "ftp://acme.example",
            WebUrlRefusal::Scheme {
                scheme: "ftp".into(),
            },
        ),
        (
            "https://usuario:clave@acme.example/acta",
            WebUrlRefusal::Credentials,
        ),
        (
            "https://acme.com@malo.example/acta",
            WebUrlRefusal::Credentials,
        ),
        ("https://:clave@acme.example/", WebUrlRefusal::Credentials),
    ] {
        assert_eq!(web_url(refused).err(), Some(expected), "{refused}");
    }
    for unparsable in ["", "acta", "https://", "https:///"] {
        assert!(
            matches!(
                web_url(unparsable),
                Err(WebUrlRefusal::Unparsable { .. } | WebUrlRefusal::NoHost)
            ),
            "{unparsable}: {:?}",
            web_url(unparsable)
        );
    }
    // Each refusal says its cause.
    assert!(
        WebUrlRefusal::Credentials.to_string().contains("usuario"),
        "{}",
        WebUrlRefusal::Credentials
    );
    assert!(
        WebUrlRefusal::Scheme {
            scheme: "file".into()
        }
        .to_string()
        .contains("file")
    );
    assert!(
        serde_json::from_value::<crate::commands::PanelOrder>(
            serde_json::json!({"command": "open_url", "url": "https://x", "extra": 1})
        )
        .is_err()
    );
    Ok(())
}

/// The opener's injected click handler uses IPC even without the npm package.
#[test]
fn main_window_web_links_allow_http_and_https() -> Result<(), Box<dyn std::error::Error>> {
    let capability: serde_json::Value =
        serde_json::from_str(include_str!("../capabilities/default.json"))?;
    let permissions = capability["permissions"]
        .as_array()
        .ok_or("default.json has no permissions")?;
    let opener = permissions
        .iter()
        .find(|permission| permission["identifier"] == "opener:allow-open-url")
        .ok_or("main window lacks scoped opener:allow-open-url for web links")?;
    let allowed = opener["allow"]
        .as_array()
        .ok_or("opener:allow-open-url has no allowed URLs")?;
    for url in ["https://*", "http://*"] {
        assert!(
            allowed.iter().any(|entry| entry["url"] == url),
            "main window web links lack {url}"
        );
    }
    Ok(())
}
