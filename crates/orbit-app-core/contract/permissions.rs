use super::*;

app_type! {
    #[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
    NotGranted,
    NotDetermined,
    Unknown,
}
}

app_type! {
    pub struct PermissionSnapshot {
        pub microphone: PermissionState,
        pub screen: PermissionState,
        pub accessibility: PermissionState,
        pub calendars: PermissionState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub availability: Option<crate::features::FeatureAvailability>,
    }
}
