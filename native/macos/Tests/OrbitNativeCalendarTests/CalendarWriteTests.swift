import Foundation
import Synchronization
import XCTest
@testable import OrbitNativeProtocol
@testable import OrbitNativeCalendar

// The controller's writes against a synthetic backend —
// create in the default calendar, move an occurrence with `this` and with
// `future`, delete, and refuse a read-only calendar and an event organized by
// someone else, by cause; every save answers with the resulting occurrence.
// No EKEventStore, TCC or account: EventKit's own behaviour is checked on the Mac.

private struct WriteFacts: Sendable {
    var now: UInt64 = 100
    var generation: UInt64 = 1
    var stopped = false
    var permission = CalendarAuthorization.fullAccess
}

private final class WriteFactBox: Sendable {
    private let value = Mutex(WriteFacts())
    func read() -> WriteFacts { value.withLock { $0 } }
    func update(_ change: (inout WriteFacts) -> Void) { value.withLock { change(&$0) } }
}

private let hour: Int64 = 3_600_000
/// Wednesday 14 October 2026, 10:00 UTC.
private let start: Int64 = 1_792_058_400_000

private func reference(_ id: String) -> CalendarReference {
    CalendarReference(calendarID: id, sourceID: "account")
}

/// A synthetic store: calendars, occurrences and every commit asked of it.
private actor FakeWriteBackend: CalendarBackend {
    let calendars: [String: CalendarWriteCalendar]
    let defaultCalendarID: String?
    var events: [CalendarOccurrenceTarget: CalendarStoredOccurrence]
    private(set) var commits = [String]()
    var held = false
    var reply: CheckedContinuation<Void, Never>?
    let started: @Sendable () -> Void

    init(defaultCalendarID: String? = "home", events: [CalendarOccurrenceTarget: CalendarStoredOccurrence] = [:],
         held: Bool = false, started: @escaping @Sendable () -> Void = {}) {
        calendars = [
            "home": CalendarWriteCalendar(reference: reference("home"), canEdit: true),
            "work": CalendarWriteCalendar(reference: reference("work"), canEdit: true),
            "birthdays": CalendarWriteCalendar(reference: reference("birthdays"), canEdit: false),
        ]
        self.defaultCalendarID = defaultCalendarID; self.events = events
        self.held = held; self.started = started
    }

    func requestAccess(gate: CalendarReadGate) throws -> CalendarAuthorization { .fullAccess }
    func sources(gate: CalendarReadGate) throws -> [CalendarEntry] { [] }
    func read(query: CalendarReadQuery, gate: CalendarReadGate) throws -> CalendarEventBatch {
        CalendarEventBatch(systemTimezone: "UTC", events: [])
    }

    func writableCalendar(_ id: String?, gate: CalendarReadGate) throws -> CalendarWriteCalendar {
        try gate.check()
        guard let id = id ?? defaultCalendarID else {
            throw CalendarWriteValidation.refusal("calendar_id", .noDefaultCalendar)
        }
        guard let calendar = calendars[id] else {
            throw CalendarWriteValidation.refusal("calendar_id", .unknownCalendar)
        }
        return calendar
    }

    func storedOccurrence(_ target: CalendarOccurrenceTarget, gate: CalendarReadGate) throws
        -> CalendarStoredOccurrence? {
        try gate.check()
        return events[target]
    }

    func create(_ draft: CalendarEventDraft, in calendar: CalendarWriteCalendar,
                gate: CalendarReadGate) async throws -> CalendarWriteOutcome {
        try gate.check()
        if held { started(); await withCheckedContinuation { reply = $0 } }
        commits.append("create:\(calendar.reference.calendar_id)")
        return try outcome(item: "new-item", calendar: calendar.reference, start: draft.start_ms,
            end: draft.end_ms, title: draft.title)
    }

    func change(_ change: CalendarEventChange, gate: CalendarReadGate) throws -> CalendarWriteOutcome {
        try gate.check()
        guard let stored = events[change.target] else { throw CalendarWriteRefusal.eventNotFound(change.target) }
        commits.append("change:\(change.calendar_item_id):\(change.span.rawValue)")
        return try outcome(item: change.calendar_item_id, calendar: stored.calendar,
            start: change.changes.start_ms ?? stored.start, end: change.changes.end_ms ?? stored.end,
            title: change.changes.title ?? "Revisión", original: change.occurrence_start_ms)
    }

    func delete(_ request: CalendarDeleteRequest, gate: CalendarReadGate) throws {
        try gate.check()
        commits.append("delete:\(request.calendar_item_id):\(request.span.rawValue)")
        events[request.target] = nil
    }

    func release() { held = false; let complete = reply; reply = nil; complete?.resume() }

    private func outcome(item: String, calendar: CalendarReference, start: Int64, end: Int64,
                         title: String, original: Int64? = nil) throws -> CalendarWriteOutcome {
        let occurrence = try CalendarOccurrence(id: UUID().uuidString.lowercased(), calendar: calendar,
            calendarItemID: item, eventID: nil, originalOccurrence: original ?? start, start: start, end: end,
            eventTimezone: "UTC", interpretedTimezone: "UTC", allDay: false, detached: original != nil,
            title: title, status: "confirmed", lastModified: nil, invitees: nil, inviteesStatus: "not_provided",
            eventURL: nil, eventURLStatus: "not_provided", availability: .busy, canEdit: true)
        return CalendarWriteOutcome(occurrence: occurrence, systemTimezone: "UTC")
    }
}

