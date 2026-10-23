use super::*;
use orbit_core::topics::Knowledge;

#[tokio::test]
async fn privacy_stop_replies_to_a_read_in_flight() -> Result<(), Box<dyn std::error::Error>> {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let (epoch_tx, epoch_rx) = watch::channel(0_u64);
    let task = tokio::spawn(async move {
        let _ = started_tx.send(());
        cancellable_remote_read(
            async {
                let _ = release_rx.await;
                orbit_core::topics::Knowledge::Unknown {
                    why: orbit_core::topics::Pending::NotObservedYet,
                }
            },
            epoch_rx,
            0,
        )
        .await
    });
    assert!(started_rx.await.is_ok());
    let mut was_stopped = false;
    publish_privacy_epoch(true, &mut was_stopped, &epoch_tx);
    let _ = release_tx.send(());
    assert!(
        matches!(
            task.await?,
            Knowledge::Absent {
                refusal: Refusal::PrivacyStopped { .. }
            }
        ),
        "an in-flight read must answer with the privacy refusal"
    );
    Ok(())
}

#[tokio::test]
async fn privacy_stop_refuses_a_local_read_while_its_worker_keeps_the_slot()
-> Result<(), Box<dyn std::error::Error>> {
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
    let permit = slots.clone().try_acquire_owned()?;
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel::<()>(0);
    let (epoch_tx, epoch_rx) = watch::channel(0_u64);
    let task = tokio::spawn(async move {
        cancellable_remote_read(
            async move {
                match tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    if started_tx.send(()).is_err() {
                        return;
                    }
                    if let Err(error) = release_rx.recv() {
                        eprintln!("local note test worker lost its release: {error}");
                    }
                })
                .await
                {
                    Ok(()) => Knowledge::Unknown {
                        why: orbit_core::topics::Pending::NotObservedYet,
                    },
                    Err(error) => Knowledge::Absent {
                        refusal: Refusal::StorageFailed {
                            operation: orbit_core::refusal::Operation::Storage,
                            cause: Some(error.to_string().into()),
                        },
                    },
                }
            },
            epoch_rx,
            0,
        )
        .await
    });
    started_rx.await?;
    let mut was_stopped = false;
    publish_privacy_epoch(true, &mut was_stopped, &epoch_tx);
    let answer = task.await?;
    let held_after_privacy_reply = slots.available_permits() == 0;
    release_tx.send(())?;
    let released =
        tokio::time::timeout(std::time::Duration::from_secs(5), slots.acquire_owned()).await??;
    drop(released);
    assert!(
        held_after_privacy_reply,
        "the local worker released its slot before finishing"
    );
    assert!(matches!(
        answer,
        Knowledge::Absent {
            refusal: Refusal::PrivacyStopped { .. }
        }
    ));
    Ok(())
}
