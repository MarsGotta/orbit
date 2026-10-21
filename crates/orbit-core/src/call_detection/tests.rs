//! The call detection's decision table, with time as an argument.
use super::*;

const ZOOM: &str = "us.zoom.xos";
const CHROME: &str = "com.google.Chrome";

fn app(pid: i32, bundle_id: &str, name: &str) -> CallApp {
    CallApp {
        application_pid: pid,
        bundle_id: bundle_id.into(),
        name: name.into(),
    }
}

fn zoom() -> CallApp {
    app(4242, ZOOM, "zoom.us")
}

/// A detection and the facts it is fed with; `now` moves forward only when
/// the test says so.
struct Table {
    detector: CallDetector,
    preference: CallPreference,
    privacy_stopped: bool,
    recording_live: bool,
    effects: Vec<CallEffect>,
}

impl Table {
    fn new(preference: CallPreference) -> Self {
        Self {
            detector: CallDetector::default(),
            preference,
            privacy_stopped: false,
            recording_live: false,
            effects: Vec::new(),
        }
    }

    fn at(&mut self, now_ms: i64, input: CallInput) -> CallUpdate {
        let facts = CallFacts {
            now_ms,
            preference: self.preference,
            privacy_stopped: self.privacy_stopped,
            recording_live: self.recording_live,
        };
        let update = self.detector.apply(input, &facts);
        self.effects.extend(update.effects.clone());
        update
    }

    fn holding(&mut self, now_ms: i64, applications: &[CallApp]) -> CallUpdate {
        self.at(
            now_ms,
            CallInput::Microphone {
                applications: applications.to_vec(),
            },
        )
    }

    fn tick(&mut self, now_ms: i64) -> CallUpdate {
        self.at(now_ms, CallInput::Tick)
    }

    /// Ticks once a second, as the daemon's maintenance does.
    fn ticks(&mut self, from_ms: i64, to_ms: i64) {
        let mut now = from_ms;
        while now <= to_ms {
            self.tick(now);
            now += 1_000;
        }
    }

    fn offer_id(&self, now_ms: i64) -> Option<String> {
        match self.detector.panel(now_ms) {
            Some(CallPanel::Offer { call_id, .. }) => Some(call_id),
            Some(CallPanel::Recording { .. } | CallPanel::Ending { .. }) | None => None,
        }
    }

    fn starts(&self) -> Vec<(String, bool)> {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                CallEffect::Start {
                    call_id, automatic, ..
                } => Some((call_id.clone(), *automatic)),
                CallEffect::ReadCalendar | CallEffect::Finish { .. } => None,
            })
            .collect()
    }

    fn finishes(&self) -> Vec<(String, FinishCause)> {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                CallEffect::Finish {
                    recording_id,
                    cause,
                    ..
                } => Some((recording_id.clone(), *cause)),
                CallEffect::ReadCalendar | CallEffect::Start { .. } => None,
            })
            .collect()
    }

    /// A recording the detection started on its own at 3 s, held by Zoom.
    fn recording_by_detection() -> Self {
        let mut table = Self::new(CallPreference::Record);
        table.holding(0, &[zoom()]);
        table.tick(3_000);
        let Some((call_id, _)) = table.starts().into_iter().next() else {
            panic!("«Grabar sin preguntar» did not start recording");
        };
        table.recording_live = true;
        table.at(
            3_100,
            CallInput::Started {
                call_id,
                recording_id: "recording-1".into(),
            },
        );
        table
    }
}

#[test]
fn zoom_for_one_second_is_not_a_call() {
    let mut table = Table::new(CallPreference::Notify);
    table.holding(0, &[zoom()]);
    table.holding(1_000, &[]);
    table.ticks(1_000, 20_000);
    assert_eq!(table.detector.panel(20_000), None);
    assert!(table.starts().is_empty());
}

#[test]
fn zoom_for_three_seconds_is_offered_with_its_name() {
    let mut table = Table::new(CallPreference::Notify);
    let first = table.holding(0, &[zoom()]);
    assert!(
        first.effects.contains(&CallEffect::ReadCalendar),
        "the agenda is read while the call confirms, so the offer can name the event"
    );
    table.tick(2_999);
    assert_eq!(table.detector.panel(2_999), None, "not a call before 3 s");
    table.tick(3_000);
    match table.detector.panel(3_000) {
        Some(CallPanel::Offer { app, .. }) => assert_eq!(app.name, "zoom.us"),
        other => panic!("expected an offer at 3 s, got {other:?}"),
    }
    assert!(table.starts().is_empty(), "«Avisarme» never records alone");
}

