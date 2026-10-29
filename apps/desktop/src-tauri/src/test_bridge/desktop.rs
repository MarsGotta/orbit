use super::{Bridge, VIEW_BOUND, View, ViewFuture};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::Manager;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};

type Pending = Mutex<Option<(oneshot::Sender<Value>, OwnedSemaphorePermit)>>;

struct Desktop<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    // Keep one permit per window until its callback is consumed or dropped,
    // even after the socket times out. A hung page cannot accumulate evals.
    views: [Arc<Semaphore>; 2],
}
impl<R: tauri::Runtime> View for Desktop<R> {
    fn request(&self, mut request: Value) -> ViewFuture {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        request["deadline"] = json!(now + VIEW_BOUND.as_millis());
        request["max_reply_bytes"] = json!(super::FRAME_LIMIT);
        let app = self.app.clone();
        let slot = self.views[usize::from(request["window"] == "panel")].clone();
        Box::pin(async move {
            let (sender, receiver) = oneshot::channel();
            // Native menu/window calls and evaluation scheduling must run on the
            // main thread. The socket task times out even if it never runs this.
            let command = request["command"].clone();
            let window = request["window"].clone();
            let target = app.clone();
            let queued = app.run_on_main_thread(move || dispatch(target, request, sender, slot));
            if let Err(error) = queued {
                return json!({"ok":false,"cause":"main_thread_unavailable","detail":error.to_string()});
            }
            receiver
                .await
                .unwrap_or_else(|_| json!({"ok":false,"cause":"view_callback_lost","command":command,"window":window,"outcome":"unknown"}))
        })
    }
}

fn dispatch<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    request: Value,
    sender: oneshot::Sender<Value>,
    slot: Arc<Semaphore>,
) {
    if sender.is_closed() {
        return;
    }
    match request["command"].as_str() {
        Some("menus") => {
            let _ = sender.send(super::menus::snapshot(&app));
        }
        Some("windows") => {
            let _ = sender.send(windows(&app));
        }
        _ => {
            let label = request["window"].as_str().unwrap_or_default();
            let Some(window) = app.get_webview_window(label) else {
                let _ = sender
                    .send(json!({"ok":false,"cause":"window_missing","window":label,"count":0}));
                return;
            };
            match window.is_visible() {
                Ok(true) => {}
                Ok(false) => {
                    let _ = sender.send(json!({"ok":false,"cause":"window_hidden","window":label}));
                    return;
                }
                Err(error) => {
                    let _ = sender.send(json!({"ok":false,"cause":"window_visibility_failed",
                        "window":label,"detail":error.to_string()}));
                    return;
                }
            }
            let Ok(permit) = slot.try_acquire_owned() else {
                let _ = sender.send(json!({"ok":false,"cause":"view_busy","window":label,
                    "in_flight":1,"max_evaluations":1}));
                return;
            };
            let script = format!(
                "(() => {{ const run = {}; return run({request}); }})()",
                include_str!("dom.js")
            );
            let pending = Arc::new(Mutex::new(Some((sender, permit))));
            let callback = pending.clone();
            let evaluated = window.eval_with_callback(script, move |result| {
                let reply = if result.len() > super::FRAME_LIMIT {
                    json!({"ok":false,"cause":"reply_too_large","bytes":result.len(),"max_bytes":super::FRAME_LIMIT})
                } else {
                    serde_json::from_str(&result).unwrap_or_else(|_| json!({"ok":false,"cause":"invalid_view_reply"}))
                };
                complete(&callback, reply);
            });
            if let Err(error) = evaluated {
                complete(
                    &pending,
                    json!({"ok":false,"cause":"evaluation_not_delivered","detail":error.to_string()}),
                );
            }
        }
    }
}
fn complete(pending: &Pending, value: Value) {
    if let Ok(mut pending) = pending.lock()
        && let Some((sender, _permit)) = pending.take()
    {
        let _ = sender.send(value);
    }
}

pub(crate) fn install<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    root: &Path,
) -> anyhow::Result<()> {
    let bridge = tauri::async_runtime::block_on(Bridge::bind(
        root,
        Arc::new(Desktop {
            app: app.clone(),
            views: [Arc::new(Semaphore::new(1)), Arc::new(Semaphore::new(1))],
        }),
        VIEW_BOUND,
    ))?;
    app.manage(bridge);
    Ok(())
}

#[cfg(target_os = "macos")]
fn windows<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Value {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let Some(main) = MainThreadMarker::new() else {
        return json!({"ok":false,"cause":"windows_require_main_thread"});
    };
    let native = NSApplication::sharedApplication(main).windows();
    let mut rows = Vec::new();
    for (label, window) in app.webview_windows() {
        if label != "main" && label != "panel" {
            continue;
        }
        let Ok(pointer) = window.ns_window() else {
            return json!({"ok":false,"cause":"native_window_unavailable","window":label});
        };
        // Compare addresses only: no raw pointer is dereferenced. AppKit owns
        // each retained NSWindow and exposes its windowNumber through safe APIs.
        let Some(found) = native.iter().find(|item| {
            std::ptr::from_ref(&**item)
                .cast_mut()
                .cast::<std::ffi::c_void>()
                == pointer
        }) else {
            return json!({"ok":false,"cause":"native_window_missing","window":label});
        };
        let frame = found.frame();
        rows.push(json!({"label":label,"window_id":found.windowNumber(),"visible":found.isVisible(),
            "frame":{"x":frame.origin.x,"y":frame.origin.y,"width":frame.size.width,"height":frame.size.height}}));
    }
    json!({"ok":true,"pid":std::process::id(),"windows":rows,"coordinates":"appkit_screen_points_bottom_left"})
}
#[cfg(not(target_os = "macos"))]
fn windows<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) -> Value {
    json!({"ok":false,"cause":"window_numbers_require_macos"})
}
