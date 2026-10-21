import AVFoundation
import Foundation
import XCTest
@testable import OrbitNativeHost
@testable import OrbitNativeProtocol

final class CompressedAudioDecoderTests: XCTestCase {
    private func appendLittleEndian(_ value: UInt16, to data: inout Data) {
        data.append(UInt8(truncatingIfNeeded: value))
        data.append(UInt8(truncatingIfNeeded: value >> 8))
    }

    private func appendLittleEndian(_ value: UInt32, to data: inout Data) {
        data.append(UInt8(truncatingIfNeeded: value))
        data.append(UInt8(truncatingIfNeeded: value >> 8))
        data.append(UInt8(truncatingIfNeeded: value >> 16))
        data.append(UInt8(truncatingIfNeeded: value >> 24))
    }

    private func pcmWave(samples: [Int16], sampleRate: UInt32) -> Data {
        let channelCount: UInt16 = 1
        let bytesPerSample: UInt16 = 2
        let dataByteCount = UInt32(samples.count) * UInt32(bytesPerSample)
        var data = Data()

        data.append(contentsOf: "RIFF".utf8)
        appendLittleEndian(36 + dataByteCount, to: &data)
        data.append(contentsOf: "WAVE".utf8)
        data.append(contentsOf: "fmt ".utf8)
        appendLittleEndian(UInt32(16), to: &data)
        appendLittleEndian(UInt16(1), to: &data)
        appendLittleEndian(channelCount, to: &data)
        appendLittleEndian(sampleRate, to: &data)
        appendLittleEndian(sampleRate * UInt32(channelCount * bytesPerSample), to: &data)
        appendLittleEndian(channelCount * bytesPerSample, to: &data)
        appendLittleEndian(UInt16(16), to: &data)
        data.append(contentsOf: "data".utf8)
        appendLittleEndian(dataByteCount, to: &data)
        for sample in samples {
            appendLittleEndian(UInt16(bitPattern: sample), to: &data)
        }
        return data
    }

    private func clip(data: Data) -> PlaybackClip {
        PlaybackClip(
            playbackID: "decoder-test",
            mimeType: "audio/wav",
            provider: "test-fixture",
            data: data
        )
    }

    private func outputFormat(sampleRate: Double = 16_000) throws -> AVAudioFormat {
        try XCTUnwrap(AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1))
    }

    func testCompressedAudioDecoderDecodesVeryShortPCMWithExpectedFramesAndRate() throws {
        let sampleRate = 16_000.0
        let samples: [Int16] = [0, 1_000, -1_000, 2_000, -2_000, 500, -500, 0]

        let decoded = try CompressedAudioDecoder.decode(
            clip(data: pcmWave(samples: samples, sampleRate: UInt32(sampleRate))),
            to: outputFormat(sampleRate: sampleRate)
        )

        XCTAssertEqual(decoded.frameLength, AVAudioFrameCount(samples.count))
        XCTAssertEqual(decoded.format.sampleRate, sampleRate, accuracy: 0.001)
    }

    func testCompressedAudioDecoderDecodesTwoHundredMillisecondsOfPCM() throws {
        let sampleRate = 16_000.0
        let frameCount = 3_200
        let samples = (0..<frameCount).map { Int16(($0 % 2_000) - 1_000) }

        let decoded = try CompressedAudioDecoder.decode(
            clip(data: pcmWave(samples: samples, sampleRate: UInt32(sampleRate))),
            to: outputFormat(sampleRate: sampleRate)
        )

        XCTAssertEqual(decoded.frameLength, AVAudioFrameCount(frameCount))
        XCTAssertEqual(decoded.format.sampleRate, sampleRate, accuracy: 0.001)
    }

    func testCompressedAudioDecoderRejectsEmptyDataAsEmptyAudio() throws {
        XCTAssertThrowsError(
            try CompressedAudioDecoder.decode(clip(data: Data()), to: outputFormat())
        ) { error in
            XCTAssertEqual((error as? NativeFailure)?.code, "empty_audio")
        }
    }

    func testCompressedAudioDecoderRejectsOpenWaveWithoutSamplesAsEmptyAudio() throws {
        XCTAssertThrowsError(
            try CompressedAudioDecoder.decode(
                clip(data: pcmWave(samples: [], sampleRate: 16_000)),
                to: outputFormat()
            )
        ) { error in
            XCTAssertEqual((error as? NativeFailure)?.code, "empty_audio")
        }
    }

    func testCompressedAudioDecoderRejectsNonAudioData() throws {
        XCTAssertThrowsError(
            try CompressedAudioDecoder.decode(
                clip(data: Data("esto no es audio".utf8)),
                to: outputFormat()
            )
        ) { error in
            XCTAssertEqual((error as? NativeFailure)?.code, "unsupported_media_type")
        }
    }
}
