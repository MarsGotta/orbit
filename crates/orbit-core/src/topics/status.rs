//! Connection, pairing and privacy facts for the desktop status view.
use crate::{AccountSummary, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_name: Option<String>,
    pub hub: Connection,
    pub account: Option<AccountSummary>,
    pub privacy_stopped: bool,
    pub helper_connected: bool,
    /// «Enviar informes de errores anónimos»: the user's choice. `None` when
    /// this build carries no Sentry address, so the switch is not offered;
    /// a daemon from before the switch says nothing, which reads the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reports: Option<bool>,
}