#[test]
fn now_not_is_not_offered_again_while_the_app_keeps_the_microphone() {
    let mut table = Table::new(CallPreference::Notify);
    table.holding(0, &[zoom()]);
    table.tick(3_000);
    let call_id = table.offer_id(3_000).unwrap_or_default();
    let update = table.at(4_000, CallInput::Dismiss { call_id });
    assert_eq!(update.refusal, None);
    assert_eq!(table.detector.panel(4_000), None);
    // A one-second glitch is not a release.
    table.holding(5_000, &[]);
    table.holding(6_000, &[zoom()]);
    table.ticks(6_000, 30_000);
    assert_eq!(table.detector.panel(30_000), None);
}

#[test]
fn releasing_and_coming_back_is_a_new_call_and_is_offered() {
    let mut table = Table::new(CallPreference::Notify);
    table.holding(0, &[zoom()]);
    table.tick(3_000);
    let first = table.offer_id(3_000).unwrap_or_default();
    table.at(
        4_000,
        CallInput::Dismiss {
            call_id: first.clone(),
        },
    );
    table.holding(5_000, &[]);
    table.ticks(5_000, 8_000);
    table.holding(9_000, &[zoom()]);
    table.tick(11_999);
    assert_eq!(table.detector.panel(11_999), None);
    table.tick(12_000);
    let second = table.offer_id(12_000);
    assert!(second.is_some(), "a new call after a release is offered");
    assert_ne!(second, Some(first), "each offer has its own name");
}

#[test]
fn an_offer_goes_away_when_the_call_ends() {
    let mut table = Table::new(CallPreference::Notify);
    table.holding(0, &[zoom()]);
    table.tick(3_000);
    assert!(table.offer_id(3_000).is_some());
    table.holding(4_000, &[]);
    table.tick(6_999);
    assert!(
        table.offer_id(6_999).is_some(),
        "a release is confirmed at 3 s"
    );
    table.tick(7_000);
    assert_eq!(table.detector.panel(7_000), None);
}

#[test]
fn record_without_asking_starts_and_says_so() {
    let mut table = Table::new(CallPreference::Record);
    table.holding(0, &[zoom()]);
    table.tick(2_999);
    assert!(table.starts().is_empty());
    table.tick(3_000);
    let starts = table.starts();
    assert_eq!(starts.len(), 1);
    assert!(starts[0].1, "started without asking");
    assert!(matches!(
        table.effects.iter().find(|effect| matches!(effect, CallEffect::Start { .. })),
        Some(CallEffect::Start { app, .. }) if app.application_pid == 4242
    ));
    table.recording_live = true;
    table.at(
        3_100,
        CallInput::Started {
            call_id: starts[0].0.clone(),
            recording_id: "recording-1".into(),
        },
    );
    assert!(matches!(
        table.detector.panel(3_100),
        Some(CallPanel::Recording { recording_id, .. }) if recording_id == "recording-1"
    ));
    assert_eq!(table.detector.owned_recording(), Some("recording-1"));
}

#[test]
fn nothing_means_nothing() {
    let mut table = Table::new(CallPreference::Nothing);
    table.holding(0, &[zoom()]);
    table.ticks(0, 20_000);
    assert_eq!(table.detector.panel(20_000), None);
    assert!(table.starts().is_empty());
}

#[test]
fn the_call_ending_counts_down_comes_back_cancels_and_ends_after_thirteen_seconds() {
    let mut table = Table::recording_by_detection();
    // Zoom leaves the microphone at 10 s: 3 s later the countdown starts.
    table.holding(10_000, &[]);
    table.tick(12_999);
    assert!(matches!(
        table.detector.panel(12_999),
        Some(CallPanel::Recording { .. })
    ));
    table.tick(13_000);
    assert_eq!(
        table.detector.panel(13_000),
        Some(CallPanel::Ending {
            recording_id: "recording-1".into(),
            app: zoom(),
            ends_at_ms: 26_000,
        })
    );
    // Five seconds into the countdown Zoom comes back: cancelled.
    table.holding(18_000, &[zoom()]);
    assert!(matches!(
        table.detector.panel(18_000),
        Some(CallPanel::Recording { .. })
    ));
    table.ticks(18_000, 40_000);
    assert!(
        table.finishes().is_empty(),
        "a cancelled countdown never ends it"
    );
    // It leaves again, and this time the thirteen seconds run out.
    table.holding(40_500, &[]);
    table.tick(43_500);
    assert!(matches!(
        table.detector.panel(43_500),
        Some(CallPanel::Ending {
            ends_at_ms: 56_500,
            ..
        })
    ));
    table.tick(56_499);
    assert!(table.finishes().is_empty());
    table.tick(56_500);
    assert_eq!(
        table.finishes(),
        vec![("recording-1".to_owned(), FinishCause::CallEnded)]
    );
    assert_eq!(table.detector.panel(56_500), None);
    assert_eq!(table.detector.owned_recording(), None);
}