private let mine = CalendarOccurrenceTarget(calendarItemID: "series-1", occurrenceStart: start)
private let invitation = CalendarOccurrenceTarget(calendarItemID: "invitation-1", occurrenceStart: start)
private let birthday = CalendarOccurrenceTarget(calendarItemID: "birthday-1", occurrenceStart: start)

private func stored() -> [CalendarOccurrenceTarget: CalendarStoredOccurrence] {
    [
        mine: CalendarStoredOccurrence(calendar: reference("work"), start: start, end: start + hour,
            calendarCanEdit: true, organizedBySomeoneElse: false),
        invitation: CalendarStoredOccurrence(calendar: reference("work"), start: start, end: start + hour,
            calendarCanEdit: true, organizedBySomeoneElse: true),
        birthday: CalendarStoredOccurrence(calendar: reference("birthdays"), start: start, end: start + hour,
            calendarCanEdit: false, organizedBySomeoneElse: false),
    ]
}

@MainActor final class CalendarWriteTests: XCTestCase {
    private let host = "00000000-0000-4000-8000-000000000001"

    private func controller(_ backend: FakeWriteBackend, facts: WriteFactBox = WriteFactBox()) -> CalendarController {
        CalendarController(nativeInstanceID: host, backend: backend,
            clock: { facts.read().now }, unixClock: { 1000 }, authorization: { facts.read().permission },
            host: { let state = facts.read(); return (state.generation, state.stopped) }, observesChanges: false)
    }

    private func move(_ target: CalendarOccurrenceTarget, span: CalendarSpan,
                      changes: CalendarEventChanges = CalendarEventChanges(start: start + hour, end: start + 2 * hour))
        -> CalendarOperation {
        .save(CalendarSaveRequest(change: CalendarEventChange(calendarItemID: target.calendarItemID,
            occurrenceStart: target.occurrenceStart, span: span, changes: changes)))
    }

    private func refusal(_ operation: CalendarOperation, on value: CalendarController) async -> CalendarWriteRefusal? {
        do { _ = try await value.perform(operation); return nil }
        catch { return error as? CalendarWriteRefusal }
    }

    func testCreateGoesToTheDefaultCalendarAndAnswersWithTheOccurrence() async throws {
        let backend = FakeWriteBackend()
        let draft = CalendarEventDraft(title: "Café con Ada", start: start, end: start + hour)
        guard case .saved(let saved) = try await controller(backend).perform(.save(CalendarSaveRequest(create: draft))) else {
            return XCTFail("a create must answer with the saved occurrence")
        }
        XCTAssertEqual(saved.occurrence.calendar.calendar_id, "home", "no calendar given: the default one")
        XCTAssertEqual(saved.occurrence.title, "Café con Ada")
        XCTAssertEqual(saved.occurrence.start_unix_ms, start)
        XCTAssertEqual(saved.native_instance_id, host)
        let commits = await backend.commits
        XCTAssertEqual(commits, ["create:home"])
    }

