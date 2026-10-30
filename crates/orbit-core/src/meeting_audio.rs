//! Choosing which application's audio a meeting should record.
//!
//! Starting a meeting from the keyboard used to pass no application at all, so
//! the recording captured the microphone and nothing else. That failure is
//! silent and only shows up afterwards, in a transcript where everyone except
//! the person holding the Mac is missing: a 35-minute meeting came back as 318
//! words, all of them the user's.
//!
//! The decision is a pure function of what the Mac reported so it can be tested
//! without a Mac, and so that a wrong guess is a visible refusal rather than an
//! hour of recording the wrong window.

/// Applications whose audio is worth recording for a meeting, best first.
///
/// A dedicated conferencing app outranks a browser because a browser is usually
/// also everything else on the screen. Order inside a rank does not matter; the
/// ranks are what decide.
///
/// The same list says which application holding the microphone is a
/// call. Whereby and Google Meet are web pages, so their call is the browser's.
const DEDICATED: [&str; 11] = [
    "us.zoom.xos",
    "com.microsoft.teams",
    "com.microsoft.teams2",
    "com.cisco.webexmeetingsapp",
    "Cisco-Systems.Spark",
    "com.google.meetings",
    "com.hnc.Discord",
    "com.tinyspeck.slackmacgap",
    "com.apple.FaceTime",
    "com.skype.skype",
    "co.teamport.around",
];

/// Kept at five so the shipping list is two helper reads of eight
/// (`audio_sources::MAX_BUNDLE_IDS`); a browser added here is one more read
/// for every inventory of the start dialog.
const BROWSERS: [&str; 5] = [
    "com.google.Chrome",
    "com.apple.Safari",
    "company.thebrowser.Browser",
    "org.mozilla.firefox",
    "com.microsoft.edgemac",
];

/// A web app Chrome installs on its own (Meet «como app»): a browser window
/// with its own bundle, `com.google.Chrome.app.<id>`.
const CHROME_APP_PREFIX: &str = "com.google.Chrome.app.";

/// Every bundle identifier worth asking the Mac about.
pub fn candidate_bundle_ids() -> Vec<String> {
    DEDICATED
        .iter()
        .chain(BROWSERS.iter())
        // Only the isolated acceptance build offers the silent remote speaker.
        // It is not a conferencing app and must not trigger call detection.
        .chain(test_source_bundle_ids())
        .map(|id| (*id).to_owned())
        .collect()
}

#[cfg(feature = "pruebas")]
fn test_source_bundle_ids() -> std::slice::Iter<'static, &'static str> {
    ["app.orbit.pruebas.remote"].iter()
}

#[cfg(not(feature = "pruebas"))]
fn test_source_bundle_ids() -> std::slice::Iter<'static, &'static str> {
    [].iter()
}

/// What kind of call application a bundle is, if it is one at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CallKind {
    /// Zoom, Teams, FaceTime…: holding the microphone means a call.
    Dedicated,
    /// A browser: a call in a tab (Meet, Whereby, Teams on the web).
    Browser,
}

/// The kind of call application `bundle_id` is; `None` for anything else
/// (dictation, voice assistants, recorders), which never counts as a call.
pub fn call_kind(bundle_id: &str) -> Option<CallKind> {
    match rank(bundle_id)? {
        0 => Some(CallKind::Dedicated),
        _ => Some(CallKind::Browser),
    }
}

/// One running application the Mac reported back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningApplication {
    pub application_pid: i32,
    pub bundle_id: String,
    pub name: String,
}

/// What to record, or why nothing was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioChoice {
    /// Exactly one plausible application; record it.
    Application { application_pid: i32, name: String },
    /// Nothing recognisable was running.
    NoCandidate,
    /// Several equally plausible applications; picking one would be a guess.
    Ambiguous { names: Vec<String> },
}

fn rank(bundle_id: &str) -> Option<u8> {
    if DEDICATED.contains(&bundle_id) {
        Some(0)
    } else if BROWSERS.contains(&bundle_id) || bundle_id.starts_with(CHROME_APP_PREFIX) {
        Some(1)
    } else {
        None
    }
}

/// Picks the application to record from what the Mac reported.
///
/// Ties are refused rather than resolved: recording the wrong window for an
/// hour is worse than recording no system audio, because only one of the two
/// can be noticed while it is happening.
pub fn choose(running: &[RunningApplication]) -> AudioChoice {
    let mut best: Option<u8> = None;
    for application in running {
        if let Some(rank) = rank(&application.bundle_id) {
            best = Some(best.map_or(rank, |current: u8| current.min(rank)));
        }
    }
    let Some(best) = best else {
        return AudioChoice::NoCandidate;
    };
    let mut winners = running
        .iter()
        .filter(|application| rank(&application.bundle_id) == Some(best));
    let Some(first) = winners.next() else {
        return AudioChoice::NoCandidate;
    };
    if winners.next().is_some() {
        let mut names: Vec<String> = running
            .iter()
            .filter(|application| rank(&application.bundle_id) == Some(best))
            .map(|application| application.name.clone())
            .collect();
        names.sort();
        return AudioChoice::Ambiguous { names };
    }
    AudioChoice::Application {
        application_pid: first.application_pid,
        name: first.name.clone(),
    }
}

