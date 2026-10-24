import AppKit
import Darwin
import AVFoundation
import ApplicationServices
import ScreenCaptureKit
import OrbitNativeProtocol
import OrbitNativeTransport
import OrbitNativeState
import OrbitNativeCalendar
import OrbitNativeHost

// One capture track failed: it ends alone, or the system's audio starts again once.
extension OrbitApplication {
    func trackFailed(_ track: CaptureTrack, reason: String) {
        let current = track == .microphone ? microphone.streamID : systemAudio.streamID
        guard let streamID = current else { stop(reason: reason); return }
        trackFailed(track, streamID: streamID, reason: reason)
    }

    /// One track failed. It used to stop both (`stop(reason:)`); now only
    /// that track ends, or the system's audio is started again once, while
    /// the other keeps recording. With nothing else live it stops as before.
    func trackFailed(_ track: CaptureTrack, streamID: String, reason: String) {
        let other = track == .microphone ? systemAudio.slot : microphone.slot
        let action = TrackFailurePolicy.decide(track: track, otherTrack: other,
            closing: latch.stopped || quitRequested,
            restartAttempted: systemAudioRestartedStreamID == streamID)
        let name: String
        switch action {
        case .stopCapture: name = "stop_capture"
        case .restartSystemAudio: name = "restart_system_audio"
        case .endTrack: name = "end_track"
        }
        diagnostics.recordOffMain("capture_track_failed", [
            "track": track.rawValue, "reason": reason, "action": name
        ])
        switch action {
        case .stopCapture:
            stop(reason: reason)
        case .endTrack:
            endTrack(track, streamID: streamID, cause: reason)
        case .restartSystemAudio:
            restartSystemAudio(streamID: streamID, cause: reason)
        }
    }

    /// Ends one track: whatever it captured is drained, then an error names
    /// the cause and a `gap` says the track stopped (`track_stopped`). The daemon
    /// keeps the recording with the other track and says which one stopped.
    private func endTrack(_ track: CaptureTrack, streamID: String, cause: String) {
        switch track {
        case .microphone:
            endedMicrophoneStreamID = streamID
            audioRouteEpoch &+= 1
            microphonePurpose = nil
            microphoneMeeting = false
            assistantTrigger.stop()
            microphone.disableImmediately()
            audioSession.stopIfIdle()
        case .systemAudio:
            endedSystemAudioStreamID = streamID
            systemAudioScope = nil
            systemAudio.disableImmediately()
        }
        Task { @MainActor in
            switch track {
            case .microphone: _ = await self.microphone.stop()
            case .systemAudio: _ = await self.systemAudio.stop()
            }
            // The cause first: the daemon's words about the stopped track,
            // said on the gap, are then what the person reads last.
            self.channel.send(.makeError(code: cause))
            self.channel.send(.makeGap(source: track.rawValue, streamID: streamID, reason: "track_stopped",
                                       frames: 0, durationUnknown: true))
            self.publishState()
        }
    }

    /// The system's audio stream stopped (the app that holds it, a display
    /// change): it is started again, once, under the same stream id, within
    /// `TrackFailurePolicy.systemAudioRestartBound`. The daemon hears the
    /// restart before the new stream's first frame, whose sequence starts
    /// over. If it does not come back, the track ends with its cause.
    private func restartSystemAudio(streamID: String, cause: String) {
        guard let scope = systemAudioScope else {
            endTrack(.systemAudio, streamID: streamID, cause: cause)
            return
        }
        let attempt = UUID()
        let epoch = privacyEpoch
        systemAudioRestartedStreamID = streamID
        systemAudioRestart = (attempt: attempt, streamID: streamID)
        systemAudio.disableImmediately()
        Task { @MainActor in
            _ = await self.systemAudio.stop()
            guard self.systemAudioRestart?.attempt == attempt else { return }
            self.channel.send(.makeGap(source: CaptureTrack.systemAudio.rawValue, streamID: streamID,
                                       reason: "system_audio_restarted", frames: 0, durationUnknown: true))
            let bound = UInt64(TrackFailurePolicy.systemAudioRestartBound * 1_000_000_000)
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: bound)
                guard self.systemAudioRestart?.attempt == attempt else { return }
                self.systemAudioRestart = nil
                self.endTrack(.systemAudio, streamID: streamID, cause: "system_audio_restart_timed_out")
            }
            do {
                _ = try await self.systemAudio.start(streamID: streamID, captureScope: scope.captureScope,
                    applicationPID: scope.applicationPID,
                    allowStart: { [weak self] in
                        self?.latch.stopped == false && self?.privacyEpoch == epoch
                            && self?.systemAudioRestart?.attempt == attempt
                    })
                guard self.systemAudioRestart?.attempt == attempt else { return }
                self.systemAudioRestart = nil
                self.systemAudioScope = scope
                self.diagnostics.recordOffMain("system_audio_restarted", ["cause": cause])
            } catch {
                guard self.systemAudioRestart?.attempt == attempt else { return }
                self.systemAudioRestart = nil
                self.diagnostics.recordOffMain("system_audio_restart_failed", [
                    "cause": cause, "code": (error as? NativeFailure)?.code ?? "system_audio_restart_failed"
                ])
                self.endTrack(.systemAudio, streamID: streamID, cause: cause)
            }
            self.publishState()
        }
    }
}
