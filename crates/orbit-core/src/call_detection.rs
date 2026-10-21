//! Orbit notices a call, offers to record it, and ends a recording the
//! call started when the call ends.
//!
//! A call is an application of the call list (`meeting_audio::call_kind`)
//! that the helper reports holding the microphone (`microphone_activity`)
//! for at least [`CALL_CONFIRM_MS`]. What happens then is the user's one
//! preference, «Al detectar una llamada» ([`CallPreference`]); the macOS
//! microphone permission is the only door, so there is nothing else to
//! accept, review or renew. A recording the detection started (accepted or on
//! its own) ends when its application leaves the microphone for
//! [`CALL_RELEASE_MS`] and a visible, cancellable [`AUTO_STOP_COUNTDOWN_MS`]
//! runs out; a recording started by hand never ends on its own.
//!
//! Pure: time is an argument (`CallFacts::now_ms`, Unix milliseconds from the
//! daemon), and the effects are descriptions the daemon carries out.
//!
//! Before the helper's first list, or with the helper gone, nothing is known
//! (AGENTS.md, «Absence read as a negative fact»): no offer is made and no
//! recording is ended for lack of a signal.
use crate::meeting_audio::{CallKind, call_kind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

mod event;
pub use event::{Attendee, LinkedEvent, LinkedEventRefusal, choose_event, conference_platform_of};

#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod tests;

/// How long an application must hold the microphone before it is a call.
///
/// Protects: the user from an offer for a dictation, a microphone test or the
/// first second of a voice message — a second or two of microphone that is
/// not a call. Why three seconds: the measured debounce before treating a
/// change as settled, and a call's first words take longer than that to
/// start. It is not a limit on how short a recording may be.
pub const CALL_CONFIRM_MS: i64 = 3_000;

/// How long the call's application must stay off the microphone before the
/// call counts as over.
///
/// Protects: a recording, and an offer, from a call application that drops
/// the microphone for an instant (changing devices, muting in some apps,
/// reconnecting). Why three seconds: the measured debounce for a release;
/// a hang-up stays released, a glitch comes back within a second. It is not
/// the countdown: that starts after it.
pub const CALL_RELEASE_MS: i64 = 3_000;

/// The visible countdown before a recording the call started ends on its own.
///
/// Protects: the end of a call that is not really over (a reconnect, a second
/// call right after) — the user sees «termino de grabar en 13 s» and can press
/// «Seguir grabando», and the application coming back cancels it. Why
/// thirteen seconds: long enough to read and press, short
/// enough not to record the silence after the call. It is not a recording
/// length limit.
pub const AUTO_STOP_COUNTDOWN_MS: i64 = 13_000;

/// How soon an event must start to be the event of a call.
///
/// Protects: the link from naming the wrong meeting — a call starts early
/// more often than it starts late, and an event an hour away is another
/// meeting. Why ten minutes: people join a few minutes before the hour; past
/// ten the call is not that event. It is not a limit on reading the agenda.
pub const EVENT_LOOKAHEAD_MS: i64 = 600_000;

/// «Al detectar una llamada»: the user's preference, kept by the daemon. A
/// preference, not a permission: the microphone permission is the decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum CallPreference {
    /// «Avisarme»: the floating panel offers to record.
    #[default]
    Notify,
    /// «Grabar sin preguntar»: recording starts when the call is confirmed.
    Record,
    /// «No hacer nada».
    Nothing,
}

/// One application the helper says holds the microphone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallApp {
    pub application_pid: i32,
    pub bundle_id: String,
    pub name: String,
}

