use orbit_core::{agent_work::WorkItem, wire::hub::ItemData};
use serde_json::json;

#[test]
fn coded_work_failures_keep_legacy_messages_readable_without_projecting_them()
-> Result<(), Box<dyn std::error::Error>> {
    let wire = json!({"session_id":"sample","item_id":"step","kind":"tool",
        "phase":"failed","label":"Sample tool", "cause":"The tool failed",
        "cause_code":"tool_failed"});
    let data: ItemData = serde_json::from_value(wire.clone())?;
    let item = WorkItem::from_wire(&data).ok_or("unknown work item")?;
    assert_eq!(item.cause.as_deref(), Some("tool_failed"));
    // Previous releases do not know cause_code; their existing field stays readable.
    assert_eq!(wire["cause"], "The tool failed");
    let external: ItemData = serde_json::from_value(json!({"session_id":"sample",
        "item_id":"step","kind":"tool", "phase":"failed","label":"Sample tool",
        "cause":"A provider explanation"}))?;
    assert_eq!(
        WorkItem::from_wire(&external)
            .ok_or("unknown external item")?
            .cause
            .as_deref(),
        Some("A provider explanation")
    );
    Ok(())
}