#[test]
fn a_recording_started_by_hand_never_ends_with_the_call() {
    let mut table = Table::new(CallPreference::Notify);
    table.recording_live = true;
    table.holding(0, &[zoom()]);
    table.ticks(0, 10_000);
    assert_eq!(table.detector.panel(10_000), None, "already recording");
    table.holding(11_000, &[]);
    table.ticks(11_000, 60_000);
    assert!(table.finishes().is_empty());
    assert_eq!(table.detector.panel(60_000), None);
}

#[test]
fn an_unknown_signal_offers_nothing_and_ends_nothing() {
    let mut table = Table::new(CallPreference::Record);
    table.ticks(0, 20_000);
    assert_eq!(table.detector.holding_call(), None);
    assert!(
        table.effects.is_empty(),
        "before the first list nothing is known"
    );

    let mut table = Table::recording_by_detection();
    table.holding(10_000, &[]);
    table.tick(13_000);
    assert!(matches!(
        table.detector.panel(13_000),
        Some(CallPanel::Ending { .. })
    ));
    // The helper goes away: the end of the call is no longer known.
    table.at(14_000, CallInput::SignalLost);
    table.ticks(14_000, 60_000);
    assert!(
        table.finishes().is_empty(),
        "a lost signal never ends a recording"
    );
    assert!(matches!(
        table.detector.panel(60_000),
        Some(CallPanel::Recording { .. })
    ));
}

#[test]
fn privacy_stop_offers_nothing_and_rearming_resumes_nothing() {
    let mut table = Table::new(CallPreference::Record);
    table.privacy_stopped = true;
    table.holding(0, &[zoom()]);
    table.ticks(0, 10_000);
    assert!(table.effects.is_empty());
    assert_eq!(table.detector.panel(10_000), None);
    let refused = table.at(
        10_000,
        CallInput::Record {
            call_id: "call-1".into(),
        },
    );
    assert_eq!(refused.refusal, Some(CallRefusal::PrivacyStopped));
    // Rearmed with Zoom still on the microphone: it was there before.
    table.privacy_stopped = false;
    table.holding(11_000, &[zoom()]);
    table.ticks(11_000, 30_000);
    assert!(table.starts().is_empty(), "nothing resumes on its own");
    // A later call is a new one.
    table.holding(31_000, &[]);
    table.ticks(31_000, 35_000);
    table.holding(36_000, &[zoom()]);
    table.tick(39_000);
    assert_eq!(table.starts().len(), 1);
}

#[test]
fn privacy_stop_forgets_a_recording_the_call_started() {
    let mut table = Table::recording_by_detection();
    table.privacy_stopped = true;
    table.tick(5_000);
    table.privacy_stopped = false;
    table.holding(6_000, &[]);
    table.ticks(6_000, 60_000);
    assert!(
        table.finishes().is_empty(),
        "after the user's stop, nothing about that recording happens on its own"
    );
}

#[test]
fn keep_recording_cancels_the_countdown_and_makes_it_manual() {
    let mut table = Table::recording_by_detection();
    table.holding(10_000, &[]);
    table.tick(13_000);
    let update = table.at(
        14_000,
        CallInput::Keep {
            recording_id: "recording-1".into(),
        },
    );
    assert_eq!(update.refusal, None);
    assert_eq!(table.detector.panel(14_000), None);
    table.ticks(14_000, 60_000);
    assert!(table.finishes().is_empty());
    let refused = table.at(
        61_000,
        CallInput::Keep {
            recording_id: "recording-1".into(),
        },
    );
    assert_eq!(
        refused.refusal,
        Some(CallRefusal::NotOwned {
            recording_id: "recording-1".into()
        })
    );
}

