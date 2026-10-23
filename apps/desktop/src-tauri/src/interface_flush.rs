//! Before Orbit quits on an order of its own (the menu bar's «Salir», the
//! window's quit or «Reiniciar Orbit», an update about to install), the main window hands over what
//! it still holds: a template saved a second after the last key, say. Orbit
//! asks it, waits for its answer within a bound, and only then closes the
//! daemon in order. The window's side is `state/quit-flush.ts`.
//!
//! macOS's own Quit (⌘Q, the Dock) is not one of these: it reaches the app as
//! `RunEvent::Exit` with the main thread held until the handler returns, and
//! neither the ask nor the window's answer can cross it. For that path, and a
//! fall, the window keeps unsent drafts in its local storage as they are typed
//! (`state/local-drafts.ts`).

use std::{future::Future, sync::Mutex, time::Duration};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::oneshot;

/// The event the main window listens to (`bridge/tauri-port.ts`).
pub const FLUSH_EVENT: &str = "orbit://flush-before-quit";
const MAIN_WINDOW: &str = "main";

/// How long quitting waits for the main window to hand over what it holds.
///
/// What it protects: the quit, from a window that never answers (a page that
/// hangs or is reloading). Its size: each thing handed over is one local order
/// to the daemon, answered in milliseconds over the app's socket, and a hidden
/// window's timers may be throttled by macOS to about one a second; two
/// seconds covers one throttled tick and those orders. Not measured on macOS.
/// What it does not limit: the daemon's own closure, which follows
/// (`DAEMON_EXIT_WAIT`, 17 s); `install.sh --relaunch` waits 30 s for the
/// whole chain, more than both together.
pub const INTERFACE_FLUSH_BOUND: Duration = Duration::from_secs(2);

/// How the ask ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flushed {
    /// The window said it handed everything over.
    Answered,
    /// There is no main window to ask.
    NoWindow,
    /// The ask could not be delivered to the window.
    NotDelivered,
    /// The window did not answer within the bound.
    TimedOut,
}

/// The asks waiting for the window's answer; one answer ends them all.
#[derive(Default)]
pub struct InterfaceFlush {
    waiting: Mutex<Vec<oneshot::Sender<()>>>,
}

impl InterfaceFlush {
    /// Asks the main window to hand over what it holds and waits for its
    /// answer, at most `bound`.
    pub async fn ask<R: Runtime>(&self, app: &AppHandle<R>, bound: Duration) -> Flushed {
        if app.get_webview_window(MAIN_WINDOW).is_none() {
            return Flushed::NoWindow;
        }
        let (sender, answer) = oneshot::channel();
        if let Ok(mut waiting) = self.waiting.lock() {
            waiting.push(sender);
        }
        if app.emit_to(MAIN_WINDOW, FLUSH_EVENT, ()).is_err() {
            return Flushed::NotDelivered;
        }
        match tokio::time::timeout(bound, answer).await {
            Ok(Ok(())) => Flushed::Answered,
            Ok(Err(_)) | Err(_) => Flushed::TimedOut,
        }
    }

    /// The window's answer: every ask waiting for it ends.
    pub fn answered(&self) {
        let waiting = match self.waiting.lock() {
            Ok(mut waiting) => std::mem::take(&mut *waiting),
            Err(_) => return,
        };
        for sender in waiting {
            let _ = sender.send(());
        }
    }
}

/// The main window handed over everything it held.
#[tauri::command]
pub fn orbit_interface_flushed(flush: tauri::State<'_, InterfaceFlush>) {
    flush.answered();
}

