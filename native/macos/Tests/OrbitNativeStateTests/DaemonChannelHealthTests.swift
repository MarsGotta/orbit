import Foundation
import XCTest
@testable import OrbitNativeState

/// The watchdog's silence is the daemon's, never the helper's own stall, and a
/// capture in progress outlives it.
final class DaemonChannelHealthTests: XCTestCase {
    /// Ticks once a second from `start` through `end`, returning the times at
    /// which the channel expired.
    private func tickEverySecond(_ health: inout DaemonChannelHealth,
                                 from start: TimeInterval, through end: TimeInterval) -> [TimeInterval] {
        var expired = [TimeInterval]()
        var now = start
        while now <= end {
            if health.tick(now: now) { expired.append(now) }
            now += 1
        }
        return expired
    }

    func testALateTickIsTheHelpersStallAndNeverExpires() {
        // The main thread was held for ten seconds: the beats sent meanwhile
        // wait in its queue, and the first tick after it must not read them
        // as the daemon's silence.
        var health = DaemonChannelHealth(now: 0)
        XCTAssertFalse(health.tick(now: 1))
        XCTAssertFalse(health.tick(now: 12))
        XCTAssertTrue(health.connected)
        XCTAssertEqual(health.lateTickGap, 11)
        // The beat that waited in the queue is read right after that tick.
        health.observedBeat(at: 12.1)
        XCTAssertEqual(tickEverySecond(&health, from: 13, through: 18), [])
        XCTAssertTrue(health.connected)
        XCTAssertNil(health.lateTickGap)
    }

    func testAStallShorterThanTheTimeoutStillWaitsForTheQueuedBeats() {
        // Last beat read at 1, thread held from 2 to 7: without the late-tick
        // rule the tick at 7 found six seconds of «silence» with beats unread.
        var health = DaemonChannelHealth(now: 0)
        health.observedBeat(at: 1)
        XCTAssertFalse(health.tick(now: 2))
        XCTAssertFalse(health.tick(now: 7.5))
        XCTAssertTrue(health.connected)
    }

    func testAfterALateTickRealSilenceExpiresOnceItHasLastedWhileAwake() {
        var health = DaemonChannelHealth(now: 0)
        XCTAssertFalse(health.tick(now: 1))
        XCTAssertFalse(health.tick(now: 20))
        // No beat follows: the silence counts from 20, the tick after the stall.
        XCTAssertEqual(tickEverySecond(&health, from: 21, through: 40), [27])
        XCTAssertFalse(health.connected)
    }

    func testRealSilenceWhileAwakeExpiresExactlyOnce() {
        var health = DaemonChannelHealth(now: 0)
        health.observedBeat(at: 0)
        XCTAssertEqual(tickEverySecond(&health, from: 1, through: 30), [7])
        XCTAssertFalse(health.connected)
    }

    func testABeatAfterExpiryRevivesTheChannel() {
        var health = DaemonChannelHealth(now: 0)
        XCTAssertEqual(tickEverySecond(&health, from: 1, through: 10), [7])
        health.observedBeat(at: 10.5)
        XCTAssertTrue(health.connected)
        XCTAssertEqual(tickEverySecond(&health, from: 11, through: 16), [])
        // And a second silence is announced again, once.
        XCTAssertEqual(tickEverySecond(&health, from: 17, through: 30), [17])
    }

    func testALateTickDoesNotReviveAChannelWithoutABeat() {
        var health = DaemonChannelHealth(now: 0)
        health.disconnected()
        XCTAssertFalse(health.tick(now: 1))
        XCTAssertFalse(health.tick(now: 20))
        XCTAssertFalse(health.connected)
    }

    func testSilenceNeverStopsACaptureInProgress() {
        // A recording or a voice note keeps capturing through the daemon's
        // silence; the transport keeps what it cannot deliver.
        let live: [(CaptureSlot, CaptureSlot)] = [
            (.active, .idle), (.idle, .active), (.active, .active),
            (.starting, .idle), (.idle, .starting),
        ]
        for (microphone, systemAudio) in live {
            XCTAssertFalse(DaemonChannelHealth.silenceStopsCapture(microphone: microphone,
                                                                    systemAudio: systemAudio),
                           "\(microphone) \(systemAudio)")
        }
    }

    func testSilenceWithNothingCapturedIsHandledAsBefore() {
        let idle: [(CaptureSlot, CaptureSlot)] = [
            (.idle, .idle), (.draining, .idle), (.idle, .unconfirmed(reason: "audio_drain_incomplete")),
        ]
        for (microphone, systemAudio) in idle {
            XCTAssertTrue(DaemonChannelHealth.silenceStopsCapture(microphone: microphone,
                                                                   systemAudio: systemAudio),
                          "\(microphone) \(systemAudio)")
        }
    }
}
