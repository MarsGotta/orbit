import Foundation

public enum AudioProcessingMode: String, Codable, Sendable {
    case unprocessed
    case voiceProcessing = "voice_processing"
}

/// Facts read from an existing audio graph, not a hardware-quality certificate.
public struct AudioProcessingObservation: Equatable, Sendable {
    public let mode: AudioProcessingMode
    public let enabled: Bool
    public let bypassed: Bool
    public let agcEnabled: Bool
    public let sharedPlaybackReference: Bool
    public let pipelineID: String

    public init(mode: AudioProcessingMode, enabled: Bool, bypassed: Bool,
                agcEnabled: Bool, sharedPlaybackReference: Bool, pipelineID: String) {
        self.mode = mode
        self.enabled = enabled
        self.bypassed = bypassed
        self.agcEnabled = agcEnabled
        self.sharedPlaybackReference = sharedPlaybackReference
        self.pipelineID = pipelineID
    }

    public var usableForContinuousVoice: Bool {
        mode == .voiceProcessing && enabled && !bypassed && sharedPlaybackReference && !pipelineID.isEmpty
    }

    public var fields: [String: Any] {
        ["processing": mode.rawValue, "processing_enabled": enabled,
         "processing_bypassed": bypassed, "agc_enabled": agcEnabled,
         "playback_reference": sharedPlaybackReference ? "shared_engine" : "none",
         "pipeline_id": pipelineID]
    }
}

public enum AudioConfigurationPolicy {
    /// Reconfiguration may not alter a recording or interrupt an active player.
    /// Raw meetings remain raw; sharing a processed microphone is a later gate.
    public static func permitsCapture(requested: AudioProcessingMode, current: AudioProcessingMode,
                                      configured: Bool, microphoneActive: Bool,
                                      remoteActive: Bool, playbackActive: Bool) -> Bool {
        guard !microphoneActive else { return false }
        if requested == .voiceProcessing && remoteActive { return false }
        if !configured || requested != current {
            return !remoteActive && !playbackActive
        }
        return true
    }
}

/// RMS of captured PCM for the local wake-phrase detector, outside the callback.
public struct PCMLevel {
    public let rmsDBFS: Double

    public init(pcm16: Data) {
        var squareSum = 0.0
        var count = 0
        pcm16.withUnsafeBytes { bytes in
            for index in stride(from: 0, to: bytes.count - bytes.count % 2, by: 2) {
                let bits = UInt16(bytes[index]) | UInt16(bytes[index + 1]) << 8
                let value = Int16(bitPattern: bits)
                let magnitude = abs(Double(value)) / 32768
                squareSum += magnitude * magnitude
                count += 1
            }
        }
        let rms = count > 0 ? sqrt(squareSum / Double(count)) : 0
        rmsDBFS = max(-120, 20 * log10(max(rms, 0.000001)))
    }
}

/// Stateful, bounded interpolation into the fixed 48 kHz mono player graph.
/// All accepted source rates are <= 48 kHz; this never downsamples. State spans
/// packets so a boundary cannot duplicate samples or reset fractional phase.
/// Runs outside realtime callbacks. No more than one source interval is delayed.
public struct PlaybackResampler {
    public static let outputRate = 48_000
    private let sourceRate: Int
    private var previous: Float?
    private var phase = 0

    public init(sourceRate: Int) throws {
        guard NativeProtocol.acceptedSampleRates.contains(sourceRate) else { throw ProtocolError.invalidAudio }
        self.sourceRate = sourceRate
    }

    public mutating func append(_ pcm16: Data) throws -> [Float] {
        guard !pcm16.isEmpty, pcm16.count.isMultiple(of: 2), pcm16.count <= sourceRate * 2 else {
            throw ProtocolError.invalidAudio
        }
        var input = [Float]()
        input.reserveCapacity(pcm16.count / 2 + 1)
        if let previous { input.append(previous) }
        pcm16.withUnsafeBytes { bytes in
            for index in stride(from: 0, to: bytes.count, by: 2) {
                let bits = UInt16(bytes[index]) | UInt16(bytes[index + 1]) << 8
                input.append(Float(Int16(bitPattern: bits)) / 32768)
            }
        }
        var output = [Float]()
        output.reserveCapacity(Self.outputRate + 3)
        while true {
            let index = phase / Self.outputRate
            let fraction = phase % Self.outputRate
            guard index < input.count, fraction == 0 || index + 1 < input.count else { break }
            let left = input[index]
            if fraction == 0 { output.append(left) }
            else { output.append(left + (input[index + 1] - left) * Float(fraction) / Float(Self.outputRate)) }
            phase += sourceRate
        }
        phase -= (input.count - 1) * Self.outputRate
        previous = input.last
        return output
    }
}
