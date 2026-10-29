import AVFoundation
import ScreenCaptureKit
import CoreMedia
import Darwin
import OrbitAudioRing
import OrbitNativeProtocol
import OrbitNativeState
import OrbitNativeTransport

/// Owns one single-producer ring for each capture source. No file, JSON or network
/// operations occur in the audio callback. The poller runs on a separate queue.
final class AudioFrameRelay {
    let ring: OpaquePointer
    let streamID: String
    let source: String
    private let channel: NativeChannel
    private let processing: AudioProcessingObservation?
    private let converter: MicrophonePCMConverter?
    private var routeResume: AudioRouteResume?
    private let failed: ((String) -> Void)?
    private(set) var lastFrameEndNS: UInt64?
    private let playbackActivity: PlaybackActivity?
    /// The «Oye Deimos» listener of a meeting's microphone, fed from
    /// this drain, never from the audio callback. Read and written only on
    /// `queue`, so it can be attached to a live stream and taken off it.
    private var frameSink: MicrophoneFrameSink?
    /// «Silenciar»: which drained blocks leave as silence. Read and written
    /// only on `queue`.
    private var mute = MicrophoneMuteGate()
    private let queue = DispatchQueue(label: "orbit.native.audio.drain")
    private var timer: DispatchSourceTimer?
    private var lastFramesAt = ProcessInfo.processInfo.systemUptime
    private var warnedAboutMissingFrames = false
    private var deliveryFailed = false
    private var warnedAboutTiming = false
    private var conversionFailed = false
    private var emptyConvertedBlocks: UInt64 = 0

    init(streamID: String, source: String, channel: NativeChannel,
         processing: AudioProcessingObservation?, playbackActivity: PlaybackActivity? = nil,
         outputRate: UInt32? = nil, routeResume: AudioRouteResume? = nil,
         failed: ((String) -> Void)? = nil) throws {
        guard let ring = orbit_ring_create() else { throw NativeFailure("audio_memory_unavailable") }
        self.ring = ring
        self.streamID = streamID
        self.source = source
        self.channel = channel
        self.processing = processing
        self.playbackActivity = playbackActivity
        converter = outputRate.map { MicrophonePCMConverter(outputRate: $0) }
        self.routeResume = routeResume
        self.failed = failed
    }

    /// Attaches (or, with `nil`, takes off) the phrase listener on
    /// this live stream. The previous listener, if another, is finished.
    func setFrameSink(_ sink: MicrophoneFrameSink?) {
        queue.async { [self] in
            let previous = frameSink
            frameSink = sink
            if let previous, previous !== sink { previous.finish() }
        }
    }

    /// Mutes or unmutes what this relay sends from the next drain on. Once it
    /// returns no muted sample leaves; false when an unmute cannot read the
    /// host clock, and the relay then stays muted.
    func setMuted(_ value: Bool) -> Bool {
        var nowNS: UInt64 = 0
        if !value {
            guard let now = CaptureClock.nanoseconds(CMClockGetTime(CMClockGetHostTimeClock())) else {
                return false
            }
            nowNS = now
        }
        queue.sync { [self] in mute.set(muted: value, nowNS: nowNS) }
        return true
    }

    func start() {
        let timer = DispatchSource.makeTimerSource(queue: queue)
        // Two milliseconds lets capture drains coalesce. The 32-block ring
        // holds 320 ms even at 10 ms per block; this is not audio loss
        // tolerance and the final stop still drains immediately.
        timer.schedule(deadline: .now(), repeating: .milliseconds(10), leeway: .milliseconds(2))
        timer.setEventHandler { [weak self] in self?.drain() }
        self.timer = timer
        timer.resume()
    }

