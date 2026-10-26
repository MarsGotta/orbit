//! The series of Hoy's next meeting: its earlier meeting and the tasks of
//! its note, read from the daemon, which reads them from the hub.
use super::*;
use orbit_core::{notes::SeriesPrevious, reads::ReadRequest};

/// The calendar series of the next meeting, its start and its title's
/// digest (`orbit_core::series_title`), for the hub's fallback to the title.
pub(super) type SeriesTarget = (String, i64, Option<String>);

impl DaemonClient {
    /// The series of the meeting Hoy shows next, if it belongs to one.
    pub(super) async fn next_series(&self) -> Option<SeriesTarget> {
        let local = self.local_snapshot();
        let topics = self.inner.topics.read().await;
        let clock = self.inner.calendar.lock().await.local_clock();
        let next = crate::features::briefing::project(&crate::features::FeatureInput::with_clock(
            &local, &topics, clock,
        ))?
        .next?;
        let digest = orbit_core::series_title::title_digest(&next.title);
        Some((next.event?.calendar_item_id, next.start_ms, digest))
    }

    /// Reads the series' earlier meeting and keeps it, or why it could not
    /// be read; with no series, nothing is kept.
    pub(super) async fn read_series_previous(&self, target: Option<SeriesTarget>) {
        let read = match target {
            None => None,
            Some((calendar_item_id, before_ms, title_digest)) => {
                let answer = note_read::read_page(
                    &self.inner.daemon,
                    ReadRequest::SeriesPrevious {
                        calendar_item_id: calendar_item_id.clone(),
                        before_ms,
                        title_digest,
                        cursor: None,
                    },
                )
                .await
                .and_then(|page| {
                    page.items
                        .into_iter()
                        .next()
                        .ok_or_else(|| "La serie de la reunión llegó vacía".to_owned())
                })
                .and_then(|item| {
                    serde_json::from_value::<SeriesPrevious>(item)
                        .map_err(|error| format!("Serie de la reunión no válida: {error}"))
                });
                Some(Box::new(crate::SeriesPreviousRead {
                    calendar_item_id,
                    before_ms,
                    answer,
                }))
            }
        };
        self.update_local(|snapshot| {
            if let Some(notes) = snapshot.notes.as_mut() {
                notes.series_previous = read;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::{
        PROTOCOL_VERSION,
        protocol::{Command, TopicResponse},
        refusal::{Refusal, Service},
        topics::{Knowledge, Page},
    };
    use std::collections::BTreeMap;
    use tokio::sync::mpsc;

    const EARLIER: &str = "11111111-1111-4111-8111-111111111111";

    /// A daemon that answers the series read with `answer` and every
    /// subscription with nothing.
    fn daemon(
        answer: Knowledge<Page<serde_json::Value>>,
    ) -> (crate::client_fixture::Endpoint, tokio::task::JoinHandle<()>) {
        let (calls, mut actor) = mpsc::channel::<orbit_ipc::Call>(8);
        let endpoint = crate::client_fixture::channel(calls, orbit_ipc::TopicChannels::new());
        let task = tokio::spawn(async move {
            while let Some(call) = actor.recv().await {
                let read = match &call.request.body {
                    Command::Read { read } => Some(read.id()),
                    _ => None,
                };
                let response = TopicResponse {
                    v: PROTOCOL_VERSION,
                    id: call.request.id,
                    ok: true,
                    revisions: BTreeMap::new(),
                    read,
                    answer: read.map(|_| answer.clone()),
                    refusal: None,
                };
                if call.reply.send(response).is_err() {
                    eprintln!("series fixture reply had no receiver");
                }
            }
        });
        (endpoint, task)
    }

    /// The title goes to the daemon as its digest, for the hub's fallback to
    /// the earlier meeting of the same title.
    #[tokio::test]
    async fn the_series_read_carries_the_titles_digest() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = orbit_ipc::Paths::resolve(Some(directory.path().to_path_buf()))?;
        let (calls, mut actor) = mpsc::channel::<orbit_ipc::Call>(8);
        let endpoint = crate::client_fixture::channel(calls, orbit_ipc::TopicChannels::new());
        let (asked, mut received) = mpsc::channel(8);
        let task = tokio::spawn(async move {
            while let Some(call) = actor.recv().await {
                if let Command::Read { read } = &call.request.body
                    && asked.send(read.clone()).await.is_err()
                {
                    break;
                }
                drop(call.reply);
            }
        });
        let client = DaemonClient::connect(paths, endpoint.client.clone());
        let digest = orbit_core::series_title::title_digest("Comité de dirección");
        client
            .read_series_previous(Some(("serie".into(), 10, digest.clone())))
            .await;
        let read = received.recv().await.ok_or("no read")?;
        assert!(matches!(
            read,
            ReadRequest::SeriesPrevious { title_digest, .. } if title_digest == digest
        ));
        endpoint.abort();
        task.abort();
        Ok(())
    }

    fn kept(client: &DaemonClient) -> Option<crate::SeriesPreviousRead> {
        client
            .local_snapshot()
            .notes
            .and_then(|notes| notes.series_previous)
            .map(|read| *read)
    }

    /// What the daemon gave for the next meeting's series is kept with what
    /// was asked; a refusal is kept as its cause, and no series keeps nothing.
    #[tokio::test]
    async fn the_series_read_is_kept_with_its_question_or_its_cause()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = orbit_ipc::Paths::resolve(Some(directory.path().to_path_buf()))?;
        let answer = serde_json::json!({
            "calendar_item_id": "serie", "before_ms": 10,
            "previous": {"recording_id": EARLIER, "title": "Comité", "started_at_ms": 1,
                "note": {"state": "ready", "revision": 3, "short_summary": null,
                    "action_items": []}}
        });
        let (endpoint, task) = daemon(Knowledge::Present {
            value: Page {
                total: 1,
                items: vec![answer.clone()],
                next: None,
                revision: 0,
                store_version: None,
                handed_over_from: None,
            },
            as_of_ms: None,
        });
        let client = DaemonClient::connect(paths.clone(), endpoint.client.clone());
        client
            .read_series_previous(Some(("serie".into(), 10, None)))
            .await;
        let read = kept(&client).ok_or("nothing kept")?;
        assert_eq!(
            (read.calendar_item_id.as_str(), read.before_ms),
            ("serie", 10)
        );
        assert_eq!(read.answer, Ok(serde_json::from_value(answer)?));
        client.read_series_previous(None).await;
        assert_eq!(kept(&client), None);
        endpoint.abort();
        task.abort();

        let (endpoint, task) = daemon(Knowledge::Absent {
            refusal: Refusal::UpstreamFailed {
                service: Service::Hub,
                status: Some(404),
            },
        });
        let client = DaemonClient::connect(paths, endpoint.client.clone());
        client
            .read_series_previous(Some(("serie".into(), 10, None)))
            .await;
        let read = kept(&client).ok_or("nothing kept")?;
        assert!(read.answer.is_err_and(|cause| cause.contains("404")));
        endpoint.abort();
        task.abort();
        Ok(())
    }
}