/// What the daemon tells the detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallInput {
    /// The helper's whole list of applications holding the microphone.
    Microphone { applications: Vec<CallApp> },
    /// The helper is gone: from now on nothing is known.
    SignalLost,
    /// Time passed.
    Tick,
    /// «Grabar» on the offer named `call_id`.
    Record { call_id: String },
    /// «Ahora no» on the offer named `call_id`.
    Dismiss { call_id: String },
    /// «Seguir grabando»: this recording no longer ends with the call.
    Keep { recording_id: String },
    /// The daemon started the recording a [`CallEffect::Start`] asked for.
    Started {
        call_id: String,
        recording_id: String,
    },
    /// The daemon could not start it.
    StartFailed { call_id: String },
    /// The recording ended, whoever ended it.
    RecordingEnded { recording_id: String },
    /// The helper cut this recording's capture because the call
    /// application's audio stream stopped (`system_audio_stream_stopped`).
    CaptureCut { recording_id: String },
}

/// The facts an input is decided with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallFacts {
    /// Unix milliseconds, from the daemon's clock.
    pub now_ms: i64,
    pub preference: CallPreference,
    pub privacy_stopped: bool,
    /// Any recording is live (started by hand or not): an offer would be
    /// pointless and a second recording cannot start.
    pub recording_live: bool,
}

/// What the daemon must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallEffect {
    /// An application that may be a call appeared: read what is on the
    /// agenda now, so the offer and the recording can name the event.
    ReadCalendar,
    /// Record this call, capturing `app`'s audio.
    Start {
        call_id: String,
        app: CallApp,
        /// Started without asking (`CallPreference::Record`).
        automatic: bool,
    },
    /// End this recording as «Terminar» would, with what was recorded.
    Finish {
        recording_id: String,
        app: CallApp,
        cause: FinishCause,
    },
}

/// Why a recording the call started ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishCause {
    /// The application left the microphone and the countdown ran out.
    CallEnded,
    /// The application's audio stopped (it quit) after it left the call.
    CallAppClosed,
}

/// Why an answer was refused, with what it named.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallRefusal {
    #[error("Privacidad detenida: Orbit no graba llamadas hasta que la rearmes")]
    PrivacyStopped,
    #[error(
        "El aviso de llamada {call_id} ya no está: la llamada terminó o ya se respondió. No se empezó a grabar."
    )]
    OfferGone { call_id: String },
    #[error(
        "La grabación {recording_id} no la empezó una llamada, o ya terminó: no hay cuenta atrás que cancelar."
    )]
    NotOwned { recording_id: String },
}

/// The effects of one input, and its refusal if it was refused.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CallUpdate {
    pub effects: Vec<CallEffect>,
    pub refusal: Option<CallRefusal>,
}

/// What the floating panel has to say, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallPanel {
    /// «Llamada en {application}»: Grabar / Ahora no.
    Offer { call_id: String, app: CallApp },
    /// A recording that started on its own: Parar.
    Recording { recording_id: String, app: CallApp },
    /// «La llamada terminó · termino de grabar en N s»: Seguir grabando.
    Ending {
        recording_id: String,
        app: CallApp,
        ends_at_ms: i64,
    },
}

#[derive(Debug, Clone)]
struct Holder {
    app: CallApp,
    since_ms: i64,
    left_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
struct Offer {
    call_id: String,
    app: CallApp,
    automatic: bool,
}

#[derive(Debug, Clone)]
struct Owned {
    recording_id: String,
    app: CallApp,
    automatic: bool,
    released_at_ms: Option<i64>,
}

/// The detection's state. RAM only: a restart starts from «unknown».
#[derive(Debug, Default)]
pub struct CallDetector {
    /// `None` until the helper's first list, and again when it is gone.
    holders: Option<BTreeMap<String, Holder>>,
    /// Applications whose current hold on the microphone was already answered
    /// (offered, dismissed, recorded, or met while recording): nothing more
    /// is offered for them until they release the microphone.
    handled: BTreeSet<String>,
    offer: Option<Offer>,
    starting: Option<Offer>,
    owned: Option<Owned>,
    /// After the user's privacy stop: what holds the microphone when they rearm
    /// was there before, and nothing resumes on its own.
    quiet: bool,
    stopped: bool,
    next_call: u64,
    last_now_ms: Option<i64>,
}

impl CallDetector {
    /// Decides one input at `facts.now_ms`, then lets time do what it does
    /// (a call confirmed, a release confirmed, a countdown run out).
    pub fn apply(&mut self, input: CallInput, facts: &CallFacts) -> CallUpdate {
        let mut update = CallUpdate::default();
        // A wall clock set back never runs a countdown backwards.
        let now = self
            .last_now_ms
            .map_or(facts.now_ms, |last| facts.now_ms.max(last));
        self.last_now_ms = Some(now);
        if facts.privacy_stopped {
            self.stop();
            if matches!(input, CallInput::Record { .. } | CallInput::Keep { .. }) {
                update.refusal = Some(CallRefusal::PrivacyStopped);
            }
            return update;
        }
        if self.stopped {
            // Rearmed: whatever holds the microphone now was there before.
            self.stopped = false;
            self.quiet = true;
        }
        if let Err(refusal) = self.input(input, now, &mut update.effects) {
            update.refusal = Some(refusal);
        }
        self.advance(now, facts, &mut update.effects);
        update
    }