    func testMovingAnOccurrenceWithThisAndWithFutureAnswersWithItWhereItIsNow() async throws {
        for span in [CalendarSpan.thisEvent, .futureEvents] {
            let backend = FakeWriteBackend(events: stored())
            guard case .saved(let saved) = try await controller(backend).perform(move(mine, span: span)) else {
                return XCTFail("a move must answer with the occurrence")
            }
            XCTAssertEqual(saved.occurrence.start_unix_ms, start + hour)
            XCTAssertEqual(saved.occurrence.end_unix_ms, start + 2 * hour)
            XCTAssertEqual(saved.occurrence.calendar_item_id, "series-1")
            XCTAssertEqual(saved.occurrence.original_occurrence_unix_ms, start)
            let commits = await backend.commits
            XCTAssertEqual(commits, ["change:series-1:\(span.rawValue)"])
        }
    }

    func testDeleteRemovesTheOccurrenceAndSaysWhich() async throws {
        let backend = FakeWriteBackend(events: stored())
        let request = CalendarDeleteRequest(calendarItemID: "series-1", occurrenceStart: start, span: .futureEvents)
        guard case .deleted(let deleted) = try await controller(backend).perform(.delete(request)) else {
            return XCTFail("a delete must answer with what it removed")
        }
        XCTAssertEqual(deleted.calendar_item_id, "series-1")
        XCTAssertEqual(deleted.occurrence_start_ms, start)
        XCTAssertEqual(deleted.span, .futureEvents)
        let commits = await backend.commits
        XCTAssertEqual(commits, ["delete:series-1:future"])
    }

    func testAReadOnlyCalendarIsRefusedByItsCauseBeforeAnyCommit() async {
        let backend = FakeWriteBackend(events: stored())
        let value = controller(backend)
        let create = CalendarEventDraft(calendarID: "birthdays", title: "Fiesta", start: start, end: start + hour)
        let refused = await refusal(.save(CalendarSaveRequest(create: create)), on: value)
        XCTAssertEqual(refused, .notEditable(.calendarReadOnly, calendarID: "birthdays"))
        let moved = await refusal(move(birthday, span: .thisEvent), on: value)
        XCTAssertEqual(moved, .notEditable(.calendarReadOnly, calendarID: "birthdays"))
        let commits = await backend.commits
        XCTAssertEqual(commits, [])
    }

    func testAnEventSomeoneElseOrganizedIsNotMovedOrDeleted() async {
        let backend = FakeWriteBackend(events: stored())
        let value = controller(backend)
        let moved = await refusal(move(invitation, span: .thisEvent), on: value)
        XCTAssertEqual(moved, .notEditable(.organizedBySomeoneElse, calendarID: "work"))
        let deleted = await refusal(.delete(CalendarDeleteRequest(calendarItemID: "invitation-1",
            occurrenceStart: start, span: .thisEvent)), on: value)
        XCTAssertEqual(deleted, .notEditable(.organizedBySomeoneElse, calendarID: "work"))
        XCTAssertEqual(deleted?.code, "calendar_not_editable")
        XCTAssertEqual(deleted?.detail["reason"] as? String, "organized_by_someone_else")
        let commits = await backend.commits
        XCTAssertEqual(commits, [])
    }

    func testAMissingOccurrenceIsRefusedWithItsEventAndDate() async {
        let backend = FakeWriteBackend(events: stored())
        let gone = CalendarOccurrenceTarget(calendarItemID: "series-1", occurrenceStart: start + 7 * 24 * hour)
        let refused = await refusal(move(gone, span: .thisEvent), on: controller(backend))
        XCTAssertEqual(refused, .eventNotFound(gone))
        XCTAssertEqual(refused?.detail["occurrence_start_ms"] as? Int64, start + 7 * 24 * hour)
    }

    func testMovingOnlyTheEndBeforeTheStartIsRefusedWithBothDates() async {
        let backend = FakeWriteBackend(events: stored())
        let refused = await refusal(move(mine, span: .thisEvent, changes: CalendarEventChanges(end: start - hour)),
                                    on: controller(backend))
        guard case .invalidChange(let change) = refused else { return XCTFail("\(String(describing: refused))") }
        XCTAssertEqual(change.problem, .endBeforeStart)
        XCTAssertEqual(change.startMs, start)
        XCTAssertEqual(change.endMs, start - hour)
        let commits = await backend.commits
        XCTAssertEqual(commits, [])
    }

