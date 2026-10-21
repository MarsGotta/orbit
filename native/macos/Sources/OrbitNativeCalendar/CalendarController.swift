import Foundation
import EventKit
import OrbitNativeProtocol

/// Opaque controller-generation proof captured before the host schedules a Task.
/// This revokes deferred admission; it neither reserves nor releases the SDK slot.
public struct CalendarCommandTicket: Equatable, Sendable {
    fileprivate let generation: UUID
}

/// Main-thread admission and response ownership. The independent backend actor
/// performs synchronous EventKit reads, never on Main and never in the audio queue.
///
/// REF-10: there is no selection to keep. Each read covers every calendar the
/// macOS permission allows and is answered in pages; the only state held here
/// is the single physical EventKit slot and the reply waiting on it.
///
/// P9-21: the same slot carries writes. A write is validated before it takes
/// the slot, and refused by name — a read-only calendar, an event someone
/// else organized, the end before the start — before EventKit is asked for a
/// commit. Once a commit may have started, nothing the controller does can
/// say it did not happen: a deadline, a stop or a withdrawn reply answer
/// `calendar_write_outcome_unknown`, never «expired» or «invalidated».
@MainActor public final class CalendarController {
    private let backend: any CalendarBackend
    private let clock: @Sendable () -> UInt64?
    private let unixClock: @Sendable () -> Int64?
    private let authorization: @Sendable () -> CalendarAuthorization
    private let host: @MainActor () -> (generation: UInt64, stopped: Bool)
    private let observesChanges: Bool
    private var policy: CalendarPolicy
    private var gate: CalendarReadGate?
    private var worker: Task<Void, Never>?
    private var timeout: Task<Void, Never>?
    private var reply: ((Result<CalendarResult, Error>) -> Void)?
    private var observer: NSObjectProtocol?
    private var lastFailure: CalendarFailure?
    private var commandGeneration = UUID()
    public var invalidated: ((CalendarFailure) -> Void)?

    public init(nativeInstanceID: String, backend: any CalendarBackend,
                clock: @escaping @Sendable () -> UInt64?, unixClock: @escaping @Sendable () -> Int64?,
                authorization: @escaping @Sendable () -> CalendarAuthorization,
                host: @escaping @MainActor () -> (generation: UInt64, stopped: Bool), observesChanges: Bool = true) {
        policy = CalendarPolicy(nativeInstanceID: nativeInstanceID); self.backend = backend
        self.clock = clock; self.unixClock = unixClock; self.authorization = authorization
        self.host = host; self.observesChanges = observesChanges
        // No store, subscription, clock read or permission request at initialization.
    }

    public func status() -> CalendarStatus {
        let permission = authorization()
        if let permit = policy.pending, reply != nil {
            if let now = clock() {
                if now < permit.started || now >= permit.deadline { invalidate(.expired) }
            } else { invalidate(.clockUnavailable) }
        }
        if permission != .fullAccess, let pending = policy.pending, pending.operation != .requestAccess, reply != nil {
            invalidate(.permissionRequired)
        }
        return CalendarStatus(authorization: permission, physicalReadPending: policy.pending != nil,
            failure: lastFailure)
    }

    /// Retires the reply in flight. Kept for the daemon's `calendar_disconnect`,
    /// which it sends when a read stopped mattering; nothing is selected to retire.
    public func disconnect() { invalidate(.invalidated) }

    public func admitCommand() -> CalendarCommandTicket {
        CalendarCommandTicket(generation: commandGeneration)
    }

    public func permitsCommand(_ ticket: CalendarCommandTicket) -> Bool {
        ticket.generation == commandGeneration
    }

    /// Why a write's answer cannot be delivered now, by name; nil when it can.
    /// The write is done either way: the cause is what the daemon hears with
    /// `calendar_write_outcome_unknown`.
    public func writeDeliveryRefusal(_ result: CalendarResult) -> String? {
        let current = host()
        if current.stopped { return "privacy_stopped" }
        if authorization() != .fullAccess { return "calendar_permission_required" }
        let instance: String
        let generation: UInt64
        switch result {
        case .saved(let value): instance = value.native_instance_id; generation = value.privacy_generation
        case .deleted(let value): instance = value.native_instance_id; generation = value.privacy_generation
        case .authorization, .inventory, .page: return "calendar_result_not_a_write"
        }
        if instance != policy.nativeInstanceID { return "native_instance_changed" }
        if generation != current.generation { return "privacy_generation_changed" }
        return nil
    }

