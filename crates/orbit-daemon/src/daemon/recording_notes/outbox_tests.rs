//! The notes outbox: what the person writes about a meeting waits on this
//! Mac until the hub took it, one lane per meeting. Every request goes to a
//! synthetic hub on loopback (`fake_hub`); none of this is evidence about
//! the real hub.
use super::fake_hub::{self, Hub, lock};
use crate::daemon::{Runtime, audio::tests::fixture};
use anyhow::{Context, Result};
use orbit_core::{notes::Section, protocol::Command};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

/// Sorts before the others: a lane taken by id would always take it first.
const REFUSED: &str = "1a000000-0000-4000-8000-000000000001";
const SILENT: &str = "2b000000-0000-4000-8000-000000000002";
const TODAY: &str = "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f";
const DEVICE: &str = "11111111-1111-4111-8111-111111111111";
const ACTION: &str = "a0123456789ab";
const START: i64 = 1_800_000_000_000;

fn hub_with(documents: &[&str]) -> fake_hub::Shared {
    let hub = Hub::default();
    let hub: fake_hub::Shared = Arc::new(Mutex::new(hub));
    for recording in documents {
        let mut document = fake_hub::document(recording, "Revisión sintética", START);
        document["note"] = fake_hub::note(7);
        lock(&hub).documents.insert((*recording).into(), document);
    }
    hub
}

async fn pair(runtime: &mut Runtime, url: &str) {
    runtime.settings.hub_url = Some(url.into());
    runtime.settings.device_id = Some(DEVICE.into());
    runtime.token = Some("synthetic-not-a-credential".into());
}

fn connection(runtime: &mut Runtime, value: &str) {
    crate::daemon::authority_fixture::set(&mut runtime.state, "connection", json!(value));
}

/// One turn of the daemon at `now_ms`: what is due goes out, and each answer
/// comes back through the event path production uses.
async fn turn(
    runtime: &mut Runtime,
    events: &mut tokio::sync::mpsc::Receiver<super::NoteEvent>,
    now_ms: i64,
) -> Result<usize> {
    runtime.features.notes.pinned_now = Some(now_ms);
    let started = runtime.maintain_recording_notes_at(now_ms);
    for _ in 0..started {
        answer(runtime, events, now_ms).await?;
    }
    Ok(started)
}

async fn answer(
    runtime: &mut Runtime,
    events: &mut tokio::sync::mpsc::Receiver<super::NoteEvent>,
    now_ms: i64,
) -> Result<()> {
    let event = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
        .await
        .context("the synthetic hub did not answer")?
        .context("the note channel closed")?;
    runtime.note_answer_at(event, now_ms).await
}

fn received(hub: &fake_hub::Shared, method: &str, route: &str) -> Vec<(String, Value)> {
    lock(hub)
        .received
        .iter()
        .filter(|request| request.method == method && request.path.ends_with(route))
        .map(|request| (request.path.clone(), request.body.clone()))
        .collect()
}

fn edit(recording: &str, done: Option<bool>, markdown: Option<&str>) -> Command {
    Command::NoteEdit {
        recording_id: recording.into(),
        sections: markdown.map(|markdown| {
            vec![Section {
                name: "Resumen".into(),
                markdown: markdown.into(),
            }]
        }),
        done: done
            .map(|done| BTreeMap::from([(ACTION.to_owned(), done)]))
            .unwrap_or_default(),
        base_revision: 7,
    }
}

fn private(recording: &str, markdown: &str) -> Command {
    Command::NotePrivateSave {
        recording_id: recording.into(),
        markdown: markdown.into(),
        base_revision: 0,
    }
}

