//! Why a hub frame did not decode, said without the hub's values.

/// The schema fields a decode error may name; any other name is `data`.
const FIELDS: &[&str] = &[
    "v",
    "id",
    "data",
    "device_id",
    "hub_protocol",
    "hub_version",
    "device_version",
    "session_id",
    "turn_id",
    "text",
    "status",
    "kind",
    "state",
    "request_id",
    "description",
    "description/question",
    "question",
    "questions",
    "qid",
    "expires_at",
    "choices",
    "revision",
    "sessions",
    "pending",
    "omitted",
    "message",
    "persisted",
    "warning",
    "applied",
    "resolved",
    "remaining",
    "stale_interaction",
    "capability",
    "parameters",
    "code",
    "refusal",
    "retry",
    "ms",
    "operation",
    "resource",
    "bytes",
    "max_bytes",
    "retry_after_ms",
    "last_seen_ms",
    "waited_ms",
    "deadline_ms",
    "item_id",
    "phase",
    "label",
    "subject",
    "steps",
    "cause",
    "multi_select",
    "storage",
    "used_bytes",
    "remaining_bytes",
    "limit",
    "nearly_full",
];

/// serde errors can quote the rejected value. Diagnostics retain only schema
/// fields and type classes, never a hub-supplied value or map key.
pub(super) fn safe_decode_error(reason: &str) -> (&'static str, &'static str, &'static str) {
    let path = reason.split_once(':').map_or(reason, |(path, _)| path);
    let field_name = reason
        .split_once("missing field `")
        .and_then(|(_, tail)| tail.split_once('`').map(|(field, _)| field))
        .or_else(|| path.rsplit(['.', '[', ']']).find(|part| !part.is_empty()))
        .unwrap_or("data");
    let field = FIELDS
        .iter()
        .copied()
        .find(|known| *known == field_name)
        .unwrap_or("data");
    let actual = actual_type(reason);
    let expected = expected_type(reason);
    (field, actual, expected)
}

/// The type serde found, as a class: never the value itself.
fn actual_type(reason: &str) -> &'static str {
    if reason.contains("missing field") {
        "missing"
    } else {
        match reason.split_once("invalid type: ").map(|(_, tail)| tail) {
            Some(tail) if tail.starts_with("string") => "string",
            Some(tail) if tail.starts_with("integer") => "integer",
            Some(tail) if tail.starts_with("floating point") => "number",
            Some(tail) if tail.starts_with("boolean") => "boolean",
            Some(tail) if tail.starts_with("null") => "null",
            Some(tail) if tail.starts_with("sequence") => "array",
            Some(tail) if tail.starts_with("map") => "object",
            Some(_) | None => "invalid",
        }
    }
}

/// The type serde expected, as a class.
fn expected_type(reason: &str) -> &'static str {
    match reason.split_once("expected ").map(|(_, tail)| tail) {
        Some(tail) if tail.starts_with("a string") || tail.starts_with("string") => "string",
        Some(tail)
            if tail.starts_with("u64")
                || tail.starts_with("i64")
                || tail.starts_with("u8")
                || tail.starts_with("integer") =>
        {
            "integer"
        }
        Some(tail) if tail.starts_with("a sequence") || tail.starts_with("array") => "array",
        Some(tail) if tail.starts_with("a map") || tail.starts_with("object") => "object",
        Some(tail) if tail.starts_with("a boolean") || tail.starts_with("boolean") => "boolean",
        Some(_) | None => "unknown",
    }
}