    func testAWriteWithoutFullAccessNamesEventKitsState() async {
        let backend = FakeWriteBackend(events: stored())
        let facts = WriteFactBox(); facts.update { $0.permission = .writeOnly }
        let refused = await refusal(move(mine, span: .thisEvent), on: controller(backend, facts: facts))
        XCTAssertEqual(refused, .permissionDenied(.writeOnly))
        XCTAssertEqual(refused?.detail["authorization"] as? String, "write_only")
    }

    /// A write EventKit has not returned from by its deadline may be applied:
    /// it is answered as unknown, never as expired, and the slot stays held
    /// until EventKit returns.
    func testAWriteThatOutlivesItsDeadlineIsAnUnknownOutcome() async throws {
        let started = expectation(description: "synthetic commit running")
        let backend = FakeWriteBackend(held: true, started: { started.fulfill() })
        let facts = WriteFactBox()
        let value = controller(backend, facts: facts)
        let draft = CalendarEventDraft(title: "Café", start: start, end: start + hour)
        let request = Task { try await value.perform(.save(CalendarSaveRequest(create: draft))) }
        await fulfillment(of: [started], timeout: 1)
        facts.update { $0.now += CalendarLimits.writeLifetimeNanoseconds }
        XCTAssertTrue(value.status().physical_read_pending) // Runs the real expiry path.
        if case .failure(let error) = await request.result {
            XCTAssertEqual(error as? CalendarWriteRefusal, .outcomeUnknown(cause: "write_deadline"))
        } else { XCTFail("a late write was answered as done") }
        await backend.release()
    }

    /// Retiring a write in flight (a stop, the daemon's disconnect) cannot say
    /// it did not happen: its answer is an unknown outcome, by cause.
    func testRetiringAWriteInFlightIsAnUnknownOutcome() async throws {
        let started = expectation(description: "synthetic commit running")
        let backend = FakeWriteBackend(held: true, started: { started.fulfill() })
        let value = controller(backend)
        let draft = CalendarEventDraft(title: "Café", start: start, end: start + hour)
        let request = Task { try await value.perform(.save(CalendarSaveRequest(create: draft))) }
        await fulfillment(of: [started], timeout: 1)
        value.disconnect()
        if case .failure(let error) = await request.result {
            XCTAssertEqual(error as? CalendarWriteRefusal, .outcomeUnknown(cause: "calendar_invalidated"))
        } else { XCTFail("a retired write was answered as done") }
        await backend.release()
    }

    /// A write that fails resets EventKit's shared store, so the event changed
    /// in memory is not shown by a later read or applied by the next commit;
    /// the failure itself goes on unchanged. (The reset of a real
    /// EKEventStore is checked on the Mac; this checks it is always called.)
    func testAFailedWriteResetsTheSharedStoreAndKeepsItsCause() throws {
        var resets = 0
        XCTAssertThrowsError(try CalendarStoreTransaction.run(reset: { resets += 1 }) {
            throw CalendarWriteRefusal.saveFailed(domain: "EKErrorDomain", code: 12)
        }) { XCTAssertEqual($0 as? CalendarWriteRefusal, .saveFailed(domain: "EKErrorDomain", code: 12)) }
        XCTAssertEqual(resets, 1)
        XCTAssertEqual(try CalendarStoreTransaction.run(reset: { resets += 1 }) { 7 }, 7)
        XCTAssertEqual(resets, 1, "a write that succeeds keeps the store as it is")
    }

    /// A stop after the commit withholds the answer, and says why.
    func testWithheldWriteAnswersNameTheirCause() async throws {
        let backend = FakeWriteBackend(events: stored())
        let facts = WriteFactBox()
        let value = controller(backend, facts: facts)
        let result = try await value.perform(move(mine, span: .thisEvent))
        XCTAssertNil(value.writeDeliveryRefusal(result))
        facts.update { $0.permission = .denied }
        XCTAssertEqual(value.writeDeliveryRefusal(result), "calendar_permission_required")
        facts.update { $0.permission = .fullAccess; $0.generation += 1 }
        XCTAssertEqual(value.writeDeliveryRefusal(result), "privacy_generation_changed")
        facts.update { $0.stopped = true }
        XCTAssertEqual(value.writeDeliveryRefusal(result), "privacy_stopped")
    }
}