    /// The awaiting transport may resume after a stop/change. Validate again in
    /// the same Main turn that serializes the reply, not only in the worker return.
    public func permitsDelivery(_ result: CalendarResult) -> Bool {
        switch result {
        case .saved, .deleted: return writeDeliveryRefusal(result) == nil
        case .authorization, .inventory, .page: break
        }
        let current = host()
        guard !current.stopped, authorization() == .fullAccess, let now = clock() else { return false }
        switch result {
        case .inventory(let value):
            return value.native_instance_id == policy.nativeInstanceID
                && value.privacy_generation == current.generation
                && now >= value.completed_monotonic_ns && now >= value.observed_monotonic_ns
                && now - value.observed_monotonic_ns < CalendarLimits.readLifetimeNanoseconds
        case .page(let value):
            return value.native_instance_id == policy.nativeInstanceID
                && value.privacy_generation == current.generation
                && now >= value.capture_completed_monotonic_ns && now >= value.capture_started_monotonic_ns
                && now - value.capture_started_monotonic_ns < CalendarLimits.readLifetimeNanoseconds
        case .saved(let value):
            // A write's answer has no freshness to keep: it says what was done.
            return value.native_instance_id == policy.nativeInstanceID
                && value.privacy_generation == current.generation
        case .deleted(let value):
            return value.native_instance_id == policy.nativeInstanceID
                && value.privacy_generation == current.generation
        case .authorization:
            return false // Permission responses use a fresh permission-only status.
        }
    }

    public func invalidate(_ reason: CalendarFailure = .invalidated) {
        commandGeneration = UUID()
        policy.invalidate(); gate?.revoke(reason); lastFailure = reason
        if let observer { NotificationCenter.default.removeObserver(observer); self.observer = nil }
        // A write in flight may already be committed: its reply cannot say why
        // it was retired as if nothing had happened.
        if policy.pending?.operation.isWrite == true {
            finishReply(.failure(CalendarWriteRefusal.outcomeUnknown(
                cause: reason == .expired ? "write_deadline" : reason.rawValue)))
        } else {
            finishReply(.failure(reason))
        }
        // A framework callback may still be running. Do NOT clear worker/pending.
        invalidated?(reason)
    }

    /// EventKit says the store changed. A read in flight is retired (pages of
    /// one window come from one store state); a write in flight is not — its
    /// own commit is usually what changed the store — and the daemon hears of
    /// the change either way, and reads again.
    private func storeChanged() {
        if policy.pending?.operation.isWrite == true {
            invalidated?(.invalidated)
        } else {
            invalidate(.invalidated)
        }
    }

