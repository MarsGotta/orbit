//! Recording and capture: a meeting's recording, its tracks and audio, the calls the Mac
//! detects in other applications, and the upload of what was recorded.

macro_rules! recording_notices {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            CaptureStarting = 0 => Silent,
            RecordingStarted = 1 => Silent,
            RecordingPaused = 2 => Silent,
            RecordingFinishing = 3 => Silent,
            RecordingFinished = 4 => Done,
            /// «Terminar sin guardar» finished: nothing recorded stays.
            RecordingDiscarded = 5 => Done,
            /// The recording will reach the safety ceiling soon: said once, with
            /// the minutes left, those recorded and the ceiling.
            RecordingCeilingNear = 6 => Fixable,
            /// The recording reached the safety ceiling.
            RecordingCeilingReached = 7 => Fixable,
            /// The Mac did not confirm closing the recording in time.
            RecordingCloseUnconfirmed = 8 => Failed,
            RecordingInterruptedByRestart = 9 => Failed,
            /// A cut the person did not ask for (`cause`) and the same recording
            /// went on by itself; `elapsed_ms` of audio is missing between both.
            RecordingResumedAfterCut = 10 => Done,
            /// A cut recording could not go on (`cause`): it was closed with the
            /// `fragments` it has, and its note is written from them.
            RecordingClosedAfterCut = 11 => Fixable,
            AudioGap = 12 => Silent,
            /// The audio route recovered; the meeting continues after a measured or unknown gap.
            AudioRouteRecovered = 13 => Done,
            /// One track of a recording stopped (`origin`: `mic_local` or
            /// `remote_app`) and could not come back; the other track goes on.
            TrackStopped = 15 => Fixable,
            /// The Mac did not confirm muting or unmuting the microphone
            /// («Silenciar»), with the helper's `code`: the microphone stays as it
            /// was shown.
            MicrophoneMuteRefused = 16 => Failed,
            /// A recording closed with no audio; what interrupted it was said before.
            RecordingEmpty = 17 => Silent,
            /// The Mac refused to start the capture for another cause.
            CaptureRefused = 20 => Failed,
            /// Something cut the live capture; Orbit keeps working.
            CaptureCut = 21 => Failed,
            CaptureCutConfirmed = 22 => Silent,
            /// Orbit could not seal the end of the recording; the audio is kept.
            RecordingSealFailed = 23 => Failed,
            AudioDrainWaiting = 24 => Silent,
            AudioDrainConfirmed = 25 => Silent,
            /// The Mac did not confirm an audio stop; the end may be missing.
            AudioStopUnconfirmed = 26 => Failed,
            FullStopRequested = 27 => Silent,
            StopRetrying = 28 => Silent,
            /// The Mac refused to start the audio (`code`).
            AudioStartRefused = 29 => Failed,
            RetiredStopUnconfirmed = 30 => Silent,
            OtherTrackStopped = 31 => Silent,
            /// Only the microphone is recorded: no call application to add.
            MeetingAudioMicrophoneOnly = 34 => Fixable,
            /// Several call applications are open: choose one to record its audio.
            MeetingAudioAmbiguous = 35 => Fixable,
            MeetingAudioApplication = 36 => Silent,
            /// The preference could not be read: choose it again in Ajustes.
            CallPreferenceUnreadable = 76 => Fixable,
            CallDetectionFailed = 77 => Failed,
            /// Recording a call without asking, as the user chose.
            CallRecordingStartedAutomatically = 78 => Done,
            CallRecordingStarted = 79 => Silent,
            CallRecordingFinished = 80 => Done,
            CallStoppedAutomatically = 81 => Silent,
            CallsRecovered = 82 => Silent,
            /// The Mac stopped its capture with nothing recording: nothing was cut.
            MacStoppedIdleCapture = 97 => Silent,
            /// Audio was lost for lack of memory: keep Orbit open while it saves.
            AudioReserveExhausted = 115 => Fixable,
            UploadRetrying = 119 => Silent,
            /// The hub refuses uploads until something is fixed on this side
            /// (`status` 401: pair this Mac again; 404: the hub's address or a
            /// deploy). `step` is the upload, `code` the hub's; retried meanwhile.
            UploadRefusedRetrying = 120 => Fixable,
            /// The hub refused an upload for good (`status`, `code`): the audio
            /// stays on the Mac and the meeting closes without it.
            UploadRefused = 121 => Failed,
        }
    };
}
pub(super) use recording_notices;
