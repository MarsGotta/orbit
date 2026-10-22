fn main() {
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
            "orbit_send",
            "orbit_panel_send",
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
