use orbit_app_core::{DaemonClient, LocalZone, calendar::CalendarClock, features::FeatureFrame};
use orbit_ipc::Paths;
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;

struct ChangingClock {
    madrid: AtomicBool,
    ticks: watch::Sender<()>,
}

impl CalendarClock for ChangingClock {
    fn now_ms(&self) -> i64 {
        1_789_511_400_000
    }

    fn local_zone(&self, _now_ms: i64) -> LocalZone {
        LocalZone::fixed(if self.madrid.load(Ordering::SeqCst) {
            7_200_000
        } else {
            0
        })
    }

    fn timezone_name(&self) -> Result<String, String> {
        Ok(if self.madrid.load(Ordering::SeqCst) {
            "Europe/Madrid"
        } else {
            "UTC"
        }
        .into())
    }

    fn subscribe_ticks(&self) -> Option<watch::Receiver<()>> {
        Some(self.ticks.subscribe())
    }
}

fn status_zone(client: &DaemonClient) -> Option<String> {
    client.bootstrap().frames.into_iter().find_map(|frame| {
        if let FeatureFrame::Status { view, .. } = frame {
            Some(view.system_timezone.clone())
        } else {
            None
        }
    })
}

#[tokio::test]
async fn clock_tick_publishes_the_new_zone_without_a_daemon_frame() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (ticks, _) = watch::channel(());
    let clock = Arc::new(ChangingClock {
        madrid: AtomicBool::new(false),
        ticks,
    });
    let client = DaemonClient::connect_with_clock(paths, clock.clone(), fake_daemon::idle());
    assert_eq!(status_zone(&client).as_deref(), Some("UTC"));
    let mut changes = client.watch_frames();
    clock.madrid.store(true, Ordering::SeqCst);
    clock.ticks.send_replace(());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            changes.changed().await?;
            if client.take_frames().into_iter().any(|frame| {
                matches!(frame,
                FeatureFrame::Status { view, .. } if view.system_timezone == "Europe/Madrid")
            }) {
                break Ok::<(), tokio::sync::watch::error::RecvError>(());
            }
        }
    })
    .await??;
    assert_eq!(status_zone(&client).as_deref(), Some("Europe/Madrid"));
    Ok(())
}

#[path = "support/fake_daemon.rs"]
mod fake_daemon;
