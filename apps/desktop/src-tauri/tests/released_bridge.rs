#![cfg(not(feature = "pruebas"))]
#[test]
fn released_executable_contains_no_test_bridge() -> anyhow::Result<()> {
    let executable = std::fs::read(std::env::var("CARGO_BIN_EXE_orbit-desktop")?)?;
    for marker in [
        b"orbit-test-bridge-v1".as_slice(),
        b"orbit-test-bridge.sock".as_slice(),
    ] {
        assert!(!executable.windows(marker.len()).any(|part| part == marker));
    }
    Ok(())
}
