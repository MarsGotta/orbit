import AVFoundation
import CoreAudio
import OrbitNativeProtocol

/// Main-thread player updates; relay-worker reads. Never used by an audio tap.
final class PlaybackActivity {
    private let lock = NSLock()
    private var active = false
    private var playbackID: String?

    func update(active: Bool, playbackID: String?) {
        lock.lock()
        self.active = active
        self.playbackID = active ? playbackID : nil
        lock.unlock()
    }

    func snapshot() -> (active: Bool, id: String?) {
        lock.lock()
        defer { lock.unlock() }
        return (active, playbackID)
    }
}

/// Default device IDs only; formats and engine restarts do not change the route.
package struct AudioRouteDevices: Equatable {
    package let input: AudioDeviceID?
    package let output: AudioDeviceID?

    package func changed(from previous: AudioRouteDevices?) -> Bool? {
        guard let previous else { return nil }
        if let input, let previousInput = previous.input, input != previousInput { return true }
        if let output, let previousOutput = previous.output, output != previousOutput { return true }
        guard input != nil, output != nil, previous.input != nil, previous.output != nil else { return nil }
        return false
    }
}

/// One graph owns microphone processing and every TTS sample. The player never
/// changes format while recording: its 48 kHz mono connection is fixed.
/// All control methods are called on the application's main thread, not a tap.
/// Idle construction, observation and stop do not create an audio graph.
package final class SharedAudioSession {
    private final class Graph {
        let engine = AVAudioEngine()
        let player = AVAudioPlayerNode()

        init() {
            engine.attach(player)
            // The mixer (and its connection to the output) exists before any
            // mode is chosen. Created only after voice processing was turned
            // on, the engine never starts: «com.apple.coreaudio.avfaudio
            // -10875», and every call with Deimos failed to open the
            // microphone (measured on a MacBook Pro, 2026-10-17: created
            // before, the same voice-processing graph starts).
            _ = engine.mainMixerNode
        }
    }

    private var graph: Graph?
    package var engine: AVAudioEngine { requireGraph().engine }
    package var player: AVAudioPlayerNode { requireGraph().player }
    let playbackActivity = PlaybackActivity()
    package private(set) var mode = AudioProcessingMode.unprocessed
    package private(set) var configured = false
    private(set) var captureActive = false
    private(set) var pipelineID = UUID().uuidString
    private var graphConnected = false
    private var expectedRoute: RouteFingerprint?
    private var observedRoute: AudioRouteDevices?
    private var observer: NSObjectProtocol?
    package var configurationChanged: ((AudioRouteDevices?, AudioRouteDevices) -> Void)?

    package init() {}

    /// Capture/playback operations create the graph; idle observations never do.
    private func requireGraph() -> Graph {
        if let graph { return graph }
        let created = Graph()
        graph = created
        observer = NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange, object: created.engine, queue: .main
        ) { [weak self] _ in
            guard let self, self.graph != nil else { return }
            let current = self.fingerprint().devices
            let previous = self.observedRoute
            // Remember before invoking the effect: a notification caused by the
            // recovery itself must not count as a second device change.
            self.observedRoute = current
            self.configurationChanged?(previous, current)
        }
        return created
    }

    package func prepareCapture(mode requested: AudioProcessingMode) throws {
        guard try device(kAudioHardwarePropertyDefaultInputDevice) != nil else {
            throw NativeFailure("microphone_input_missing")
        }
        let input = engine.inputNode
        if !configured || mode != requested {
            engine.stop()
            expectedRoute = nil
            configured = false
            graphConnected = false
            pipelineID = UUID().uuidString
            do {
                try input.setVoiceProcessingEnabled(requested == .voiceProcessing)
                if requested == .voiceProcessing {
                    input.isVoiceProcessingBypassed = false
                    input.isVoiceProcessingAGCEnabled = false
                    // Keep other apps as loud as Apple's voice processing allows.
                    // Advanced ducking stays off pending acoustic comparison on Mac.
                    input.voiceProcessingOtherAudioDuckingConfiguration = .init(
                        enableAdvancedDucking: false, duckingLevel: .min)
                    guard !input.isVoiceProcessingBypassed, !input.isVoiceProcessingAGCEnabled else {
                        throw NativeFailure("voice_processing_settings_not_applied")
                    }
                }
                guard input.isVoiceProcessingEnabled == (requested == .voiceProcessing),
                      engine.outputNode.isVoiceProcessingEnabled == (requested == .voiceProcessing) else {
                    throw NativeFailure("voice_processing_not_enabled")
                }
                mode = requested
                try connectPlaybackGraph()
                configured = true
            } catch let failure as NativeFailure {
                engine.stop()
                throw failure
            } catch {
                engine.stop()
                throw NativeFailure("audio_processing_configuration_failed", underlying: error)
            }
        } else {
            try connectPlaybackGraph()
            pipelineID = UUID().uuidString
        }
        if requested == .voiceProcessing && !observation.usableForContinuousVoice {
            throw NativeFailure("voice_processing_reference_unavailable")
        }
    }

    package func start() throws {
        if let expected = expectedRoute,
           !engine.isRunning || fingerprint() != expected {
            throw NativeFailure("audio_route_changed")
        }
        try connectPlaybackGraph()
        if !engine.isRunning { engine.prepare(); try engine.start() }
        if configured && mode == .voiceProcessing && !observation.usableForContinuousVoice {
            engine.stop()
            throw NativeFailure("voice_processing_reference_unavailable")
        }
        let route = fingerprint()
        guard route.outputDevice != nil, !configured || route.inputDevice != nil else {
            engine.stop()
            throw NativeFailure("audio_route_unobservable")
        }
        expectedRoute = route
        observedRoute = route.devices
    }

    package func stop() {
        guard let graph else { return }
        graph.engine.stop()
        expectedRoute = nil
        observedRoute = nil
        // A stopped engine still owns VoiceProcessingIO. Release the graph,
        // not just rendering, so its processing cannot outlive the call.
        if let observer { NotificationCenter.default.removeObserver(observer) }
        observer = nil
        self.graph = nil
        graphConnected = false
        configured = false
        mode = .unprocessed
        pipelineID = UUID().uuidString
    }

    /// Either capture or playback may be the last user of the shared graph.
    package func stopIfIdle() {
        guard !captureActive, !playbackActivity.snapshot().active else { return }
        stop()
    }

    /// Cancellation must not construct the player it is trying to stop.
    package func stopPlayerIfCreated() {
        graph?.player.stop()
    }

    func captureDidStart() { captureActive = true }
    func captureDidStop() { captureActive = false }

    package func startPlayback() throws {
        // A later text-only response must not restart VoiceProcessingIO's input
        // after the call has ended. Never change the mode of a running graph.
        if !captureActive && !engine.isRunning && configured && mode == .voiceProcessing {
            try prepareCapture(mode: .unprocessed)
        }
        try start()
    }

    package var playbackFormat: AVAudioFormat? {
        AVAudioFormat(standardFormatWithSampleRate: Double(PlaybackResampler.outputRate), channels: 1)
    }

    /// Readback is intentionally false before hardware has been configured.
    var observation: AudioProcessingObservation {
        guard configured, let graph else {
            return AudioProcessingObservation(mode: mode, enabled: false, bypassed: true,
                agcEnabled: false, sharedPlaybackReference: false, pipelineID: pipelineID)
        }
        let input = graph.engine.inputNode
        let enabled = input.isVoiceProcessingEnabled && graph.engine.outputNode.isVoiceProcessingEnabled
        return AudioProcessingObservation(mode: mode, enabled: enabled,
            bypassed: !enabled || input.isVoiceProcessingBypassed,
            agcEnabled: enabled && input.isVoiceProcessingAGCEnabled,
            sharedPlaybackReference: enabled && referenceConnected(), pipelineID: pipelineID)
    }

    private var duckingConfiguration: AVAudioVoiceProcessingOtherAudioDuckingConfiguration? {
        guard configured, mode == .voiceProcessing, let graph,
              graph.engine.inputNode.isVoiceProcessingEnabled else { return nil }
        return graph.engine.inputNode.voiceProcessingOtherAudioDuckingConfiguration
    }

    package var status: [String: Any] {
        let value = observation
        var fields: [String: Any] = ["mode": value.mode.rawValue, "enabled": value.enabled,
                "bypassed": value.bypassed, "agc_enabled": value.agcEnabled,
                "playback_reference": value.sharedPlaybackReference ? "shared_engine" : "none",
                "pipeline_id": value.pipelineID, "engine_running": graph?.engine.isRunning ?? false,
                "render_sample_rate": PlaybackResampler.outputRate]
        if let ducking = duckingConfiguration {
            fields["other_audio_ducking_level"] = UInt(ducking.duckingLevel.rawValue)
            fields["other_audio_ducking_advanced"] = ducking.enableAdvancedDucking.boolValue
        }
        return fields
    }

    package var statusFields: NativeAudioProcessingResultFields {
        let value = observation
        let ducking = duckingConfiguration
        return NativeAudioProcessingResultFields(mode: value.mode.rawValue,
            enabled: value.enabled, bypassed: value.bypassed, agcEnabled: value.agcEnabled,
            playbackReference: value.sharedPlaybackReference ? "shared_engine" : "none",
            pipelineID: value.pipelineID, engineRunning: graph?.engine.isRunning ?? false,
            renderSampleRate: UInt32(PlaybackResampler.outputRate),
            otherAudioDuckingLevel: ducking.map { UInt($0.duckingLevel.rawValue) },
            otherAudioDuckingAdvanced: ducking?.enableAdvancedDucking.boolValue)
    }

    package var voiceProcessingAvailable: Bool { graph?.engine.isRunning == true && observation.usableForContinuousVoice }
    package var playbackReferenceAvailable: Bool { graph?.engine.isRunning == true && observation.sharedPlaybackReference }

    package func referenceIsHealthy() -> Bool {
        !configured || mode != .voiceProcessing || (graph?.engine.isRunning == true && observation.usableForContinuousVoice)
    }

    private func connectPlaybackGraph() throws {
        if graphConnected {
            guard referenceConnected() else { throw NativeFailure("playback_graph_changed") }
            return
        }
        guard let format = playbackFormat else {
            throw NativeFailure("unsupported_playback_format")
        }
        engine.disconnectNodeOutput(player)
        engine.connect(player, to: engine.mainMixerNode, format: format)
        graphConnected = referenceConnected()
        guard graphConnected else { throw NativeFailure("playback_reference_unavailable") }
    }

    private func referenceConnected() -> Bool {
        guard let graph else { return false }
        let engine = graph.engine
        return engine.outputConnectionPoints(for: graph.player, outputBus: 0).contains { $0.node === engine.mainMixerNode }
            && engine.outputConnectionPoints(for: engine.mainMixerNode, outputBus: 0).contains { $0.node === engine.outputNode }
    }

    private struct RouteFingerprint: Equatable {
        let inputDevice: AudioDeviceID?
        let outputDevice: AudioDeviceID?
        let inputRate: Double
        let inputChannels: AVAudioChannelCount
        let outputRate: Double
        let outputChannels: AVAudioChannelCount
        let voiceProcessing: Bool
        let bypassed: Bool

        var devices: AudioRouteDevices {
            AudioRouteDevices(input: inputDevice, output: outputDevice)
        }
    }

    private func fingerprint() -> RouteFingerprint {
        let input = configured ? engine.inputNode.outputFormat(forBus: 0) : nil
        let output = engine.outputNode.outputFormat(forBus: 0)
        return RouteFingerprint(inputDevice: try? device(kAudioHardwarePropertyDefaultInputDevice),
            outputDevice: try? device(kAudioHardwarePropertyDefaultOutputDevice),
            inputRate: input?.sampleRate ?? 0, inputChannels: input?.channelCount ?? 0,
            outputRate: output.sampleRate, outputChannels: output.channelCount,
            voiceProcessing: observation.enabled, bypassed: observation.bypassed)
    }

    private func device(_ selector: AudioObjectPropertySelector) throws -> AudioDeviceID? {
        var address = AudioObjectPropertyAddress(mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var id = AudioDeviceID(0)
        var size = UInt32(MemoryLayout<AudioDeviceID>.size)
        let status = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &id)
        guard status == noErr else {
            throw NativeFailure("audio_device_query_failed",
                underlying: NSError(domain: NSOSStatusErrorDomain, code: Int(status)))
        }
        return id == kAudioObjectUnknown ? nil : id
    }

    deinit {
        if let observer { NotificationCenter.default.removeObserver(observer) }
    }
}
