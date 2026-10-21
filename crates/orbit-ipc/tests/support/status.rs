use orbit_core::protocol::Command;
use serde_json::Value;
use std::path::Path;

/// Collect the same public facts the app reads, without the retired State wire.
pub async fn status_async(path: &Path) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    use orbit_core::topics::TopicId;
    let paths = orbit_ipc::Paths::resolve(Some(path.to_owned()))?;
    let response = orbit_ipc::request(&paths, Command::Status).await?;
    let status = response.status.ok_or("status omitted startup facts")?;
    let mut result = serde_json::to_value(&status)?;
    if status.recovery != orbit_core::RecoveryStatus::Ready {
        return Ok(result);
    }
    let ids = vec![
        TopicId::Status,
        TopicId::Permissions,
        TopicId::Recording,
        TopicId::Assistant,
        TopicId::Notices,
    ];
    let (_, mut subscription) = orbit_ipc::subscribe_topics(&paths, ids.clone()).await?;
    let mut values = std::collections::BTreeMap::new();
    while values.len() < ids.len() {
        let frame = subscription.next_topic().await?;
        if frame.revision > 0 {
            values.insert(frame.id(), serde_json::to_value(frame.data)?);
        }
    }
    let status = &values[&TopicId::Status]["knowledge"]["value"];
    result["connection"] = status["hub"].clone();
    result["privacy_stopped"] = status["privacy_stopped"].clone();
    result["capabilities"] = serde_json::json!({
        "native_helper": status["helper_connected"],
        "microphone": values[&TopicId::Permissions]["knowledge"]["value"]["microphone"] == "granted",
        "microphone_state": if values[&TopicId::Permissions]["knowledge"]["value"]["microphone"] == "unknown" {
            Value::Null
        } else { values[&TopicId::Permissions]["knowledge"]["value"]["microphone"].clone() }
    });
    let recording = &values[&TopicId::Recording]["knowledge"]["value"];
    for key in ["recording", "recording_signal", "pending_jobs"] {
        result[key] = recording[key].clone();
    }
    let assistant = &values[&TopicId::Assistant]["knowledge"]["value"];
    for key in ["turn", "voice_active", "session_id"] {
        result[key] = assistant[key].clone();
    }
    result["pending_approval"] = assistant["pending"].clone();
    result["call"] = assistant["call"]["value"].clone();
    result["active_request"] = if assistant["turn_in_flight"] == true {
        serde_json::json!("in_flight")
    } else {
        Value::Null
    };
    result["notice"] = values[&TopicId::Notices]["knowledge"]["value"]["notice"].clone();
    result["notice_log"] =
        serde_json::json!({"records": values[&TopicId::Notices]["knowledge"]["value"]["notices"]});
    Ok(result)
}
