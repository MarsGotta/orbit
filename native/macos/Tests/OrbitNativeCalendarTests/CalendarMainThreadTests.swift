import Darwin
import EventKit
import Foundation
import Synchronization
import XCTest
import OrbitNativeProtocol
@testable import OrbitNativeCalendar

// The main thread reads the daemon's heartbeat and starts and stops audio.
// On 2026-10-22 a meeting was cut in the same second as a calendar
// invalidation, while every step of the calendar controller ran there. These
// tests hold EventKit's calls for seconds and measure how long the main queue
// takes to run a block meanwhile. No EKEventStore, TCC or account: the slow
// calls are synthetic stand-ins, and EventKit's own timing is the Mac's.

/// How long each synthetic EventKit call holds its thread. The real ones are
/// synchronous: `enumerateEvents` and the TCC preflight behind
/// `authorizationStatus`, which a busy `tccd` can hold.
private let readHold: TimeInterval = 1.5
private let permissionHold: TimeInterval = 1.0

/// The longest the main queue may take to run a block while those calls are
/// held: half the shortest hold. A main thread that ran either call would
/// take the whole hold; one that ran neither answers in milliseconds.
private let mainQueueBound: TimeInterval = permissionHold / 2

/// Where each synthetic EventKit call ran.
private final class Threads: Sendable {
    private let onMain = Mutex<[String]>([])
    private let calls = Mutex(0)
    func record(_ name: String) {
        calls.withLock { $0 += 1 }
        if pthread_main_np() != 0 { onMain.withLock { $0.append(name) } }
    }
    var mainCalls: [String] { onMain.withLock { $0 } }
    var count: Int { calls.withLock { $0 } }
}

/// EventKit as a blocking SDK: each call holds the thread it runs on.
private actor SlowBackend: CalendarBackend {
    let threads: Threads
    var held: CheckedContinuation<Void, Never>?
    let holdsUntilReleased: Bool
    let entered: @Sendable () -> Void
    init(threads: Threads, holdsUntilReleased: Bool = false, entered: @escaping @Sendable () -> Void = {}) {
        self.threads = threads; self.holdsUntilReleased = holdsUntilReleased; self.entered = entered
    }
    func requestAccess(gate: CalendarReadGate) throws -> CalendarAuthorization { .fullAccess }
    func sources(gate: CalendarReadGate) async throws -> [CalendarEntry] {
        threads.record("sources"); entered()
        if holdsUntilReleased { await withCheckedContinuation { held = $0 } }
        try gate.check()
        return []
    }
    func read(query: CalendarReadQuery, gate: CalendarReadGate) throws -> CalendarEventBatch {
        threads.record("read")
        Thread.sleep(forTimeInterval: readHold)
        try gate.check()
        return CalendarEventBatch(systemTimezone: "UTC", events: [])
    }
    func release() { let value = held; held = nil; value?.resume() }
    func writableCalendar(_ id: String?, gate: CalendarReadGate) throws -> CalendarWriteCalendar {
        throw CalendarFailure.invalidRequest
    }
    func storedOccurrence(_ target: CalendarOccurrenceTarget, gate: CalendarReadGate) throws
        -> CalendarStoredOccurrence? { nil }
    func create(_ draft: CalendarEventDraft, in calendar: CalendarWriteCalendar,
                gate: CalendarReadGate) throws -> CalendarWriteOutcome { throw CalendarFailure.invalidRequest }
    func change(_ change: CalendarEventChange, gate: CalendarReadGate) throws -> CalendarWriteOutcome {
        throw CalendarFailure.invalidRequest
    }
    func delete(_ request: CalendarDeleteRequest, gate: CalendarReadGate) throws {
        throw CalendarFailure.invalidRequest
    }
}

/// The first permission check holds its thread, as a busy `tccd` would.
private final class SlowPermission: Sendable {
    private let first = Mutex(true)
    let threads: Threads
    init(threads: Threads) { self.threads = threads }
    func authorization() -> CalendarAuthorization {
        threads.record("authorization")
        if first.withLock({ value in defer { value = false }; return value }) {
            Thread.sleep(forTimeInterval: permissionHold)
        }
        return .fullAccess
    }
}