    /// What the panel says now.
    pub fn panel(&self, now_ms: i64) -> Option<CallPanel> {
        if let Some(owned) = &self.owned {
            if let Some(released) = owned.released_at_ms {
                let counting_from = released.saturating_add(CALL_RELEASE_MS);
                if now_ms >= counting_from {
                    return Some(CallPanel::Ending {
                        recording_id: owned.recording_id.clone(),
                        app: owned.app.clone(),
                        ends_at_ms: counting_from.saturating_add(AUTO_STOP_COUNTDOWN_MS),
                    });
                }
            }
            return owned.automatic.then(|| CallPanel::Recording {
                recording_id: owned.recording_id.clone(),
                app: owned.app.clone(),
            });
        }
        self.offer.as_ref().map(|offer| CallPanel::Offer {
            call_id: offer.call_id.clone(),
            app: offer.app.clone(),
        })
    }

    /// The call application holding the microphone now, best first (a
    /// call application before a browser), if the helper said so.
    pub fn holding_call(&self) -> Option<&CallApp> {
        self.holders
            .as_ref()?
            .values()
            .filter(|holder| holder.left_at_ms.is_none())
            .min_by(|first, second| {
                kind_rank(&first.app)
                    .cmp(&kind_rank(&second.app))
                    .then(first.since_ms.cmp(&second.since_ms))
                    .then(first.app.bundle_id.cmp(&second.app.bundle_id))
            })
            .map(|holder| &holder.app)
    }

    /// The recording this detection started and still owns.
    pub fn owned_recording(&self) -> Option<&str> {
        self.owned.as_ref().map(|owned| owned.recording_id.as_str())
    }

    /// Retired from outside an input (the user's stop, a revocation, Orbit
    /// closing): everything is forgotten and nothing resumes on its own.
    pub fn retire(&mut self) {
        self.stop();
    }

    /// The user's privacy stop: everything is forgotten, including a recording
    /// the call started (their stop cut it), and nothing resumes on its own.
    fn stop(&mut self) {
        self.holders = None;
        self.handled.clear();
        self.offer = None;
        self.starting = None;
        self.owned = None;
        self.stopped = true;
    }

