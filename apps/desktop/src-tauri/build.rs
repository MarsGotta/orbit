fn main() {
    if let Err(error) = configure_identity() {
        println!("cargo:warning={error}");
        std::process::exit(1);
    }
    // App orders are allowed per window. Without a manifest every
    // window could invoke every order; with it each capability names its own
    // (`capabilities/default.json` for the main window, `panel.json` for the
    // floating panel, which only answers a call).
    let attributes =
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "orbit_snapshot",
            "orbit_read_sessions_page",
            "orbit_read_session_messages",
            "orbit_agent",
            "orbit_vault",
            "orbit_send",
            "orbit_panel_send",
            "orbit_menu_state",
            "orbit_panel_resize",
            "orbit_report",
            "orbit_attachment_add",
            "orbit_attachment_content",
            "orbit_attachment_open",
            "orbit_interface_flushed",
        ]));
    if let Err(error) = tauri_build::try_build(attributes) {
        // A build script reports by exiting: the message names the cause.
        println!("cargo:warning={error:#}");
        std::process::exit(1);
    }
}

/// The context macro reads TAURI_CONFIG during compilation. Direct Cargo
/// builds must select the same identity as Tauri CLI builds and the bundle.
fn configure_identity() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=tauri.pruebas.conf.json");
    if std::env::var_os("CARGO_FEATURE_PRUEBAS").is_none() {
        return Ok(());
    }
    let mut config: serde_json::Value =
        serde_json::from_str(&std::env::var("TAURI_CONFIG").unwrap_or_else(|_| "{}".into()))?;
    let identity: serde_json::Value =
        serde_json::from_str(include_str!("tauri.pruebas.conf.json"))?;
    let object = config
        .as_object_mut()
        .ok_or("TAURI_CONFIG must be a JSON object")?;
    for (key, value) in identity
        .as_object()
        .ok_or("test identity must be a JSON object")?
    {
        object.insert(key.clone(), value.clone());
    }
    println!("cargo:rustc-env=TAURI_CONFIG={config}");
    Ok(())
}