/// What to tell the person about the audio their meeting will contain.
///
/// Always says something. The silence is the defect being fixed: a meeting that
/// records only one voice must say so while there is still time to act, not in
/// a transcript read the next day.
pub fn notice(choice: &AudioChoice) -> crate::notice::Notice {
    use crate::notice::{Notice, NoticeCode, NoticeKey};
    match choice {
        AudioChoice::Application { .. } => Notice::new(NoticeCode::MeetingAudioApplication),
        AudioChoice::NoCandidate => Notice::new(NoticeCode::MeetingAudioMicrophoneOnly),
        AudioChoice::Ambiguous { names } => {
            Notice::new(NoticeCode::MeetingAudioAmbiguous).with(NoticeKey::Count, names.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(pid: i32, bundle_id: &str, name: &str) -> RunningApplication {
        RunningApplication {
            application_pid: pid,
            bundle_id: bundle_id.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn the_only_conferencing_app_running_is_the_one_recorded() {
        let choice = choose(&[app(42, "us.zoom.xos", "zoom.us")]);
        assert_eq!(
            choice,
            AudioChoice::Application {
                application_pid: 42,
                name: "zoom.us".to_owned(),
            }
        );
    }

    #[test]
    fn a_dedicated_app_wins_over_a_browser_that_is_always_open() {
        // A browser is usually also mail, docs and everything else, so it is the
        // weaker signal even when both are running.
        let choice = choose(&[
            app(7, "com.google.Chrome", "Google Chrome"),
            app(9, "com.microsoft.teams2", "Microsoft Teams"),
        ]);
        assert_eq!(
            choice,
            AudioChoice::Application {
                application_pid: 9,
                name: "Microsoft Teams".to_owned(),
            }
        );
    }

    #[test]
    fn a_browser_is_recorded_when_it_is_the_only_thing_that_could_hold_a_call() {
        // Meet has no app of its own; refusing here would mean never capturing
        // a Meet call at all.
        let choice = choose(&[app(3, "com.apple.Safari", "Safari")]);
        assert_eq!(
            choice,
            AudioChoice::Application {
                application_pid: 3,
                name: "Safari".to_owned(),
            }
        );
    }

    #[test]
    fn two_equally_likely_apps_are_refused_instead_of_guessed() {
        // An hour spent recording the wrong window cannot be noticed while it
        // happens; a refusal can.
        let choice = choose(&[
            app(1, "us.zoom.xos", "zoom.us"),
            app(2, "com.microsoft.teams2", "Microsoft Teams"),
        ]);
        assert_eq!(
            choice,
            AudioChoice::Ambiguous {
                names: vec!["Microsoft Teams".to_owned(), "zoom.us".to_owned()],
            }
        );
    }

    #[test]
    fn an_unknown_application_is_never_recorded_by_accident() {
        let choice = choose(&[app(5, "com.apple.Notes", "Notas")]);
        assert_eq!(choice, AudioChoice::NoCandidate);
        assert_eq!(choose(&[]), AudioChoice::NoCandidate);
    }

    #[test]
    fn every_outcome_says_what_the_recording_will_contain() {
        // The defect being fixed is silence: the user recorded 35 minutes and only
        // learned afterwards that no other voice was in it.
        use crate::notice::{NoticeClass, NoticeCode};
        let recorded = notice(&AudioChoice::Application {
            application_pid: 1,
            name: "zoom.us".to_owned(),
        });
        assert_eq!(recorded.code, NoticeCode::MeetingAudioApplication);

        for single_voice in [
            AudioChoice::NoCandidate,
            AudioChoice::Ambiguous {
                names: vec!["zoom.us".to_owned()],
            },
        ] {
            let warning = notice(&single_voice);
            assert_eq!(
                warning.class(),
                NoticeClass::Fixable,
                "a recording with a single voice must say so: {warning:?}"
            );
        }
    }

    #[test]
    fn a_call_is_a_call_application_or_a_browser_and_nothing_else() {
        assert_eq!(call_kind("com.apple.FaceTime"), Some(CallKind::Dedicated));
        assert_eq!(call_kind("us.zoom.xos"), Some(CallKind::Dedicated));
        assert_eq!(call_kind("com.google.Chrome"), Some(CallKind::Browser));
        assert_eq!(
            call_kind("com.google.Chrome.app.kjgfgldnnfoeklkmfkjfagphfepbbdan"),
            Some(CallKind::Browser)
        );
        // Dictation and voice assistants hold the microphone too.
        assert_eq!(call_kind("com.apple.SpeechRecognitionCore"), None);
        assert_eq!(call_kind("com.apple.Siri"), None);
        assert_eq!(call_kind(""), None);
    }

    #[test]
    fn the_mac_is_only_asked_about_applications_worth_recording() {
        let ids = candidate_bundle_ids();
        assert!(ids.contains(&"us.zoom.xos".to_owned()));
        assert!(ids.contains(&"com.google.Chrome".to_owned()));
        assert!(!ids.contains(&"com.apple.Notes".to_owned()));
        // The native side caps how many identifiers it will accept.
        assert!(ids.len() <= 32, "the list cannot grow without a bound");
    }

    #[test]
    fn the_remote_speaker_fixture_is_only_offered_by_the_test_build() {
        let offered = candidate_bundle_ids().contains(&"app.orbit.pruebas.remote".to_owned());
        assert_eq!(offered, cfg!(feature = "pruebas"));
        // A test source is never classified as a live conferencing call.
        assert_eq!(call_kind("app.orbit.pruebas.remote"), None);
    }
}
