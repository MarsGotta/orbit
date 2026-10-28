//! What the engine reads of the screen: the windows, one window's capture with
//! its text, and the full screen written to a file of Orbit's.
use super::*;

/// A window's state with its screenshot, and its text when the capture is
/// for an attachment.
pub(super) async fn capture_window(
    service: &mut ActionService,
    pid: i64,
    window_id: u64,
    with_text: bool,
) -> Result<(
    crate::cua::NativeObservation,
    Option<std::result::Result<String, String>>,
)> {
    let observation = service
        .client_mut()
        .call_tool(
            "get_window_state",
            json!({
                "pid":pid,
                "window_id":window_id,
                "include_accessibility_tree":false,
                "include_screenshot":true,
                "max_dimension":1280
            }),
        )
        .await?;
    let text = if with_text {
        Some(window_tree(service, pid, window_id).await)
    } else {
        None
    };
    Ok((observation, text))
}

/// «Adjuntar ventana»: the engine's tree of the window it just captured,
/// without a picture, or the cause it gave none. Its text is the user's and
/// is never logged; a cause is a fixed code.
pub(super) async fn window_tree(
    service: &mut ActionService,
    pid: i64,
    window_id: u64,
) -> std::result::Result<String, String> {
    let observation = service
        .client_mut()
        .call_tool(
            "get_window_state",
            json!({
                "pid":pid,
                "window_id":window_id,
                "include_accessibility_tree":true,
                "include_screenshot":false
            }),
        )
        .await
        .map_err(|error| {
            if error
                .downcast_ref::<crate::cua::ResponseTooLarge>()
                .is_some()
            {
                "engine_answer_too_large".to_owned()
            } else {
                "engine_failed".to_owned()
            }
        })?;
    let structured = observation.structured.unwrap_or_default();
    // The engine leaves the tree empty on purpose when it cannot tie the
    // window to its Accessibility element (another Space, for one).
    if structured["degraded"] == true {
        return Err("engine_window_unreadable".into());
    }
    structured["tree_markdown"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "engine_tree_absent".into())
}

/// Asks the engine for the full screen in `path`, then takes the file back:
/// read whole under its ceiling and removed whatever happened, before
/// anything else is decided. The engine's own failure comes first; then the
/// file's; then the fit's.
pub(super) async fn desktop_capture(
    service: &mut ActionService,
    path: PathBuf,
) -> Result<orbit_core::sources::FittedDesktop> {
    let target = path
        .to_str()
        .context("The private capture path is not UTF-8")?
        .to_owned();
    let read = service
        .client_mut()
        .call_tool("get_desktop_state", json!({"screenshot_out_file": target}))
        .await;
    tokio::task::spawn_blocking(move || {
        let taken = take_desktop_file(&path);
        read?;
        let bytes = taken?;
        Ok(orbit_core::sources::fit_desktop_png(&bytes).map_err(DesktopCaptureError::from)?)
    })
    .await
    .context("Screen resizing finished without a result")?
}

/// The engine's file, whole, if it is a regular file within
/// `MAX_DESKTOP_PNG_BYTES`; removed in every case.
pub(super) fn take_desktop_file(
    path: &std::path::Path,
) -> std::result::Result<Vec<u8>, DesktopCaptureError> {
    let taken = read_desktop_file(path);
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(DesktopCaptureError::Unreadable(format!(
                "could not delete it after reading: {error}"
            )));
        }
    }
    taken
}

