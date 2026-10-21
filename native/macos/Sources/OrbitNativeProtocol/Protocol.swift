import Foundation

public enum NativeProtocol {
    public static let version = 1
    public static let maximumLineBytes = NativeLimits.maximumCommandLineBytes
    public static let maximumPlaybackBytes = 96_000
    public static let maximumPlaybackClipBytes = NativeLimits.maximumPlaybackClipBytes
    public static let acceptedSampleRates = [16_000, 24_000, 32_000, 44_100, 48_000]
}

extension NativePlaybackFields {
    public func playbackData() throws -> Data {
        guard !playback_id.isEmpty, playback_id.utf8.count <= 128,
              channels == 1,
              NativeProtocol.acceptedSampleRates.contains(sample_rate),
              pcm_s16le_base64.utf8.count <= NativeProtocol.maximumPlaybackBytes * 2,
              let data = Data(base64Encoded: pcm_s16le_base64), !data.isEmpty,
              data.count.isMultiple(of: 2), data.count <= NativeProtocol.maximumPlaybackBytes,
              data.count <= sample_rate * 2 else {
            throw ProtocolError.invalidAudio
        }
        return data
    }
}

extension NativePlaybackClipFields {
    public func playbackClip() throws -> PlaybackClip {
        guard !playback_id.isEmpty, playback_id.utf8.count <= 128,
              !mime_type.isEmpty, mime_type.utf8.count <= 128,
              mime_type.unicodeScalars.allSatisfy({ (0x20...0x7E).contains($0.value) }),
              !provider.isEmpty, provider.utf8.count <= 128,
              provider.unicodeScalars.allSatisfy({ (0x20...0x7E).contains($0.value) }),
              audio_base64.utf8.count <= 4 * ((NativeProtocol.maximumPlaybackClipBytes + 2) / 3),
              let data = Data(base64Encoded: audio_base64), !data.isEmpty,
              data.count <= NativeProtocol.maximumPlaybackClipBytes else {
            throw ProtocolError.invalidAudio
        }
        let mediaType = mime_type.split(separator: ";", maxSplits: 1)[0]
            .trimmingCharacters(in: .whitespaces).lowercased()
        guard ["audio/mpeg", "audio/mp4", "audio/aac", "audio/x-aac",
               "audio/wav", "audio/x-wav"].contains(mediaType) else {
            throw ProtocolError.unsupportedMediaType
        }
        return PlaybackClip(playbackID: playback_id, mimeType: mime_type,
                            provider: provider, data: data)
    }
}

public enum ProtocolError: Error, Equatable {
    case messageTooLarge, versionMismatch, invalidIdentifier, invalidCommand, invalidAudio, unsupportedMediaType
}

public struct PlaybackClip: Equatable, Sendable {
    public let playbackID: String
    public let mimeType: String
    public let provider: String
    public let data: Data
}

/// A bounded decoder keeps a malicious or damaged pipe from growing memory forever.
public struct LineDecoder {
    private var pending = Data()
    public init() {}

    public mutating func append(_ bytes: Data) throws -> [Data] {
        var lines = [Data]()
        for byte in bytes {
            if byte == 10 {
                if !pending.isEmpty { lines.append(pending) }
                pending.removeAll(keepingCapacity: true)
            } else {
                guard pending.count < NativeProtocol.maximumLineBytes else {
                    throw ProtocolError.messageTooLarge
                }
                pending.append(byte)
            }
        }
        return lines
    }
}

/// Pure acceptance rules for playback. Stop fences late packets of that generation.
public struct PlaybackFence {
    public private(set) var currentID: String?
    public private(set) var nextSequence: UInt64 = 0
    private var retired = Set<String>()

    public init() {}

    public mutating func accept(id: String, sequence: UInt64) -> Bool {
        guard !retired.contains(id) else { return false }
        if currentID != id {
            guard sequence == 0 else { return false }
            if let currentID { retired.insert(currentID) }
            currentID = id
            nextSequence = 0
        }
        guard sequence == nextSequence, nextSequence < UInt64.max else { return false }
        nextSequence += 1
        return true
    }

    public mutating func stop() {
        if let currentID { retired.insert(currentID) }
        currentID = nil
        nextSequence = 0
        // A session has a bounded number of generations; reconnect before exhausting it.
    }

    public var generationCount: Int { retired.count }
}