#[test]
fn an_accepted_offer_records_and_also_ends_with_the_call() {
    let mut table = Table::new(CallPreference::Notify);
    table.holding(0, &[zoom()]);
    table.tick(3_000);
    let call_id = table.offer_id(3_000).unwrap_or_default();
    let update = table.at(
        4_000,
        CallInput::Record {
            call_id: call_id.clone(),
        },
    );
    assert_eq!(update.refusal, None);
    assert_eq!(table.starts(), vec![(call_id.clone(), false)]);
    table.recording_live = true;
    table.at(
        4_100,
        CallInput::Started {
            call_id,
            recording_id: "recording-2".into(),
        },
    );
    assert_eq!(
        table.detector.panel(4_100),
        None,
        "an accepted recording is shown by the pill, not the panel"
    );
    table.holding(10_000, &[]);
    table.tick(13_000);
    assert!(matches!(
        table.detector.panel(13_000),
        Some(CallPanel::Ending { .. })
    ));
    table.tick(26_000);
    assert_eq!(
        table.finishes(),
        vec![("recording-2".to_owned(), FinishCause::CallEnded)]
    );
}

#[test]
fn an_answer_to_an_offer_that_is_gone_is_refused_by_name() {
    let mut table = Table::new(CallPreference::Notify);
    table.holding(0, &[zoom()]);
    table.tick(3_000);
    let refused = table.at(
        4_000,
        CallInput::Record {
            call_id: "an-old-offer".into(),
        },
    );
    assert_eq!(
        refused.refusal,
        Some(CallRefusal::OfferGone {
            call_id: "an-old-offer".into()
        })
    );
    assert!(table.starts().is_empty());
    // «Ahora no» on something already gone is what the user asked for.
    let dismissed = table.at(
        4_000,
        CallInput::Dismiss {
            call_id: "an-old-offer".into(),
        },
    );
    assert_eq!(dismissed.refusal, None);
    assert!(table.offer_id(4_000).is_some());
}

#[test]
fn the_call_app_closing_during_the_countdown_ends_as_finish() {
    let mut table = Table::recording_by_detection();
    table.holding(10_000, &[]);
    table.tick(13_000);
    table.at(
        15_000,
        CallInput::CaptureCut {
            recording_id: "recording-1".into(),
        },
    );
    assert_eq!(
        table.finishes(),
        vec![("recording-1".to_owned(), FinishCause::CallAppClosed)]
    );
}

#[test]
fn a_capture_cut_while_the_app_still_holds_the_microphone_ends_nothing() {
    let mut table = Table::recording_by_detection();
    table.at(
        5_000,
        CallInput::CaptureCut {
            recording_id: "recording-1".into(),
        },
    );
    assert!(table.finishes().is_empty());
    assert_eq!(table.detector.owned_recording(), Some("recording-1"));
}

#[test]
fn a_dedicated_call_app_is_offered_before_a_browser() {
    let mut table = Table::new(CallPreference::Notify);
    table.holding(0, &[app(10, CHROME, "Google Chrome"), zoom()]);
    table.tick(3_000);
    assert!(matches!(
        table.detector.panel(3_000),
        Some(CallPanel::Offer { app, .. }) if app.bundle_id == ZOOM
    ));
}

#[test]
fn an_application_that_is_not_a_call_is_never_offered() {
    let mut table = Table::new(CallPreference::Record);
    let update = table.holding(0, &[app(77, "com.apple.Siri", "Siri")]);
    assert!(update.effects.is_empty());
    table.ticks(0, 20_000);
    assert!(table.effects.is_empty());
}

#[test]
fn a_recording_started_by_hand_answers_the_offer() {
    let mut table = Table::new(CallPreference::Notify);
    table.holding(0, &[zoom()]);
    table.tick(3_000);
    assert!(table.offer_id(3_000).is_some());
    // The user pressed «Grabar» on the calendar instead.
    table.recording_live = true;
    table.tick(4_000);
    assert_eq!(table.detector.panel(4_000), None);
    table.recording_live = false;
    table.ticks(5_000, 30_000);
    assert_eq!(
        table.detector.panel(30_000),
        None,
        "the same call is not offered again"
    );
}

/// «Grabar» on a calendar event captures the call's application
/// when the detection knows one holds the microphone; otherwise the system.
#[test]
fn the_call_holding_the_microphone_is_known_only_from_the_signal() {
    let mut table = Table::new(CallPreference::Nothing);
    assert_eq!(table.detector.holding_call(), None, "unknown is not «none»");
    table.holding(0, &[app(10, CHROME, "Google Chrome"), zoom()]);
    assert_eq!(table.detector.holding_call(), Some(&zoom()));
    table.holding(5_000, &[app(10, CHROME, "Google Chrome")]);
    assert_eq!(
        table
            .detector
            .holding_call()
            .map(|app| app.bundle_id.as_str()),
        Some(CHROME),
        "a call that left is not the one to record"
    );
    table.at(6_000, CallInput::SignalLost);
    assert_eq!(table.detector.holding_call(), None);
}