/// With the hub down, a correction of the summary and a private note are
/// taken by the daemon; the daemon falls (no orderly closure), starts again,
/// still owes both, shows them over the hub's note, and they reach the hub
/// once it is back.
#[tokio::test]
async fn a_correction_and_a_private_note_written_with_the_hub_down_survive_a_restart() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let hub = hub_with(&[TODAY]);
    let url = fake_hub::start(hub.clone()).await?;
    {
        let (mut runtime, _) = fixture(directory.path())?;
        pair(&mut runtime, &url).await;
        connection(&mut runtime, "offline");
        runtime
            .command(edit(TODAY, None, Some("- la cifra corregida a mano")))
            .await
            .context("the correction was refused with the hub down")?;
        runtime
            .command(private(TODAY, "- llamar a la gestoría"))
            .await?;
        // What the store actor admitted is on disk; then the process dies.
        runtime.store_actor.run(|_| Ok(())).await?;
    }
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    pair(&mut runtime, &url).await;
    runtime.recover_recording_notes()?;
    connection(&mut runtime, "offline");
    assert_eq!(turn(&mut runtime, &mut events, START).await?, 0);
    assert!(lock(&hub).received.is_empty());

    connection(&mut runtime, "online");
    // Before it is sent, the note shows the correction as this Mac's.
    let detail = runtime
        .notes_read(&orbit_core::reads::ReadRequest::NoteDetail {
            id: TODAY.into(),
            cursor: None,
        })
        .context("a notes read")?
        .await;
    let orbit_core::topics::Knowledge::Present { value, .. } = detail else {
        anyhow::bail!("the note was not read: {detail:?}");
    };
    let shown: orbit_core::notes::MeetingDocument =
        serde_json::from_value(value.items.into_iter().next().context("empty detail")?)?;
    let sections = shown
        .note
        .as_ref()
        .and_then(|note| note.note.as_ref())
        .map(|note| note.sections.clone())
        .unwrap_or_default();
    assert_eq!(sections[0].markdown, "- la cifra corregida a mano");
    assert!(shown.pending_changes.contains(&"sections".to_owned()));
    assert!(shown.private_notes.pending);

    runtime.notes_hub_ready();
    for step in 0..3 {
        turn(&mut runtime, &mut events, START + 1_000 + step).await?;
    }
    let edits = received(&hub, "PUT", "/note/edits");
    assert_eq!(edits.len(), 1, "{edits:?}");
    assert_eq!(
        edits[0].1["sections"][0]["markdown"],
        "- la cifra corregida a mano"
    );
    assert_eq!(
        received(&hub, "PUT", "/private-notes")
            .into_iter()
            .map(|(_, body)| body["text"].clone())
            .collect::<Vec<_>>(),
        [json!("- llamar a la gestoría")]
    );
    assert!(runtime.reader.pending_private_notes()?.is_empty());
    assert!(runtime.reader.note_orders()?.is_empty());
    Ok(())
}

/// One meeting the hub refuses for ever (a recording of an earlier pairing)
/// and one whose request the hub never answers: today's meeting's private
/// note and title still reach the hub at once, the refusal is said with its
/// status, and the refused private note stays on this Mac without being
/// asked for again before its wait.
#[tokio::test]
async fn a_meeting_whose_order_the_hub_refuses_does_not_hold_the_others() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let hub = hub_with(&[TODAY]);
    lock(&hub).refusing.insert(REFUSED.into(), 404);
    lock(&hub).silent.insert(SILENT.into());
    let url = fake_hub::start(hub.clone()).await?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    pair(&mut runtime, &url).await;
    connection(&mut runtime, "offline");
    for recording in [REFUSED, SILENT, TODAY] {
        runtime
            .command(private(recording, "- lo de esta reunión"))
            .await?;
        runtime
            .command(Command::MeetingRename {
                recording_id: recording.into(),
                title: "Revisión del trimestre".into(),
            })
            .await
            .context("the title was refused with the hub down")?;
    }
    connection(&mut runtime, "online");
    runtime.notes_hub_ready();
    runtime.features.notes.pinned_now = Some(START);
    let started = runtime.maintain_recording_notes_at(START);
    // The silent meeting's two lanes never answer; every other one does.
    for _ in 0..started.saturating_sub(2) {
        answer(&mut runtime, &mut events, START).await?;
    }
    for _ in 0..4 {
        turn(&mut runtime, &mut events, START + 1).await?;
    }
    let today = |route: &str| {
        received(&hub, "PUT", route)
            .into_iter()
            .filter(|(path, _)| path.contains(TODAY))
            .count()
    };
    assert_eq!(today("/private-notes"), 1);
    assert_eq!(today("/title"), 1);
    let notice = serde_json::to_value(runtime.state.durable.notice.clone().context("no notice")?)?;
    assert_eq!(notice["code"], "note_change_refused", "{notice}");
    assert_eq!(notice["data"]["status"], 404);
    assert_eq!(
        runtime.reader.pending_private_notes()?,
        vec![REFUSED.to_owned(), SILENT.to_owned()]
    );
    let refused_before = received(&hub, "PUT", "/private-notes")
        .iter()
        .filter(|(path, _)| path.contains(REFUSED))
        .count();
    // Ten turns within the first wait: nothing more is asked of it.
    for step in 0..10 {
        turn(&mut runtime, &mut events, START + 2 + step).await?;
    }
    let refused_after = received(&hub, "PUT", "/private-notes")
        .iter()
        .filter(|(path, _)| path.contains(REFUSED))
        .count();
    assert_eq!((refused_before, refused_after), (1, 1));
    Ok(())
}

