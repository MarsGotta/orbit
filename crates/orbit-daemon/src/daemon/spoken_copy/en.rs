use super::{PresenceVocabulary, SpokenCopy};

pub(super) const COPY: SpokenCopy = SpokenCopy {
    meeting_title: "Meeting",
    call_question: "Are you still there? If you don't say anything, I'll hang up in 15 seconds",
    answers: PresenceVocabulary {
        keep: [
            &["yes", "yeah"],
            &["still here", "i'm still here", "im still here"],
            &["here", "i'm here", "im here"],
            &["wait"],
            &["keep going", "stay on"],
            &["don't hang up", "dont hang up", "do not hang up"],
        ],
        hang_up: &["hang up"],
    },
};
