// The crate is also compiled with cfg(test) by --all-targets; fixtures may
// inspect foreign enums with catch-all arms. Production always gets the lint.
#![cfg_attr(not(test), deny(clippy::wildcard_enum_match_arm))]

pub mod capture;
pub mod config;
#[cfg(feature = "control")]
pub use orbit_control::cua;
#[cfg(not(feature = "control"))]
#[path = "control_absent.rs"]
pub mod cua;
pub mod daemon;
pub mod diagnostics;
pub mod error_reports;
pub use orbit_hub_client::{hub, hub_agents, hub_notes, hub_plugin, hub_vault};
pub use orbit_ipc as ipc;
pub mod native;
pub use orbit_ipc::permissions;
pub mod platform;
pub mod retry;
pub mod speech;
pub mod supervision;
