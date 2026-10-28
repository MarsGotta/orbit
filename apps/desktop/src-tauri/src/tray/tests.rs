//! The menu bar: what it shows and offers, from the frames it follows.
use super::*;

#[test]
fn tray_recording_delegates_its_automatic_title_to_app_core() {
    let AppCommand::Recording(
        orbit_app_core::features::recording::RecordingCommand::MeetingStart { title },
    ) = recording_command()
    else {
        panic!("the tray did not start a recording");
    };
    assert!(title.is_empty(), "only app-core reads the local clock");
}

fn labels(state: &TrayState) -> Vec<(String, bool)> {
    entries(state)
        .into_iter()
        .map(|entry| match entry {
            Entry::Item { label, enabled, .. } => (label, enabled),
            Entry::Separator => ("—".to_owned(), false),
        })
        .collect()
}

fn owned(rows: &[(&str, bool)]) -> Vec<(String, bool)> {
    rows.iter()
        .map(|(label, on)| ((*label).to_owned(), *on))
        .collect()
}

/// The menu bar's menu, grouped as the window is: open it; record or
/// finish, only the one that can be done now; stop everything; quit.
#[test]
fn the_menu_offers_only_what_can_be_done_now() {
    assert_eq!(
        labels(&spanish_state()),
        owned(&[
            ("Abrir Orbit", true),
            ("—", false),
            ("Grabar reunión ahora", true),
            ("Terminar grabación", false),
            ("Terminar sin guardar…", false),
            ("—", false),
            ("Parar todo", true),
            ("—", false),
            ("Salir de Orbit", true),
        ])
    );
    let mut recording = spanish_state();
    recording.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-1", START),
        None,
        START,
    );
    let rows = labels(&recording);
    assert_eq!(rows[2], ("Grabar reunión ahora".to_owned(), false));
    assert_eq!(rows[3], ("Terminar grabación".to_owned(), true));
    assert_eq!(rows[4], ("Terminar sin guardar…".to_owned(), true));
}

/// «Terminar sin guardar…» only asks: the menu, opened again, says what it
/// deletes and offers «Borrar y terminar» or «Seguir grabando». Only the
/// confirmation sends the order, naming the recording it asked about.
#[test]
fn ending_without_saving_asks_in_the_menu_first() {
    let mut state = spanish_state();
    state.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-1", START),
        None,
        START,
    );
    assert!(
        discard_command(&state).is_none(),
        "the first click only asks"
    );
    state.ask_discard();
    let rows = labels(&state);
    assert_eq!(
        rows[3..7],
        owned(&[
            ("Terminar grabación", true),
            (
                "¿Terminar sin guardar? Se borra lo grabado y no se escribe nota.",
                false
            ),
            ("Borrar y terminar", true),
            ("Seguir grabando", true),
        ])
    );
    let Some(AppCommand::Daemon(command)) = discard_command(&state) else {
        panic!("the confirmation sends nothing");
    };
    assert_eq!(
        serde_json::to_value(*command).ok(),
        serde_json::to_value(orbit_core::protocol::Command::MeetingDiscard {
            recording_id: "rec-1".into()
        })
        .ok()
    );
    state.keep_recording();
    assert_eq!(
        labels(&state)[4],
        ("Terminar sin guardar…".to_owned(), true)
    );
    assert!(discard_command(&state).is_none());
}

/// The question never outlives the recording it asked about, and a
/// recording still starting cannot be thrown away yet.
#[test]
fn the_question_goes_with_its_recording() {
    let mut state = spanish_state();
    state.follow_meeting(
        &meeting(MeetingPhase::Starting, "rec-1", START),
        None,
        START,
    );
    assert_eq!(
        labels(&state)[4],
        ("Terminar sin guardar…".to_owned(), false)
    );
    state.follow_meeting(&meeting(MeetingPhase::Paused, "rec-1", START), None, START);
    state.ask_discard();
    state.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-2", START),
        None,
        START,
    );
    assert!(discard_command(&state).is_none());
    assert_eq!(
        labels(&state)[4],
        ("Terminar sin guardar…".to_owned(), true)
    );
    state.ask_discard();
    state.follow_meeting(&meeting(MeetingPhase::Idle, "rec-2", START), None, START);
    assert!(discard_command(&state).is_none());
    assert_eq!(
        labels(&state)[4],
        ("Terminar sin guardar…".to_owned(), false)
    );
}

