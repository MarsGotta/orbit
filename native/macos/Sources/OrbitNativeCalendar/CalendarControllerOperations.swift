import Foundation
import OrbitNativeProtocol

/// One operation's work, on the controller's actor: EventKit's answers are
/// awaited from the backend actor and ordered, paged and checked here, never
/// on the main thread.
extension CalendarController {
    /// One write, start to answer. Before a commit is asked for, every
    /// refusal names its cause and the store is known to be unchanged. After,
    /// the answer is what EventKit did, or that its outcome is unknown.
    func write(_ operation: CalendarOperation, permit: CalendarReadPermit,
               gate: CalendarReadGate) async -> Result<CalendarResult, Error> {
        do {
            let response: CalendarResult
            switch operation {
            case .save(let request):
                let outcome = try await save(request, gate: gate)
                response = .saved(CalendarSaveResult(id: permit.id, nativeInstanceID: policy.nativeInstanceID,
                    generation: permit.generation, systemTimezone: outcome.systemTimezone,
                    occurrence: outcome.occurrence))
            case .delete(let request):
                guard let stored = try await backend.storedOccurrence(request.target, gate: gate) else {
                    throw CalendarWriteRefusal.eventNotFound(request.target)
                }
                if let refusal = CalendarWritePolicy.refusal(for: stored) { throw refusal }
                try gate.check()
                try await backend.delete(request, gate: gate)
                response = .deleted(CalendarDeleteResult(id: permit.id, nativeInstanceID: policy.nativeInstanceID,
                    generation: permit.generation, request: request))
            case .requestAccess, .sources, .read:
                throw CalendarFailure.invalidRequest
            }
            // Done. A stop or a new epoch since then withholds the answer —
            // what was written is not sent after the user's stop — and says which.
            let current = await host()
            if current.stopped {
                return .failure(CalendarWriteRefusal.outcomeUnknown(cause: "privacy_stopped"))
            }
            if current.generation != permit.generation {
                return .failure(CalendarWriteRefusal.outcomeUnknown(cause: "privacy_generation_changed"))
            }
            return .success(response)
        } catch let refusal as CalendarWriteRefusal {
            return .failure(refusal)
        } catch CalendarFailure.permissionRequired {
            // Withdrawn while the write was being prepared: EventKit's state now.
            return .failure(CalendarWriteRefusal.permissionDenied(currentAuthorization()))
        } catch let failure as CalendarFailure {
            // The gate closed before any commit was asked for (a stop, a
            // withdrawn reply, a dead clock): the store is unchanged.
            return .failure(failure)
        } catch {
            return .failure(CalendarFailure.readFailed)
        }
    }

    /// A create or a change, each refused by cause before its commit.
    private func save(_ request: CalendarSaveRequest, gate: CalendarReadGate) async throws -> CalendarWriteOutcome {
        if let draft = request.create {
            let calendar = try await backend.writableCalendar(draft.calendar_id, gate: gate)
            if let refusal = CalendarWritePolicy.refusal(for: calendar) { throw refusal }
            try gate.check()
            return try await backend.create(draft, in: calendar, gate: gate)
        }
        guard let change = request.change else { throw CalendarFailure.invalidRequest }
        guard let stored = try await backend.storedOccurrence(change.target, gate: gate) else {
            throw CalendarWriteRefusal.eventNotFound(change.target)
        }
        if let refusal = CalendarWritePolicy.refusal(for: stored) { throw refusal }
        if let destination = change.changes.calendar_id, destination != stored.calendar.calendar_id {
            let calendar = try await backend.writableCalendar(destination, gate: gate)
            if let refusal = CalendarWritePolicy.refusal(for: calendar) { throw refusal }
        }
        try CalendarWritePolicy.checkDates(stored, change.changes)
        try gate.check()
        return try await backend.change(change, gate: gate)
    }

    func read(_ operation: CalendarOperation, permit: CalendarReadPermit,
              gate: CalendarReadGate) async throws -> CalendarResult {
        try gate.check()
        switch operation {
        case .requestAccess:
            return .authorization(try await backend.requestAccess(gate: gate))
        case .sources(let afterIndex):
            return .inventory(try await inventory(after: afterIndex, permit: permit, gate: gate))
        case .read(let query):
            return .page(try await window(query, permit: permit, gate: gate))
        case .save, .delete:
            throw CalendarFailure.invalidRequest // Writes go through `write`.
        }
    }

