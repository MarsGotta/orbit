//! Notes and meetings, on the hub: transcripts, changes to a note, the template library,
//! and the reads of them the interface reports when they fail.

macro_rules! notes_notices {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            TranscriptRetryRequested = 131 => Done,
            /// The manual final pass was refused, with the hub code and HTTP status.
            TranscriptRetranscribeRefused = 132 => Fixable,
            /// The hub did not confirm the manual final pass; it may be in progress.
            TranscriptRetranscribeUnconfirmed = 133 => Fixable,
            /// A change of a note (an edit, a rewrite, a speaker's name, a retry)
            /// did not reach the hub: nothing changed; the user can do it again when the
            /// hub is back.
            NoteChangeUnsent = 134 => Fixable,
            /// The hub refused a change of a note (`step` the order, `status` and
            /// `code` the hub's).
            NoteChangeRefused = 135 => Failed,
            /// The hub cannot delete recordings (0.11 and before): this Mac deleted
            /// nothing of the meeting it was asked to delete, or keeps the order of
            /// one it already threw away until the hub can take it.
            MeetingDeleteNeedsHubUpdate = 136 => Fixable,
            /// The hub replaced the transcript after the selected turn was read.
            TurnNoLongerExists = 137 => Fixable,
            /// The meeting was not transcribed again in another language
            /// because the hub cannot read the audio of `count` of its `total`
            /// groups; nothing changed.
            MeetingAudioUnreadable = 138 => Fixable,
            /// An open note could not be read (`stage` `failed`) or came with parts
            /// missing (`stage` `incomplete`).
            NoteReadProblem = 140 => Silent,
            /// The transcript of an open note could not be read.
            TranscriptReadFailed = 141 => Silent,
            /// The notes index could not be read.
            NotesIndexFailed = 142 => Silent,
            /// The template library could not be read.
            LibraryReadFailed = 143 => Silent,
            /// The hub refused a template change (`code`, the hub's own).
            LibraryRefused = 144 => Silent,
        }
    };
}
pub(super) use notes_notices;
