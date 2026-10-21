use orbit_core::refusal::RefusalCode;

#[test]
fn refusal_codes_are_written_for_the_interface() -> std::io::Result<()> {
    <RefusalCode as ts_rs::TS>::export_all()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    <orbit_core::refusal::Permission as ts_rs::TS>::export_all()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    <orbit_core::RecoveryStep as ts_rs::TS>::export_all()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    <orbit_core::refusal::HelperCode as ts_rs::TS>::export_all()
        .map_err(|error| std::io::Error::other(error.to_string()))
}

#[test]
fn every_refusal_code_fits_the_hub_contract() {
    let mut names = std::collections::BTreeSet::new();
    for code in RefusalCode::ALL {
        let name = code.name();
        assert!(!name.is_empty() && name.len() <= 64);
        assert!(
            name.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        );
        assert!(names.insert(name), "duplicate refusal code: {name}");
    }
}