    /// Every calendar, in the same order on every listing, so a page index
    /// names the same calendar on the next page request.
    private func inventory(after afterIndex: UInt32, permit: CalendarReadPermit,
                           gate: CalendarReadGate) async throws -> CalendarInventoryPage {
        let rows = try await backend.sources(gate: gate).sorted {
            $0.reference.calendar_id < $1.reference.calendar_id
        }
        try gate.check()
        guard rows.allSatisfy(\.isValid),
              Set(rows.map { $0.reference.calendar_id }).count == rows.count,
              let total = UInt32(exactly: rows.count) else {
            throw CalendarFailure.readFailed
        }
        let page = try CalendarPaging.page(rows, after: Int(afterIndex), budget: CalendarLimits.pageEventBytes)
        guard let completed = clock() else { throw CalendarFailure.clockUnavailable }
        let value = CalendarInventoryPage(id: permit.id, nativeInstanceID: policy.nativeInstanceID,
            generation: permit.generation, started: permit.started, completed: completed,
            calendars: page.items, afterIndex: afterIndex, total: total,
            nextAfterIndex: page.next.flatMap { UInt32(exactly: $0) })
        guard try CalendarLimits.encodedSize(value) <= CalendarLimits.maximumPageBytes else {
            throw CalendarFailure.limitExceeded
        }
        return value
    }

    private func window(_ query: CalendarReadQuery, permit: CalendarReadPermit,
                        gate: CalendarReadGate) async throws -> CalendarReadPage {
        let batch = try await backend.read(query: query, gate: gate)
        try gate.check()
        guard let completed = clock(), let observed = unixClock(), CalendarLimits.date(observed),
              Set(batch.events.map(\.observation_id)).count == batch.events.count,
              batch.omitted.allSatisfy({ !$0.reason.isEmpty && $0.reason.utf8.count <= 64
                && !$0.reason.unicodeScalars.contains(where: { $0.value < 32 || $0.value == 127 })
                && $0.count > 0 }),
              // Invitees and URL are always read: none may say «not requested».
              batch.events.allSatisfy({ $0.invitees_status != "not_requested"
                && $0.event_url_status != "not_requested" }),
              batch.oversized.allSatisfy({ $0.bytes > $0.limit && !$0.field.isEmpty }) else {
            throw CalendarFailure.readFailed
        }
        // Decision 4: an event larger than a page can carry is left out by
        // name, and the window's other events arrive; never the whole read.
        var travelling = [CalendarOccurrence]()
        var oversized = batch.oversized
        for event in batch.events {
            let size = try CalendarLimits.encodedSize(event)
            if size > CalendarLimits.maximumEventBytes {
                oversized.append(CalendarOversizedEvent(calendar: event.calendar,
                    calendarItemID: event.calendar_item_id, start: event.start_unix_ms, end: event.end_unix_ms,
                    field: (event.invitees?.isEmpty ?? true) ? "event" : "invitees",
                    bytes: size, limit: CalendarLimits.maximumEventBytes))
            } else {
                travelling.append(event)
            }
        }
        guard let total = UInt32(exactly: travelling.count) else { throw CalendarFailure.readFailed }
        let ordered = CalendarPaging.ordered(travelling)
        let page = try CalendarPaging.page(ordered, after: Int(query.after_index),
            budget: CalendarLimits.pageEventBytes)
        let value = CalendarReadPage(id: permit.id, nativeInstanceID: policy.nativeInstanceID,
            generation: permit.generation,
            window: CalendarWindow(start: query.start_unix_ms, end: query.end_unix_ms),
            started: permit.started, completed: completed, observedAt: observed,
            systemTimezone: batch.systemTimezone, events: page.items, omitted: batch.omitted,
            oversized: oversized, afterIndex: query.after_index, total: total,
            nextAfterIndex: page.next.flatMap { UInt32(exactly: $0) })
        // Every event was held to `maximumEventBytes` above and the page stops
        // filling at `pageEventBytes`; only an absurd list of events left
        // out could still pass the page, and that is refused by name.
        guard try CalendarLimits.encodedSize(value) <= CalendarLimits.maximumPageBytes else {
            throw CalendarFailure.eventTooLarge
        }
        return value
    }
}
