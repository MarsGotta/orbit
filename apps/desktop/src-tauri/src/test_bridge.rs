//! Owner-local acceptance bridge. This entire module, including its DOM script,
//! is compiled only into Orbit Pruebas, never the released application.
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    net::{UnixListener, UnixStream},
    task::JoinHandle,
};

mod desktop;
mod listener;
mod menus;
pub(crate) use desktop::install;
pub use menus::application_built;
pub(crate) use menus::{Menus, tray_built, tray_clock};

/// The only fake boundary in Linux protocol tests: the real desktop adapter
/// evaluates the included DOM function and reads menus/windows from the shell.
pub type ViewFuture = Pin<Box<dyn Future<Output = Value> + Send>>;
pub trait View: Send + Sync + 'static {
    fn request(&self, request: Value) -> ViewFuture;
}

/// Protect one request/reply allocation, using the daemon's existing frame
/// ceiling. This does not cap recordings, screenshots or retained app data.
const FRAME_LIMIT: usize = orbit_core::limits::MAX_IPC_LINE_BYTES;
/// Bounds a stalled web view or a main thread that never runs the queued work.
/// Two seconds allows a throttled background tick; not measured on macOS yet.
/// Actions are never retried after this bound, since their outcome is uncertain.
pub const VIEW_BOUND: Duration = Duration::from_secs(2);
/// Bounds a peer holding a partial request or never draining the reply. Twice
/// the view budget allows that work plus local transport; not a retry budget.
const IO_BOUND: Duration = Duration::from_secs(4);

pub struct Bridge {
    path: PathBuf,
    task: JoinHandle<()>,
    stopped: AtomicBool,
}
impl Bridge {
    /// The caller owns the daemon's instance lock throughout this lifetime.
    pub async fn bind(root: &Path, view: Arc<dyn View>, bound: Duration) -> anyhow::Result<Self> {
        let path = root.join("orbit-test-bridge.sock");
        let mut paths = orbit_ipc::Paths::resolve(Some(root.to_owned()))?;
        paths.socket = path.clone();
        match tokio::time::timeout(IO_BOUND, UnixStream::connect(&path)).await {
            Ok(Ok(_)) => anyhow::bail!("test bridge already listening at {}", path.display()),
            Ok(Err(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) => {}
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => anyhow::bail!(
                "test bridge peer did not answer within {} ms",
                IO_BOUND.as_millis()
            ),
        }
        orbit_ipc::remove_owned_socket(&paths)?;
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let task = tokio::spawn(listener::run(listener, view, bound));
        Ok(Self {
            path,
            task,
            stopped: AtomicBool::new(false),
        })
    }
    pub fn stop(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum Request {
    Press {
        window: Window,
        role: String,
        name: String,
    },
    Type {
        window: Window,
        role: String,
        name: String,
        text: String,
    },
    Read {
        window: Window,
    },
    Menus,
    Windows,
}
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Window {
    Main,
    Panel,
}

async fn answer(stream: &mut UnixStream, view: &dyn View, bound: Duration) -> Value {
    let read = orbit_ipc::read_frame_with_limit(tokio::io::BufReader::new(stream), FRAME_LIMIT);
    let line = match tokio::time::timeout(IO_BOUND, read).await {
        Ok(Ok(line)) => line,
        Ok(Err(error)) => {
            if let Some(orbit_core::refusal::Refusal::TooLarge {
                bytes, max_bytes, ..
            }) = error.downcast_ref()
            {
                return json!({"ok":false,"cause":"request_too_large","bytes":bytes,"max_bytes":max_bytes});
            }
            return json!({"ok":false,"cause":"request_io","detail":error.to_string()});
        }
        Err(_) => {
            return json!({"ok":false,"cause":"request_timeout","bound_ms":IO_BOUND.as_millis()});
        }
    };
    let request: Value = match serde_json::from_slice::<Request>(&line) {
        Ok(request) => match serde_json::to_value(request) {
            Ok(value) => value,
            Err(_) => return json!({"ok":false,"cause":"invalid_json"}),
        },
        Err(error) => {
            return json!({"ok":false,"cause":"invalid_request","detail":error.to_string()});
        }
    };
    match tokio::time::timeout(bound, view.request(request.clone())).await {
        Ok(reply) => reply,
        Err(_) => json!({"ok":false,"cause":"view_timeout","bound_ms":bound.as_millis(),
            "command":request["command"],"window":request["window"],"outcome":"unknown"}),
    }
}

async fn serve(mut stream: UnixStream, view: &dyn View, bound: Duration) {
    let reply = answer(&mut stream, view, bound).await;
    write_reply(&mut stream, reply).await;
}

async fn write_reply(stream: &mut UnixStream, mut reply: Value) {
    if !reply.is_object() || reply.get("ok").and_then(Value::as_bool).is_none() {
        reply = json!({"ok":false,"cause":"invalid_view_reply"});
    }
    reply["bridge"] = json!("orbit-test-bridge-v1");
    let Ok(mut line) = serde_json::to_vec(&reply) else {
        return;
    };
    if line.len() + 1 > FRAME_LIMIT {
        line = serde_json::to_vec(&json!({"ok":false,"cause":"reply_too_large",
            "bytes":line.len()+1,"max_bytes":FRAME_LIMIT}))
        .unwrap_or_default();
    }
    line.push(b'\n');
    let _ = tokio::time::timeout(IO_BOUND, stream.write_all(&line)).await;
}
