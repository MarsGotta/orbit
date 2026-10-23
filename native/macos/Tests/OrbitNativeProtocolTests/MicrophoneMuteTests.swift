import XCTest
import OrbitAudioRing
import OrbitNativeTransport
@testable import OrbitNativeHost

/// «Silenciar»: the helper sends zeros while muted, with the same frames and
/// timing. Synthetic PCM only; no audio device opens.
final class MicrophoneMuteTests: XCTestCase {
    func testAMutedGateSendsSilenceOfTheSameLength() {
        var gate = MicrophoneMuteGate()
        let pcm = Data([1, 2, 3, 4])
        XCTAssertEqual(gate.outgoing(pcm, capturedAtNS: 10), pcm)
        gate.set(muted: true, nowNS: 20)
        XCTAssertEqual(gate.outgoing(pcm, capturedAtNS: 30), Data(count: 4))
        gate.set(muted: false, nowNS: 40)
        XCTAssertEqual(gate.outgoing(pcm, capturedAtNS: 35), Data(count: 4),
                       "a block captured while muted stays silent after the unmute")
        XCTAssertEqual(gate.outgoing(pcm, capturedAtNS: 40), pcm)
    }

    func testAMutedRelaySendsZerosWithTheSameFramesAndTime() async throws {
        let input = Pipe()
        let output = Pipe()
        defer { withExtendedLifetime((input, output)) {} }
        let channel = NativeChannel(input: input.fileHandleForReading, output: output.fileHandleForWriting)
        let relay = try AudioFrameRelay(streamID: "muted-stream", source: "mic_local", channel: channel,
                                        processing: nil, outputRate: 8_000)
        XCTAssertTrue(relay.setMuted(true))
        let voice = [Float](repeating: 0.5, count: 800)
        voice.withUnsafeBufferPointer { (samples: UnsafeBufferPointer<Float>) in
            XCTAssertTrue(orbit_ring_push_interleaved_timed(relay.ring, samples.baseAddress,
                1, 800, 8_000, 1_000_000_000, 1_000_000_000))
        }
        let drained = await relay.stop()
        XCTAssertTrue(drained)
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        try output.fileHandleForWriting.close()
        let lines = output.fileHandleForReading.readDataToEndOfFile().split(separator: 10)
        let frame = try XCTUnwrap(lines.first.flatMap { (line: Data.SubSequence) -> [String: Any]? in
            try? JSONSerialization.jsonObject(with: Data(line)) as? [String: Any]
        })
        XCTAssertEqual(frame["event"] as? String, "audio")
        XCTAssertEqual(frame["frames"] as? UInt32, 800)
        XCTAssertEqual(frame["monotonic_ns"] as? UInt64, 1_000_000_000)
        let encoded = try XCTUnwrap(frame["pcm_s16le_base64"] as? String)
        let pcm = try XCTUnwrap(Data(base64Encoded: encoded))
        XCTAssertEqual(pcm.count, 1_600)
        XCTAssertTrue(pcm.allSatisfy { (byte: UInt8) -> Bool in byte == 0 }, "nothing said while muted leaves")
    }
}
