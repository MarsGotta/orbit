//! Privacy and authority: the stop the user can latch and come back to, and
//! the generation that retires every older authority when it changes.
//!
//! Reads of other domains: the hub link (a revoked device cannot rearm).
use super::Step;
use crate::authority::AuthorityChange;
use crate::notice::{Notice, NoticeCode, NoticeKey};
use crate::privacy_stop::StopRequester;
use crate::refusal::Refusal;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivacyState {
    pub(crate) generation: u64,
    pub(crate) stopped: bool,
    /// Who asked for the stop in force: only the user's is carried across
    /// a restart as theirs. `None` with a stop in force is a stop nobody
    /// can attribute, and is kept.
    pub(crate) stop_by: Option<StopRequester>,
}

impl PrivacyState {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn stopped(&self) -> bool {
        self.stopped
    }
    pub fn stop_by(&self) -> Option<StopRequester> {
        self.stop_by
    }

    /// Refuses whatever needs Orbit working while the stop is in force.
    pub(super) fn ensure_active(&self) -> Result<(), Refusal> {
        if self.stopped {
            Err(Refusal::PrivacyStopped { since_ms: None })
        } else {
            Ok(())
        }
    }

    /// Every older authority ends here.
    pub(super) fn advance(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }

    /// The only latch the user can come back to. `requester` is `None`
    /// only for a latch the helper already held: a stop the user pressed
    /// again stays theirs, and nothing downgrades one of theirs.
    pub(super) fn latch(&mut self, requester: Option<StopRequester>) {
        self.advance();
        if requester.is_some() || !self.stopped {
            self.stop_by = requester;
        }
        self.stopped = true;
    }

    /// The hub revoked this device. Remote traffic may stop Orbit, never
    /// rearm it. A stop the user already pressed stays theirs; otherwise
    /// this one is the hub's to repeat.
    pub(super) fn revoke(&mut self) {
        self.advance();
        if !self.stopped {
            self.stop_by = Some(StopRequester::Revocation);
        }
        self.stopped = true;
    }

    /// The user pressed their stop while one was already in force: it is
    /// theirs now.
    pub(super) fn pressed_again(&mut self) {
        self.stop_by = Some(StopRequester::User);
    }

    /// The helper reported its latch while a stop was in force: a stop the
    /// hub's revocation set is now one nobody can attribute, and is kept.
    pub(super) fn helper_latch_kept(&mut self) {
        if self.stop_by == Some(StopRequester::Revocation) {
            self.stop_by = None;
        }
    }

    /// A restart keeps only the user's stop, or one nobody can attribute;
    /// what it says about it, if anything.
    pub(crate) fn recovered(&mut self) -> Option<Notice> {
        let said = match (self.stopped, self.stop_by) {
            // The hub decides a revocation again at the next connection; a
            // restart does not carry it as if the user had asked for it.
            (true, Some(StopRequester::Revocation)) | (false, _) => {
                self.stopped = false;
                self.stop_by = None;
                None
            }
            (true, Some(StopRequester::User)) => {
                Some(Notice::new(NoticeCode::PrivacyStopKept).with(NoticeKey::Attributed, true))
            }
            // Saved by a version that did not record who asked. Absence is
            // not a "no": the stop is kept, and says why.
            (true, None) => {
                Some(Notice::new(NoticeCode::PrivacyStopKept).with(NoticeKey::Attributed, false))
            }
        };
        self.advance();
        said
    }
}

/// «Reanudar»: the user lifts their stop. A revoked device stays stopped;
/// a rearm that finds nothing stopped changes nothing here.
pub(super) fn rearm(privacy: &mut PrivacyState, cx: &mut Step) -> Result<(), Refusal> {
    if cx.before.link.revoked() {
        return Err(Refusal::PrivacyStopped { since_ms: None });
    }
    // The base also withdrew control and meeting detection when there was
    // no latch to clear. Only that no-op uses the narrow plan; a generation
    // change closes the base's old authority.
    if !privacy.stopped {
        cx.authority(AuthorityChange::RearmWithoutStop);
        return Ok(());
    }
    privacy.advance();
    privacy.stopped = false;
    privacy.stop_by = None;
    cx.say(NoticeCode::PrivacyRearmed);
    cx.authority(AuthorityChange::Rearm);
    Ok(())
}
