#[test]
fn exports_the_helpers_resolved_speech_locale() -> Result<(), Box<dyn std::error::Error>> {
    <orbit_core::wire::native::SpeechLocale as ts_rs::TS>::export_all()?;
    Ok(())
}
