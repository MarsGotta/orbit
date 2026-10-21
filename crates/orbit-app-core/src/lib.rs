pub mod calendar;
mod client;
#[path = "../contract/mod.rs"]
mod contract;
pub mod features;
mod local_time;
mod projection;
mod system_zone;
pub mod topics;
mod update;

#[cfg(test)]
#[allow(dead_code)]
#[path = "../build.rs"]
mod build_script;

pub use client::DaemonClient;
pub use contract::*;
pub use local_time::{LocalZone, ZoneTransition};
pub use update::{
    AUTOMATIC_CHECK_INTERVAL, DEFAULT_UPDATE_BASE_URL, UpdateInstallOutcome, UpdateService,
    UpdateState,
};

#[cfg(test)]
#[path = "../tests/support/fake_daemon.rs"]
mod client_fixture;