#[test]
fn a_new_version_sits_beside_parar_todo_and_says_when_it_cannot_install() {
    let mut offered = TrayState {
        update: Some(UpdateOffer {
            version: "0.10.2".to_owned(),
            installable: false,
        }),
        ..spanish_state()
    };
    offered.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-1", START),
        None,
        START,
    );
    let rows = labels(&offered);
    assert_eq!(rows[7], ("Parar todo".to_owned(), true));
    assert_eq!(rows[8], ("Actualizar a la 0.10.2".to_owned(), false));
    assert_eq!(rows.last(), Some(&("Salir de Orbit".to_owned(), true)));
}

/// «Silenciar» beside «Terminar sin guardar…» while a recording records or
/// a call is on, «Reactivar micrófono» once the daemon says it is muted;
/// never another mark.
#[test]
fn the_menu_mutes_and_unmutes_the_microphone_of_a_recording_or_a_call() {
    let mut state = spanish_state();
    state.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-1", START),
        None,
        START,
    );
    assert_eq!(
        labels(&state)[4],
        ("Terminar sin guardar…".to_owned(), true)
    );
    assert_eq!(labels(&state)[5], ("Silenciar".to_owned(), true));
    let mut muted = meeting(MeetingPhase::Recording, "rec-1", START);
    muted.microphone_muted = true;
    state.follow_meeting(&muted, None, START);
    assert_eq!(labels(&state)[5], ("Reactivar micrófono".to_owned(), true));
    assert_eq!(look(&state, START).image, MarkImage::RecordingOnDark);
    state.follow_meeting(&meeting(MeetingPhase::Paused, "rec-1", START), None, START);
    assert!(!labels(&state).iter().any(|(label, _)| label == "Silenciar"));
    let call = CallSnapshot {
        started_at_ms: START,
        seeing_screen: false,
        last_line: None,
        speaking: false,
        voice_unavailable: false,
        expires_at_ms: START + 60_000,
        origin: orbit_app_core::CallOrigin::App,
        agent_id: None,
        idle_hangup_ms: 60_000,
        microphone_muted: true,
    };
    state.follow_meeting(
        &meeting(MeetingPhase::Idle, "rec-1", START),
        Some(&call),
        START,
    );
    assert!(labels(&state).contains(&("Reactivar micrófono".to_owned(), true)));
    assert_eq!(look(&state, START), rest(), "a call keeps the mark at rest");
}

#[test]
fn a_recording_can_be_finished_while_it_starts_or_is_paused_but_not_while_it_saves() {
    assert!(finishable(&MeetingPhase::Starting));
    assert!(finishable(&MeetingPhase::Recording));
    assert!(finishable(&MeetingPhase::Paused));
    assert!(!finishable(&MeetingPhase::Finishing));
    assert!(!finishable(&MeetingPhase::Idle));
}

fn meeting(phase: MeetingPhase, id: &str, started_at_ms: i64) -> MeetingSnapshot {
    let mut meeting =
        orbit_app_core::AppSnapshot::empty(1, orbit_app_core::DaemonConnection::Ready).meeting;
    meeting.phase = phase;
    meeting.recording_id = Some(id.to_owned());
    meeting.started_at_ms = Some(started_at_ms);
    meeting
}

fn chat(pending: Option<orbit_app_core::ChatPending>) -> ChatSnapshot {
    let mut chat =
        orbit_app_core::AppSnapshot::empty(1, orbit_app_core::DaemonConnection::Ready).chat;
    chat.pending_contract = pending;
    chat
}

fn approval() -> orbit_app_core::ChatPending {
    orbit_app_core::ChatPending::Approval {
        id: "approval-1".to_owned(),
        text: "Borrar un borrador".to_owned(),
        choices: vec!["once".to_owned(), "deny".to_owned()],
        deletes: None,
    }
}

fn rest() -> Look {
    Look {
        image: MarkImage::Template,
        title: String::new(),
    }
}

const START: i64 = 1_800_000_000_000;

/// At rest the bar shows the brand's small symbol as a template, which
/// macOS tints like its own marks, with nothing beside it.
#[test]
fn at_rest_the_mark_is_the_template_symbol_with_nothing_beside_it() {
    for appearance in [Appearance::Dark, Appearance::Light] {
        let state = TrayState {
            appearance,
            ..spanish_state()
        };
        assert_eq!(look(&state, START), rest());
    }
    assert!(MarkImage::Template.is_template());
}