    func stop() async -> Bool {
        orbit_ring_set_enabled(ring, false)
        timer?.cancel()
        timer = nil
        let drained = await withCheckedContinuation { continuation in
            queue.async { self.finishDrain(attempts: 250, continuation: continuation) }
        }
        // The stream ended: so does any listener on it, before stop returns.
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            queue.async { [self] in
                let previous = frameSink
                frameSink = nil
                previous?.finish()
                continuation.resume()
            }
        }
        return drained
    }

    private func finishDrain(attempts: Int, continuation: CheckedContinuation<Bool, Never>) {
        guard orbit_ring_is_quiescent(ring) else {
            guard attempts > 0 else { continuation.resume(returning: false); return }
            queue.asyncAfter(deadline: .now() + .milliseconds(1)) {
                self.finishDrain(attempts: attempts - 1, continuation: continuation)
            }
            return
        }
        drain()
        let captureLoss = orbit_ring_admitted_frames(ring) > 0
            && orbit_ring_has_timing_failure(ring)
        continuation.resume(returning: !deliveryFailed && !captureLoss)
    }

    private func drain() {
        guard !conversionFailed else { return }
        if orbit_ring_admitted_frames(ring) > 0 && orbit_ring_has_timing_failure(ring) {
            if !warnedAboutTiming {
                warnedAboutTiming = true
                if !channel.send(.makeGap(source: source, streamID: streamID,
                    reason: "capture_clock_unavailable", frames: 0,
                    preStopPossible: true, durationUnknown: true)) { deliveryFailed = true }
            }
        }
        let dropped = orbit_ring_take_dropped_frames(ring)
        if dropped > 0 {
            if !channel.send(.makeGap(source: source, streamID: streamID,
                reason: "audio_ring_overflow", frames: dropped)) { deliveryFailed = true }
        }
        var block = OrbitAudioBlock()
        while orbit_ring_pop(ring, &block) {
            lastFramesAt = ProcessInfo.processInfo.systemUptime
            warnedAboutMissingFrames = false
            let frames = Int(block.frames)
            let original = withUnsafeBytes(of: &block.samples) { Data($0.prefix(frames * 2)) }
            let data: Data
            do { data = try converter?.convert(original, sampleRate: block.sample_rate) ?? original }
            catch {
                deliveryFailed = true
                conversionFailed = true
                orbit_ring_set_enabled(ring, false)
                let reason = reportAudioRouteFailure(error, channel: channel)
                DispatchQueue.main.async { [failed] in failed?(reason) }
                return
            }
            guard !data.isEmpty else { emptyConvertedBlocks += 1; continue }
            if let routeResume {
                let duration = routeResume.durationNS(until: block.monotonic_ns)
                guard channel.send(.makeGap(source: source, streamID: streamID,
                    reason: "audio_device_changed", frames: 0, durationUnknown: duration == nil,
                    durationNS: duration)) else { deliveryFailed = true; return }
                self.routeResume = nil
            }
            let sampleRate = converter?.outputRate ?? block.sample_rate
            // The phrase listener hears what leaves: silence while muted.
            let sent = mute.outgoing(data, capturedAtNS: block.monotonic_ns)
            let measured = PCMLevel(pcm16: sent)
            frameSink?.receive(pcm16: sent, sampleRate: sampleRate, rmsDBFS: measured.rmsDBFS)
            let typed = NativeEvent.makeAudio(source: source, streamID: streamID,
                sequence: block.sequence - emptyConvertedBlocks, sampleRate: sampleRate, frames: UInt32(sent.count / 2),
                monotonicNS: block.monotonic_ns, pcmBase64: sent.base64EncodedString(),
                processing: processing, playback: playbackActivity?.snapshot())
            guard channel.send(typed) else { deliveryFailed = true; break }
            let end = block.monotonic_ns.addingReportingOverflow(
                UInt64(block.frames) * 1_000_000_000 / UInt64(block.sample_rate))
            lastFrameEndNS = end.overflow ? nil : end.partialValue
        }
        if !warnedAboutMissingFrames && ProcessInfo.processInfo.systemUptime - lastFramesAt > 3 {
            warnedAboutMissingFrames = true
            if !channel.send(.makeGap(source: source, streamID: streamID,
                reason: "no_capture_frames", frames: 0, durationUnknown: true)) { deliveryFailed = true }
        }
    }

    deinit {
        timer?.cancel()
        orbit_ring_destroy(ring)
    }
}

