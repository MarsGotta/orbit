//! Where the helper's events go: audio to the capture task on its own
//! channel, everything else to the daemon's loop.
//!
//! Each event gets its place in the helper's order (`seq`). The loop's
//! events carry it as `_seq`, and for each one the capture task's channel
//! gets a marker at the same place, after the event itself was handed over.
//! The loop serves an event only once the task has handled every frame that
//! came before it, and the task stops serving frames on its own at a marker
//! whose event may change who consumes them. Order across both channels is
//! the helper's order, without the audio ever waiting for the loop.
use super::NativeEventHeader;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::mpsc;

/// The field that carries an event's place in the helper's order to the
/// loop. Only the bridge writes it; the loop removes it before serving.
pub const SEQ_FIELD: &str = "_seq";

/// What the capture task's channel carries.
pub enum AudioItem {
    /// An `audio` or `gap` event of the helper, as it came.
    Frame(Value),
    /// The place of an event handed to the loop. `disarms`: that event may
    /// change who consumes the frames after it, so the task hands those to
    /// the loop until the loop says otherwise.
    Marker { disarms: bool },
}

pub struct RoutedAudio {
    pub seq: u64,
    pub item: AudioItem,
}

/// The channel to the loop closed: the daemon is gone.
#[derive(Debug)]
pub struct RouteClosed;

/// The helper's events, routed. Without an audio channel everything goes to
/// the loop as before, unstamped: what the bridge's own tests drive.
#[derive(Clone)]
pub struct NativeRoute {
    events: mpsc::Sender<Value>,
    audio: Option<mpsc::Sender<RoutedAudio>>,
    next: Arc<AtomicU64>,
}

impl From<mpsc::Sender<Value>> for NativeRoute {
    fn from(events: mpsc::Sender<Value>) -> Self {
        Self {
            events,
            audio: None,
            next: Arc::default(),
        }
    }
}

/// Events after which the frames still go where they went: they change
/// nothing of who consumes them. A response is judged by the order it
/// answers. Anything else stops the task's own handling until the loop has
/// served it, which only costs that the loop serves those frames, as before.
fn keeps_consumers(name: &str, value: &Value) -> bool {
    match name {
        "notice" | "playback_progress" | "microphone_activity" => true,
        "response" => value
            .get("_bridge_command")
            .and_then(Value::as_str)
            .is_some_and(answers_no_capture_order),
        _ => false,
    }
}

fn answers_no_capture_order(command: &str) -> bool {
    matches!(
        command,
        "hello"
            | "heartbeat"
            | "screenshot"
            | "screen_sources"
            | "application_sources"
            | "selected_text"
            | "window_text"
            | "photo_jpeg"
            | "playback"
            | "playback_clip"
            | "playback_stop"
    ) || command.starts_with("calendar_")
        || command.starts_with("assistant_")
}

impl NativeRoute {
    pub fn new(events: mpsc::Sender<Value>, audio: mpsc::Sender<RoutedAudio>) -> Self {
        Self {
            events,
            audio: Some(audio),
            next: Arc::default(),
        }
    }

    fn place(&self) -> u64 {
        self.next.fetch_add(1, Ordering::SeqCst).saturating_add(1)
    }

    /// Hands one event over, waiting for room as the bridge always has.
    pub async fn send(&self, mut value: Value) -> Result<(), RouteClosed> {
        let Some(audio) = &self.audio else {
            return self.events.send(value).await.map_err(|_| RouteClosed);
        };
        let name = NativeEventHeader::from_value(&value)
            .map(|header| header.event.to_owned())
            .unwrap_or_default();
        let seq = self.place();
        if matches!(name.as_str(), "audio" | "gap") {
            let routed = RoutedAudio {
                seq,
                item: AudioItem::Frame(value),
            };
            // A capture task that is gone leaves the frames to the loop,
            // which serves them as it did before the task existed.
            return match audio.send(routed).await {
                Ok(()) => Ok(()),
                Err(mpsc::error::SendError(RoutedAudio {
                    item: AudioItem::Frame(value),
                    ..
                })) => self.events.send(value).await.map_err(|_| RouteClosed),
                Err(_) => Err(RouteClosed),
            };
        }
        let disarms = !keeps_consumers(&name, &value);
        if let Some(object) = value.as_object_mut() {
            object.insert(SEQ_FIELD.into(), Value::from(seq));
        }
        self.events.send(value).await.map_err(|_| RouteClosed)?;
        let marker = RoutedAudio {
            seq,
            item: AudioItem::Marker { disarms },
        };
        // A task that is gone reads no markers; the loop still gets its event.
        let _ = audio.send(marker).await;
        Ok(())
    }

    /// The same without waiting, for a thread that has no runtime.
    pub fn try_send(&self, value: Value) -> Result<(), RouteClosed> {
        if self.audio.is_some() {
            let seq = self.place();
            let mut value = value;
            if let Some(object) = value.as_object_mut() {
                object.insert(SEQ_FIELD.into(), Value::from(seq));
            }
            self.events.try_send(value).map_err(|_| RouteClosed)?;
            if let Some(audio) = &self.audio {
                let _ = audio.try_send(RoutedAudio {
                    seq,
                    item: AudioItem::Marker { disarms: true },
                });
            }
            return Ok(());
        }
        self.events.try_send(value).map_err(|_| RouteClosed)
    }

    /// The same, blocking the calling thread for room.
    pub fn blocking_send(&self, value: Value) -> Result<(), RouteClosed> {
        let mut value = value;
        let seq = self.audio.as_ref().map(|_| self.place());
        if let (Some(seq), Some(object)) = (seq, value.as_object_mut()) {
            object.insert(SEQ_FIELD.into(), Value::from(seq));
        }
        self.events.blocking_send(value).map_err(|_| RouteClosed)?;
        if let (Some(seq), Some(audio)) = (seq, &self.audio) {
            let _ = audio.blocking_send(RoutedAudio {
                seq,
                item: AudioItem::Marker { disarms: true },
            });
        }
        Ok(())
    }
}
