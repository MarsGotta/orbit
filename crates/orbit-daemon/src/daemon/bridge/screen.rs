//! What Deimos sees of the screen: where a window listing, a screenshot or a
//! selection read starts, and the engine's answers to them. The helper's
//! answers are `screen_sources.rs`; the image's way to the hub is
//! `capture_upload.rs`.
use super::refusals::source_start_refusal;
use super::{BridgeAsk, result_payload};
use crate::daemon::{Runtime, hub};
use anyhow::Result;
use base64::Engine;
use orbit_core::device_bridge::Capability;
use orbit_core::refusal::Refusal;
use orbit_core::wire::bridge::{BridgeResult, ScreenSourcesResult, WindowSummary};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct ComputerWindows {
    structured: ComputerWindowRows,
}

#[derive(Deserialize)]
struct ComputerWindowRows {
    windows: Vec<ComputerWindow>,
}

#[derive(Deserialize)]
struct ComputerWindow {
    #[serde(default)]
    app_name: Option<String>,
    #[serde(default)]
    application_name: Option<String>,
    #[serde(default)]
    application: Option<String>,
    title: String,
    #[serde(default)]
    pid: Option<i64>,
    #[serde(default)]
    window_id: Option<u64>,
    #[serde(default)]
    bundle_id: Option<String>,
    #[serde(default)]
    z_index: Option<i64>,
}

impl ComputerWindow {
    fn name(&self) -> Option<&str> {
        self.app_name
            .as_deref()
            .or(self.application_name.as_deref())
            .or(self.application.as_deref())
    }
}

#[derive(Deserialize)]
struct ComputerScreenshotSize {
    #[serde(default)]
    screenshot_width: Option<u64>,
    #[serde(default)]
    screenshot_height: Option<u64>,
}

/// The bundle identifiers of Orbit's own windows (Orbit, Orbit Pruebas, the
/// engine): what the helper also calls its own (`ownBundlePrefix`).
const ORBIT_BUNDLE_PREFIX: &str = "app.orbit.";

impl BridgeAsk {
    pub(super) fn matches_window_filter(
        &self,
        application: &str,
        bundle: Option<&str>,
        title: &str,
    ) -> bool {
        if self.for_attachment
            && bundle.is_some_and(|bundle| bundle.starts_with(ORBIT_BUNDLE_PREFIX))
        {
            return false;
        }
        self.application.as_ref().is_none_or(|wanted| {
            application.eq_ignore_ascii_case(wanted)
                || bundle.is_some_and(|bundle| bundle.eq_ignore_ascii_case(wanted))
        }) && self
            .title
            .as_ref()
            .is_none_or(|wanted| title.contains(wanted))
    }
}

