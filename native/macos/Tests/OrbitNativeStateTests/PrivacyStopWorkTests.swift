import Foundation
import XCTest
@testable import OrbitNativeState

/// A stop that repeats itself is not idempotent noise: it cancels permission
/// dialogs and answers the daemon with a failure that provokes another stop.
final class PrivacyStopWorkTests: XCTestCase {
    private func work(microphoneSlot: CaptureSlot = .idle,
                      systemAudioSlot: CaptureSlot = .idle, playbackLive: Bool = false,
                      cleanupInFlight: Int = 0) -> PrivacyStopWork {
        PrivacyStopWork(latchAlreadyStopped: true, microphoneSlot: microphoneSlot,
                        systemAudioSlot: systemAudioSlot, playbackLive: playbackLive,
                        cleanupInFlight: cleanupInFlight)
    }

    func testAnUnconfirmedDrainMakesTheStopRedundantAndReportsItsPreviousOutcome() {
        let value = work(microphoneSlot: .unconfirmed(reason: "audio_drain_incomplete"))
        XCTAssertTrue(value.isRedundant)
        XCTAssertTrue(value.reportsDrained)
        XCTAssertEqual(value.drain, "unconfirmed_previously")
        XCTAssertFalse(value.microphoneLive)
        XCTAssertFalse(value.systemAudioLive)
    }

    func testAnyLiveCaptureRunsTheFullStop() {
        // Failing open here would leave a microphone recording, so every doubt
        // resolves towards doing the work.
        XCTAssertFalse(work(microphoneSlot: .active).isRedundant)
        XCTAssertFalse(work(systemAudioSlot: .active).isRedundant)
        XCTAssertFalse(work(playbackLive: true).isRedundant)
        XCTAssertFalse(work(cleanupInFlight: 1).isRedundant)
    }

    func testARedundantStopReportsItsDrainAsCompleteRatherThanFailed() {
        // Reporting false is what made the daemon treat the answer as a failure
        // and ask for yet another stop, once every couple of seconds.
        XCTAssertTrue(work().isRedundant)
        XCTAssertTrue(work().reportsDrained)
        XCTAssertFalse(work(microphoneSlot: .active).reportsDrained)
    }
}
