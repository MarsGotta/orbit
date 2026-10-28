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

// The daemon's audio commands: start a stream, say who holds the microphone,
// mute it, listen for «Oye Deimos» on it, and stop a stream.
extension OrbitApplication {
    @MainActor func startAudio(_ fields: NativeAudioStartFields) async throws -> [String: Any] {
        let id = fields.stream_id
        guard !id.isEmpty, id.utf8.count <= 128 else {
            throw NativeFailure("invalid_stream_id")
        }
        switch fields.source {
        case .micLocal:
            return try startMicrophone(fields, streamID: id)
        case .remoteApp:
            return try await startSystemAudio(fields, streamID: id)
        }
    }

    @MainActor private func startMicrophone(_ fields: NativeAudioStartFields, streamID id: String) throws -> [String: Any] {
        let requested = fields.processing ?? .unprocessed
        guard AudioConfigurationPolicy.permitsCapture(requested: requested, current: audioSession.mode,
            configured: audioSession.configured, microphoneActive: microphone.slot.isActive,
            remoteActive: systemAudio.slot.isActive || privacyCleanupInFlight > 0,
            playbackActive: playback.playing) else { throw NativeFailure("audio_session_busy") }
        let result = try microphone.start(streamID: id, processing: requested,
                                          muted: fields.muted == true)
        microphonePurpose = fields.purpose
        microphoneMeeting = fields.purpose == .meeting
        return result
    }

    @MainActor private func startSystemAudio(_ fields: NativeAudioStartFields,
                                              streamID id: String) async throws -> [String: Any] {
        guard fields.processing == nil || fields.processing == .unprocessed else {
            throw NativeFailure("processing_not_supported_for_source")
        }
        guard audioSession.mode != .voiceProcessing || !audioSession.engine.isRunning else {
            throw NativeFailure("audio_session_busy")
        }
        let captureScope = fields.capture_scope ?? .system
        let applicationPID: Int32?
        switch captureScope {
        case .system:
            applicationPID = nil
        case .application:
            guard let pid = fields.application_pid, pid > 0 else {
                throw NativeFailure("application_pid_required")
            }
            applicationPID = pid
        }
        // The application's audio is captured by its PID, the
        // one `microphone_activity` reported; no identity receipt
        // travels with the start (the meeting rules that sent one were
        // retired).
        let result = try await systemAudio.start(streamID: id, captureScope: captureScope.rawValue,
            applicationPID: applicationPID,
            allowStart: { [weak self] in self?.latch.stopped == false && self?.connected == true })
        systemAudioScope = (captureScope: captureScope.rawValue, applicationPID: applicationPID)
        return result
    }

    @MainActor func changePurpose(_ fields: NativeAudioPurposeFields) throws -> [String: Any] {
        guard microphone.streamID == fields.stream_id
            || audioRouteRecoveringStreamID == fields.stream_id else {
            throw NativeFailure("stream_not_found")
        }
        microphonePurpose = fields.purpose
        microphoneMeeting = fields.purpose == .meeting
        return ["stream_id": fields.stream_id, "purpose": fields.purpose.rawValue]
    }

    @MainActor func changeMute(_ fields: NativeAudioMuteFields) throws -> [String: Any] {
        // «Silenciar»: the samples leave as silence from now on, same
        // frames and timing; kept over a route change's restart.
        let id = fields.stream_id
        guard microphone.streamID == id || audioRouteRecoveringStreamID == id else {
            throw NativeFailure("stream_not_found")
        }
        try microphone.setMuted(fields.muted, streamID: id)
        return ["stream_id": id, "muted": fields.muted]
    }

    @MainActor func changeWakePhrase(_ fields: NativeAudioWakePhraseFields) throws -> [String: Any] {
        // «Oye Deimos» goes with the meeting recording, on the
        // live microphone stream whoever opened it: the daemon turns it
        // on when the meeting starts or resumes and off when it pauses
        // or ends, also while a voice note keeps the stream open.
        let id = fields.stream_id
        guard !id.isEmpty, id.utf8.count <= 128 else { throw NativeFailure("invalid_stream_id") }
        if fields.enabled {
            guard microphone.activeStreamID == id else { throw NativeFailure("stream_not_found") }
            let sink = assistantTrigger.start(streamID: id, assistantName: fields.assistant_name ?? "Deimos",
                                              assistantNames: fields.assistant_names, locale: fields.locale, allow: { [weak self] stream in
                guard let self else { return false }
                return !self.latch.stopped && self.microphone.activeStreamID == stream
            })
            do { try microphone.setFrameSink(sink, streamID: id) }
            catch { assistantTrigger.stop(); throw error }
        } else {
            assistantTrigger.stop()
            if microphone.streamID == id { try? microphone.setFrameSink(nil, streamID: id) }
        }
        return ["stream_id": id, "enabled": fields.enabled]
    }

    @MainActor func stopAudio(_ fields: NativeStreamFields) async throws -> [String: Any] {
        let id = fields.stream_id
        // The person's stop wins over a restart of the system's audio.
        let wasRestarting = systemAudioRestart?.streamID == id
        if wasRestarting { systemAudioRestart = nil }
        if microphone.streamID == id || audioRouteRecoveringStreamID == id {
            audioRouteEpoch &+= 1
            microphonePurpose = nil
            microphoneMeeting = false
            assistantTrigger.stop()
            microphone.disableImmediately()
            audioSession.stopIfIdle()
            guard await microphone.stop() else { throw NativeFailure("audio_drain_incomplete") }
        }
        else if systemAudio.streamID == id {
            guard await systemAudio.stop() else { throw NativeFailure("audio_drain_incomplete") }
        }
        // Between the old stream and the new one of a restart.
        else if wasRestarting { _ = await systemAudio.stop() }
        else if endedMicrophoneStreamID == id { endedMicrophoneStreamID = nil }
        else if endedSystemAudioStreamID == id { endedSystemAudioStreamID = nil }
        else { throw NativeFailure("stream_not_found") }
        return ["stream_id": id, "drain_complete": true]
    }
}
