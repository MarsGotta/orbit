//! The hub link: pairing, the connection, and what the hub or the device
//! at the other end answered.
use super::{Retry, fmt};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeviceReplyRetry {
    AfterFix,
    CheckOutcome,
}

impl From<DeviceReplyRetry> for Retry {
    fn from(retry: DeviceReplyRetry) -> Self {
        match retry {
            DeviceReplyRetry::AfterFix => Self::AfterFix,
            DeviceReplyRetry::CheckOutcome => Self::CheckOutcome,
        }
    }
}

parameter_enum!(Service { Hub => "hub", Provider => "provider", NativeHelper => "native_helper" });

macro_rules! hub_refusals {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            HubReplyInvalid = 8 => "hub_reply_invalid" { field: String, received_bytes: Option<u64> },
                retry Retry::CheckOutcome;
            // The device's reply carries its own rule; the wire says it as `retry`.
            DeviceReplyInvalid = 17 => "device_reply_invalid" {
                field: String,
                expected: String,
                device_version: String,
                #[serde(skip_serializing)]
                retry: DeviceReplyRetry,
            }, retry Retry::from(*retry);
            DeviceReplyTimeout = 18 => "device_reply_timeout" {
                expected: String,
                deadline_ms: u64,
            }, retry Retry::CheckOutcome;
            NotPaired = 19 => "not_paired", retry Retry::AfterFix;
            MacDisconnected = 20 => "mac_disconnected" {
                last_seen_ms: Option<i64>,
            }, retry Retry::CheckOutcome;
            HubUnreachable = 21 => "hub_unreachable" {
                since_ms: Option<i64>,
            }, retry Retry::CheckOutcome;
            UpstreamFailed = 39 => "upstream_failed" {
                service: Service,
                status: Option<u16>,
            }, retry Retry::CheckOutcome;
        }
    };
}
pub(super) use hub_refusals;
