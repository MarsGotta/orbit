import Foundation
import Synchronization
import XCTest
@testable import OrbitNativeState

/// The daemon's liveness is observed where its commands are read and judged
/// on a queue of its own: a held main thread is never the daemon's silence,
/// and a capture in progress is told to go on without the main thread.
final class DaemonLivenessWatchTests: XCTestCase {
    private final class Clock: @unchecked Sendable {
        private let now = Mutex<TimeInterval>(0)
        func read() -> TimeInterval { now.withLock { $0 } }
        func set(_ value: TimeInterval) { now.withLock { $0 = value } }
    }

    private final class Record: @unchecked Sendable {
        let silences = Mutex<[DaemonSilence]>([])
        let gaps = Mutex<[TimeInterval]>([])
    }

    private func watch(_ clock: Clock, _ record: Record) -> DaemonLivenessWatch {
        DaemonLivenessWatch(clock: { clock.read() },
                            fellSilent: { silence in record.silences.withLock { $0.append(silence) } },
                            lateTick: { gap in record.gaps.withLock { $0.append(gap) } })
    }

    /// Ticks once a second from `start` through `end`.
    private func ticks(_ watch: DaemonLivenessWatch, _ clock: Clock,
                       from start: TimeInterval, through end: TimeInterval) {
        var now = start
        while now <= end {
            clock.set(now)
            watch.tick()
            now += 1
        }
    }

    func testABeatObservedOffTheMainThreadKeepsTheChannel() {
        let (clock, record) = (Clock(), Record())
        let watch = watch(clock, record)
        let beat = expectation(description: "beat on another queue")
        DispatchQueue.global().async {
            XCTAssertFalse(Thread.isMainThread)
            clock.set(5)
            watch.observedBeat()
            beat.fulfill()
        }
        wait(for: [beat], timeout: 2)
        ticks(watch, clock, from: 6, through: 10)
        XCTAssertTrue(watch.connected)
        XCTAssertEqual(record.silences.withLock { $0 }, [])
    }

    func testSilenceDuringACaptureIsSaidOnceWithTheCapture() {
        let (clock, record) = (Clock(), Record())
        let watch = watch(clock, record)
        watch.captureChanged(microphone: true, systemAudio: false)
        ticks(watch, clock, from: 1, through: 20)
        XCTAssertFalse(watch.connected)
        let silences = record.silences.withLock { $0 }
        XCTAssertEqual(silences, [DaemonSilence(microphone: true, systemAudio: false)])
        XCTAssertTrue(silences.first?.capturing == true)
    }

    func testSilenceWithNothingCapturedIsSaidAsNotCapturing() {
        let (clock, record) = (Clock(), Record())
        let watch = watch(clock, record)
        ticks(watch, clock, from: 1, through: 10)
        XCTAssertEqual(record.silences.withLock { $0 }.map(\.capturing), [false])
    }

    func testALateTickIsTheHelpersOwnSuspensionNotTheDaemonsSilence() {
        let (clock, record) = (Clock(), Record())
        let watch = watch(clock, record)
        clock.set(1)
        watch.tick()
        clock.set(12)
        watch.tick()
        XCTAssertTrue(watch.connected)
        XCTAssertEqual(record.gaps.withLock { $0 }, [11])
        XCTAssertEqual(record.silences.withLock { $0 }, [])
    }

    func testADisconnectionWaitsForNewTrafficToRevive() {
        let (clock, record) = (Clock(), Record())
        let watch = watch(clock, record)
        watch.disconnected()
        XCTAssertFalse(watch.connected)
        clock.set(2)
        watch.observedBeat()
        XCTAssertTrue(watch.connected)
    }

    func testTheTimerJudgesOnItsOwnQueueWhileTheMainThreadIsHeld() {
        let clock = Clock()
        let fired = DispatchSemaphore(value: 0)
        let onMain = Mutex<Bool?>(nil)
        let watch = DaemonLivenessWatch(
            clock: { clock.read() },
            fellSilent: { _ in
                onMain.withLock { $0 = Thread.isMainThread }
                fired.signal()
            },
            lateTick: { _ in })
        // The daemon's last beat is long past: the first tick finds silence.
        clock.set(100)
        watch.start()
        defer { watch.stop() }
        // This test runs on the main thread and blocks it until the watch has
        // judged: the judgement cannot need main.
        XCTAssertEqual(fired.wait(timeout: .now() + 5), .success)
        XCTAssertEqual(onMain.withLock { $0 }, false)
    }
}
