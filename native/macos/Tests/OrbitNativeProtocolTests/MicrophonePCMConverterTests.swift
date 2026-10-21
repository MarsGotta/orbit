import XCTest
import OrbitAudioRing
import OrbitNativeState
import OrbitNativeTransport
@testable import OrbitNativeHost

/// Synthetic PCM only: AVAudioConverter does not open an audio device.
final class MicrophonePCMConverterTests: XCTestCase {
    func testUnchangedRatePreservesEveryByte() throws {
        let pcm = Data([1, 2, 3, 4, 5, 6])
        XCTAssertEqual(try MicrophonePCMConverter(outputRate: 48_000).convert(pcm, sampleRate: 48_000), pcm)
    }

    func testNewHardwareRateKeepsTheOriginalWireRateAndDuration() throws {
        let routes: [(UInt32, UInt32)] = [(48_000, 24_000), (24_000, 48_000)]
        for (inputRate, outputRate) in routes {
            let converter = MicrophonePCMConverter(outputRate: outputRate)
            let pcm = Data(repeating: 0, count: Int(inputRate) * 2)
            let converted = try converter.convert(pcm, sampleRate: inputRate)
            XCTAssertEqual(converted.count, Int(outputRate) * 2)
            XCTAssertTrue(converted.allSatisfy { $0 == 0 })
        }
    }

    func testRelayPublishesTheMeasuredGapBeforeResumedPCMAtTheOriginalRate() async throws {
        let input = Pipe()
        let output = Pipe()
        defer { withExtendedLifetime((input, output)) {} }
        let channel = NativeChannel(input: input.fileHandleForReading, output: output.fileHandleForWriting)
        let before = try AudioFrameRelay(streamID: "same-stream", source: "mic_local", channel: channel,
            processing: nil, outputRate: 8_000)
        let first = [Float](repeating: 0, count: 800)
        first.withUnsafeBufferPointer {
            XCTAssertTrue(orbit_ring_push_interleaved_timed(before.ring, $0.baseAddress,
                1, 800, 8_000, 1_000_000_000, 1_000_000_000))
        }
        let oldDrained = await before.stop()
        XCTAssertTrue(oldDrained)
        XCTAssertEqual(before.lastFrameEndNS, 1_100_000_000)
        let after = try AudioFrameRelay(streamID: "same-stream", source: "mic_local", channel: channel,
            processing: nil, outputRate: 8_000,
            routeResume: AudioRouteResume(previousEndNS: before.lastFrameEndNS))
        let resumed = [Float](repeating: 0, count: 480)
        resumed.withUnsafeBufferPointer {
            XCTAssertTrue(orbit_ring_push_interleaved_timed(after.ring, $0.baseAddress,
                1, 480, 48_000, 3_100_000_000, 3_100_000_000))
        }
        let newDrained = await after.stop()
        XCTAssertTrue(newDrained)
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        try output.fileHandleForWriting.close()
        let lines = output.fileHandleForReading.readDataToEndOfFile().split(separator: 10)
        let events = try lines.map {
            try XCTUnwrap(JSONSerialization.jsonObject(with: Data($0)) as? [String: Any])
        }
        XCTAssertEqual(events.compactMap { $0["event"] as? String }, ["audio", "gap", "audio"])
        let gap = try XCTUnwrap(events.dropFirst().first)
        let frame = try XCTUnwrap(events.dropFirst(2).first)
        XCTAssertEqual(gap["duration_ns"] as? UInt64, 2_000_000_000)
        XCTAssertEqual(gap["duration_unknown"] as? Bool, false)
        XCTAssertEqual(frame["stream_id"] as? String, "same-stream")
        XCTAssertEqual(frame["sequence"] as? UInt64, 0)
        XCTAssertEqual(frame["sample_rate"] as? UInt32, 8_000)
        XCTAssertEqual(frame["frames"] as? UInt32, 80)
    }
}
