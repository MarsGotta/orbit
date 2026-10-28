use super::{ProjectTopic, TopicSource};
use orbit_core::{
    calendar::CalendarAuthorization,
    calendar_control::{CalendarWriteStage, CalendarWriteView},
    topics::{
        Knowledge, TopicData, TopicId,
        agenda::{Value, WriteOutcome, WriteTarget, access},
    },
};

pub(super) struct Projector;

impl ProjectTopic for Projector {
    fn project(&self, source: &TopicSource<'_>) -> TopicData {
        project(source)
    }
}

fn project(source: &TopicSource<'_>) -> TopicData {
    let view = source.runtime.map(|runtime| &runtime.calendar.view);
    let authorization = view
        .map(|view| view.authorization)
        .or(source.state.capabilities().calendar())
        .unwrap_or(CalendarAuthorization::Unknown);
    let access = source.runtime.map_or_else(
        || access(authorization, source.state.privacy_stopped()),
        |runtime| runtime.calendar_access(),
    );
    let knowledge = match access {
        Knowledge::Present { .. } => {
            let value = Value {
                invalidation: source
                    .runtime
                    .map_or(0, |runtime| runtime.calendar.topic_invalidation()),
                calendars: view
                    .filter(|_| {
                        source
                            .runtime
                            .is_none_or(|runtime| runtime.calendar.inventory_valid)
                    })
                    .and_then(|view| view.calendars.as_ref())
                    .and_then(|calendars| u32::try_from(calendars.len()).ok()),
                read_window: source
                    .runtime
                    .and_then(|runtime| runtime.calendar.topic_window()),
                read_id: source
                    .runtime
                    .and_then(|runtime| runtime.calendar.complete.as_ref())
                    .map(|snapshot| snapshot.snapshot_id.clone()),
                timezone: source
                    .runtime
                    .and_then(|runtime| runtime.calendar.complete.as_ref())
                    .map(|snapshot| snapshot.system_timezone.clone()),
                writes: Vec::new(),
            };
            Knowledge::Present {
                value: with_writes(value, view.map_or(&[][..], |view| &view.writes)),
                as_of_ms: None,
            }
        }
        Knowledge::Absent { refusal } => Knowledge::Absent { refusal },
        Knowledge::Unknown { why } => Knowledge::Unknown { why },
    };
    TopicData::Agenda(knowledge)
}

/// The newest write outcomes that keep the head inside its registered
/// ceiling (`TopicId::Agenda.max_bytes()`, 4 KiB), oldest first as the daemon
/// keeps them. The daemon follows up to `calendar::writes::WRITE_RECORDS`;
/// the head is not where they are kept, only how the app hears the latest
/// ones, so the oldest are the ones left out. No new ceiling: the head's own
/// is the bound, and an outcome carries its notice, not its occurrence.
fn with_writes(mut value: Value, writes: &[CalendarWriteView]) -> Value {
    let fits = |value: &Value| {
        serde_json::to_vec(&TopicData::Agenda(Knowledge::Present {
            value: value.clone(),
            as_of_ms: None,
        }))
        .is_ok_and(|bytes| bytes.len() <= TopicId::Agenda.max_bytes())
    };
    for write in writes.iter().rev() {
        value.writes.insert(0, outcome(write));
        if !fits(&value) {
            value.writes.remove(0);
            break;
        }
    }
    value
}

