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

// The audio devices changed: nothing happens, playback ends, capture stops,
// or a meeting's microphone follows the new device.
extension OrbitApplication {
    func audioRouteChanged(previous: AudioRouteDevices?, current: AudioRouteDevices) {
        // A silent daemon is not a closing: a meeting kept through its silence
        // follows the new device too. A pipe that is gone has already stopped
        // the capture, and that stop's cleanup or epoch is what closes here.
        let decision = AudioRouteChangePolicy.decide(stopped: latch.stopped,
            closing: quitRequested || privacyCleanupInFlight > 0,
            microphone: microphone.slot, processing: audioSession.mode, playback: playback.playing,
            routeChanged: current.changed(from: previous), purpose: microphonePurpose,
            meetingUnderneath: microphoneMeeting || systemAudio.slot.isActive)
        recordRouteChange(decision, previous: previous, current: current)
        switch decision {
        case .none:
            return
        case .stop(let reason):
            stop(reason: reason)
        case .cancelPlayback:
            channel.send(.makeError(code: "playback_cancelled_audio_device_changed"))
            cancelPlayback()
            audioSession.stopIfIdle()
            publishState()
        case .restartMicrophone, .restartMicrophoneCancellingVoiceNote:
            restartMicrophone(after: decision)
        }
    }

    /// The change and what was decided, by device numbers and causes only.
    private func recordRouteChange(_ decision: AudioRouteChangeAction, previous: AudioRouteDevices?,
                                   current: AudioRouteDevices) {
        let action: String
        let reason: String
        switch decision {
        case .none(let cause): action = "none"; reason = cause
        case .stop(let cause): action = "stop"; reason = cause
        case .cancelPlayback: action = "cancel_playback"; reason = "audio_device_changed"
        case .restartMicrophone: action = "restart_microphone"; reason = "audio_device_changed_meeting"
        case .restartMicrophoneCancellingVoiceNote:
            action = "restart_microphone_cancel_voice_note"; reason = "audio_device_changed_voice_note_over_meeting"
        }
        diagnostics.recordOffMain("audio_configuration_changed", [
            "previous_input": previous.flatMap { $0.input }.map { String($0) } ?? "unknown",
            "previous_output": previous.flatMap { $0.output }.map { String($0) } ?? "unknown",
            "current_input": current.input.map { String($0) } ?? "unknown",
            "current_output": current.output.map { String($0) } ?? "unknown",
            "action": action, "reason": reason,
            "purpose": microphonePurpose?.rawValue ?? "unknown"
        ])
    }

    /// Drains the meeting's microphone on the old device, then starts the
    /// same stream again on the new one (`resumeMicrophoneAfterRoute`).
    private func restartMicrophone(after decision: AudioRouteChangeAction) {
        // The voice note ends with the route gap the new stream opens
        // with (the daemon cancels it and says so); the meeting goes on.
        if decision == .restartMicrophoneCancellingVoiceNote { microphonePurpose = .meeting }
        guard let streamID = microphone.streamID, let sampleRate = microphone.sampleRate else {
            stop(reason: "audio_route_stream_identity_missing")
            return
        }
        let epoch = privacyEpoch
        let routeEpoch = audioRouteEpoch
        let wasListening = assistantTrigger.isListening
        // Reuse the existing cleanup admission: a privacy stop during the
        // drain still runs in full and invalidates this epoch before restart.
        audioRouteRecoveringStreamID = streamID
        privacyCleanupInFlight += 1
        assistantTrigger.stop()
        microphone.disableImmediately()
        if playback.playing { channel.send(.makeError(code: "playback_cancelled_audio_device_changed")) }
        cancelPlayback()
        audioSession.stop()
        Task { @MainActor in
            await self.resumeMicrophoneAfterRoute(streamID: streamID, sampleRate: sampleRate,
                                                  epoch: epoch, routeEpoch: routeEpoch,
                                                  wasListening: wasListening)
        }
    }

    @MainActor private func resumeMicrophoneAfterRoute(streamID: String, sampleRate: UInt32, epoch: UInt64,
                                                       routeEpoch: UInt64, wasListening: Bool) async {
        defer {
            self.audioRouteRecoveringStreamID = nil
            self.privacyCleanupInFlight -= 1
            self.publishState()
        }
        let stopped = await self.microphone.stopForRouteChange()
        let action = AudioRouteChangePolicy.decide(stopped: self.latch.stopped,
            closing: self.quitRequested || self.privacyEpoch != epoch
                || self.audioRouteEpoch != routeEpoch,
            microphone: self.microphone.slot, processing: .unprocessed, playback: false,
            failure: stopped.drained ? nil : "audio_route_drain_incomplete", recovering: true,
            routeChanged: true, purpose: self.microphonePurpose,
            meetingUnderneath: self.microphoneMeeting || self.systemAudio.slot.isActive)
        switch action {
        case .none:
            return
        case .stop(let reason):
            self.stop(reason: reason)
        case .cancelPlayback:
            self.cancelPlayback()
        case .restartMicrophone, .restartMicrophoneCancellingVoiceNote:
            do {
                _ = try self.microphone.start(streamID: streamID, processing: .unprocessed,
                    outputRate: sampleRate, routeResume: stopped.resume)
                if wasListening {
                    let sink = self.assistantTrigger.start(streamID: streamID, allow: { [weak self] stream in
                        guard let self else { return false }
                        return !self.latch.stopped && self.microphone.activeStreamID == stream
                    })
                    try self.microphone.setFrameSink(sink, streamID: streamID)
                }
            } catch {
                // The microphone did not come back: with the system's
                // audio still recording, only the microphone ends.
                self.trackFailed(.microphone, streamID: streamID,
                                 reason: reportAudioRouteFailure(error, channel: self.channel))
            }
        }
    }
}