/// A recording turns the moon red, in the file for the bar's appearance,
/// with the time recorded beside it: «mm:ss», then «h:mm:ss».
#[test]
fn a_recording_turns_the_moon_red_and_shows_its_time() {
    let mut state = spanish_state();
    state.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-1", START),
        None,
        START,
    );
    assert_eq!(
        look(&state, START + 42_000),
        Look {
            image: MarkImage::RecordingOnDark,
            title: "00:42".to_owned(),
        }
    );
    assert_eq!(look(&state, START + 724_000).title, "12:04");
    assert_eq!(look(&state, START + 3_723_000).title, "1:02:03");
    assert!(!MarkImage::RecordingOnDark.is_template());
    state.appearance = Appearance::Light;
    assert_eq!(
        look(&state, START + 42_000).image,
        MarkImage::RecordingOnLight
    );
    assert!(!MarkImage::RecordingOnLight.is_template());
}

/// The time stops in a pause and goes on from there, as the sidebar's
/// card counts it.
#[test]
fn a_pause_stops_the_time_beside_the_mark() {
    let mut state = spanish_state();
    state.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-1", START),
        None,
        START,
    );
    state.follow_meeting(
        &meeting(MeetingPhase::Paused, "rec-1", START),
        None,
        START + 60_000,
    );
    assert_eq!(look(&state, START + 600_000).title, "01:00");
    state.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-1", START),
        None,
        START + 120_000,
    );
    assert_eq!(look(&state, START + 130_000).title, "01:10");
    // Another recording starts its own count.
    state.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-2", START + 200_000),
        None,
        START + 200_000,
    );
    assert_eq!(look(&state, START + 205_000).title, "00:05");
}

/// An assistant that waits for an approval or an answer turns the moon
/// amber until the user answers.
#[test]
fn an_assistant_waiting_for_you_turns_the_moon_amber_until_you_answer() {
    let mut state = spanish_state();
    state.follow_chat(&chat(Some(approval())));
    assert_eq!(
        look(&state, START),
        Look {
            image: MarkImage::NeedsYouOnDark,
            title: String::new(),
        }
    );
    state.appearance = Appearance::Light;
    assert_eq!(look(&state, START).image, MarkImage::NeedsYouOnLight);
    state.follow_chat(&chat(Some(orbit_app_core::ChatPending::Clarify {
        id: "clarify-1".to_owned(),
        text: "¿Qué calendario?".to_owned(),
        choices: Vec::new(),
        questions: Vec::new(),
    })));
    assert_eq!(look(&state, START).image, MarkImage::NeedsYouOnLight);
    assert!(!MarkImage::NeedsYouOnLight.is_template());
    state.follow_chat(&chat(None));
    assert_eq!(look(&state, START), rest());
}

/// When the recording ends the bar goes back to the template, or to the
/// amber moon of an assistant still waiting.
#[test]
fn the_mark_goes_back_when_the_recording_ends() {
    let mut state = spanish_state();
    state.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-1", START),
        None,
        START,
    );
    state.follow_meeting(
        &meeting(MeetingPhase::Finishing, "rec-1", START),
        None,
        START + 5_000,
    );
    assert_eq!(look(&state, START + 6_000), rest());
    state.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-2", START),
        None,
        START,
    );
    state.follow_chat(&chat(Some(approval())));
    assert_eq!(look(&state, START).image, MarkImage::RecordingOnDark);
    state.follow_meeting(&meeting(MeetingPhase::Idle, "rec-2", START), None, START);
    assert_eq!(
        look(&state, START),
        Look {
            image: MarkImage::NeedsYouOnDark,
            title: String::new(),
        }
    );
}

fn opaque(image: &Image<'_>) -> Vec<[u8; 3]> {
    image
        .rgba()
        .chunks(4)
        .filter(|pixel| pixel[3] > 200)
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect()
}

