//! Voice and calls: voice notes, the continuous call with the assistant, its spoken answers
//! and the «Oye Deimos» detector the helper reports through the daemon, so a lost
//! question can be followed step by step in the log.

macro_rules! voice_notices {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            /// A device change ended the voice note spoken over a meeting; the
            /// meeting goes on with the new device and the note is kept, unsent.
            VoiceNoteCancelledByAudioRoute = 14 => Fixable,
            VoiceNoteRefused = 41 => Failed,
            VoiceNoteEmpty = 42 => Fixable,
            /// A voice note shorter than the speech minimum was not sent
            /// (`duration_ms`, `minimum_ms`). The composer says it, from the
            /// assistant topic, where the note was recorded.
            VoiceNoteTooShort = 43 => Silent,
            VoiceNoteSaved = 44 => Silent,
            VoiceTranscribing = 45 => Working,
            VoiceTurnCancelled = 46 => Silent,
            /// Nothing was heard in a call's turn (the hub found no words): it was
            /// dropped without a word to the user, or waits for what they say next.
            VoiceNotHeard = 47 => Silent,
            CallStarting = 69 => Silent,
            CallActive = 70 => Silent,
            CallStopped = 71 => Silent,
            CallStoppedByConnection = 72 => Failed,
            CallExpired = 73 => Fixable,
            CallClosed = 74 => Silent,
            CallDrainUnconfirmed = 75 => Failed,
            VoiceNotRecognized = 116 => Fixable,
            /// A voice note was not transcribed. It loses nothing: the note stays in
            /// the conversation with its audio, its cause and what to do, so the
            /// notice is a small line, not a failure with a code to share.
            VoiceTranscriptionFailed = 117 => Fixable,
            /// Deimos's spoken answer could not be played.
            SpeechFailed = 118 => Failed,
            /// The detector listens on the meeting microphone.
            WakePhraseListening = 150 => Silent,
            /// The detector stopped listening.
            WakePhraseOff = 151 => Silent,
            /// The detector is downloading the Spanish model.
            WakePhraseDownloading = 152 => Silent,
            /// The detector cannot run on this Mac (`cause`: `macos_version`,
            /// `locale_unsupported`).
            WakePhraseUnavailable = 153 => Silent,
            /// The detector failed (`cause`); Apple's text stays on the Mac.
            WakePhraseFailed = 154 => Silent,
            /// The helper heard the phrase and waits for the question.
            WakePhraseHeard = 155 => Silent,
            /// The phrase was heard and no question followed it.
            WakePhraseAbandoned = 156 => Silent,
            /// The helper heard a question and did not send it: the user's stop, or the
            /// microphone stream changed.
            WakePhraseQuestionDropped = 157 => Silent,
            /// A spoken question reached the daemon and is asked.
            WakePhraseQuestionReceived = 158 => Silent,
            /// A spoken question reached the daemon and was not asked (`cause`).
            WakePhraseQuestionIgnored = 159 => Silent,
        }
    };
}
pub(super) use voice_notices;
