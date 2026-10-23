import Darwin
import Foundation
import Synchronization
import XCTest
import OrbitNativeProtocol
import OrbitNativeCalendar
import OrbitNativeTransport
@testable import OrbitNativeHost

// A calendar command's whole path through the helper — admission on Main,
// EventKit, the paging and checks, the encoding of the page and its reply —
// with EventKit holding its thread for seconds. The main thread, which reads
// the daemon's heartbeat and starts and stops audio, keeps answering.
// Synthetic EventKit only: no EKEventStore, TCC or account.

private let readHold: TimeInterval = 1.5
private let permissionHold: TimeInterval = 1.0
/// Half the shortest hold: a main thread that ran either call would take the
/// whole hold, one that ran neither answers in milliseconds.
private let mainQueueBound: TimeInterval = permissionHold / 2
private let instance = "00000000-0000-4000-8000-000000000001"
private let events = 300

private final class Flag: Sendable {
    private let value = Mutex(false)
    private let onMain = Mutex<[String]>([])
    func set() { value.withLock { $0 = true } }
    var isSet: Bool { value.withLock { $0 } }
    func ran(_ name: String) { if pthread_main_np() != 0 { onMain.withLock { $0.append(name) } } }
    var ranOnMain: [String] { onMain.withLock { $0 } }
}

/// EventKit as a blocking SDK, with a window of ordinary events.
private actor HeldEventKit: CalendarBackend {
    let flag: Flag
    init(flag: Flag) { self.flag = flag }
    func requestAccess(gate: CalendarReadGate) throws -> CalendarAuthorization { .fullAccess }
    func sources(gate: CalendarReadGate) throws -> [CalendarEntry] { [] }
    func read(query: CalendarReadQuery, gate: CalendarReadGate) throws -> CalendarEventBatch {
        flag.ran("read")
        Thread.sleep(forTimeInterval: readHold)
        try gate.check()
        let occurrences = try (0..<events).map { index in
            try CalendarOccurrence(id: String(format: "00000000-0000-4000-8000-%012ld", 1_000 + index),
                calendar: CalendarReference(calendarID: "calendar-\(index % 5)", sourceID: "account"),
                calendarItemID: "item-\(index)", eventID: nil, originalOccurrence: nil,
                start: Int64(3_600_000 + index * 60_000), end: Int64(5_400_000 + index * 60_000),
                eventTimezone: "UTC", interpretedTimezone: "UTC", allDay: false, detached: false,
                title: "Reunión sintética \(index)", status: "confirmed", lastModified: nil,
                invitees: nil, inviteesStatus: "not_provided", eventURL: nil, eventURLStatus: "not_provided")
        }
        return CalendarEventBatch(systemTimezone: "UTC", events: occurrences)
    }
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
private final class HeldPermission: Sendable {
    private let first = Mutex(true)
    let flag: Flag
    init(flag: Flag) { self.flag = flag }
    func authorization() -> CalendarAuthorization {
        flag.ran("authorization")
        if first.withLock({ value in defer { value = false }; return value }) {
            Thread.sleep(forTimeInterval: permissionHold)
        }
        return .fullAccess
    }
}

final class CalendarHostTests: XCTestCase {
    /// The channel's pipes live as long as the test.
    private var pipes = [Pipe]()

    @MainActor
    func testACalendarReadIsAnsweredWithoutHoldingTheMainQueue() async throws {
        let input = Pipe(), output = Pipe()
        pipes += [input, output]
        let channel = NativeChannel(input: input.fileHandleForReading, output: output.fileHandleForWriting)
        let flag = Flag(), answered = Flag()
        let permission = HeldPermission(flag: flag)
        let controller = CalendarController(nativeInstanceID: instance, backend: HeldEventKit(flag: flag),
            clock: { DispatchTime.now().uptimeNanoseconds }, unixClock: { 1_000 },
            authorization: { permission.authorization() }, host: { (1, false) }, observesChanges: false)
        let host = CalendarHost(controller: controller, channel: channel,
            facts: { CalendarHostFacts(connected: true, generation: 1, stopped: false, latched: false) },
            changed: { answered.set() })
        let command = try NativeWireCommand.decode(JSONSerialization.data(withJSONObject: [
            "v": 1, "id": "calendar-local-1", "command": "calendar_snapshot",
            "calendar_query": ["start_unix_ms": 0, "end_unix_ms": 31 * 86_400_000, "after_index": 0]]))

        // A page of 300 events is larger than a pipe holds: read it while it
        // is written, or the channel's writer waits on a full pipe for ever.
        let reader = output.fileHandleForReading
        let reading = Task.detached { try reader.readToEnd() }
        let admittedAt = Date()
        host.read(command)
        let admission = Date().timeIntervalSince(admittedAt)
        let slowest = await Task.detached { () -> TimeInterval in
            var slowest: TimeInterval = 0
            repeat {
                let started = Date()
                await MainActor.run {}
                slowest = max(slowest, Date().timeIntervalSince(started))
                try? await Task.sleep(nanoseconds: 20_000_000)
            } while !answered.isSet
            return slowest
        }.value

        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        try output.fileHandleForWriting.close()
        let read = try await reading.value
        let bytes = try XCTUnwrap(read)
        let replies = try String(decoding: bytes, as: UTF8.self).split(separator: "\n").map {
            try XCTUnwrap(JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any])
        }
        let reply = try XCTUnwrap(replies.first { $0["id"] as? String == "calendar-local-1" })
        XCTAssertEqual(reply["ok"] as? Bool, true, "\(reply)")
        let result = try XCTUnwrap(reply["result"] as? [String: Any])
        XCTAssertEqual(result["total"] as? Int, events)
        XCTAssertLessThan(slowest, mainQueueBound,
            "the main queue waited \(slowest) s while EventKit held the read")
        XCTAssertLessThan(admission, mainQueueBound, "admitting the command waited for EventKit")
        XCTAssertEqual(flag.ranOnMain, [], "an EventKit call ran on the main thread")
    }
}