package struct NativeFailure: Error {
    package let code: String
    /// The macOS error under it, when there was one: its domain and number,
    /// never its text. «audio_processing_configuration_failed» alone could
    /// not say why a call's microphone did not open (2026-10-17).
    package let underlying: NSError?
    package init(_ code: String, underlying: Error? = nil) {
        self.code = code
        self.underlying = underlying.map { $0 as NSError }
    }
}

package func reportAudioRouteFailure(_ error: Error, channel: NativeChannel) -> String {
    let failure = error as? NativeFailure
    let code = failure?.code ?? "audio_route_start_failed"
    let underlying = failure?.underlying ?? (failure == nil ? error as NSError : nil)
    let detail = osErrorDetail(underlying)
    channel.send(.makeError(code: code, osDomain: detail?["domain"] as? String,
        osStatus: detail?["os_status"] as? Int))
    return code
}

/// The detail a rejection carries for a macOS error: its domain and number.
package func osErrorDetail(_ error: NSError?) -> [String: Any]? {
    guard let error else { return nil }
    return ["domain": String(error.domain.prefix(64)), "os_status": error.code]
}

/// Captures the single shared engine's input. Provenance is immutable per stream.
package final class MicrophoneCapture {
    private let session: SharedAudioSession
    private(set) var relay: AudioFrameRelay?
    package private(set) var slot = CaptureSlot.idle
    package var streamID: String? { relay?.streamID }
    private let channel: NativeChannel
    private var tapInstalled = false
    private var stopTask: Task<Bool, Never>?
    package private(set) var sampleRate: UInt32?
    package var failed: ((String) -> Void)?
    /// The stream «Silenciar» muted. A route change restarts the same stream
    /// ID and stays muted; a new stream starts unmuted unless asked.
    private var mutedStreamID: String?
    package init(session: SharedAudioSession, channel: NativeChannel) {
        self.session = session
        self.channel = channel
    }

    /// A retained relay may be draining; it is not authority for a new receipt.
    @MainActor package var activeStreamID: String? {
        guard slot == .active, tapInstalled, stopTask == nil, session.captureActive,
              session.engine.isRunning, let relay,
              !orbit_ring_has_timing_failure(relay.ring) else { return nil }
        return relay.streamID
    }

    package func start(streamID: String, processing: AudioProcessingMode,
                       outputRate: UInt32? = nil, routeResume: AudioRouteResume? = nil,
                       muted: Bool = false) throws -> [String: Any] {
        if muted { mutedStreamID = streamID }
        let startsMuted = mutedStreamID == streamID
        guard stopTask == nil, slot.transition(to: .starting) else {
            throw NativeFailure("microphone_already_active")
        }
        relay = nil
        var started = false
        defer {
            if !started { _ = slot.transition(to: .idle) }
        }
        guard AVCaptureDevice.authorizationStatus(for: .audio) == .authorized else {
            throw NativeFailure("microphone_permission_required")
        }
        try session.prepareCapture(mode: processing)
        let input = session.engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.channelCount > 0, format.channelCount <= 32,
              format.sampleRate >= 8_000, format.sampleRate <= 192_000,
              format.commonFormat == .pcmFormatFloat32, !format.isInterleaved else {
            throw NativeFailure("unsupported_microphone_format")
        }
        let wireRate = outputRate ?? UInt32(format.sampleRate)
        let observation = session.observation
        let relay = try AudioFrameRelay(streamID: streamID, source: "mic_local", channel: channel,
            processing: observation, playbackActivity: session.playbackActivity,
            outputRate: wireRate, routeResume: routeResume, failed: failed)
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { [relay] buffer, time in
            let ring = relay.ring
            orbit_ring_callback_begin(ring)
            defer { orbit_ring_callback_end(ring) }
            guard let planes = buffer.floatChannelData, time.isHostTimeValid,
                  let ns = CaptureClock.nanoseconds(CMClockMakeHostTimeFromSystemUnits(time.hostTime)) else {
                orbit_ring_mark_timing_failure(ring)
                return
            }
            // CoreAudio owns the planes for this callback; the C ring copies samples now.
            let pointers = UnsafeRawPointer(planes).assumingMemoryBound(to: UnsafePointer<Float>?.self)
            _ = orbit_ring_push_planar(ring, pointers, buffer.format.channelCount,
                                       buffer.frameLength, UInt32(buffer.format.sampleRate), ns)
        }
        tapInstalled = true
        do {
            try session.start()
            guard processing != .voiceProcessing || session.observation.usableForContinuousVoice else {
                throw NativeFailure("voice_processing_reference_unavailable")
            }
            let effective = input.outputFormat(forBus: 0)
            guard session.observation == observation, effective.sampleRate == format.sampleRate,
                  effective.channelCount == format.channelCount else {
                throw NativeFailure("microphone_processing_changed_during_start")
            }
        } catch {
            input.removeTap(onBus: 0)
            tapInstalled = false
            session.stop()
            throw error
        }
        self.relay = relay
        self.sampleRate = wireRate
        // Muting never fails: only an unmute reads the clock.
        _ = relay.setMuted(startsMuted)
        session.captureDidStart()
        relay.start()
        guard slot.transition(to: .active) else {
            input.removeTap(onBus: 0)
            tapInstalled = false
            session.captureDidStop()
            session.stop()
            self.relay = nil
            throw NativeFailure("operation_cancelled_by_stop")
        }
        started = true
        var result: [String: Any] = ["stream_id": streamID, "source": "mic_local", "sample_rate": Int(wireRate),
                "channels": 1, "input_channels": Int(format.channelCount),
                "echo_control": processing == .voiceProcessing ? "apple_voice_processing_unvalidated" : "unprocessed"]
        result.merge(session.observation.fields) { _, observed in observed }
        if startsMuted { result["muted"] = true }
        return result
    }

    /// «Silenciar» on `streamID`, the live stream or the one a route change
    /// is restarting. An unmute that cannot read the host clock is refused
    /// and the stream stays muted.
    @MainActor package func setMuted(_ muted: Bool, streamID: String) throws {
        if let relay, relay.streamID == streamID, !relay.setMuted(muted) {
            throw NativeFailure("capture_clock_unavailable")
        }
        mutedStreamID = muted ? streamID : nil
    }

    @MainActor package func stopForRouteChange() async -> (drained: Bool, resume: AudioRouteResume) {
        let previous = relay
        let drained = await stop()
        return (drained, AudioRouteResume(previousEndNS: previous?.lastFrameEndNS))
    }

    /// Feeds the live microphone stream `streamID` to `sink` (or
    /// stops, with `nil`). Refused when that stream is not the active one.
    @MainActor package func setFrameSink(_ sink: MicrophoneFrameSink?, streamID: String) throws {
        guard let relay, relay.streamID == streamID else {
            throw NativeFailure("stream_not_found")
        }
        if sink != nil, activeStreamID != streamID {
            throw NativeFailure("stream_not_found")
        }
        relay.setFrameSink(sink)
    }

    package func disableImmediately() {
        if slot == .starting || slot == .active { _ = slot.transition(to: .draining) }
        if let relay {
            if let cutoff = CaptureClock.nanoseconds(CMClockGetTime(CMClockGetHostTimeClock())) {
                orbit_ring_set_capture_cutoff_ns(relay.ring, cutoff)
            } else {
                orbit_ring_mark_timing_failure(relay.ring)
                orbit_ring_set_enabled(relay.ring, false)
            }
        }
        if tapInstalled { session.engine.inputNode.removeTap(onBus: 0); tapInstalled = false }
        session.captureDidStop()
    }

    @MainActor package func stop() async -> Bool {
        disableImmediately()
        if let stopTask { return await stopTask.value }
        guard let stoppedRelay = relay else {
            _ = slot.transition(to: .idle)
            return true
        }
        let task = Task { @MainActor in
            let drained = await stoppedRelay.stop()
            if self.relay === stoppedRelay {
                self.relay = nil
                let destination: CaptureSlot = drained
                    ? .idle : .unconfirmed(reason: "audio_drain_incomplete")
                _ = self.slot.transition(to: destination)
            }
            self.stopTask = nil
            return drained
        }
        stopTask = task
        return await task.value
    }

}