fn outcome(write: &CalendarWriteView) -> WriteOutcome {
    let finished_badly = matches!(
        write.stage,
        CalendarWriteStage::Refused | CalendarWriteStage::OutcomeUnknown
    );
    WriteOutcome {
        request_id: write.request_id.clone(),
        operation: write.operation,
        stage: write.stage,
        code: write
            .refusal
            .as_ref()
            .map(|refusal| refusal.code().to_owned()),
        notice: finished_badly.then(|| write.detail.clone()),
        saved: write.occurrence.as_deref().map(|occurrence| WriteTarget {
            calendar_item_id: occurrence.calendar_item_id.clone(),
            occurrence_start_ms: occurrence
                .original_occurrence_unix_ms
                .unwrap_or(occurrence.start_unix_ms),
            start_ms: occurrence.start_unix_ms,
            end_ms: occurrence.end_unix_ms,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::protocol::Command;
    use orbit_core::{calendar::CalendarAuthorization, topics::Knowledge};

    #[tokio::test]
    async fn native_invalidation_changes_the_published_head() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = crate::daemon::audio::tests::fixture(directory.path())?;
        crate::daemon::calendar::tests::seed(&mut runtime, &mut helper).await?;
        runtime.calendar.view.authorization = CalendarAuthorization::FullAccess;
        let project = |runtime: &crate::daemon::Runtime| {
            Projector.project(&TopicSource {
                state: &runtime.state,
                runtime: Some(runtime),
                now_ms: 0,
            })
        };
        let before = project(&runtime);
        runtime
            .native_event(serde_json::json!({"event": "calendar_invalidated"}))
            .await?;
        let after = project(&runtime);
        assert_ne!(before, after);
        Ok(())
    }

    const HOST: &str = "22222222-2222-4222-8222-222222222222";
    /// 1 October 2026, 10:00 UTC.
    const START: i64 = 1_790_848_800_000;

    async fn helper_command(
        receiver: &mut tokio::sync::mpsc::Receiver<serde_json::Value>,
        name: &str,
    ) -> anyhow::Result<serde_json::Value> {
        loop {
            let command = tokio::time::timeout(std::time::Duration::from_secs(2), receiver.recv())
                .await
                .map_err(|_| anyhow::anyhow!("the helper never received {name}"))?
                .ok_or_else(|| anyhow::anyhow!("the fake helper disconnected"))?;
            if command["command"] == name {
                return Ok(command);
            }
        }
    }

    fn head(runtime: &crate::daemon::Runtime) -> anyhow::Result<Value> {
        match Projector.project(&TopicSource {
            state: &runtime.state,
            runtime: Some(runtime),
            now_ms: 0,
        }) {
            TopicData::Agenda(Knowledge::Present { value, .. }) => Ok(value),
            other => anyhow::bail!("the agenda head is not present: {other:?}"),
        }
    }

    fn create(request_id: &str, title: &str) -> Command {
        use orbit_core::calendar::{CalendarEventDraft, CalendarSave};
        Command::Calendar {
            request: orbit_core::calendar_control::CalendarCommand::Save {
                request_id: request_id.into(),
                save: CalendarSave::Create(Box::new(CalendarEventDraft {
                    calendar_id: None,
                    title: title.into(),
                    start_ms: START,
                    end_ms: START + 1_800_000,
                    all_day: false,
                    time_zone: None,
                    location: None,
                    notes: None,
                    url: None,
                    availability: None,
                    recurrence: None,
                    alarms_minutes: None,
                })),
            },
        }
    }

    /// The app learns how each write ended from the agenda head —
    /// saved with the occurrence it left (what an undo names), or refused
    /// with its code and the notice that carries its numbers.
    #[tokio::test]
    async fn the_agenda_head_carries_each_write_outcome() -> anyhow::Result<()> {
        use orbit_core::calendar_control::{CalendarWriteOperation, CalendarWriteStage};
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = crate::daemon::audio::tests::fixture(directory.path())?;
        crate::daemon::calendar::tests::seed(&mut runtime, &mut helper).await?;

        runtime.command(create("create-1", "Café con Ada")).await?;
        let save = helper_command(&mut helper, "calendar_save").await?;
        let occurrence = serde_json::json!({
            "observation_id": "55555555-5555-4555-8555-000000000001",
            "calendar": {"calendar_id": "calendar-0000", "source_id": "account"},
            "calendar_item_id": "item-new", "start_unix_ms": START,
            "end_unix_ms": START + 1_800_000, "interpreted_timezone": "UTC",
            "is_floating": true, "is_all_day": false, "is_detached": false,
            "title": "Café con Ada", "status": "confirmed",
            "invitees_status": "not_provided", "event_url_status": "not_provided"
        });
        runtime
            .native_event(
                serde_json::json!({"event":"response","id":save["id"],"ok":true,
                "result":{"write_id":"77777777-7777-4777-8777-777777777777",
                    "native_instance_id":HOST,"privacy_generation":2,
                    "system_timezone":"UTC","occurrence":occurrence}}),
            )
            .await?;
        let saved = head(&runtime)?
            .writes
            .into_iter()
            .find(|write| write.request_id == "create-1")
            .ok_or_else(|| anyhow::anyhow!("the creation is not in the head"))?;
        assert_eq!(saved.operation, CalendarWriteOperation::Create);
        assert_eq!(saved.stage, CalendarWriteStage::Saved);
        assert_eq!(
            saved.saved,
            Some(orbit_core::topics::agenda::WriteTarget {
                calendar_item_id: "item-new".into(),
                occurrence_start_ms: START,
                start_ms: START,
                end_ms: START + 1_800_000,
            })
        );
        assert_eq!(saved.notice, None);

        runtime.command(create("create-2", "Comité")).await?;
        let refused = helper_command(&mut helper, "calendar_save").await?;
        runtime
            .native_event(
                serde_json::json!({"event":"response","id":refused["id"],"ok":false,
                "error":{"code":"calendar_not_editable","message":"read only",
                    "detail":{"reason":"calendar_read_only","calendar_id":"calendar-0002"}}}),
            )
            .await?;
        let writes = head(&runtime)?.writes;
        let refused = writes
            .iter()
            .find(|write| write.request_id == "create-2")
            .ok_or_else(|| anyhow::anyhow!("the refusal is not in the head"))?;
        assert_eq!(refused.stage, CalendarWriteStage::Refused);
        assert_eq!(refused.code.as_deref(), Some("calendar_not_editable"));
        assert!(
            refused
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("calendar-0002")),
            "{refused:?}"
        );
        assert_eq!(
            writes
                .iter()
                .map(|write| write.request_id.as_str())
                .collect::<Vec<_>>(),
            ["create-1", "create-2"],
            "oldest first, as the daemon keeps them"
        );
        Ok(())
    }

    /// The head stays inside its registered ceiling
    /// (`TopicId::Agenda.max_bytes()`, 4 KiB) however many writes the daemon
    /// follows: the newest that fit travel, the rest stay in the daemon.
    #[tokio::test]
    async fn the_head_keeps_the_newest_writes_within_its_ceiling() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = crate::daemon::audio::tests::fixture(directory.path())?;
        crate::daemon::calendar::tests::seed(&mut runtime, &mut helper).await?;
        for index in 0..16 {
            let request_id = format!("refused-{index:02}-{}", "x".repeat(40));
            runtime.command(create(&request_id, "Comité")).await?;
            let save = helper_command(&mut helper, "calendar_save").await?;
            runtime
                .native_event(
                    serde_json::json!({"event":"response","id":save["id"],"ok":false,
                    "error":{"code":"calendar_not_editable","message":"read only",
                        "detail":{"reason":"calendar_read_only",
                            "calendar_id":format!("calendar-{}", "c".repeat(120))}}}),
                )
                .await?;
        }
        let head = head(&runtime)?;
        let bytes = serde_json::to_vec(&TopicData::Agenda(Knowledge::Present {
            value: head.clone(),
            as_of_ms: None,
        }))?
        .len();
        assert!(
            bytes <= orbit_core::topics::TopicId::Agenda.max_bytes(),
            "{bytes} bytes"
        );
        assert!(!head.writes.is_empty() && head.writes.len() < 16);
        let newest = head
            .writes
            .last()
            .map(|write| write.request_id.clone())
            .unwrap_or_default();
        assert!(newest.starts_with("refused-15-"), "{newest}");
        Ok(())
    }
}
