//! The control engine's adapter, apart from the daemon: the verified CUA MCP
//! client that starts and owns the bundled engine (`cua`), and the bounded
//! process census its cleanup reads (`app_control`). It needs nothing of the
//! daemon's runtime; its decisions are orbit-core's.
// The crate is also compiled with cfg(test) by --all-targets; fixtures may
// inspect foreign enums with catch-all arms. Production always gets the lint.
#![cfg_attr(not(test), deny(clippy::wildcard_enum_match_arm))]

pub mod app_control;
pub mod cua;
