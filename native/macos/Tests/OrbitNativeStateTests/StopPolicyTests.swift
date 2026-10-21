import Foundation
import XCTest
@testable import OrbitNativeState

/// Only the user's brake writes the durable latch. Everything else stops
/// capture and playback and leaves the latch as it was, so a sleeping Mac or a
/// changed audio route never leaves Orbit stopped after a restart.
final class StopPolicyTests: XCTestCase {
    /// Every reason the helper stops with, as `OrbitApplication.swift` sends them.
    private let automatic = [
        "session_inactive", "audio_device_changed", "voice_processing_state_lost",
        "system_audio_stream_stopped", "daemon_heartbeat_expired", "helper_disconnected",
        "invalid_protocol_or_disconnected", "daemon_output_disconnected", "shutdown",
        "daemon_cut", "a_reason_from_a_newer_daemon",
    ]

    func testOnlyTheUsersBrakeLatches() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = root.appendingPathComponent("contracts/fixtures/native-latching-stops.json")
        let expected = try JSONDecoder().decode(Set<String>.self, from: Data(contentsOf: fixture))
        XCTAssertEqual(StopPolicy.latchingReasons, expected)
        for reason in StopPolicy.latchingReasons {
            XCTAssertTrue(StopPolicy.latches(reason: reason), reason)
            XCTAssertEqual(StopPolicy.eventName(reason: reason), "privacy_stop", reason)
        }
        for reason in automatic {
            XCTAssertFalse(StopPolicy.latches(reason: reason), reason)
            XCTAssertEqual(StopPolicy.eventName(reason: reason), "capture_stopped", reason)
        }
    }

    func testTheDaemonsCutIsNotALatchingReason() {
        XCTAssertFalse(StopPolicy.latches(reason: StopPolicy.daemonCutReason))
    }

    func testACaptureStopWithNothingLiveHasNothingToDoWhateverTheLatchSays() {
        // Closing the lid with nothing recording must not bump the epoch (and
        // cancel a permission dialog) just because the latch is not set.
        let idle = PrivacyStopWork(latchAlreadyStopped: false, microphoneSlot: .idle,
                                   systemAudioSlot: .idle, playbackLive: false,
                                   cleanupInFlight: 0, latches: false)
        XCTAssertTrue(idle.isRedundant)
        XCTAssertTrue(idle.reportsDrained)
    }

    func testACaptureStopStillStopsAnythingLive() {
        // Failing open here would leave a microphone recording.
        for work in [
            PrivacyStopWork(latchAlreadyStopped: false, microphoneSlot: .active, systemAudioSlot: .idle,
                            playbackLive: false, cleanupInFlight: 0, latches: false),
            PrivacyStopWork(latchAlreadyStopped: false, microphoneSlot: .idle, systemAudioSlot: .starting,
                            playbackLive: false, cleanupInFlight: 0, latches: false),
            PrivacyStopWork(latchAlreadyStopped: false, microphoneSlot: .idle, systemAudioSlot: .idle,
                            playbackLive: true, cleanupInFlight: 0, latches: false),
            PrivacyStopWork(latchAlreadyStopped: false, microphoneSlot: .idle, systemAudioSlot: .idle,
                            playbackLive: false, cleanupInFlight: 1, latches: false),
        ] {
            XCTAssertFalse(work.isRedundant)
        }
    }

    func testTheUsersBrakeOnAnUnlatchedMacAlwaysDoesTheWork() {
        // The durable latch is the whole point of the user's stop.
        let work = PrivacyStopWork(latchAlreadyStopped: false, microphoneSlot: .idle,
                                   systemAudioSlot: .idle, playbackLive: false,
                                   cleanupInFlight: 0, latches: true)
        XCTAssertFalse(work.isRedundant)
    }
}
