//! The daemon's socket, spoken the way the app speaks it: `orbit_ipc::request`.
//!
//! Until 2026-10-15 these tests asked through `orbit status --json`, `orbit
//! stop` and the rest of the command line retired that day. The answer is the same `State`
//! that command printed, so assertions read it as the same JSON.
#![allow(dead_code)]
use anyhow::Result;
use orbit_core::protocol::{Command, Response};
use serde_json::Value;
use std::path::Path;

fn paths(path: &Path) -> Result<orbit_ipc::Paths> {
    orbit_ipc::Paths::resolve(Some(path.to_owned()))
}

/// One command to the daemon at `path`, from synchronous test code. It runs on
/// a thread of its own, so a synchronous helper called from a `tokio::test`
/// does not start a runtime inside that test's runtime.
pub fn request(path: &Path, command: Command) -> Result<Response> {
    let paths = paths(path)?;
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?
                    .block_on(orbit_ipc::request(&paths, command))
            })
            .join()
            .map_err(|_| anyhow::anyhow!("the IPC request thread failed"))?
    })
}

/// The same, from a test already running on a Tokio runtime.
pub async fn request_async(path: &Path, command: Command) -> Result<Response> {
    orbit_ipc::request(&paths(path)?, command).await
}

/// Public topic facts, available from synchronous process tests.
pub fn status(path: &Path) -> Result<Value> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?
                    .block_on(status_async(path))
            })
            .join()
            .map_err(|_| anyhow::anyhow!("topic reader panicked"))?
    })
}

#[path = "../../../orbit-ipc/tests/support/status.rs"]
mod topics;

pub async fn status_async(path: &Path) -> Result<Value> {
    topics::status_async(path)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
}
