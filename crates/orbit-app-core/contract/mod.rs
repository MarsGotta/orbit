use serde::{Deserialize, Serialize};

use ts_rs::TS;

fn is_false(value: &bool) -> bool {
    !*value
}

macro_rules! app_type {
    ($item:item) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
        #[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
        $item
    };
}

mod agents;
pub use agents::*;
mod connection;
pub use connection::*;
mod permissions;
pub use permissions::*;
mod recording;
pub use recording::*;
mod assistant;
pub use assistant::*;
mod status;
pub use status::*;
mod settings;
pub use settings::*;
mod agenda;
pub use agenda::*;
mod briefing;
pub use briefing::*;
mod notes;
pub use notes::*;
mod library;
pub use library::*;
mod snapshot;
pub use snapshot::*;
mod commands;
pub use commands::*;
mod errors;
pub use errors::*;
