import XCTest
@testable import OrbitNativeProtocol
import OrbitAudioRing

final class ProtocolTests: XCTestCase {
    func testFragmentedAndCoalescedLines() throws {
        var decoder = LineDecoder()
        XCTAssertEqual(try decoder.append(Data("one\ntw".utf8)), [Data("one".utf8)])
        XCTAssertEqual(try decoder.append(Data("o\n\nthree\n".utf8)), [Data("two".utf8), Data("three".utf8)])
    }

    func testLineWithoutNewlineCannotGrowWithoutBound() {
        var decoder = LineDecoder()
        XCTAssertThrowsError(try decoder.append(Data(repeating: 97, count: NativeProtocol.maximumLineBytes + 1)))
    }

    func testUnsupportedProtocolVersionIsRejected() throws {
        XCTAssertThrowsError(try NativeWireCommand.decode(Data(#"{"v":2,"id":"a","command":"hello"}"#.utf8)))
    }

    func testCancelledPlaybackProgressKeepsItsPublishedNull() throws {
        let event = NativeEvent.makePlaybackProgress(id: "fixture", state: "cancelled", active: false)
        let encoded = try JSONEncoder().encode(event)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        XCTAssertTrue(object["played_frames"] is NSNull)
    }

    func testIdleAudioFrameKeepsItsPublishedPlaybackNull() throws {
        let event = NativeEvent.makeAudio(source: "mic_local", streamID: "fixture",
            sequence: 0, sampleRate: 48_000, frames: 1, monotonicNS: 100,
            pcmBase64: "AAA=", processing: nil,
            playback: (active: false, id: nil))
        let encoded = try JSONEncoder().encode(event)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        XCTAssertTrue(object["playback_id"] is NSNull)
    }

    func testNonFiniteResponseNumberReturnsACatchableFieldError() {
        XCTAssertThrowsError(try NativeEvent.makeResponse(id: "fixture",
            result: ["levels": ["rms_dbfs": Double.nan]])) { error in
            let diagnostic = String(describing: error)
            XCTAssertTrue(diagnostic.contains("response"), diagnostic)
            XCTAssertTrue(diagnostic.contains("result.levels.rms_dbfs"), diagnostic)
            XCTAssertTrue(diagnostic.contains("finite_number"), diagnostic)
        }
    }

    func testResponsePreservesAnUnsignedCounterWithoutRounding() throws {
        let event = try NativeEvent.makeResponse(id: "fixture",
            result: ["privacy_generation": UInt64.max])
        let encoded = try JSONEncoder().encode(event)
        XCTAssertTrue(String(decoding: encoded, as: UTF8.self).contains("18446744073709551615"))
    }

    func testPlaybackFenceRejectsLateAudioAfterBargeIn() {
        var fence = PlaybackFence()
        XCTAssertTrue(fence.accept(id: "first", sequence: 0))
        XCTAssertFalse(fence.accept(id: "first", sequence: 0))
        XCTAssertFalse(fence.accept(id: "first", sequence: 2))
        fence.stop()
        XCTAssertFalse(fence.accept(id: "first", sequence: 1))
        XCTAssertTrue(fence.accept(id: "second", sequence: 0))
        XCTAssertFalse(fence.accept(id: "first", sequence: 0))
        XCTAssertTrue(fence.accept(id: "second", sequence: 1))
    }

    func testRingReportsOverflowAndPreservesSequenceGap() throws {
        let ring = try XCTUnwrap(orbit_ring_create())
        defer { orbit_ring_destroy(ring) }
        let samples: [Float] = [0.5, -0.5]
        samples.withUnsafeBufferPointer { buffer in
            for _ in 0..<32 { XCTAssertTrue(orbit_ring_push_interleaved_timed(ring, buffer.baseAddress, 1, 2, 48_000, 123, 123)) }
            XCTAssertFalse(orbit_ring_push_interleaved_timed(ring, buffer.baseAddress, 1, 2, 48_000, 123, 123))
        }
        XCTAssertEqual(orbit_ring_take_dropped_frames(ring), 2)
        var block = OrbitAudioBlock()
        for expected in 0..<32 {
            XCTAssertTrue(orbit_ring_pop(ring, &block))
            XCTAssertEqual(block.sequence, UInt64(expected))
            XCTAssertEqual(block.frames, 2)
        }
        samples.withUnsafeBufferPointer { buffer in
            XCTAssertTrue(orbit_ring_push_interleaved_timed(ring, buffer.baseAddress, 1, 2, 48_000, 123, 123))
        }
        XCTAssertTrue(orbit_ring_pop(ring, &block))
        XCTAssertEqual(block.sequence, 33)
        orbit_ring_set_enabled(ring, false)
        samples.withUnsafeBufferPointer { buffer in
            XCTAssertFalse(orbit_ring_push_interleaved_timed(ring, buffer.baseAddress, 1, 2, 48_000, 123, 123))
        }
    }
}
