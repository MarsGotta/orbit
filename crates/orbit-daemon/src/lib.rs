// The crate is also compiled with cfg(test) by --all-targets; fixtures may
// inspect foreign enums with catch-all arms. Production always gets the lint.
#![cfg_attr(not(test), deny(clippy::wildcard_enum_match_arm))]

pub mod app_control;
pub mod capture;
pub mod config;
pub mod cua;
pub mod daemon;
pub mod diagnostics;
pub mod error_reports;
pub mod hub;
pub mod hub_agents;
pub mod hub_notes;
pub use orbit_ipc as ipc;
pub mod native;
pub use orbit_ipc::permissions;
pub mod retry;
pub mod speech;