/// Asks the window to hand over what it holds, then runs `then` (closing the
/// daemon and exiting), whatever the window answered or after the bound.
pub async fn flush_then<R: Runtime, F: Future<Output = ()>>(
    app: &AppHandle<R>,
    bound: Duration,
    then: F,
) {
    if let Some(flush) = app.try_state::<InterfaceFlush>() {
        let outcome = flush.ask(app, bound).await;
        if outcome != Flushed::Answered && outcome != Flushed::NoWindow {
            eprintln!(
                "Orbit: la ventana no confirmó en {} s que entregó lo pendiente ({outcome:?}); se sale igualmente",
                bound.as_secs()
            );
        }
    }
    then.await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};
    use tauri::Listener;

    fn app_with_main_window() -> Result<
        (
            tauri::App<tauri::test::MockRuntime>,
            tauri::WebviewWindow<tauri::test::MockRuntime>,
        ),
        Box<dyn std::error::Error>,
    > {
        let app = tauri::test::mock_builder()
            .manage(InterfaceFlush::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))?;
        let window =
            tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, tauri::WebviewUrl::default())
                .build()?;
        Ok((app, window))
    }

    #[test]
    fn the_quit_goes_on_once_the_window_says_it_handed_everything_over()
    -> Result<(), Box<dyn std::error::Error>> {
        let (app, window) = app_with_main_window()?;
        let handle = app.handle().clone();
        let answering = handle.clone();
        window.listen(FLUSH_EVENT, move |_| {
            if let Some(flush) = answering.try_state::<InterfaceFlush>() {
                flush.answered();
            }
        });
        let started = std::time::Instant::now();
        let outcome = tauri::async_runtime::block_on(async {
            handle
                .state::<InterfaceFlush>()
                .ask(&handle, Duration::from_secs(10))
                .await
        });
        assert_eq!(outcome, Flushed::Answered);
        // It ended at the answer, not at the bound.
        assert!(started.elapsed() < Duration::from_secs(10));
        Ok(())
    }

    #[test]
    fn a_window_that_never_answers_holds_the_quit_only_for_the_bound()
    -> Result<(), Box<dyn std::error::Error>> {
        let (app, _window) = app_with_main_window()?;
        let handle = app.handle().clone();
        let bound = Duration::from_millis(50);
        let started = std::time::Instant::now();
        let outcome = tauri::async_runtime::block_on(async {
            handle.state::<InterfaceFlush>().ask(&handle, bound).await
        });
        assert_eq!(outcome, Flushed::TimedOut);
        assert!(started.elapsed() >= bound);
        Ok(())
    }

    #[test]
    fn without_a_main_window_nothing_is_waited_for() -> Result<(), Box<dyn std::error::Error>> {
        let app = tauri::test::mock_builder()
            .manage(InterfaceFlush::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))?;
        let handle = app.handle().clone();
        let outcome = tauri::async_runtime::block_on(async {
            handle
                .state::<InterfaceFlush>()
                .ask(&handle, Duration::from_secs(10))
                .await
        });
        assert_eq!(outcome, Flushed::NoWindow);
        Ok(())
    }

    #[test]
    fn the_daemon_closes_only_after_the_window_handed_everything_over()
    -> Result<(), Box<dyn std::error::Error>> {
        let (app, window) = app_with_main_window()?;
        let handle = app.handle().clone();
        let steps = Arc::new(StdMutex::new(Vec::<&str>::new()));
        let answering = handle.clone();
        let flushed = Arc::clone(&steps);
        window.listen(FLUSH_EVENT, move |_| {
            if let Ok(mut steps) = flushed.lock() {
                steps.push("window handed over");
            }
            if let Some(flush) = answering.try_state::<InterfaceFlush>() {
                flush.answered();
            }
        });
        let closed = Arc::clone(&steps);
        tauri::async_runtime::block_on(flush_then(&handle, Duration::from_secs(10), async move {
            if let Ok(mut steps) = closed.lock() {
                steps.push("daemon closed");
            }
        }));
        let steps = steps.lock().map_err(|_| "steps poisoned")?.clone();
        assert_eq!(steps, ["window handed over", "daemon closed"]);
        Ok(())
    }

    #[test]
    fn two_quits_at_once_both_wait_for_the_one_answer() -> Result<(), Box<dyn std::error::Error>> {
        let (app, _window) = app_with_main_window()?;
        let handle = app.handle().clone();
        let outcomes = tauri::async_runtime::block_on(async {
            let flush = handle.state::<InterfaceFlush>();
            let first = flush.ask(&handle, Duration::from_secs(10));
            let second = flush.ask(&handle, Duration::from_secs(10));
            let answer = async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                flush.answered();
            };
            let (first, second, ()) = tokio::join!(first, second, answer);
            (first, second)
        });
        assert_eq!(outcomes, (Flushed::Answered, Flushed::Answered));
        Ok(())
    }
}
