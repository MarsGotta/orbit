/// Numeric bounds mirrored from contracts/constants.json. This file has only
/// constants; the protocol and transport apply them at their own boundaries.
public enum NativeLimits {
    // A 4 MiB clip occupies 5,592,408 base64 bytes; the measured command with
    // a UUID id and 128-byte playback/provider fields is 5,592,815 bytes.
    // This protects stdin, not events.
    public static let maximumCommandLineBytes = 6 * 1024 * 1024
    // The daemon already rejects stdout lines above 1 MiB. This is not the
    // playback command allowance.
    public static let maximumEventLineBytes = 1024 * 1024
    // One decoded playback clip, not its JSON/base64 frame.
    public static let maximumPlaybackClipBytes = 4 * 1024 * 1024
    // One window screenshot's PNG before base64. Its 933,336 base64 bytes
    // and their envelope fit the 1 MiB event line above; that line is what
    // this protects. It is not a screen inventory or a count of windows.
    public static let maximumScreenshotPNGBytes = 700_000
    // The window inventory's 160 KiB includes its 2 KiB reserved envelope;
    // it is not a bound on a screenshot's bytes.
    public static let maximumWindowInventoryBytes = 160 * 1024
    // P9-12: a spoken question («Oye Deimos, …») ends after this much
    // silence, the pause between spoken phrases (orbit-core
    // `QUESTION_PAUSE_MS`). It is not a recognition timeout.
    public static let assistantQuestionPauseMilliseconds = 1_500
    // The longest spoken question from the phrase to its end (orbit-core
    // `MAX_SPOKEN_QUESTION_MS`): a call heard through the speakers keeps the
    // microphone busy and the question would never pause. Not a limit on
    // the recording.
    public static let maximumAssistantQuestionMilliseconds = 30_000
}
