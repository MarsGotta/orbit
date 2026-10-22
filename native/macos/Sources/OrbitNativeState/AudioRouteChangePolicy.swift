import OrbitNativeProtocol

public enum AudioRouteChangeAction: Equatable, Sendable {
    case none(reason: String)
    case restartMicrophone
    /// A voice note was dictated over a live meeting: the meeting's microphone
    /// restarts on the new route and only the voice note ends.
    case restartMicrophoneCancellingVoiceNote
    case cancelPlayback
    case stop(reason: String)
}

/// Facts from the host, without opening a device. A drain or the user's brake is never
/// authority to restart; continuous voice retains its processing safety cut.
public enum AudioRouteChangePolicy {
    public static func decide(stopped: Bool, closing: Bool, microphone: CaptureSlot,
                              processing: AudioProcessingMode, playback: Bool,
                              failure: String? = nil, recovering: Bool = false,
                              routeChanged: Bool?, purpose: NativeMicrophonePurpose?,
                              meetingUnderneath: Bool = false) -> AudioRouteChangeAction {
        guard !stopped else { return .none(reason: "privacy_stopped") }
        guard !closing else { return .none(reason: "closing_or_draining") }
        if let failure { return .stop(reason: failure) }
        if routeChanged == false { return .none(reason: "route_unchanged") }
        if microphone == .active || recovering {
            guard routeChanged != nil else { return .stop(reason: "audio_route_unobservable") }
            if processing == .voiceProcessing {
                return .stop(reason: "audio_device_changed_voice_processing")
            }
            switch purpose {
            case .meeting: return .restartMicrophone
            // A voice note alone stops once. Over a meeting, stopping it took
            // the whole meeting with it (both tracks): the meeting goes on.
            case .voiceNote:
                return meetingUnderneath
                    ? .restartMicrophoneCancellingVoiceNote : .stop(reason: "audio_device_changed_voice_note")
            case nil: return .stop(reason: "audio_device_changed_purpose_unknown")
            }
        }
        guard routeChanged != nil else { return .none(reason: "audio_route_unobservable") }
        return playback ? .cancelPlayback : .none(reason: "microphone_inactive")
    }
}

/// A restart exists even when no prior frame arrived. Unknown timing stays
/// unknown; neither notification arrival nor restart completion is captured PCM.
public struct AudioRouteResume: Equatable, Sendable {
    public let previousEndNS: UInt64?
    public init(previousEndNS: UInt64?) { self.previousEndNS = previousEndNS }

    public func durationNS(until firstFrameNS: UInt64) -> UInt64? {
        guard let previousEndNS, firstFrameNS >= previousEndNS else { return nil }
        return firstFrameNS - previousEndNS
    }
}
