//! Typed actor fixtures for the in-process client boundary.
#![allow(dead_code)]

use orbit_ipc::{Call, TopicChannels};
use tokio::sync::{mpsc, watch};

pub struct Endpoint {
    pub client: orbit_ipc::LocalClient,
    ended: watch::Sender<Option<orbit_ipc::DaemonEnded>>,
}

impl Endpoint {
    pub fn abort(&self) {
        self.ended.send_replace(Some(orbit_ipc::DaemonEnded::Failed(
            "Fixture: actor terminado".into(),
        )));
    }
}

pub fn channel(calls: mpsc::Sender<Call>, channels: TopicChannels) -> Endpoint {
    let (ended, end) = watch::channel(None);
    Endpoint {
        client: orbit_ipc::LocalClient::new(calls, &channels, end),
        ended,
    }
}

pub fn idle() -> orbit_ipc::LocalClient {
    let (calls, receiver) = mpsc::channel(1);
    drop(receiver);
    let (_ended, end) = watch::channel(Some(orbit_ipc::DaemonEnded::Closed));
    orbit_ipc::LocalClient::new(calls, &TopicChannels::new(), end)
}
