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

// Stopping capture and playback, the latch the user's brake writes, and what
// a rearm needs first.
extension OrbitApplication {
    func stopWork(latches: Bool) -> PrivacyStopWork {
        PrivacyStopWork(
            latchAlreadyStopped: latch.stopped,
            microphoneSlot: microphone.slot,
            systemAudioSlot: systemAudio.slot,
            playbackLive: playback.playing,
            cleanupInFlight: privacyCleanupInFlight,
            latches: latches)
    }

    /// Stops capture and playback for `reason`. Only the user's own brake writes
    /// the durable latch (`StopPolicy`): every other reason leaves it
    /// as it was, so a sleeping Mac, a changed audio route, a silent service
    /// or a closing never leaves Orbit stopped after a restart. The event says
    /// which of the two happened: `privacy_stop` or `capture_stopped`.
    func stop(reason: String, completion: ((Bool) -> Void)? = nil) {
        let latches = StopPolicy.latches(reason: reason)
        // The reason is the single most useful line in the log: it separates a
        // deliberate stop from a dead daemon, a sleeping Mac or a lost device.
        diagnostics.recordOffMain(latches ? "privacy_stopped" : "capture_stopped", ["reason": reason])
        // «Oye Deimos» stops first, with every stop — the user's brake
        // included — even when nothing else is live (a listener can still be
        // preparing its model).
        assistantTrigger.stop()
        let work = stopWork(latches: latches)
        if work.isRedundant {
            // Nothing is live and nothing is left to persist. Running the full
            // stop would bump privacyEpoch, and a pending permission request
            // compares that epoch before handing macOS's answer back, so this
            // silently cancelled the TCC dialog the person was reading. It also
            // reported an incomplete drain, which the daemon treats as a failure
            // and answers with another stop: that is how it became a loop.
            channel.send(.makeStop(latches: latches, reason: reason, stopped: latch.stopped,
                drainComplete: true))
            completion?(work.reportsDrained)
            publishState()
            return
        }
        privacyEpoch &+= 1
        microphonePurpose = nil
        microphoneMeeting = false
        systemAudioRestart = nil
        if latches { latch.beginStop() }
        invalidateCalendars(latches ? .privacyStopped : .invalidated)
        systemAudio.disableImmediately()
        microphone.disableImmediately()
        cancelPlayback()
        audioSession.stop()
        windowScreenshots.invalidate()
        applicationIdentities.invalidate()
        // The user's brake silences the microphone observer too; any other stop
        // (sleep, a changed device) leaves it watching for the next call.
        if latches { microphoneActivity.stop() }
        channel.send(.makeStop(latches: latches, reason: reason, stopped: latch.stopped,
            drainComplete: false))
        privacyCleanupInFlight += 1
        Task { @MainActor in
            async let latchSaved = self.persistLatchIfUserBrake(latches)
            let drained = await self.privacyDrainBarrier.complete { @MainActor in
                async let microphoneDrained = self.microphone.stop()
                async let systemDrained = self.systemAudio.stop()
                let results = await (microphoneDrained, systemDrained)
                return results.0 && results.1
            }
            let persisted = await latchSaved
            self.privacyCleanupInFlight -= 1
            self.publishState()
            completion?(drained && persisted)
        }
        publishState()
    }

    /// Writes the durable latch only for the user's brake; any other stop has
    /// nothing to persist.
    private func persistLatchIfUserBrake(_ latches: Bool) async -> Bool {
        guard latches else { return true }
        return await persistPrivacyStop()
    }

    private func persistPrivacyStop() async -> Bool {
        do {
            try await latch.persistStopOffMain()
            return true
        } catch {
            channel.send(.makeError(code: "privacy_stop_persistence_failed"))
            return false
        }
    }

    func rearmReadiness() -> RearmReadiness {
        RearmReadiness(
            connected: connected, quitRequested: quitRequested,
            commandInFlight: commandRouter.hasCommandInFlight, cleanupInFlight: privacyCleanupInFlight,
            microphoneSlot: microphone.slot,
            systemAudioSlot: systemAudio.slot)
    }

    func cancelPlayback() {
        playbackEpoch &+= 1
        let cancelled = commandRouter.removeQueued {
            $0.command == "playback" || $0.command == "playback_clip"
        }
        for command in cancelled { channel.respond(command, result: ["state": "cancelled"]) }
        playback.stop()
    }
}
