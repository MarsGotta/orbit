//! Remote device bridge requests and replies: a router.
//!
//! A `device.request` enters through `admission`, which checks it and makes
//! it the one request being served (`BridgeAsk`). The router hands it to its
//! domain, which owns its start, its waits and its reply:
//!
//! - `screen` and `screen_sources`: the windows, a screenshot and the selected
//!   text, through the engine or the helper;
//! - `capture_upload`: an image's way to the hub before its answer;
//! - `control`: the engine's tools and one action at a time;
//! - `calendar`: an agenda read's start (its pages are `calendar/deimos.rs`);
//! - the assistant's folders, in `assistant_files.rs`.
//!
//! `refusals` names what each helper code and each ending says; `release`
//! frees the slot when the operation ended without the helper's answer.
use super::{Runtime, hub_link};
use orbit_core::device_bridge::Capability;
use orbit_core::notice::{Notice, NoticeCode};
use orbit_core::refusal::Refusal;
use orbit_core::wire::bridge::{BridgeResult, DeviceReplyParams};
use serde_json::{Value, json};

mod admission;
mod calendar;
mod capture_upload;
mod control;
mod parameters;
mod refusals;
mod release;
mod screen;
mod screen_sources;

pub(super) use capture_upload::{CaptureUploadOutcome, PendingCapture};
pub(super) use refusals::BridgeReleaseReason;

/// The remote request being served, and what it asked for.
///
/// The capability travels with the id because the answer is assembled somewhere
/// else entirely, when the helper replies: without it, whatever came back would
/// be reported as whatever was asked for last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BridgeAsk {
    pub(super) id: String,
    pub(super) capability: Capability,
    pub(super) application: Option<String>,
    pub(super) title: Option<String>,
    pub(super) full_screen: bool,
    pub(super) source_id: Option<String>,
    pub(super) tool_call: Option<orbit_core::computer_control::ToolCall>,
    /// Where the catalogue page this request wants begins. Zero for every
    /// capability that is not `ControlTools`, and for the first page of one.
    pub(super) catalogue_after: usize,
    /// The dates a `CalendarRead` asked about, as they were asked for.
    ///
    /// Kept whole: the read covers the whole range, in pages, whatever its width.
    pub(super) calendar_window: Option<(i64, i64)>,
    /// Where a `CalendarRead` continues, when it is not the first page.
    pub(super) calendar_cursor: Option<super::calendar::Cursor>,
    /// What a `files_*` request asked, as it asked it.
    pub(super) files: Option<super::assistant_files::FilesAsk>,
    /// «Adjuntar ventana»: the user's own capture of the frontmost
    /// window. Its answer goes to the app as an attachment, never to the hub
    /// as a `device.reply` (`window_attachment`), and Orbit's own windows,
    /// in front while the user presses the button, are never the one taken.
    pub(super) for_attachment: bool,
}

fn result_payload(request_id: &str, result: BridgeResult) -> Value {
    json!(DeviceReplyParams::answered(request_id.to_owned(), result))
}

/// A refusal of what Deimos asked, said on the Mac with its code only:
/// Deimos reads the full sentence in its reply and tells the user.
fn bridge_refused(refusal: &Refusal) -> Notice {
    Notice::new(NoticeCode::BridgeRefused)
        .with_token(orbit_core::notice::NoticeKey::Code, refusal.code().name())
}

impl Runtime {
    pub(super) fn note_bridge_send(&mut self, result: &hub_link::SendResult, subject: &str) {
        if result.delivered_to_queue() {
            self.log("bridge_reply_queued", &[("subject", subject)]);
            self.say(NoticeCode::BridgeReplySent);
        } else {
            self.log(
                "bridge_reply_undelivered",
                &[("subject", subject), ("cause", &result.cause().to_string())],
            );
            self.say(NoticeCode::BridgeReplyUndelivered);
        }
    }

    pub(super) fn bridge_refusal_frame(&self, request_id: &str, refusal: &Refusal) -> Value {
        json!(DeviceReplyParams::refused(
            request_id.to_owned(),
            refusal.clone(),
            None
        ))
    }

    /// Carries out a remote request once it has been admitted, and answers it.
    ///
    /// Only admitted capabilities arrive here, so a request cannot
    /// be accepted and then quietly dropped.
    pub(super) async fn serve_bridge_request(&mut self, ask: &BridgeAsk) -> Result<(), Refusal> {
        match ask.capability {
            Capability::ScreenSources => self.serve_screen_sources().await,
            Capability::Screenshot => self.serve_screenshot(ask).await,
            Capability::SelectedText => self.serve_selected_text().await,
            Capability::ControlTools => self.serve_control_tools(ask).await,
            Capability::PerformAction => self.serve_perform_action(ask),
            Capability::CalendarRead => self.serve_calendar_read(ask).await,
            // The folders the user chose in the macOS panel, resolved by
            // the helper and decided in `orbit_core::assistant_files`.
            Capability::FilesList | Capability::FilesSearch | Capability::FilesRead => {
                self.files_bridge_start(ask)
            }
        }
    }
}
