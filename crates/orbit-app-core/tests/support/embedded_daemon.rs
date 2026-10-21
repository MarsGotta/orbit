//! The real daemon in an isolated runtime on a thread of this test process.
#![allow(dead_code)]
use orbit_core::protocol::Command;
use orbit_ipc::{DaemonEnded, LocalClient, Paths};
use std::{
    error::Error,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::{oneshot, watch};

pub struct TestDaemon {
    pub client: LocalClient,
    cancel: Option<oneshot::Sender<()>>,
    ended: watch::Receiver<Option<DaemonEnded>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TestDaemon {
    pub fn start(root: &Path, helper: Option<PathBuf>) -> Result<Self, Box<dyn Error>> {
        let paths = Paths::resolve(Some(root.to_owned()))?;
        let (connected, connection) = oneshot::channel();
        let (ended_tx, ended) = watch::channel(None);
        let end = ended.clone();
        let (cancel, cancelled) = oneshot::channel();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        // Match the app's 8 MiB daemon stack: a 113,328-byte
        // startup future was observed overflowing the usual 2 MiB test thread. Two workers
        // serve actor and helper I/O without one pool per host core per fixture.
        let thread = std::thread::Builder::new().name("orbit-test-daemon".into())
            .stack_size(8 * 1024 * 1024).spawn(move || {
                let result = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
                    Ok(runtime) => {
                        let result = runtime.block_on(async {
                            let run = orbit_daemon::daemon::run_embedded(paths, true, helper, connected, end);
                            tokio::pin!(run);
                            tokio::pin!(connection);
                            tokio::pin!(cancelled);
                            let mut delivered = false;
                            loop {
                                tokio::select! {
                                    result = &mut run => break result.map_err(|error| format!("{error:#}")),
                                    result = &mut connection, if !delivered => {
                                        delivered = true;
                                        if let Ok(client) = result { let _ = sender.send(Ok(client)); }
                                    }
                                    _ = &mut cancelled => break Err("Fixture: daemon run interrupted".into()),
                                }
                            }
                        });
                        drop(runtime);
                        result
                    }
                    Err(error) => Err(error.to_string()),
                };
                let outcome = match result {
                    Ok(()) => DaemonEnded::Closed,
                    Err(error) => { let _ = sender.try_send(Err(error.clone())); DaemonEnded::Failed(error) }
                };
                ended_tx.send_replace(Some(outcome));
            })?;
        // Preserve the fixtures' 30 s cold-start guard (3,969 ms worst
        // measured), distinct from their 120 s disk-recovery guard. Channel
        // delivery decides readiness, rather than a filesystem probe.
        let client = receiver.recv_timeout(Duration::from_secs(30))??;
        Ok(Self {
            client,
            cancel: Some(cancel),
            ended,
            thread: Some(thread),
        })
    }

    pub async fn ready(&self, budget: Duration) -> Result<(), Box<dyn Error>> {
        tokio::time::timeout(budget, async {
            loop {
                let reply = self.client.request_command(Command::Status).await?;
                if reply.status.ok_or("status absent")?.recovery
                    == orbit_core::RecoveryStatus::Ready
                {
                    return Ok::<_, Box<dyn Error>>(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await??;
        Ok(())
    }

    pub fn outcome(&self) -> Option<DaemonEnded> {
        self.ended.borrow().clone().or_else(|| {
            self.ended
                .has_changed()
                .is_err()
                .then_some(DaemonEnded::Vanished)
        })
    }

    pub async fn shutdown(mut self, budget: Duration) -> Result<(), Box<dyn Error>> {
        self.client.request_command(Command::Shutdown).await?;
        let ended = tokio::time::timeout(budget, self.client.ended()).await?;
        assert_eq!(ended, DaemonEnded::Closed);
        self.cancel()?;
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<(), Box<dyn Error>> {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| "daemon fixture thread panicked")?;
        }
        Ok(())
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

pub struct Topics {
    receiver: tokio::sync::mpsc::Receiver<orbit_core::topics::TopicFrame>,
    _watchers: tokio::task::JoinSet<()>,
}

impl Topics {
    pub async fn next_topic(&mut self) -> Result<orbit_core::topics::TopicFrame, Box<dyn Error>> {
        self.receiver
            .recv()
            .await
            .ok_or_else(|| "topic watches ended".into())
    }
}

pub async fn subscribe_topics(
    client: &LocalClient,
    ids: Vec<orbit_core::topics::TopicId>,
) -> Result<((), Topics), Box<dyn Error>> {
    let (sender, receiver) = tokio::sync::mpsc::channel(orbit_core::topics::TopicId::ALL.len());
    let mut watchers = tokio::task::JoinSet::new();
    for (id, mut watch) in client.topics() {
        if !ids.is_empty() && !ids.contains(&id) {
            continue;
        }
        let sender = sender.clone();
        watchers.spawn(async move {
            loop {
                let frame = watch.borrow_and_update().clone();
                if sender.send(frame).await.is_err() || watch.changed().await.is_err() {
                    break;
                }
            }
        });
    }
    Ok((
        (),
        Topics {
            receiver,
            _watchers: watchers,
        },
    ))
}

pub async fn request(
    client: &LocalClient,
    command: Command,
) -> Result<orbit_core::protocol::Response, Box<dyn Error>> {
    Ok(client.request_command(command).await?)
}

pub async fn read_topic(
    client: &LocalClient,
    read: orbit_core::reads::ReadRequest,
) -> Result<orbit_core::protocol::TopicResponse, Box<dyn Error>> {
    Ok(client.read_topic(read).await?)
}
