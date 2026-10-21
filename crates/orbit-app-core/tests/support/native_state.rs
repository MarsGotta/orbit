//! Complete synthetic native state replies from the generated wire fixture.
use serde_json::{Value, json};

pub fn result(permissions: Option<Value>) -> Value {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../contracts/fixtures/native/results/hello.json"
    ))
    .expect("generated native state fixture");
    let mut result = fixture;
    result["calendar"]["authorization"] = json!("not_determined");
    if let Some(permissions) = permissions {
        result["calendar"]["authorization"] = permissions["calendars"].clone();
        result["permissions"] = permissions;
    }
    result
}
