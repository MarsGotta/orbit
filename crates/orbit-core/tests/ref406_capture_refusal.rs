use orbit_core::refusal::Refusal;
use serde_json::json;

#[test]
fn capture_capacity_is_a_typed_too_many_refusal() -> Result<(), serde_json::Error> {
    let wire = json!({"code":"too_many","resource":"capture","count":5,"max":4,
        "retry":{"kind":"after_fix"}});
    let refusal: Refusal = serde_json::from_value(wire.clone())?;
    assert_eq!(serde_json::to_value(refusal)?, wire);
    Ok(())
}