/// Two quick clicks on the same task: «done», then «not done». With the hub
/// down only the newest goes, once; with the hub up the second waits for the
/// first, and the hub keeps the last click either way.
#[tokio::test]
async fn two_quick_clicks_deliver_the_newest() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let hub = hub_with(&[TODAY]);
    let url = fake_hub::start(hub.clone()).await?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    pair(&mut runtime, &url).await;

    connection(&mut runtime, "offline");
    for done in [true, false] {
        runtime
            .command(edit(TODAY, Some(done), None))
            .await
            .context("the click was refused with the hub down")?;
    }
    connection(&mut runtime, "online");
    runtime.notes_hub_ready();
    turn(&mut runtime, &mut events, START).await?;
    let sent = received(&hub, "PUT", "/note/edits");
    assert_eq!(sent.len(), 1, "the superseded click was sent: {sent:?}");
    assert_eq!(sent[0].1["done"][ACTION], false);

    // With the hub up and slow: the first click is on its way when the
    // second is given.
    let hold = Arc::new(tokio::sync::Notify::new());
    lock(&hub).hold = Some(hold.clone());
    for done in [true, false] {
        runtime.command(edit(TODAY, Some(done), None)).await?;
    }
    lock(&hub).hold = None;
    hold.notify_waiters();
    answer(&mut runtime, &mut events, START + 1).await?;
    for step in 0..3 {
        turn(&mut runtime, &mut events, START + 2 + step).await?;
    }
    let kept = lock(&hub).documents[TODAY]["note"]["note"]["action_items"][0]["done"].clone();
    assert_eq!(
        kept,
        json!(false),
        "{:?}",
        received(&hub, "PUT", "/note/edits")
    );
    assert!(runtime.reader.note_orders()?.is_empty());
    Ok(())
}

/// A template edited with the hub down is owed after a restart (the
/// library read still shows it on its way) and reaches the hub once it is
/// back; a second edit of it before then replaces the first.
#[tokio::test]
async fn a_template_edited_with_the_hub_down_is_still_owed_after_a_restart() -> Result<()> {
    use orbit_core::notes::{LibraryItem, LibraryRequest, TemplateDraft, TemplateSection};
    let directory = tempfile::tempdir()?;
    let hub = hub_with(&[]);
    let url = fake_hub::start(hub.clone()).await?;
    let draft = |name: &str| TemplateDraft {
        name: name.into(),
        context: "Contexto escrito a mano".into(),
        sections: vec![TemplateSection {
            name: "Acuerdos".into(),
            instructions: "Lo que se decidió".into(),
        }],
        forked_from: None,
    };
    let pending = LibraryItem::Pending {
        target: "template:plantilla-sintetica".into(),
    };
    {
        let (mut runtime, _) = fixture(directory.path())?;
        pair(&mut runtime, &url).await;
        connection(&mut runtime, "offline");
        for name in ["Primera versión", "Segunda versión"] {
            runtime
                .command(Command::NoteLibrary {
                    request: LibraryRequest::SaveTemplate {
                        template_id: "plantilla-sintetica".into(),
                        draft: draft(name),
                    },
                })
                .await?;
        }
        assert!(runtime.library_state().contains(&pending));
        runtime.store_actor.run(|_| Ok(())).await?;
    }
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    pair(&mut runtime, &url).await;
    runtime.recover_recording_notes()?;
    assert!(runtime.library_state().contains(&pending));
    connection(&mut runtime, "online");
    runtime.notes_hub_ready();
    turn(&mut runtime, &mut events, START).await?;
    let saved = received(&hub, "PUT", "/v1/note-templates/plantilla-sintetica");
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_eq!(saved[0].1["name"], "Segunda versión");
    assert!(!runtime.library_state().contains(&pending));
    Ok(())
}

/// The person types again while their private notes are on their way: the
/// hub's answer confirms the text it was sent, and the newer one stays on
/// this Mac and goes next, instead of leaving with the older one's answer.
#[tokio::test]
async fn private_notes_typed_while_the_last_ones_are_on_their_way_still_go() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let hub = hub_with(&[TODAY]);
    let url = fake_hub::start(hub.clone()).await?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    pair(&mut runtime, &url).await;
    runtime.notes_hub_ready();
    turn(&mut runtime, &mut events, START).await?;
    let hold = Arc::new(tokio::sync::Notify::new());
    lock(&hub).hold = Some(hold.clone());
    runtime.command(private(TODAY, "- primera")).await?;
    runtime.features.notes.pinned_now = Some(START + 1);
    assert_eq!(runtime.maintain_recording_notes_at(START + 1), 1);
    runtime
        .command(private(TODAY, "- primera y segunda"))
        .await?;
    lock(&hub).hold = None;
    hold.notify_one();
    answer(&mut runtime, &mut events, START + 2).await?;
    assert_eq!(
        runtime.reader.pending_private_notes()?,
        vec![TODAY.to_owned()]
    );
    turn(&mut runtime, &mut events, START + 3).await?;
    let texts = received(&hub, "PUT", "/private-notes")
        .into_iter()
        .map(|(_, body)| body["text"].clone())
        .collect::<Vec<_>>();
    // The newer text was written on the revision before the first one's: the
    // hub's 409 names the current one and it goes once more on it.
    assert_eq!(texts.first(), Some(&json!("- primera")));
    assert_eq!(texts.last(), Some(&json!("- primera y segunda")));
    let kept = lock(&hub).documents[TODAY]["private_notes"]["text"].clone();
    assert_eq!(kept, json!("- primera y segunda"));
    assert!(runtime.reader.pending_private_notes()?.is_empty());
    Ok(())
}
