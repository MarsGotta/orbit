//! What the engine knows of the control client when it is built without one
//! (the `control` feature off): the types of `orbit_control::cua` that the
//! computer-control domain names, with no client behind them.
//!
//! [`LocalCua`] and [`OwnedCua`] have no values, so no connection can exist
//! and every method of the domain that would use one is unreachable by type.
//! The engine does not try to start one, and the hub's orders for it are
//! refused by name (`served_by_this_build`).
use anyhow::{Result, bail};
use orbit_core::{
    computer_control::EngineTool,
    device::{DeviceGroup, DeviceProfile},
    topics::control::EngineCleanup,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::ExitStatus,
    time::Duration,
};

/// No driver is verified by a build without its client: no connection can
/// report this.
pub const VERIFIED_DRIVER_VERSION: &str = "none";

/// What this build can be asked of a device: what it declared, less the
/// screen and the computer's control, which a build without its client
/// refuses whatever the helper could answer. What a device says of itself
/// does not put back what the build left out; its kind stays its own.
pub(crate) fn served_by_this_build(device: DeviceProfile) -> DeviceProfile {
    let served: Vec<DeviceGroup> = DeviceGroup::ALL
        .into_iter()
        .filter(|group| !matches!(group, DeviceGroup::Screen | DeviceGroup::Control))
        .filter(|group| device.handles(*group))
        .collect();
    DeviceProfile::declared(Some(device.kind()), Some(&served))
}

#[derive(Debug, Clone)]
pub struct CuaConfig {
    pub driver_path: PathBuf,
    pub socket_path: PathBuf,
    pub request_timeout: Duration,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeObservation {
    pub structured: Option<Value>,
    pub text: Vec<String>,
    pub images: Vec<NativeImage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeImage {
    pub data: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Control response too large")]
pub struct ResponseTooLarge {
    pub part: TooLargePart,
    pub bytes: usize,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooLargePart {
    Message,
    Image,
}

#[derive(Debug, thiserror::Error)]
#[error("Control engine exited")]
pub struct EngineExit {
    pub process: &'static str,
    pub status: ExitStatus,
}

/// The local connection: no value of it exists without the client.
pub enum LocalCua {}

impl LocalCua {
    pub async fn connect(_config: CuaConfig) -> Result<Self> {
        bail!("This build has no computer-control client")
    }

    pub fn into_owned(self) -> OwnedCua {
        match self {}
    }
}

/// The owned connection: no value of it exists without the client.
pub enum OwnedCua {}

impl OwnedCua {
    pub fn is_connected(&self) -> bool {
        match *self {}
    }

    pub fn connection_epoch(&self) -> &str {
        match *self {}
    }

    pub fn engine_tools(&self) -> Vec<EngineTool> {
        match *self {}
    }

    pub fn declares_tool(&self, _name: &str) -> bool {
        match *self {}
    }

    pub fn invalidate(&mut self) {
        match *self {}
    }

    pub fn check_processes(&mut self) -> Result<()> {
        match *self {}
    }

    pub fn take_engine_cleanup(&mut self) -> Vec<EngineCleanup> {
        match *self {}
    }

    pub fn take_engine_inspection_notice(&mut self) -> Option<String> {
        match *self {}
    }

    pub async fn windows(&mut self, _pid: Option<i64>) -> Result<NativeObservation> {
        match *self {}
    }

    pub async fn call_tool(&mut self, _name: &str, _arguments: Value) -> Result<NativeObservation> {
        match *self {}
    }
}

/// Never reached: the only caller starts a client this build does not have.
pub fn resolve_driver_path(_configured: &str, _daemon_executable: &Path) -> Result<PathBuf> {
    bail!("This build has no computer-control client")
}

/// Never reached: it is asked for together with the driver's path.
pub fn default_socket_path(root: &Path) -> PathBuf {
    root.to_path_buf()
}

/// Never reached: the full screen is asked of an engine this build does not have.
pub fn desktop_capture_path(_root: &Path, _request: &str) -> Result<PathBuf> {
    bail!("This build has no computer-control client")
}
