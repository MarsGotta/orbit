//! A `device.request` whose parameters do not have the wire's shape, refused
//! with the field, what it should be and what came instead.
use super::refusals::MAX_BRIDGE_DETAIL_CHARS;
use orbit_core::device_bridge::Capability;
use orbit_core::refusal::{Expected, Parameter, Refusal};
use serde_json::Value;

pub(super) fn invalid_parameter_refusal(raw: &Value, capability: Capability) -> Refusal {
    let parameters = raw.get("parameters");
    let field = failing_field(parameters, capability);
    let observed = if field == "parameters" {
        parameters
    } else {
        parameters.and_then(|parameters| {
            field
                .split('.')
                .try_fold(parameters, |value, key| value.get(key))
        })
    };
    let expected = expected_type(&field);
    Refusal::InvalidParameter {
        parameter: Parameter::Request,
        expected: Expected::WireField {
            field,
            expected: expected.into(),
            received_type: received_type(observed).into(),
        },
    }
}

/// The first field the capability's parameters fail on, by its path.
fn failing_field(parameters: Option<&Value>, capability: Capability) -> String {
    use orbit_core::wire::bridge::{
        CalendarReadParameters, ControlToolsParameters, EmptyParameters, FilesListParameters,
        FilesReadParameters, FilesSearchParameters, PerformActionParameters, ScreenshotParameters,
        SelectedTextParameters,
    };
    fn path<T: serde::de::DeserializeOwned>(value: &Value) -> Option<String> {
        let encoded = serde_json::to_vec(value).ok()?;
        let mut decoder = serde_json::Deserializer::from_slice(&encoded);
        serde_path_to_error::deserialize::<_, T>(&mut decoder)
            .err()
            .map(|error| error.path().to_string())
    }
    let Some(parameters) = parameters.filter(|value| value.is_object()) else {
        return "parameters".into();
    };
    let field = match capability {
        Capability::Screenshot => path::<ScreenshotParameters>(parameters),
        Capability::ScreenSources => path::<EmptyParameters>(parameters),
        Capability::SelectedText => path::<SelectedTextParameters>(parameters),
        Capability::ControlTools => path::<ControlToolsParameters>(parameters),
        Capability::PerformAction => path::<PerformActionParameters>(parameters),
        Capability::CalendarRead => path::<CalendarReadParameters>(parameters),
        Capability::FilesList => path::<FilesListParameters>(parameters),
        Capability::FilesSearch => path::<FilesSearchParameters>(parameters),
        Capability::FilesRead => path::<FilesReadParameters>(parameters),
    }
    .unwrap_or_else(|| "parameters".into());
    if capability == Capability::CalendarRead {
        if parameters.get("start_ms").is_none() {
            return "start_ms".into();
        } else if parameters.get("end_ms").is_none() {
            return "end_ms".into();
        }
    }
    field
}

fn received_type(observed: Option<&Value>) -> &'static str {
    match observed {
        None => "absent",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

fn expected_type(field: &str) -> &'static str {
    if field == "parameters" || field.ends_with("arguments") || field.ends_with("action") {
        "object"
    } else if field.ends_with("full_screen") || field.ends_with("content") {
        "boolean"
    } else if field.ends_with("after_index")
        || field.ends_with("after_invitee")
        || field.ends_with("after_page")
    {
        "nonnegative_integer"
    } else if field.ends_with("start_ms")
        || field.ends_with("end_ms")
        || field.ends_with("after_ms")
    {
        "unix_milliseconds_integer"
    } else {
        "string"
    }
}

pub(super) fn unknown_capture_filter(raw: &Value, capability: Capability) -> Option<Refusal> {
    let allowed: &[&str] = match capability {
        Capability::Screenshot => &["application", "title", "full_screen"],
        Capability::SelectedText => &["application", "title"],
        Capability::ScreenSources
        | Capability::ControlTools
        | Capability::PerformAction
        | Capability::CalendarRead
        | Capability::FilesList
        | Capability::FilesSearch
        | Capability::FilesRead => return None,
    };
    let parameters = raw.get("parameters")?.as_object()?;
    let key = parameters
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))?;
    Some(Refusal::InvalidParameter {
        parameter: Parameter::Request,
        expected: Expected::SelectionFilters {
            unknown: key.chars().take(MAX_BRIDGE_DETAIL_CHARS).collect(),
            allowed: allowed.iter().map(|value| (*value).to_owned()).collect(),
        },
    })
}

/// A tool call that does not validate, with its sizes and its ceilings.
pub(super) fn tool_call_refusal(action: &orbit_core::computer_control::ToolCall) -> Refusal {
    let arguments_type = match &action.arguments {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    Refusal::InvalidParameter {
        parameter: Parameter::Request,
        expected: Expected::ToolCall {
            tool_bytes: action.tool.len() as u64,
            max_tool_bytes: orbit_core::computer_control::MAX_TOOL_NAME_BYTES as u64,
            arguments_type: arguments_type.into(),
            bytes: serde_json::to_vec(action).ok().map(|v| v.len() as u64),
            max_bytes: orbit_core::limits::MAX_ACTION_BYTES as u64,
        },
    }
}

/// A calendar range that ends where or before it starts.
pub(super) fn calendar_range_refusal(start_ms: i64, end_ms: i64) -> Refusal {
    Refusal::InvalidParameter {
        parameter: Parameter::Request,
        expected: Expected::CalendarRange {
            start_ms,
            end_ms,
            min_ms: orbit_core::calendar::MIN_UNIX_MS,
            max_ms: orbit_core::calendar::MAX_UNIX_MS,
        },
    }
}
