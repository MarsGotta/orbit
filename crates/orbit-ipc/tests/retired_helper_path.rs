//! A `config.toml` from before the helper's path left the settings.
//!
//! `Settings` refuses unknown keys, so a key it stops knowing would stop every
//! `config.toml` that still names it from loading: this one is accepted,
//! nothing reads it and the next save drops it, like the other retired keys.
use anyhow::Result;
use orbit_ipc::Paths;

const OLD_CONFIG: &str = "version = 1\n\
device_name = \"Mi Mac\"\n\
speak_responses = false\n\
helper_path = \"/opt/ejemplo/Fake.app/Contents/MacOS/OrbitNative\"\n";

fn paths_with(config: &str) -> Result<(tempfile::TempDir, Paths)> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
    paths.initialize()?;
    std::fs::write(&paths.settings, config)?;
    Ok((directory, paths))
}

#[test]
fn an_old_config_that_names_a_helper_still_loads() -> Result<()> {
    let (_directory, paths) = paths_with(OLD_CONFIG)?;
    let settings = paths.load_settings()?;
    assert_eq!(settings.device_name, "Mi Mac");
    Ok(())
}

#[test]
fn the_next_save_drops_the_key() -> Result<()> {
    let (_directory, paths) = paths_with(OLD_CONFIG)?;
    let settings = paths.load_settings()?;
    paths.save_settings(&settings)?;
    let written = std::fs::read_to_string(&paths.settings)?;
    assert!(!written.contains("helper_path"), "{written}");
    paths.load_settings()?;
    Ok(())
}
