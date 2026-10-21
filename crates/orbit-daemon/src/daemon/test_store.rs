use orbit_store::{Result, Store};
use std::{path::Path, time::Duration};

/// Unit fixtures tolerate scheduler and SQLite contention without changing
/// the bounded two-second wait used by production connections.
const TEST_BUSY_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) fn configure(store: Store) -> Result<Store> {
    store.set_busy_timeout(TEST_BUSY_TIMEOUT)?;
    Ok(store)
}

pub(super) fn open(directory: impl AsRef<Path>, key: [u8; 32]) -> Result<Store> {
    configure(Store::open(directory, key)?)
}

pub(super) fn open_existing(directory: impl AsRef<Path>, key: [u8; 32]) -> Result<Store> {
    configure(Store::open_existing(directory, key)?)
}
