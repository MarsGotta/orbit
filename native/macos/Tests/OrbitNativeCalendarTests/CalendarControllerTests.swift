import Foundation
import Synchronization
import XCTest
import OrbitNativeProtocol
@testable import OrbitNativeCalendar

private struct Facts: Sendable {
    var now: UInt64 = 100
    var generation: UInt64 = 1
    var stopped = false
    var permission = CalendarAuthorization.fullAccess
}

private final class FactBox: Sendable {
    private let value = Mutex(Facts())
    func read() -> Facts { value.withLock { $0 } }
    func update(_ change: (inout Facts) -> Void) { value.withLock { change(&$0) } }
}

private struct FakeCalendarEvent: Sendable {
    let observationID: String
    let calendarID: String
    let calendarItemID: String
    let title: String?
    var start: Int64 = 100
    var invitees = 0
}

private func uuid(_ index: Int) -> String {
    String(format: "00000000-0000-4000-8000-%012ld", index)
}

/// Synthetic actor only. No EKEventStore, TCC, notification subscription or account.
private actor FakeCalendarBackend: CalendarBackend {
    var reads = 0
    var windows = 0
    var permissionRequests = 0
    var contentGetters = 0
    var held: Bool
    var holdsPermission: Bool
    var reply: CheckedContinuation<Void, Never>?
    let started: @Sendable () -> Void
    let failure: CalendarFailure?
    let events: [FakeCalendarEvent]
    let calendars: Int
    init(held: Bool = false, holdsPermission: Bool = false, failure: CalendarFailure? = nil,
         events: [FakeCalendarEvent] = [], calendars: Int = 1,
         started: @escaping @Sendable () -> Void = {}) {
        self.held = held; self.holdsPermission = holdsPermission
        self.failure = failure; self.events = events; self.calendars = calendars; self.started = started
    }
    func requestAccess(gate: CalendarReadGate) async throws -> CalendarAuthorization {
        try gate.check(); permissionRequests += 1
        // Stands in for a TCC dialog waiting on a person, which is the only
        // calendar operation whose duration a human decides.
        if holdsPermission { started(); await withCheckedContinuation { reply = $0 } }
        try gate.check()
        return .fullAccess
    }
    func sources(gate: CalendarReadGate) async throws -> [CalendarEntry] {
        try gate.check(); reads += 1
        if reads == 1 { started() }
        if held { await withCheckedContinuation { reply = $0 } }
        try gate.check() // Real cancellation gate protects the next synthetic getter.
        if let failure { throw failure }
        contentGetters += 1
        // Returned in reverse on purpose: the controller orders the listing.
        return (0..<calendars).reversed().map {
            CalendarEntry(reference: CalendarReference(calendarID: String(format: "calendar-%05ld", $0), sourceID: "account"),
                title: "Calendario \($0)", sourceTitle: "Local", calendarType: "local")
        }
    }
    func read(query: CalendarReadQuery, gate: CalendarReadGate) throws -> CalendarEventBatch {
        try gate.check(); windows += 1
        if let failure { throw failure }
        var occurrences = [CalendarOccurrence]()
        var untitled = 0
        // Returned in reverse on purpose: the controller orders the window.
        for event in events.reversed() {
            guard let title = event.title else { untitled += 1; continue }
            let guests: [CalendarInvitee]? = try event.invitees == 0 ? nil : (0..<event.invitees).map {
                try CalendarInvitee(name: "Invitado \($0)", url: "mailto:invitado\($0)@example.test",
                    role: "required", responseStatus: "accepted", participantType: "person", isCurrentUser: false)
            }
            occurrences.append(try CalendarOccurrence(id: event.observationID,
                calendar: CalendarReference(calendarID: event.calendarID, sourceID: "account"),
                calendarItemID: event.calendarItemID, eventID: nil, originalOccurrence: nil,
                start: event.start, end: event.start + 100, eventTimezone: "UTC", interpretedTimezone: "UTC",
                allDay: false, detached: false, title: title, status: "confirmed",
                lastModified: nil, invitees: guests, inviteesStatus: guests == nil ? "not_provided" : "provided",
                eventURL: nil, eventURLStatus: "not_provided"))
        }
        return CalendarEventBatch(systemTimezone: "UTC", events: occurrences,
            omitted: untitled == 0 ? [] : [.init(reason: "missing_title", count: untitled)])
    }
    func release() { held = false; let complete = reply; reply = nil; complete?.resume() }
    // P9-21: this fake only reads; `CalendarWriteTests` has the writing one.
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

@MainActor final class CalendarControllerTests: XCTestCase {
    private let host = "00000000-0000-4000-8000-000000000001"
    private let window = CalendarReadQuery(start: 0, end: 31 * 86_400_000)
    private func controller(_ backend: FakeCalendarBackend, facts: FactBox) -> CalendarController {
        CalendarController(nativeInstanceID: host, backend: backend,
            clock: { facts.read().now }, unixClock: { 1000 }, authorization: { facts.read().permission },
            host: { let state = facts.read(); return (state.generation, state.stopped) }, observesChanges: false)
    }
    private func settle(_ controller: CalendarController) async throws {
        let deadline = ContinuousClock.now.advanced(by: .seconds(1))
        while controller.status().physical_read_pending && ContinuousClock.now < deadline {
            try await Task.sleep(nanoseconds: 1_000_000)
        }
        XCTAssertFalse(controller.status().physical_read_pending)
    }
    private func page(_ controller: CalendarController, _ query: CalendarReadQuery) async throws -> CalendarReadPage {
        guard case .page(let value) = try await controller.perform(.read(query)) else {
            throw CalendarFailure.readFailed
        }
        return value
    }

    func testInitializationAndStatusDoNotReadOrRequestPermission() async {
        let backend = FakeCalendarBackend(); let facts = FactBox()
        let value = controller(backend, facts: facts)
        XCTAssertFalse(value.status().physical_read_pending)
        value.disconnect()
        let reads = await backend.reads; let prompts = await backend.permissionRequests
        XCTAssertEqual(reads, 0); XCTAssertEqual(prompts, 0)
    }

    func testWriteOnlyIsDeniedBeforeBackendInsteadOfReturningEmptyAgenda() async {
        let backend = FakeCalendarBackend(); let facts = FactBox()
        facts.update { $0.permission = .writeOnly }
        let value = controller(backend, facts: facts)
        for operation in [CalendarOperation.sources(afterIndex: 0), .read(window)] {
            do { _ = try await value.perform(operation); XCTFail("write-only must not read") }
            catch { XCTAssertEqual(error as? CalendarFailure, .permissionRequired) }
        }
        let reads = await backend.reads; let windows = await backend.windows
        XCTAssertEqual(reads, 0); XCTAssertEqual(windows, 0)
    }

    func testExplicitAccessWhileStoppedDoesNotReadCalendars() async throws {
        let backend = FakeCalendarBackend(); let facts = FactBox()
        facts.update { $0.permission = .notDetermined; $0.stopped = true }
        let value = controller(backend, facts: facts)
        _ = try await value.perform(.requestAccess)
        let prompts = await backend.permissionRequests; let reads = await backend.reads
        XCTAssertEqual(prompts, 1); XCTAssertEqual(reads, 0)
        facts.update { $0.permission = .fullAccess }
        do { _ = try await value.perform(.read(window)); XCTFail("permission is not rearm") }
        catch { XCTAssertEqual(error as? CalendarFailure, .privacyStopped) }
    }

    /// REF-10 A1: with the permission granted, a read needs nothing else — no
    /// listing, no selection — and an empty window is an answer, not an error.
    func testA1AReadNeedsOnlyThePermission() async throws {
        let backend = FakeCalendarBackend(); let facts = FactBox()
        let value = controller(backend, facts: facts)
        let empty = try await page(value, window)
        XCTAssertEqual(empty.events, []); XCTAssertEqual(empty.total, 0); XCTAssertNil(empty.next_after_index)
        XCTAssertEqual(empty.provider_sync_status, "unknown")
        XCTAssertEqual(empty.enumeration, "bounded_local_read")
        XCTAssertEqual(empty.window, CalendarWindow(start: window.start_unix_ms, end: window.end_unix_ms))
        let reads = await backend.reads; XCTAssertEqual(reads, 0, "no listing was needed to read")
    }

    func testUntitledEventIsReportedAsOmittedWhileValidEventRemains() async throws {
        let backend = FakeCalendarBackend(events: [
            FakeCalendarEvent(observationID: uuid(2), calendarID: "work", calendarItemID: "valid-event", title: "Planificación"),
            FakeCalendarEvent(observationID: uuid(3), calendarID: "work", calendarItemID: "untitled-event", title: nil)
        ])
        let value = controller(backend, facts: FactBox())
        let result = try await page(value, window)
        XCTAssertEqual(result.events.map(\.title), ["Planificación"])
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(
            with: JSONEncoder().encode(result)) as? [String: Any])
        let omitted = try XCTUnwrap(payload["omitted"] as? [[String: Any]])
        XCTAssertEqual(omitted.first?["reason"] as? String, "missing_title")
        XCTAssertEqual(omitted.first?["count"] as? Int, 1)
    }

    /// REF-10 A3: a window with more events than a page holds is read in pages,
    /// in one order, every event exactly once, every page under the limit the
    /// daemon admits — and no event is counted as left out.
    func testA3AFullWindowArrivesWholeInOrderedPages() async throws {
        let events = (0..<1_500).map {
            FakeCalendarEvent(observationID: uuid(10_000 + $0), calendarID: "calendar-\($0 % 7)",
                calendarItemID: "item-\($0)", title: "Reunión número \($0) con un título de longitud normal",
                start: Int64(100 + $0), invitees: 3)
        }
        let value = controller(FakeCalendarBackend(events: events), facts: FactBox())
        var collected = [CalendarOccurrence]()
        var after: UInt32 = 0
        var pages = 0
        while true {
            let current = try await page(value, CalendarReadQuery(start: window.start_unix_ms,
                end: window.end_unix_ms, afterIndex: after))
            XCTAssertEqual(current.total, 1_500)
            XCTAssertEqual(current.after_index, after)
            XCTAssertFalse(current.events.isEmpty)
            XCTAssertLessThanOrEqual(try CalendarLimits.encodedSize(current), CalendarLimits.maximumPageBytes)
            XCTAssertTrue(current.omitted.isEmpty)
            collected += current.events
            pages += 1
            guard let next = current.next_after_index else { break }
            XCTAssertEqual(Int(next), collected.count)
            after = next
        }
        XCTAssertGreaterThan(pages, 1, "1,500 events do not fit one page")
        XCTAssertEqual(collected.count, 1_500)
        XCTAssertEqual(Set(collected.map(\.observation_id)).count, 1_500)
        XCTAssertEqual(collected.map(\.start_unix_ms), collected.map(\.start_unix_ms).sorted())
    }

    /// REF-10 A3: no constant limits the calendars listed. A thousand calendars
    /// come in pages, ordered, each under the limit, and all of them arrive.
    func testA3EveryCalendarIsListedWhateverTheirNumber() async throws {
        let value = controller(FakeCalendarBackend(calendars: 3_000), facts: FactBox())
        var names = [String]()
        var after: UInt32 = 0
        while true {
            guard case .inventory(let current) = try await value.perform(.sources(afterIndex: after)) else {
                return XCTFail("missing inventory page")
            }
            XCTAssertEqual(current.total, 3_000)
            XCTAssertLessThanOrEqual(try CalendarLimits.encodedSize(current), CalendarLimits.maximumPageBytes)
            names += current.calendars.map(\.reference.calendar_id)
            guard let next = current.next_after_index else { break }
            after = next
        }
        XCTAssertEqual(names.count, 3_000)
        XCTAssertEqual(names, names.sorted())
    }

    /// A meeting with more guests than the old ceiling of 64 is one event.
    func testAMeetingWithHundredsOfGuestsIsReadWhole() async throws {
        let backend = FakeCalendarBackend(events: [
            FakeCalendarEvent(observationID: uuid(5), calendarID: "work", calendarItemID: "all-hands",
                title: "All hands", invitees: 400)
        ])
        let result = try await page(controller(backend, facts: FactBox()), window)
        XCTAssertEqual(result.events.first?.invitees?.count, 400)
        XCTAssertEqual(result.events.first?.invitees_status, "provided")
    }

    /// Decision 4: one event that alone cannot fit the largest page is left
    /// out by name — identifier, dates, field and sizes — and the window's
    /// other events arrive; the read is not refused and nothing is cut.
    func testASingleEventLargerThanAnyPageIsLeftOutByNameAndTheRestArrive() async throws {
        let backend = FakeCalendarBackend(events: [
            FakeCalendarEvent(observationID: uuid(6), calendarID: "work", calendarItemID: "giant",
                title: "Evento enorme", invitees: 12_000),
            FakeCalendarEvent(observationID: uuid(7), calendarID: "work", calendarItemID: "small",
                title: "Evento normal", start: 200, invitees: 2),
        ])
        let result = try await page(controller(backend, facts: FactBox()), window)
        XCTAssertEqual(result.events.map(\.calendar_item_id), ["small"])
        XCTAssertEqual(result.total, 1)
        let left = try XCTUnwrap(result.oversized?.first)
        XCTAssertEqual(left.calendar_item_id, "giant")
        XCTAssertEqual(left.start_unix_ms, 100)
        XCTAssertEqual(left.field, "invitees")
        XCTAssertEqual(left.limit, CalendarLimits.maximumEventBytes)
        XCTAssertGreaterThan(left.bytes, left.limit)
    }

    /// A position past the end means the window changed since that index was
    /// computed: the read is refused and the daemon reads the window again.
    func testAPositionPastTheEndIsAChangedWindow() async {
        let backend = FakeCalendarBackend(events: [
            FakeCalendarEvent(observationID: uuid(7), calendarID: "work", calendarItemID: "one", title: "Uno")
        ])
        do {
            _ = try await page(controller(backend, facts: FactBox()),
                CalendarReadQuery(start: window.start_unix_ms, end: window.end_unix_ms, afterIndex: 5))
            XCTFail("an index into another enumeration was served")
        } catch { XCTAssertEqual(error as? CalendarFailure, .invalidated) }
    }

    func testDisconnectRevokesFutureGettersButRetainsPhysicalSlotUntilReturn() async throws {
        let started = expectation(description: "synthetic read admitted")
        let backend = FakeCalendarBackend(held: true, started: { started.fulfill() }); let facts = FactBox()
        let value = controller(backend, facts: facts)
        let request = Task { try await value.perform(.sources(afterIndex: 0)) }
        await fulfillment(of: [started], timeout: 1)
        value.disconnect()
        let result = await request.result
        if case .failure(let error) = result { XCTAssertEqual(error as? CalendarFailure, .invalidated) }
        else { XCTFail("revoked result delivered") }
        XCTAssertTrue(value.status().physical_read_pending)
        do { _ = try await value.perform(.sources(afterIndex: 0)); XCTFail("physical slot reused") }
        catch { XCTAssertEqual(error as? CalendarFailure, .busy) }
        await backend.release(); try await settle(value)
        let getters = await backend.contentGetters; XCTAssertEqual(getters, 0)
        _ = try await value.perform(.sources(afterIndex: 0))
    }

    func testDeadlineExpiresDeliveryWithoutWaitingForUncancelableBackend() async throws {
        let started = expectation(description: "synthetic slow read")
        let backend = FakeCalendarBackend(held: true, started: { started.fulfill() }); let facts = FactBox()
        let value = controller(backend, facts: facts)
        let request = Task { try await value.perform(.sources(afterIndex: 0)) }
        await fulfillment(of: [started], timeout: 1)
        facts.update { $0.now += CalendarLimits.readLifetimeNanoseconds }
        XCTAssertTrue(value.status().physical_read_pending) // Also runs the real expiry path.
        if case .failure(let error) = await request.result { XCTAssertEqual(error as? CalendarFailure, .expired) }
        else { XCTFail("expired read delivered") }
        await backend.release(); try await settle(value)
        let getters = await backend.contentGetters; XCTAssertEqual(getters, 0)
    }

    func testAPermissionRequestOutlivesAReadBecauseItWaitsForAPerson() async throws {
        // Sharing the read deadline gave the person five seconds to notice a
        // system dialog, read it and decide. The prompt was still on screen when
        // the permit expired, so answering it changed nothing and the status
        // stayed undetermined with no error to explain why.
        let started = expectation(description: "synthetic dialog on screen")
        let backend = FakeCalendarBackend(holdsPermission: true, started: { started.fulfill() })
        let facts = FactBox(); facts.update { $0.permission = .notDetermined }
        let value = controller(backend, facts: facts)
        let request = Task { try await value.perform(.requestAccess) }
        await fulfillment(of: [started], timeout: 1)
        facts.update { $0.now += CalendarLimits.readLifetimeNanoseconds }
        XCTAssertTrue(value.status().physical_read_pending,
                      "la petición caducó mientras el diálogo seguía en pantalla")
        await backend.release()
        guard case .success(.authorization(let granted)) = await request.result else {
            return XCTFail("la respuesta del diálogo se descartó")
        }
        XCTAssertEqual(granted, .fullAccess)
        try await settle(value)
    }

    func testAnUnansweredPermissionDialogStillExpires() async throws {
        // The physical slot cannot be held for good by a dialog nobody answers.
        let started = expectation(description: "synthetic dialog on screen")
        let backend = FakeCalendarBackend(holdsPermission: true, started: { started.fulfill() })
        let facts = FactBox(); facts.update { $0.permission = .notDetermined }
        let value = controller(backend, facts: facts)
        let request = Task { try await value.perform(.requestAccess) }
        await fulfillment(of: [started], timeout: 1)
        facts.update { $0.now += CalendarLimits.permissionLifetimeNanoseconds }
        XCTAssertTrue(value.status().physical_read_pending) // Runs the real expiry path.
        if case .failure(let error) = await request.result {
            XCTAssertEqual(error as? CalendarFailure, .expired)
        } else { XCTFail("expired permission delivered") }
        await backend.release(); try await settle(value)
    }

    /// REF-10 A9: withdrawing the permission cuts the next read, by name.
    func testA9WithdrawingThePermissionCutsTheNextRead() async throws {
        let backend = FakeCalendarBackend(events: [
            FakeCalendarEvent(observationID: uuid(8), calendarID: "work", calendarItemID: "one", title: "Uno")
        ])
        let facts = FactBox()
        let value = controller(backend, facts: facts)
        _ = try await page(value, window)
        facts.update { $0.permission = .denied }
        do { _ = try await page(value, window); XCTFail("a read without permission delivered events") }
        catch { XCTAssertEqual(error as? CalendarFailure, .permissionRequired) }
        XCTAssertEqual(value.status().authorization, .denied)
        let windows = await backend.windows; XCTAssertEqual(windows, 1)
    }

    func testBackendLimitFailureCannotBecomeSuccessfulEmptyInventory() async {
        let backend = FakeCalendarBackend(failure: .limitExceeded); let facts = FactBox()
        let value = controller(backend, facts: facts)
        do { _ = try await value.perform(.sources(afterIndex: 0)); XCTFail("error became empty success") }
        catch { XCTAssertEqual(error as? CalendarFailure, .limitExceeded) }
        XCTAssertFalse(value.status().physical_read_pending)
        XCTAssertEqual(value.status().failure, .limitExceeded)
    }

    func testStopBetweenWorkerResultAndTransportSerializationRejectsDelivery() async throws {
        let backend = FakeCalendarBackend(); let facts = FactBox()
        let value = controller(backend, facts: facts)
        let result = try await value.perform(.read(window))
        XCTAssertTrue(value.permitsDelivery(result))
        facts.update { $0.stopped = true }
        XCTAssertFalse(value.permitsDelivery(result))
        facts.update { $0.stopped = false }
        let fresh = try await value.perform(.sources(afterIndex: 0))
        facts.update { $0.generation += 1 }
        XCTAssertFalse(value.permitsDelivery(fresh))
    }

    func testDisconnectBeforeDeferredTaskStartsRevokesAdmissionWithoutReading() async throws {
        let backend = FakeCalendarBackend(); let facts = FactBox()
        let value = controller(backend, facts: facts)
        // These two synchronous turns model native admission followed by
        // calendar_disconnect, before the host's deferred Task can start.
        let admitted = value.admitCommand()
        value.disconnect()
        let deferred = Task { try await value.perform(.read(window), admitted: admitted) }
        if case .failure(let error) = await deferred.result {
            XCTAssertEqual(error as? CalendarFailure, .invalidated)
        } else { XCTFail("disconnected queued command delivered a page") }
        let windows = await backend.windows; XCTAssertEqual(windows, 0)
        XCTAssertFalse(value.status().physical_read_pending)
        XCTAssertFalse(value.permitsCommand(admitted))

        let fresh = value.admitCommand()
        guard case .page = try await value.perform(.read(window), admitted: fresh) else {
            return XCTFail("a new command should read")
        }
        let freshWindows = await backend.windows; XCTAssertEqual(freshWindows, 1)
    }

    /// A store change retires the read in flight and every ticket issued before
    /// it: the daemon reads that window again, from its first page.
    func testCalendarChangeRevokesDeferredAdmissionAndPreviouslyReturnedTicket() async throws {
        let backend = FakeCalendarBackend(); let facts = FactBox()
        let value = controller(backend, facts: facts)
        let completed = value.admitCommand()
        let result = try await value.perform(.read(window), admitted: completed)
        XCTAssertTrue(value.permitsCommand(completed)); XCTAssertTrue(value.permitsDelivery(result))
        let deferred = value.admitCommand()
        value.invalidate(.invalidated) // Same transition as the EventKit change observer.
        do { _ = try await value.perform(.read(window), admitted: deferred); XCTFail("changed calendar admitted old work") }
        catch { XCTAssertEqual(error as? CalendarFailure, .invalidated) }
        XCTAssertFalse(value.permitsCommand(completed))
        let windows = await backend.windows; XCTAssertEqual(windows, 1)
    }

    func testTicketRevocationDoesNotReleasePhysicalReadOrAuthorizeAnotherController() async throws {
        let started = expectation(description: "ticket read admitted")
        let backend = FakeCalendarBackend(held: true, started: { started.fulfill() }); let facts = FactBox()
        let value = controller(backend, facts: facts)
        let admitted = value.admitCommand()
        let pending = Task { try await value.perform(.sources(afterIndex: 0), admitted: admitted) }
        await fulfillment(of: [started], timeout: 1)
        value.disconnect()
        if case .failure(let error) = await pending.result { XCTAssertEqual(error as? CalendarFailure, .invalidated) }
        else { XCTFail("revoked physical read delivered") }
        XCTAssertTrue(value.status().physical_read_pending)
        do { _ = try await value.perform(.sources(afterIndex: 0), admitted: value.admitCommand()); XCTFail("SDK slot released early") }
        catch { XCTAssertEqual(error as? CalendarFailure, .busy) }
        let other = controller(FakeCalendarBackend(), facts: facts)
        do { _ = try await other.perform(.sources(afterIndex: 0), admitted: admitted); XCTFail("ticket crossed controller identity") }
        catch { XCTAssertEqual(error as? CalendarFailure, .invalidated) }
        await backend.release(); try await settle(value)
        let getters = await backend.contentGetters; XCTAssertEqual(getters, 0)
    }

    func testInvalidQueryIsRefusedWithoutReading() async throws {
        let backend = FakeCalendarBackend(); let facts = FactBox()
        let value = controller(backend, facts: facts)
        let ticket = value.admitCommand()
        do { _ = try await value.perform(.read(CalendarReadQuery(start: 0, end: 0))); XCTFail("invalid query admitted") }
        catch { XCTAssertEqual(error as? CalendarFailure, .invalidRequest) }
        XCTAssertTrue(value.permitsCommand(ticket))
        let windows = await backend.windows; XCTAssertEqual(windows, 0)
        _ = try await value.perform(.read(window))
    }
}
