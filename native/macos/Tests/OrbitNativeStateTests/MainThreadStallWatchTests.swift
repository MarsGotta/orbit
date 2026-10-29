import Foundation
import XCTest
@testable import OrbitNativeState

/// The main thread's holds are measured from a background queue, with the
/// command it had started and whether a capture was live; never content.
final class MainThreadStallWatchTests: XCTestCase {
    func testIdleWatchPostsNoPingsAndAnOperationRearmsIt() {
        var watch = MainThreadStallWatch()
        watch.activityChanged(false)
        XCTAssertFalse(watch.needsTimer)
        XCTAssertFalse(watch.timerFired(at: 1).postPing)
        watch.activityChanged(true)
        XCTAssertTrue(watch.needsTimer)
        XCTAssertTrue(watch.timerFired(at: 2).postPing)
        // Finishing the operation retains the outstanding ping and its report.
        watch.activityChanged(false)
        XCTAssertTrue(watch.needsTimer)
        XCTAssertNotNil(watch.pingAnswered(at: 2.6))
        XCTAssertFalse(watch.needsTimer)
        XCTAssertFalse(watch.timerFired(at: 3).postPing)
    }

    func testCommandsWaitingForMainKeepTheWatchArmedUntilDelivered() {
        var watch = MainThreadStallWatch()
        watch.activityChanged(false)
        watch.commandPending(id: "first", queued: true)
        watch.commandPending(id: "second", queued: true)
        watch.commandPending(id: "first", queued: false)
        XCTAssertTrue(watch.needsTimer)
        XCTAssertTrue(watch.timerFired(at: 1).postPing)
        watch.commandPending(id: "second", queued: false)
        XCTAssertNotNil(watch.pingAnswered(at: 2))
        XCTAssertFalse(watch.needsTimer)
    }

    func testAPromptAnswerIsNotAStall() {
        var watch = MainThreadStallWatch()
        XCTAssertTrue(watch.timerFired(at: 10).postPing)
        XCTAssertNil(watch.pingAnswered(at: 10.05))
        // The next tick posts a new ping.
        XCTAssertTrue(watch.timerFired(at: 10.1).postPing)
    }

    func testAPingAnsweredLateIsAStallWithTheLastCommand() {
        var watch = MainThreadStallWatch()
        watch.captureChanged(true)
        watch.commandStarted("audio_start")
        XCTAssertTrue(watch.timerFired(at: 1).postPing)
        let stall = watch.pingAnswered(at: 1.4)
        XCTAssertEqual(stall, MainThreadStall(stalledMs: 400, ongoing: false, recording: true,
                                              lastCommand: "audio_start"))
        XCTAssertEqual(stall?.fields, ["stalled_ms": "400", "ongoing": "false",
                                       "recording": "true", "last_command": "audio_start"])
    }

    func testOnlyOnePingIsOutstanding() {
        var watch = MainThreadStallWatch()
        XCTAssertTrue(watch.timerFired(at: 0).postPing)
        // The main thread is held: the timer keeps firing, no ping piles up.
        for tick in 1...15 {
            XCTAssertFalse(watch.timerFired(at: Double(tick) * 0.1).postPing)
        }
        XCTAssertNotNil(watch.pingAnswered(at: 1.6))
        XCTAssertTrue(watch.timerFired(at: 1.7).postPing)
    }

    func testAHoldPastTheBeatPeriodIsSaidOnceWhileItLastsAndAgainWhenItEnds() {
        // The 11.3 s of 2026-10-22, had the helper been the one held.
        var watch = MainThreadStallWatch()
        watch.commandStarted("calendar_snapshot")
        XCTAssertTrue(watch.timerFired(at: 0).postPing)
        var ongoing = [MainThreadStall]()
        var now = 0.1
        while now < 11.3 {
            let decision = watch.timerFired(at: now)
            XCTAssertFalse(decision.postPing)
            if let stall = decision.ongoing { ongoing.append(stall) }
            now += 0.1
        }
        XCTAssertEqual(ongoing.count, 1)
        XCTAssertEqual(ongoing.first?.ongoing, true)
        // Said on the first tick past the mark; the tenths the loop adds can
        // round that to the mark itself.
        XCTAssertGreaterThanOrEqual(ongoing.first?.stalledMs ?? 0, Int(MainThreadStallWatch.ongoingMark * 1000))
        let ended = watch.pingAnswered(at: 11.3)
        XCTAssertEqual(ended?.ongoing, false)
        XCTAssertEqual(ended?.stalledMs, 11_300)
        XCTAssertEqual(ended?.lastCommand, "calendar_snapshot")
        // The next long hold is said again while it lasts.
        XCTAssertTrue(watch.timerFired(at: 11.4).postPing)
        XCTAssertEqual(watch.timerFired(at: 14).ongoing?.ongoing, true)
    }

    func testAnAnswerWithoutAPingIsIgnored() {
        var watch = MainThreadStallWatch()
        XCTAssertNil(watch.pingAnswered(at: 5))
    }

    func testOnlyWireNamesReachTheLog() {
        var watch = MainThreadStallWatch()
        XCTAssertEqual(watch.lastCommand, "none")
        for kind in ["Hola, esto es una frase", "", "audio\nstart", String(repeating: "a", count: 41)] {
            watch.commandStarted(kind)
            XCTAssertEqual(watch.lastCommand, "unrecognized", kind)
        }
        watch.commandStarted("playback_stop")
        XCTAssertEqual(watch.lastCommand, "playback_stop")
    }

    func testTheThresholdIsAppleHangLength() {
        var watch = MainThreadStallWatch()
        XCTAssertTrue(watch.timerFired(at: 0).postPing)
        XCTAssertNil(watch.pingAnswered(at: MainThreadStallWatch.threshold))
        XCTAssertTrue(watch.timerFired(at: 1).postPing)
        XCTAssertNotNil(watch.pingAnswered(at: 1 + MainThreadStallWatch.threshold + 0.001))
    }

    /// The detector itself, against a serial queue standing in for the main
    /// one: holding that queue is reported from the watcher's own queue.
    func testTheDetectorReportsAHeldQueueFromItsOwnQueue() {
        let watched = DispatchQueue(label: "orbit.tests.watched-main")
        let reported = expectation(description: "the hold was reported")
        let stalls = StallBox()
        let detector = MainThreadHangDetector(target: watched, report: { stall in
            if stalls.recordFirstEnded(stall) { reported.fulfill() }
        })
        detector.commandStarted("screenshot")
        detector.start()
        // Let the first pings be answered, then hold the queue for 600 ms.
        watched.asyncAfter(deadline: .now() + 0.3) { Thread.sleep(forTimeInterval: 0.6) }
        // An event, not a clock, decides: the wait only bounds a hang.
        wait(for: [reported], timeout: 10)
        detector.stop()
        let stall = stalls.first
        XCTAssertEqual(stall?.lastCommand, "screenshot")
        XCTAssertEqual(stall?.recording, false)
        XCTAssertGreaterThan(stall?.stalledMs ?? 0, Int(MainThreadStallWatch.threshold * 1000))
    }
}

/// The first hold that ended, kept once: later reports, after the test has
/// stopped waiting, change nothing.
private final class StallBox: @unchecked Sendable {
    private let lock = NSLock()
    private var stall: MainThreadStall?

    func recordFirstEnded(_ candidate: MainThreadStall) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard stall == nil, !candidate.ongoing else { return false }
        stall = candidate
        return true
    }

    var first: MainThreadStall? { lock.lock(); defer { lock.unlock() }; return stall }
}
