//! Compile the native catalogue and core default, rather than parsing source.
#[test]
fn real_test_native_catalogue_export_is_current() -> anyhow::Result<()> {
    let generated = serde_json::json!({
        "es": super::spanish(),
        "en": super::english(),
        "core": {"owner_name": orbit_core::notes::owner_name(None)},
    });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../scripts/e2e-mac/native-copy.json");
    if std::env::var("ORBIT_UPDATE_TEST_CATALOGUES").as_deref() == Ok("1") {
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&generated)?),
        )?;
    }
    let current: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    assert_eq!(
        current, generated,
        "refresh it with ORBIT_UPDATE_TEST_CATALOGUES=1 cargo test -p orbit-desktop \
         real_test_native_catalogue_export_is_current"
    );
    Ok(())
}
