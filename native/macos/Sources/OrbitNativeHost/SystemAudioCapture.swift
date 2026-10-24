import AVFoundation
import ScreenCaptureKit
import CoreMedia
import Darwin
import OrbitAudioRing
import OrbitNativeProtocol
import OrbitNativeState
import OrbitNativeTransport

/// Main-thread ownership of a single physical producer and its fixed destination.
/// Normal stop keeps this session through the queue/drain barrier; a failed
/// startup is then released without turning its missing receipt into drain loss.
private final class SystemAudioSession {
    let stream: SCStream
    let output: SystemAudioOutput
    let relay: AudioFrameRelay
    var starting: Task<Void, Error>?

    init(stream: SCStream, relay: AudioFrameRelay, queue: DispatchQueue) {
        self.stream = stream
        self.relay = relay
        output = SystemAudioOutput(stream: stream, relay: relay, queue: queue)
    }
}

package enum AudioScopeFilter: Equatable {
    case excludeApplications([String])
    case includeApplication(processID: pid_t)
}

package enum AudioScopeRejection: Error, Equatable {
    case applicationPIDRequired
    case unknownScope(String)
}

package final class SystemAudioCapture: NSObject, SCStreamDelegate {
    private static let orbitBundleIDs = ["app.orbit.companion"]
    private var session: SystemAudioSession?
    var relay: AudioFrameRelay? { session?.relay }
    package private(set) var slot = CaptureSlot.idle
    package var streamID: String? { relay?.streamID }
    private let channel: NativeChannel
    private let queue = DispatchQueue(label: "orbit.native.system-audio")
    private var stopTask: Task<Bool, Never>?
    private var stoppingFailedStart = false
    package var failed: ((String) -> Void)?

    package init(channel: NativeChannel) { self.channel = channel }

    @MainActor package var activeStreamID: String? {
        guard slot == .active, let session, session.starting == nil, stopTask == nil,
              let relay, !orbit_ring_has_timing_failure(relay.ring) else { return nil }
        return relay.streamID
    }

    package static func scopeFilter(for scope: String?, applicationPID: Int32?) throws -> AudioScopeFilter {
        switch scope ?? "system" {
        case "system":
            return .excludeApplications(orbitBundleIDs)
        case "application":
            guard let applicationPID, applicationPID > 0 else {
                throw AudioScopeRejection.applicationPIDRequired
            }
            return .includeApplication(processID: applicationPID)
        case let scope:
            throw AudioScopeRejection.unknownScope(scope)
        }
    }

    package static func scopeFilter(for command: NativeWireCommand) throws -> AudioScopeFilter {
        guard case .audioStart(let wire) = command else { throw ProtocolError.invalidCommand }
        return try scopeFilter(for: wire.fields.capture_scope?.rawValue, applicationPID: wire.fields.application_pid)
    }

    @MainActor package func start(streamID: String, captureScope: String?, applicationPID: Int32?,
                          expectedBundleID: String? = nil,
                          validateApplication: () -> Bool = { true },
                          allowStart: () -> Bool) async throws -> [String: Any] {
        let scopeFilter = try Self.scopeFilter(for: captureScope, applicationPID: applicationPID)
        guard stopTask == nil, slot.transition(to: .starting) else {
            throw NativeFailure("system_audio_already_active")
        }
        session = nil
        stoppingFailedStart = false
        var started = false
        defer {
            if !started { _ = slot.transition(to: .idle) }
        }
        guard CGPreflightScreenCaptureAccess() else { throw NativeFailure("screen_permission_required") }
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: false)
        guard allowStart() else { throw NativeFailure("operation_cancelled_by_stop") }
        guard slot == .starting, session == nil, stopTask == nil else {
            throw NativeFailure("operation_cancelled_by_stop")
        }
        guard let display = content.displays.first else {
            throw NativeFailure("display_not_available")
        }
        let selection = try Self.contentFilter(scopeFilter, content: content, display: display,
            expectedBundleID: expectedBundleID, validateApplication: validateApplication)
        let selectedApplication = selection.application
        let relay = try AudioFrameRelay(streamID: streamID, source: "remote_app", channel: channel,
            processing: nil)
        let stream = SCStream(filter: selection.filter, configuration: Self.audioConfiguration(), delegate: self)
        let session = SystemAudioSession(stream: stream, relay: relay, queue: queue)
        self.session = session
        do {
            try stream.addStreamOutput(session.output, type: .audio, sampleHandlerQueue: queue)
            // This task owns only the SDK start, not this outer start method.
            // A simultaneous stop can await it even if the catch below awaits stop.
            let starting = Task { try await stream.startCapture() }
            session.starting = starting
            try await starting.value
            session.starting = nil
            guard allowStart(), stopTask == nil, self.session === session else {
                throw NativeFailure("operation_cancelled_by_stop")
            }
            if selectedApplication != nil {
                guard validateApplication() else { throw NativeFailure("application_binding_changed") }
            }
            guard let clock = stream.synchronizationClock,
                  CaptureClock.nanoseconds(CMClockGetTime(clock)) != nil,
                  CaptureClock.hostNanoseconds(CMClockGetTime(clock), from: clock) != nil else {
                throw NativeFailure("system_audio_clock_unavailable")
            }
        } catch {
            session.starting = nil // The SDK task has returned (or was never started).
            if self.session === session { _ = await stop() }
            throw error
        }
        relay.start()
        guard slot.transition(to: .active) else {
            _ = await stop()
            throw NativeFailure("operation_cancelled_by_stop")
        }
        stoppingFailedStart = false
        started = true
        var result: [String: Any] = ["stream_id": streamID, "source": "remote_app",
                                     "sample_rate": 48_000, "channels": 1,
                                     "capture_scope": selection.scope]
        if case .includeApplication(let processID) = scopeFilter, let selectedApplication {
            result["application_pid"] = processID
            result["bundle_id"] = selectedApplication.bundleIdentifier
        }
        return result
    }

    /// What the stream records: the whole system without Orbit's own audio,
    /// or the one application the daemon named, while it is still that one.
    private static func contentFilter(_ scopeFilter: AudioScopeFilter, content: SCShareableContent,
                                      display: SCDisplay, expectedBundleID: String?,
                                      validateApplication: () -> Bool) throws
        -> (filter: SCContentFilter, application: SCRunningApplication?, scope: String) {
        switch scopeFilter {
        case .excludeApplications(let bundleIDs):
            let applications = content.applications.filter { bundleIDs.contains($0.bundleIdentifier) }
            let filter = SCContentFilter(display: display, excludingApplications: applications,
                                         exceptingWindows: [])
            return (filter: filter, application: nil, scope: "system")
        case .includeApplication(let processID):
            guard validateApplication() else { throw NativeFailure("application_binding_changed") }
            guard let application = content.applications.first(where: { $0.processID == processID }) else {
                throw NativeFailure("application_not_available")
            }
            guard expectedBundleID == nil || expectedBundleID == application.bundleIdentifier else {
                throw NativeFailure("application_binding_changed")
            }
            guard processID != ProcessInfo.processInfo.processIdentifier else {
                throw NativeFailure("cannot_capture_orbit_audio")
            }
            let filter = SCContentFilter(display: display, including: [application], exceptingWindows: [])
            return (filter: filter, application: application, scope: "application")
        }
    }

    /// Audio only, mono at 48 kHz; the video the stream must carry is the
    /// smallest there is, 2×2 at one frame a second.
    private static func audioConfiguration() -> SCStreamConfiguration {
        let configuration = SCStreamConfiguration()
        configuration.capturesAudio = true
        configuration.excludesCurrentProcessAudio = true
        configuration.captureMicrophone = false
        configuration.sampleRate = 48_000
        configuration.channelCount = 1
        configuration.width = 2
        configuration.height = 2
        configuration.minimumFrameInterval = CMTime(value: 1, timescale: 1)
        return configuration
    }

    @MainActor package func stop() async -> Bool {
        disableImmediately()
        if let stopTask { return await stopTask.value }
        guard let session else {
            _ = slot.transition(to: .idle)
            return true
        }
        let starting = session.starting
        let task = Task { @MainActor in
            if let starting { _ = await starting.result }
            var stopped = true
            do { try await session.stream.stopCapture() }
            catch { stopped = false; self.channel.send(.makeError(code: "system_audio_stop_failed")) }
            // stopCapture completing precedes this FIFO marker. All callbacks
            // queued before the stopped producer must finish before relay.stop.
            await withCheckedContinuation { continuation in
                self.queue.async { continuation.resume() }
            }
            let drained = await session.relay.stop()
            let failedStart = self.stoppingFailedStart
            let drainHealthy = failedStart || (stopped && drained)
            if self.session === session {
                self.session = nil
                let reason = stopped ? "audio_drain_incomplete" : "system_audio_stop_failed"
                let destination: CaptureSlot = drainHealthy
                    ? .idle : .unconfirmed(reason: reason)
                _ = self.slot.transition(to: destination)
            }
            self.stoppingFailedStart = false
            self.stopTask = nil
            return drainHealthy
        }
        stopTask = task
        return await task.value
    }

    package func disableImmediately() {
        let startWasPending = slot == .starting
        if startWasPending { stoppingFailedStart = true }
        if startWasPending || slot == .active { _ = slot.transition(to: .draining) }
        guard let session else { return }
        let relay = session.relay
        if let clock = session.stream.synchronizationClock,
           let cutoff = CaptureClock.nanoseconds(CMClockGetTime(clock)) {
            // Do NOT disable admission here: SCK can have pre-stop PCM waiting
            // on its sampleHandlerQueue. Its PTS, not callback arrival, gates it.
            orbit_ring_set_capture_cutoff_ns(relay.ring, cutoff)
        } else {
            if !startWasPending { orbit_ring_mark_timing_failure(relay.ring) }
            orbit_ring_set_enabled(relay.ring, false)
        }
    }

    package func stream(_ stream: SCStream, didStopWithError error: Error) {
        // Keep the observed object alive across the dispatch hop: an unretained
        // ObjectIdentifier could match a new stream reusing its old address.
        DispatchQueue.main.async { [weak self, stream] in
            guard let self, let current = self.session,
                  current.stream === stream else { return }
            self.failed?("system_audio_stream_stopped")
        }
    }
}
