//! Daemon-owned Cua session. Storage commits stay in the actor; no Store is held
//! across an RPC. A cancelled worker must be dropped, never returned for reuse.
use anyhow::Result;

use crate::cua::{CuaConfig, LocalCua, NativeObservation, OwnedCua};

pub(crate) struct ActionService {
    client: OwnedCua,
}

impl ActionService {
    /// Call only following an explicit local read/control request and configured
    /// private driver socket. This starts a proxy, never a driver/browser daemon.
    pub async fn connect(config: CuaConfig) -> Result<Self> {
        Ok(Self::from_client(
            LocalCua::connect(config).await?.into_owned(),
        ))
    }

    /// The supplied transport must already have completed protocol initialization.
    pub fn from_client(client: OwnedCua) -> Self {
        Self { client }
    }

    /// Immutable access for the meeting detector's separately authorized reader.
    pub fn client(&self) -> &OwnedCua {
        &self.client
    }

    pub fn client_mut(&mut self) -> &mut OwnedCua {
        &mut self.client
    }

    pub async fn windows(&mut self, pid: Option<i64>) -> Result<NativeObservation> {
        self.client.windows(pid).await
    }

    /// Abort/drop any worker before calling this on an idle service.
    pub fn invalidate(&mut self) {
        self.client.invalidate();
    }
}
