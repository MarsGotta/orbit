use super::*;

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum DaemonConnection {
        Connecting,
        Ready,
        Lost,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum HubConnection {
        Paired,
        Connecting,
        Unpaired,
        Offline,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum HubLinkConnection {
        Online,
        Connecting,
        Offline,
        Revoked,
    }
}

app_type! {
    pub struct ConnectionSnapshot {
        pub daemon: DaemonConnection,
        pub hub: HubConnection,
    }
}
