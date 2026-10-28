//! Where a privacy stop admitted during startup is written before its
//! reply: the store file itself while the store recovers, then the store
//! actor.
use super::*;

#[derive(Clone)]
pub(in crate::daemon) enum StopStore {
    Direct {
        path: std::path::PathBuf,
        key: [u8; 32],
        snapshot_gate: Arc<tokio::sync::Mutex<()>>,
        worker_done: watch::Sender<bool>,
    },
    Actor(store_actor::StartupStopWriter),
}

impl StopStore {
    pub(super) async fn persist(&self, state: State) -> Result<()> {
        match self {
            Self::Direct {
                path,
                key,
                snapshot_gate,
                worker_done,
            } => {
                let _guard = snapshot_gate.clone().lock_owned().await;
                let path = path.clone();
                let key = *key;
                let mut done = worker_done.subscribe();
                let snapshot = state.durable();
                let write = |path: std::path::PathBuf, snapshot: DurableState| async move {
                    tokio::task::spawn_blocking(move || -> orbit_store::Result<()> {
                        Store::open_existing(&path, key)?.save_snapshot(&snapshot)
                    })
                    .await
                    .context("Early stop save finished")?
                    .map_err(anyhow::Error::from)
                };
                match write(path.clone(), snapshot.clone()).await {
                    Err(error)
                        if error
                            .downcast_ref::<orbit_store::Error>()
                            .is_some_and(|store| store.sqlite_contention().is_some())
                            && !*done.borrow() =>
                    {
                        done.changed()
                            .await
                            .context("Recovery finished without notifying the stop save")?;
                        write(path, snapshot).await
                    }
                    result => result,
                }
            }
            Self::Actor(writer) => writer.persist(state).await,
        }
    }
}