    fn input(
        &mut self,
        input: CallInput,
        now: i64,
        effects: &mut Vec<CallEffect>,
    ) -> Result<(), CallRefusal> {
        match input {
            CallInput::Tick => {}
            CallInput::Microphone { applications } => self.observe(applications, now, effects),
            CallInput::SignalLost => {
                self.holders = None;
                // No offer stands without a helper to record it, and no
                // countdown runs on the absence of a signal.
                self.offer = None;
                if let Some(owned) = self.owned.as_mut() {
                    owned.released_at_ms = None;
                }
            }
            CallInput::Record { call_id } => {
                let offer = self
                    .offer
                    .take_if(|offer| offer.call_id == call_id)
                    .ok_or(CallRefusal::OfferGone { call_id })?;
                effects.push(CallEffect::Start {
                    call_id: offer.call_id.clone(),
                    app: offer.app.clone(),
                    automatic: false,
                });
                self.starting = Some(offer);
            }
            CallInput::Dismiss { call_id } => {
                // An offer already gone is what the user asked for.
                self.offer.take_if(|offer| offer.call_id == call_id);
            }
            CallInput::Keep { recording_id } => {
                self.owned
                    .take_if(|owned| owned.recording_id == recording_id)
                    .ok_or(CallRefusal::NotOwned { recording_id })?;
            }
            CallInput::Started {
                call_id,
                recording_id,
            } => {
                if let Some(started) = self
                    .starting
                    .take_if(|starting| starting.call_id == call_id)
                {
                    let released_at_ms = self.released_at(&started.app.bundle_id, now);
                    self.owned = Some(Owned {
                        recording_id,
                        app: started.app,
                        automatic: started.automatic,
                        released_at_ms,
                    });
                }
            }
            CallInput::StartFailed { call_id } => {
                self.starting
                    .take_if(|starting| starting.call_id == call_id);
            }
            CallInput::RecordingEnded { recording_id } => {
                self.owned
                    .take_if(|owned| owned.recording_id == recording_id);
            }
            CallInput::CaptureCut { recording_id } => {
                // Only an application already off the microphone closed with
                // its call; a cut while it still holds it is a capture failure,
                // said as such by the daemon, and the recording stays.
                if let Some(owned) = self.owned.take_if(|owned| {
                    owned.recording_id == recording_id && owned.released_at_ms.is_some()
                }) {
                    effects.push(CallEffect::Finish {
                        recording_id: owned.recording_id,
                        app: owned.app,
                        cause: FinishCause::CallAppClosed,
                    });
                }
            }
        }
        Ok(())
    }

    /// When `bundle_id` left the microphone, if it is not holding it now.
    fn released_at(&self, bundle_id: &str, now: i64) -> Option<i64> {
        let holders = self.holders.as_ref()?;
        match holders.get(bundle_id) {
            Some(holder) => holder.left_at_ms,
            None => Some(now),
        }
    }

    /// The helper's whole list.
    fn observe(&mut self, applications: Vec<CallApp>, now: i64, effects: &mut Vec<CallEffect>) {
        let mut present: BTreeMap<String, CallApp> = BTreeMap::new();
        for app in applications {
            if call_kind(&app.bundle_id).is_some() {
                present.entry(app.bundle_id.clone()).or_insert(app);
            }
        }
        let first_list = self.holders.is_none();
        let quiet = std::mem::take(&mut self.quiet);
        let holders = self.holders.get_or_insert_with(BTreeMap::new);
        if first_list {
            // What was answered before the signal was lost and is gone now
            // released at some unknown moment: it is free again.
            self.handled
                .retain(|bundle_id| present.contains_key(bundle_id));
        }
        let mut new_call = false;
        for (bundle_id, app) in &present {
            match holders.get_mut(bundle_id) {
                Some(holder) => {
                    holder.left_at_ms = None;
                    holder.app = app.clone();
                }
                None => {
                    holders.insert(
                        bundle_id.clone(),
                        Holder {
                            app: app.clone(),
                            since_ms: now,
                            left_at_ms: None,
                        },
                    );
                    if quiet {
                        self.handled.insert(bundle_id.clone());
                    } else if !self.handled.contains(bundle_id) {
                        new_call = true;
                    }
                }
            }
        }
        for (bundle_id, holder) in holders.iter_mut() {
            if !present.contains_key(bundle_id) && holder.left_at_ms.is_none() {
                holder.left_at_ms = Some(now);
            }
        }
        if let Some(owned) = self.owned.as_mut() {
            owned.released_at_ms = if present.contains_key(&owned.app.bundle_id) {
                None
            } else {
                owned.released_at_ms.or(Some(now))
            };
        }
        if new_call {
            effects.push(CallEffect::ReadCalendar);
        }
    }