    /// A calendar disconnect can precede the first turn of a queued host Task.
    /// Check its ticket inside the actual operation, before the first suspension
    /// or SDK admission. The host checks it again in the reply serialization turn.
    public func perform(_ operation: CalendarOperation, admitted ticket: CalendarCommandTicket? = nil) async throws -> CalendarResult {
        if let ticket, !permitsCommand(ticket) { throw CalendarFailure.invalidated }
        guard policy.pending == nil else { throw CalendarFailure.busy }
        guard let now = clock() else {
            invalidate(.clockUnavailable); throw CalendarFailure.clockUnavailable
        }
        let current = host()
        let permission = authorization()
        if operation != .requestAccess {
            guard !current.stopped else {
                invalidate(.privacyStopped); throw CalendarFailure.privacyStopped
            }
            guard permission == .fullAccess else {
                // A write names the permission EventKit reports, as it is.
                if operation.isWrite { throw CalendarWriteRefusal.permissionDenied(permission) }
                invalidate(.permissionRequired); throw CalendarFailure.permissionRequired
            }
        }
        let permit = try policy.begin(operation, id: UUID().uuidString.lowercased(), now: now,
            generation: current.generation, stopped: current.stopped, authorization: permission)
        let readGate = CalendarReadGate(started: now, deadline: permit.deadline, clock: clock)
        gate = readGate; lastFailure = nil
        observeChangesIfNeeded()
        return try await withCheckedThrowingContinuation { continuation in
            reply = { continuation.resume(with: $0) }
            timeout = Task { @MainActor [weak self] in
                // The same deadline the permit was issued with: a permission
                // request outlives a read because it waits for a person.
                do { try await Task.sleep(nanoseconds: CalendarLimits.lifetime(for: operation)) }
                catch { return }
                guard let self, self.policy.pending == permit else { return }
                self.policy.expire(permit)
                readGate.revoke(.expired)
                if operation.isWrite {
                    // EventKit has not returned: the commit may or may not be done.
                    self.finishReply(.failure(CalendarWriteRefusal.outcomeUnknown(cause: "write_deadline")))
                } else {
                    self.lastFailure = .expired
                    self.finishReply(.failure(CalendarFailure.expired))
                }
            }
            worker = Task { @MainActor [self] in
                let result: Result<CalendarResult, Error>
                if operation.isWrite {
                    result = await write(operation, permit: permit, gate: readGate)
                } else {
                    do {
                        let response = try await read(operation, permit: permit, gate: readGate)
                        try readGate.check()
                        guard let completed = clock() else { throw CalendarFailure.clockUnavailable }
                        let current = host()
                        try policy.validate(permit, now: completed, generation: current.generation,
                            stopped: current.stopped, authorization: authorization())
                        result = .success(response)
                    } catch {
                        let failure = (error as? CalendarFailure) ?? .readFailed
                        lastFailure = failure
                        result = .failure(failure)
                    }
                }
                policy.complete(permit); gate = nil; worker = nil
                finishReply(result)
            }
        }
    }

    /// One write, start to answer. Before a commit is asked for, every
    /// refusal names its cause and the store is known to be unchanged. After,
    /// the answer is what EventKit did, or that its outcome is unknown.
    private func write(_ operation: CalendarOperation, permit: CalendarReadPermit,
                       gate: CalendarReadGate) async -> Result<CalendarResult, Error> {
        do {
            let response: CalendarResult
            switch operation {
            case .save(let request):
                let outcome: CalendarWriteOutcome
                if let draft = request.create {
                    let calendar = try await backend.writableCalendar(draft.calendar_id, gate: gate)
                    if let refusal = CalendarWritePolicy.refusal(for: calendar) { throw refusal }
                    try gate.check()
                    outcome = try await backend.create(draft, in: calendar, gate: gate)
                } else if let change = request.change {
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
                    outcome = try await backend.change(change, gate: gate)
                } else {
                    throw CalendarFailure.invalidRequest
                }
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
            // what was written is not sent after her stop — and says which.
            let current = host()
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
            return .failure(CalendarWriteRefusal.permissionDenied(authorization()))
        } catch let failure as CalendarFailure {
            // The gate closed before any commit was asked for (a stop, a
            // withdrawn reply, a dead clock): the store is unchanged.
            return .failure(failure)
        } catch {
            return .failure(CalendarFailure.readFailed)
        }
    }

    private func read(_ operation: CalendarOperation, permit: CalendarReadPermit,
                      gate: CalendarReadGate) async throws -> CalendarResult {
        try gate.check()
        switch operation {
        case .requestAccess:
            return .authorization(try await backend.requestAccess(gate: gate))
        case .sources(let afterIndex):
            // Every calendar, in the same order on every listing, so a page
            // index names the same calendar on the next page request.
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
            return .inventory(value)
        case .read(let query):
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
            return .page(value)
        case .save, .delete:
            throw CalendarFailure.invalidRequest // Writes go through `write`.
        }
    }

    private func finishReply(_ result: Result<CalendarResult, Error>) {
        timeout?.cancel(); timeout = nil
        let complete = reply; reply = nil
        complete?(result)
    }

    private func observeChangesIfNeeded() {
        guard observesChanges, observer == nil else { return }
        observer = NotificationCenter.default.addObserver(forName: .EKEventStoreChanged, object: nil, queue: .main) {
            [weak self] _ in
            // This observer explicitly uses OperationQueue.main. Retire the read
            // in flight in the notification turn, not in a later unstructured
            // task: pages of one window must come from one store state, and the
            // daemon reads a window again when it hears the store changed.
            MainActor.assumeIsolated { self?.storeChanged() }
        }
    }
}