/// The windows a capture may take, of those that match what Deimos asked:
/// the titled ones when there are any. macOS draws its own overlays —
/// the cursor (CursorUIViewService, 128×128 in a real call on 2026-10-18),
/// menu-bar items, banners — as untitled windows in front of every other, so
/// the frontmost window is not the one the user is looking at. A person's
/// window has the title macOS shows in the Window menu and in Mission
/// Control. An untitled window
/// is still taken when no titled one matches, so an application whose window
/// has no title can be captured by its name. Neither inventory carries a
/// window's level, which is what macOS itself tells overlays apart by; size
/// is not used because a threshold would be a guess and the helper's rows
/// carry none, nor the owner, because a list of system services goes stale.
pub(super) fn titled_first<T>(candidates: Vec<T>, title: impl Fn(&T) -> &str) -> Vec<T> {
    let titled = |candidate: &T| !title(candidate).trim().is_empty();
    if candidates.iter().any(titled) {
        candidates.into_iter().filter(titled).collect()
    } else {
        candidates
    }
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

impl Runtime {
    /// The engine's listing when it is connected, the helper's otherwise.
    pub(super) async fn serve_screen_sources(&mut self) -> Result<(), Refusal> {
        if self.computer.computer_connection_epoch().is_some()
            && self.read_computer_windows().is_ok()
        {
            return Ok(());
        }
        self.source_windows().await.map_err(source_start_refusal)
    }

    pub(super) async fn serve_screenshot(&mut self, ask: &BridgeAsk) -> Result<(), Refusal> {
        if ask.full_screen {
            if ask.application.is_some() || ask.title.is_some() {
                return Err(Refusal::InvalidParameter {
                    parameter: orbit_core::refusal::Parameter::Request,
                    expected: orbit_core::refusal::Expected::FullScreenFilters {
                        application: ask.application.is_some(),
                        title: ask.title.is_some(),
                    },
                });
            }
            return self
                .read_bridge_screenshot(None, None, "Escritorio".into(), "Pantalla completa".into())
                .map_err(|error| self.bridge_computer_start_failure(error));
        }
        self.serve_screen_sources().await
    }

    /// The same path the person's own window listing takes. A second route
    /// to the helper would be a second set of rules about when capture is
    /// allowed, and only one of them would be maintained.
    pub(super) async fn serve_selected_text(&mut self) -> Result<(), Refusal> {
        self.source_windows().await.map_err(source_start_refusal)
    }

    pub(in crate::daemon) async fn answer_bridge_from_computer_windows(
        &mut self,
        observation: &Value,
    ) -> Result<bool> {
        let Some(ask) = self
            .assistant
            .bridge_request
            .as_ref()
            .filter(|ask| ask.capability == Capability::ScreenSources)
            .cloned()
        else {
            return Ok(true);
        };
        let Ok(listing) = ComputerWindows::deserialize(observation) else {
            return Ok(false);
        };
        let mut windows = Vec::with_capacity(listing.structured.windows.len());
        for row in listing.structured.windows {
            let Some(application) = row.name() else {
                return Ok(false);
            };
            windows.push(WindowSummary {
                application: application.to_owned(),
                title: row.title,
            });
        }
        let sent = self.send_frame(hub::command(
            &ask.id,
            "device.reply",
            result_payload(
                &ask.id,
                BridgeResult::ScreenSources(ScreenSourcesResult { windows }),
            ),
        ));
        self.note_bridge_send(&sent, "the engine's windows reply");
        self.assistant.bridge_request = None;
        Ok(true)
    }

    pub(in crate::daemon) async fn answer_bridge_from_computer_screenshot(
        &mut self,
        observation: &crate::cua::NativeObservation,
        application: &str,
        title: &str,
    ) -> Result<bool> {
        let Some(ask) = self
            .assistant
            .bridge_request
            .as_ref()
            .filter(|ask| ask.capability == Capability::Screenshot)
            .cloned()
        else {
            return Ok(true);
        };
        let Some(image) = observation
            .images
            .first()
            .filter(|_| observation.images.len() == 1)
        else {
            return Ok(false);
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&image.data) else {
            return Ok(false);
        };
        let Some((width, height)) = png_dimensions(&bytes) else {
            return Ok(false);
        };
        if orbit_core::sources::validate_png(&bytes, width, height).is_err()
            || !engine_size_matches(observation.structured.as_ref(), width, height)
        {
            return Ok(false);
        }
        let mime_type = image.mime_type.clone();
        self.upload_engine_capture(&ask, bytes, (width, height), mime_type, application, title)
            .await
            .map(|_| true)
    }

    /// The full screen, fitted, goes out as any engine capture does;
    /// its scale is kept with the upload and becomes the one desktop actions
    /// are mapped with once Deimos has been sent this image.
    pub(in crate::daemon) async fn answer_bridge_with_desktop(
        &mut self,
        fitted: orbit_core::sources::FittedDesktop,
    ) -> Result<()> {
        let Some(ask) = self
            .assistant
            .bridge_request
            .as_ref()
            .filter(|ask| ask.capability == Capability::Screenshot)
            .cloned()
        else {
            return Ok(());
        };
        let scale = fitted.scale;
        self.log(
            "desktop_capture_fitted",
            &[
                (
                    "screen",
                    &format!("{}x{}", scale.screen_width, scale.screen_height),
                ),
                (
                    "image",
                    &format!("{}x{}", scale.image_width, scale.image_height),
                ),
                ("bytes", &fitted.png.len().to_string()),
            ],
        );
        let uploading = self
            .upload_engine_capture(
                &ask,
                fitted.png,
                (scale.image_width, scale.image_height),
                "image/png".into(),
                "Escritorio",
                "Pantalla completa",
            )
            .await?;
        self.computer.pending_desktop = uploading.map(|capture_id| (capture_id, scale));
        Ok(())
    }
}

/// The size the engine reports for its image, when it reports one, is the
/// image's own.
fn engine_size_matches(structured: Option<&Value>, width: u32, height: u32) -> bool {
    let size = structured.and_then(|value| ComputerScreenshotSize::deserialize(value).ok());
    !(size
        .as_ref()
        .and_then(|value| value.screenshot_width)
        .is_some_and(|value| value != u64::from(width))
        || size
            .as_ref()
            .and_then(|value| value.screenshot_height)
            .is_some_and(|value| value != u64::from(height)))
}

impl crate::daemon::assistant::AssistantHandle {
    pub(in crate::daemon) fn bridge_screenshot_target(
        &self,
        observation: &Value,
    ) -> Option<(i64, u64, String, String)> {
        let ask = self
            .bridge_request
            .as_ref()
            .filter(|ask| ask.capability == Capability::Screenshot)?;
        let listing = ComputerWindows::deserialize(observation).ok()?;
        let matching = listing
            .structured
            .windows
            .iter()
            .filter_map(|row| {
                let pid = row.pid?;
                let window_id = row.window_id?;
                let application = row.name()?;
                let title = row.title.as_str();
                if pid <= 0
                    || window_id == 0
                    || !ask.matches_window_filter(application, row.bundle_id.as_deref(), title)
                {
                    return None;
                }
                Some((
                    row.z_index.unwrap_or(i64::MIN),
                    pid,
                    window_id,
                    application.to_owned(),
                    title.to_owned(),
                ))
            })
            .collect();
        titled_first(matching, |candidate| candidate.4.as_str())
            .into_iter()
            .max_by_key(|candidate| candidate.0)
            .map(|(_, pid, window_id, application, title)| (pid, window_id, application, title))
    }
}