pub(super) fn read_desktop_file(
    path: &std::path::Path,
) -> std::result::Result<Vec<u8>, DesktopCaptureError> {
    use std::io::Read as _;
    let max_bytes = orbit_core::sources::MAX_DESKTOP_PNG_BYTES;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(DesktopCaptureError::Missing);
        }
        Err(error) => return Err(DesktopCaptureError::Unreadable(error.to_string())),
    };
    if !metadata.is_file() {
        return Err(DesktopCaptureError::Unreadable(
            "is not a regular file".into(),
        ));
    }
    let too_large = |bytes: u64| {
        DesktopCaptureError::Fit(orbit_core::sources::DesktopFitError::FileTooLarge {
            bytes: usize::try_from(bytes).unwrap_or(usize::MAX),
            max_bytes,
        })
    };
    if metadata.len() > max_bytes as u64 {
        return Err(too_large(metadata.len()));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(max_bytes as u64 + 1).read_to_end(&mut bytes))
        .map_err(|error| DesktopCaptureError::Unreadable(error.to_string()))?;
    if bytes.len() > max_bytes {
        return Err(too_large(bytes.len() as u64));
    }
    Ok(bytes)
}

impl Runtime {
    /// The engine's window list, which Deimos asks for through the bridge.
    pub(in crate::daemon) fn read_computer_windows(&mut self) -> Result<()> {
        let authority = self.require_computer_authority()?;
        self.computer.require_computer_idle()?;
        let mut service = self
            .computer
            .service
            .take()
            .context("Connect control explicitly first")?;
        let events = self.work_events.clone();
        self.begin_computer(
            ComputerStage::Reading,
            "Observing the chosen target; content does not authorize actions.",
            authority.generation,
            move |ticket| {
                tokio::spawn(async move {
                    let result = async {
                        let data = bounded_observation(&service.windows(None).await?)?;
                        Ok(ComputerResult::Read(data))
                    }
                    .await;
                    let event = ComputerCompletion { service, result };
                    crate::daemon::deliver_work(
                        &events,
                        WorkResult::Control(crate::daemon::ControlWork::Computer {
                            ticket,
                            event: Box::new(Ok(event)),
                        }),
                    )
                    .await;
                })
            },
        )?;
        Ok(())
    }

    pub(in crate::daemon) fn read_bridge_screenshot(
        &mut self,
        pid: Option<i64>,
        window_id: Option<u64>,
        application: String,
        title: String,
    ) -> std::result::Result<(), ComputerStartError> {
        let authority = self.computer_start_authority()?;
        self.computer
            .require_computer_idle()
            .map_err(|error| ComputerStartError::Busy(error.to_string()))?;
        // The full screen goes to a file of Orbit's, not into the
        // engine's answer, where a Retina screen does not fit.
        let desktop_file = match (pid, window_id) {
            (None, None) => Some(
                crate::cua::desktop_capture_path(&self.paths.root, &id())
                    .map_err(ComputerStartError::CaptureFile)?,
            ),
            _ => None,
        };
        let with_text = self
            .assistant
            .bridge_ask(crate::daemon::BridgeKind::Screen)
            .is_some_and(|ask| ask.for_attachment);
        let mut service = self
            .computer
            .service
            .take()
            .ok_or(ComputerStartError::EngineUnavailable)?;
        let events = self.work_events.clone();
        self.begin_computer(
            ComputerStage::Reading,
            "Capturing the chosen window with the general engine.",
            authority.generation,
            move |ticket| {
                tokio::spawn(async move {
                    let result = match (pid, window_id, desktop_file) {
                        (Some(pid), Some(window_id), _) => {
                            capture_window(&mut service, pid, window_id, with_text)
                                .await
                                .map(|(observation, text)| ComputerResult::BridgeScreenshot {
                                    observation,
                                    application,
                                    title,
                                    text,
                                })
                        }
                        (None, None, Some(path)) => desktop_capture(&mut service, path)
                            .await
                            .map(|fitted| ComputerResult::DesktopCapture { fitted }),
                        _ => Err(anyhow::anyhow!("Incomplete capture target")),
                    };
                    deliver_computer(&events, ticket, ComputerCompletion { service, result }).await;
                })
            },
        )
        .map_err(|error| ComputerStartError::Busy(error.to_string()))?;
        Ok(())
    }
}