private func now() -> UInt64 { DispatchTime.now().uptimeNanoseconds }

/// One value shared with the closures the controller keeps.
private final class Box<Value: Sendable>: Sendable {
    private let value: Mutex<Value>
    init(_ value: Value) { self.value = Mutex(value) }
    func get() -> Value { value.withLock { $0 } }
    func set(_ new: Value) { value.withLock { $0 = new } }
}

/// How long the main queue takes to run an empty block, sampled until `until`.
private func slowestMainQueueTurn(until done: @escaping @Sendable () -> Bool) async -> TimeInterval {
    var slowest: TimeInterval = 0
    repeat {
        let started = Date()
        await MainActor.run {}
        slowest = max(slowest, Date().timeIntervalSince(started))
        try? await Task.sleep(nanoseconds: 20_000_000)
    } while !done()
    return slowest
}

final class CalendarMainThreadTests: XCTestCase {
    private let host = "00000000-0000-4000-8000-000000000001"
    private let window = CalendarReadQuery(start: 0, end: 31 * 86_400_000)

    /// A read whose EventKit calls take seconds never holds the main queue,
    /// and the main thread reads the calendar's status without asking EventKit.
    func testASlowEventKitCallNeverHoldsTheMainQueue() async throws {
        let threads = Threads()
        let permission = SlowPermission(threads: threads)
        let controller = CalendarController(nativeInstanceID: host, backend: SlowBackend(threads: threads),
            clock: { now() }, unixClock: { 1_000 }, authorization: { permission.authorization() },
            host: { (1, false) }, observesChanges: false)
        let finished = Box(false)
        let window = self.window
        let read = Task.detached { () -> Result<CalendarResult, Error> in
            defer { finished.set(true) }
            do { return .success(try await controller.perform(.read(window))) }
            catch { return .failure(error) }
        }
        let statusOnMain = await MainActor.run { () -> TimeInterval in
            let started = Date()
            _ = controller.lastStatus()
            return Date().timeIntervalSince(started)
        }
        let slowest = await slowestMainQueueTurn(until: { finished.get() })
        guard case .success(.page(let page)) = await read.value else {
            return XCTFail("the slow read must still answer with its page")
        }
        XCTAssertEqual(page.events, [])
        XCTAssertLessThan(slowest, mainQueueBound,
            "the main queue waited \(slowest) s while EventKit held the read")
        XCTAssertLessThan(statusOnMain, mainQueueBound, "the status read on Main waited for EventKit")
        XCTAssertGreaterThanOrEqual(threads.count, 2, "both slow calls ran")
        XCTAssertEqual(threads.mainCalls, [], "an EventKit call ran on the main thread")
    }

    /// EventKit's `EKEventStoreChanged` is handled off the main thread: the
    /// read in flight is retired and the daemon told, from the controller.
    func testAStoreChangeIsHandledOffTheMainThread() async throws {
        let threads = Threads()
        let entered = expectation(description: "the read is in EventKit")
        let told = expectation(description: "the daemon is told the store changed")
        let toldOnMain = Box<Bool?>(nil)
        let backend = SlowBackend(threads: threads, holdsUntilReleased: true, entered: { entered.fulfill() })
        let controller = CalendarController(nativeInstanceID: host, backend: backend,
            clock: { now() }, unixClock: { 1_000 }, authorization: { .fullAccess },
            host: { (1, false) },
            invalidated: { reason in
                XCTAssertEqual(reason, .invalidated)
                toldOnMain.set(pthread_main_np() != 0)
                told.fulfill()
            })
        let read = Task.detached { try await controller.perform(.sources(afterIndex: 0)) }
        await fulfillment(of: [entered], timeout: 2)
        await MainActor.run { NotificationCenter.default.post(name: .EKEventStoreChanged, object: nil) }
        await fulfillment(of: [told], timeout: 2)
        if case .failure(let error) = await read.result {
            XCTAssertEqual(error as? CalendarFailure, .invalidated)
        } else { XCTFail("a read the store changed under was delivered") }
        XCTAssertEqual(toldOnMain.get(), false, "the store change was handled on the main thread")
        await backend.release()
    }
}
