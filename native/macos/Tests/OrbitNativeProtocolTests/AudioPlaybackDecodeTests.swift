import AVFoundation
import XCTest
@testable import OrbitNativeHost
import OrbitNativeProtocol
import OrbitNativeTransport

/// Synthetic decoder and PCM only: a cancelled decode must never open a device.
final class AudioPlaybackDecodeTests: XCTestCase {
    @MainActor
    func testStopWhileDecodingNeverStartsPlaybackOrAcceptsTheRetiredClip() async throws {
        let input = Pipe()
        let output = Pipe()
        defer { withExtendedLifetime((input, output)) {} }
        let started = expectation(description: "decoder started off main")
        let release = DispatchSemaphore(value: 0)
        let session = SharedAudioSession()
        let playback = AudioPlayback(session: session,
            channel: NativeChannel(input: input.fileHandleForReading, output: output.fileHandleForWriting),
            decodeClip: { _, format in
                XCTAssertFalse(Thread.isMainThread)
                started.fulfill()
                guard release.wait(timeout: .now() + 10) == .success else {
                    throw NativeFailure("fixture_decoder_wait_expired")
                }
                let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 48))
                buffer.frameLength = 48
                return buffer
            })
        let command = try NativeWireCommand.decode(JSONSerialization.data(withJSONObject: [
            "v": 1, "id": "clip", "command": "playback_clip", "playback_id": "answer",
            "mime_type": "audio/wav", "provider": "fixture", "audio_base64": "AA=="]))
        guard case .playbackClip(let wire) = command else { return XCTFail("missing clip") }
        let task = Task { try await playback.enqueueClip(wire.fields)["state"] as? String }
        await fulfillment(of: [started], timeout: 10)
        playback.stop()
        release.signal()
        let result = try await task.value
        XCTAssertEqual(result, "cancelled")
        XCTAssertFalse(playback.playing)
        XCTAssertFalse(session.configured)
        do {
            _ = try await playback.enqueueClip(wire.fields)
            XCTFail("the retired playback id was accepted")
        } catch {
            XCTAssertEqual((error as? NativeFailure)?.code, "stale_or_out_of_order_playback")
        }
    }

    @MainActor
    func testReplacementRetiresTheDecodingClipBeforeItsWorkerReturns() async throws {
        let input = Pipe()
        let output = Pipe()
        defer { withExtendedLifetime((input, output)) {} }
        let firstStarted = expectation(description: "first decoder started")
        let secondStarted = expectation(description: "replacement decoder started")
        let firstRelease = DispatchSemaphore(value: 0)
        let secondRelease = DispatchSemaphore(value: 0)
        let session = SharedAudioSession()
        let playback = AudioPlayback(session: session,
            channel: NativeChannel(input: input.fileHandleForReading, output: output.fileHandleForWriting),
            decodeClip: { clip, format in
                let first = clip.playbackID == "first"
                (first ? firstStarted : secondStarted).fulfill()
                let released = (first ? firstRelease : secondRelease).wait(timeout: .now() + 10)
                guard released == .success else { throw NativeFailure("fixture_decoder_wait_expired") }
                let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 48))
                buffer.frameLength = 48
                return buffer
            })
        func command(_ id: String) throws -> NativePlaybackClipFields {
            let command = try NativeWireCommand.decode(JSONSerialization.data(withJSONObject: [
                "v": 1, "id": id, "command": "playback_clip", "playback_id": id,
                "mime_type": "audio/wav", "provider": "fixture", "audio_base64": "AA=="]))
            guard case .playbackClip(let wire) = command else { throw NativeFailure("fixture_clip_missing") }
            return wire.fields
        }
        let firstFields = try command("first")
        let first = Task { try await playback.enqueueClip(firstFields)["state"] as? String }
        await fulfillment(of: [firstStarted], timeout: 10)
        let secondFields = try command("second")
        let second = Task { try await playback.enqueueClip(secondFields)["state"] as? String }
        await fulfillment(of: [secondStarted], timeout: 10)
        firstRelease.signal()
        let old = try await first.value
        XCTAssertEqual(old, "cancelled")
        XCTAssertFalse(playback.playing)
        XCTAssertFalse(session.configured)
        playback.stop()
        secondRelease.signal()
        let replacement = try await second.value
        XCTAssertEqual(replacement, "cancelled")
        XCTAssertFalse(session.configured)
    }
}