    /// What time alone decides.
    fn advance(&mut self, now: i64, facts: &CallFacts, effects: &mut Vec<CallEffect>) {
        let Some(holders) = self.holders.as_mut() else {
            return;
        };
        // Releases confirmed: the call of that application is over.
        let released: Vec<String> = holders
            .iter()
            .filter(|(_, holder)| {
                holder
                    .left_at_ms
                    .is_some_and(|left| now.saturating_sub(left) >= CALL_RELEASE_MS)
            })
            .map(|(bundle_id, _)| bundle_id.clone())
            .collect();
        for bundle_id in &released {
            holders.remove(bundle_id);
            self.handled.remove(bundle_id);
            self.offer
                .take_if(|offer| &offer.app.bundle_id == bundle_id);
        }
        // A countdown that ran out ends its recording.
        if let Some(owned) = self.owned.take_if(|owned| {
            owned.released_at_ms.is_some_and(|released| {
                now >= released
                    .saturating_add(CALL_RELEASE_MS)
                    .saturating_add(AUTO_STOP_COUNTDOWN_MS)
            })
        }) {
            effects.push(CallEffect::Finish {
                recording_id: owned.recording_id,
                app: owned.app,
                cause: FinishCause::CallEnded,
            });
        }
        // A recording started meanwhile (by hand, from the calendar) answers
        // the offer: there is nothing left to offer.
        if facts.recording_live && self.owned.is_none() {
            self.offer = None;
        }
        // Calls confirmed: held for long enough and not answered yet.
        let Some(holders) = self.holders.as_ref() else {
            return;
        };
        let mut confirmed: Vec<&Holder> = holders
            .iter()
            .filter(|(bundle_id, holder)| {
                holder.left_at_ms.is_none()
                    && !self.handled.contains(*bundle_id)
                    && now.saturating_sub(holder.since_ms) >= CALL_CONFIRM_MS
            })
            .map(|(_, holder)| holder)
            .collect();
        if confirmed.is_empty() {
            return;
        }
        if facts.recording_live || self.owned.is_some() {
            // Already recording: there is nothing to offer for these calls.
            let bundles: Vec<String> = confirmed
                .iter()
                .map(|holder| holder.app.bundle_id.clone())
                .collect();
            self.handled.extend(bundles);
            return;
        }
        if self.offer.is_some() || self.starting.is_some() {
            // One call at a time; the others wait for this one's answer.
            return;
        }
        confirmed.sort_by(|first, second| {
            kind_rank(&first.app)
                .cmp(&kind_rank(&second.app))
                .then(first.since_ms.cmp(&second.since_ms))
                .then(first.app.bundle_id.cmp(&second.app.bundle_id))
        });
        let Some(app) = confirmed.first().map(|holder| holder.app.clone()) else {
            return;
        };
        self.handled.insert(app.bundle_id.clone());
        match facts.preference {
            CallPreference::Nothing => {}
            CallPreference::Notify => {
                self.next_call = self.next_call.wrapping_add(1);
                self.offer = Some(Offer {
                    call_id: format!("call-{}-{}", now, self.next_call),
                    app,
                    automatic: false,
                });
            }
            CallPreference::Record => {
                self.next_call = self.next_call.wrapping_add(1);
                let offer = Offer {
                    call_id: format!("call-{}-{}", now, self.next_call),
                    app,
                    automatic: true,
                };
                effects.push(CallEffect::Start {
                    call_id: offer.call_id.clone(),
                    app: offer.app.clone(),
                    automatic: true,
                });
                self.starting = Some(offer);
            }
        }
    }
}

fn kind_rank(app: &CallApp) -> u8 {
    match call_kind(&app.bundle_id) {
        Some(CallKind::Dedicated) => 0,
        Some(CallKind::Browser) | None => 1,
    }
}