/// The embedded files are the pack's, 18 points at 2×: the template
/// carries its shape only in its alpha; the recording one has a red
/// moon and the warning one an amber moon, with the symbol in white for
/// a dark bar and in near black for a light one.
#[test]
fn each_mark_is_the_brand_image_it_names() {
    let red = |[r, g, b]: [u8; 3]| r > 200 && g < 100 && b < 100;
    let amber = |[r, g, b]: [u8; 3]| r > 200 && (120..200).contains(&g) && b < 40;
    let white = |[r, g, b]: [u8; 3]| r > 240 && g > 240 && b > 240;
    let ink = |[r, g, b]: [u8; 3]| r < 40 && g < 40 && b < 40;
    for (mark, moon, symbol) in [
        (
            MarkImage::RecordingOnDark,
            &red as &dyn Fn([u8; 3]) -> bool,
            &white as &dyn Fn([u8; 3]) -> bool,
        ),
        (MarkImage::RecordingOnLight, &red, &ink),
        (MarkImage::NeedsYouOnDark, &amber, &white),
        (MarkImage::NeedsYouOnLight, &amber, &ink),
    ] {
        let image = mark.pixels();
        assert_eq!((image.width(), image.height()), (36, 36), "{mark:?}");
        let pixels = opaque(&image);
        assert!(pixels.iter().any(|pixel| moon(*pixel)), "{mark:?}: no moon");
        assert!(
            pixels.iter().any(|pixel| symbol(*pixel)),
            "{mark:?}: no symbol"
        );
    }
    let template = MarkImage::Template.pixels();
    assert_eq!((template.width(), template.height()), (36, 36));
    assert!(
        template.rgba().chunks(4).any(|pixel| pixel[3] > 0),
        "no ink"
    );
    assert!(
        template
            .rgba()
            .chunks(4)
            .all(|pixel| pixel[3] == 0 || pixel[..3] == [0, 0, 0])
    );
}

fn spanish_state() -> TrayState {
    TrayState {
        language: orbit_core::language::ResolvedLanguage::Spanish,
        ..TrayState::default()
    }
}

#[test]
fn english_menu_covers_recording_confirmation_mute_and_update() {
    let mut state = TrayState {
        language: orbit_core::language::ResolvedLanguage::English,
        ..TrayState::default()
    };
    state.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-1", START),
        None,
        START,
    );
    state.ask_discard();
    state.update = Some(UpdateOffer {
        version: "1.2.3".into(),
        installable: true,
    });
    let rows = labels(&state);
    for label in [
        "Open Orbit",
        "Record meeting now",
        "End recording",
        "End without saving? The recording will be deleted and no note will be written.",
        "Delete and end",
        "Keep recording",
        "Mute",
        "Stop everything",
        "Update to 1.2.3",
        "Quit Orbit",
    ] {
        assert!(
            rows.iter().any(|(text, _)| text == label),
            "missing {label}"
        );
    }
    state.microphone_muted = Some(true);
    state.keep_recording();
    let rows = labels(&state);
    assert!(rows.contains(&("Unmute microphone".into(), true)));
    assert!(rows.contains(&("End without saving…".into(), true)));
}

#[test]
fn changing_the_setting_rebuilds_the_menu_and_preserves_recording_and_confirmation() {
    use orbit_core::language::LanguageChoice;
    let mut state = spanish_state();
    state.follow_meeting(
        &meeting(MeetingPhase::Recording, "rec-1", START),
        None,
        START,
    );
    state.ask_discard();
    let mut menus = Vec::new();
    for choice in [
        LanguageChoice::English,
        LanguageChoice::English,
        LanguageChoice::System,
    ] {
        let result = render_change(
            &mut state,
            |state, _| state.language = choice.resolve(&["fr-FR", "es-ES"]),
            START,
            |menu, _, _| {
                if let Some(menu) = menu {
                    menus.push(format!("{menu:?}"));
                }
                Ok::<_, ()>(())
            },
        );
        assert!(result.is_ok(), "menu redraw failed");
    }
    assert_eq!(menus.len(), 2, "only changes rebuild the menu");
    assert!(menus[0].contains("Delete and end"));
    assert!(menus[1].contains("Borrar y terminar"));
    assert!(state.asking_discard);
    assert!(state.recording.is_some());
}

#[test]
fn a_failed_language_redraw_keeps_the_previous_menu_and_can_be_retried() {
    let mut state = spanish_state();
    let english = orbit_core::language::ResolvedLanguage::English;
    assert!(
        render_change(
            &mut state,
            |state, _| state.language = english,
            START,
            |_, _, _| Err::<(), _>("native menu failed")
        )
        .is_err()
    );
    assert_eq!(
        state.language,
        orbit_core::language::ResolvedLanguage::Spanish
    );
    let mut replaced = false;
    let result = render_change(
        &mut state,
        |state, _| state.language = english,
        START,
        |menu, _, _| {
            replaced = menu.is_some();
            Ok::<_, ()>(())
        },
    );
    assert!(result.is_ok());
    assert!(replaced);
}
