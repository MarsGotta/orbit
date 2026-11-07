//! What the engine is handed by the machine it runs on, instead of looking for
//! it itself: the link to its native helper, its secrets, the requests to
//! close and the owner's name. A Mac's own pieces are in [`system`].
use futures_util::future::BoxFuture;

pub mod system;

/// What the engine is handed at start instead of looking for it itself.
pub struct Platform {
    /// The link to the native helper; `None` runs without one (development).
    pub helper: Option<Box<dyn HelperLink>>,
    /// The store key (`"storage"`) and the device token (`"device"`).
    pub secrets: std::sync::Arc<dyn Secrets>,
    /// Requests from outside to close in order (SIGINT, SIGTERM on the Mac).
    pub signals: Box<dyn StopSignals>,
    /// The account's full name, which names the user's microphone on the hub.
    pub owner: std::sync::Arc<dyn OwnerName>,
}

/// How the engine reaches its helper. Opened at the start and again at each
/// relaunch the supervisor decides.
pub trait HelperLink: Send {
    fn open(&mut self) -> anyhow::Result<HelperPipes>;
    /// Tests only: the file whose appearance makes the supervision panic while
    /// this link's helper runs; `None` never panics.
    #[cfg(test)]
    fn panic_trigger(&self) -> Option<std::path::PathBuf> {
        None
    }
}

/// One open link: NDJSON frames in and out, and the process behind them if
/// there is one.
pub struct HelperPipes {
    pub reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    pub writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    pub process: Option<Box<dyn HelperProcess>>,
}

/// The helper's process, for its closure in order.
pub trait HelperProcess: Send {
    /// Resolves when it has left.
    fn wait(&mut self) -> BoxFuture<'_, std::io::Result<std::process::ExitStatus>>;
    /// Kills it and reaps it.
    fn kill(&mut self) -> BoxFuture<'_, ()>;
}

/// Where the engine keeps its secrets, by account name.
pub trait Secrets: Send + Sync {
    fn get(&self, account: &str) -> anyhow::Result<Option<String>>;
    fn set(&self, account: &str, value: &str) -> anyhow::Result<()>;
}

pub trait StopSignals: Send {
    /// The next request to close, by name (`"SIGTERM"`). One that can never
    /// come again waits for ever.
    fn next(&mut self) -> BoxFuture<'_, &'static str>;
}

pub trait OwnerName: Send + Sync {
    /// The name, already the one the hub gets («Me» when there is none).
    fn read(&self) -> BoxFuture<'static, String>;
}
