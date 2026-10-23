import Foundation
import EventKit
import Synchronization
import OrbitNativeProtocol

/// Opaque controller-generation proof captured before the host schedules a Task.
/// This revokes deferred admission; it neither reserves nor releases the SDK slot.
public struct CalendarCommandTicket: Equatable, Sendable {
    fileprivate let generation: UUID
}

/// What the helper's privacy state says about calendar work, as the host
/// keeps it on its main thread: the privacy epoch, and whether anything
/// (the user's latch, a stop still cleaning up, a lost daemon) stops it.
public typealias CalendarHostState = (generation: UInt64, stopped: Bool)

/// Admission and response ownership, on an actor of its own. The backend
/// actor performs synchronous EventKit reads, never on Main and never in the
/// audio queue; this controller orders, pages and checks what they return,
/// off Main too. The main thread reads the heartbeat and starts and stops
/// audio: on 2026-10-22 a meeting was cut in the same second as a calendar
/// invalidation, while every step of this controller ran there. The main
/// thread now only answers `host` and reads `lastStatus()`, which never asks
/// EventKit anything.
///
/// There is no selection to keep. Each read covers every calendar the
/// macOS permission allows and is answered in pages; the only state held here
/// is the single physical EventKit slot and the reply waiting on it.
///
/// The same slot carries writes. A write is validated before it takes
/// the slot, and refused by name — a read-only calendar, an event someone
/// else organized, the end before the start — before EventKit is asked for a
/// commit. Once a commit may have started, nothing the controller does can
/// say it did not happen: a deadline, a stop or a withdrawn reply answer
/// `calendar_write_outcome_unknown`, never «expired» or «invalidated».
public actor CalendarController {
    let backend: any CalendarBackend
    let clock: @Sendable () -> UInt64?
    let unixClock: @Sendable () -> Int64?
    private let authorization: @Sendable () -> CalendarAuthorization
    /// The host's privacy state, asked of the thread that keeps it. The
    /// controller waits for the host; the host never waits for the controller.
    let host: @Sendable () async -> CalendarHostState
    private let invalidated: @Sendable (CalendarFailure) -> Void
    private let observesChanges: Bool
    private let changes: OperationQueue
    let shared: CalendarControllerShared
    var policy: CalendarPolicy
    private var gate: CalendarReadGate?
    private var worker: Task<Void, Never>?
    private var timeout: Task<Void, Never>?
    private var reply: ((Result<CalendarResult, Error>) -> Void)?
    private var observer: NSObjectProtocol?
    private var lastFailure: CalendarFailure?
    private var lastAuthorization: CalendarAuthorization

    /// `initialAuthorization` is what `lastStatus()` says before the first
    /// `status()` asked EventKit: unknown, not a guess.
    public init(nativeInstanceID: String, backend: any CalendarBackend,
                clock: @escaping @Sendable () -> UInt64?, unixClock: @escaping @Sendable () -> Int64?,
                authorization: @escaping @Sendable () -> CalendarAuthorization,
                host: @escaping @Sendable () async -> CalendarHostState,
                invalidated: @escaping @Sendable (CalendarFailure) -> Void = { _ in },
                initialAuthorization: CalendarAuthorization = .unknown, observesChanges: Bool = true) {
        policy = CalendarPolicy(nativeInstanceID: nativeInstanceID); self.backend = backend
        self.clock = clock; self.unixClock = unixClock; self.authorization = authorization
        self.host = host; self.invalidated = invalidated; self.observesChanges = observesChanges
        lastAuthorization = initialAuthorization
        shared = CalendarControllerShared(status: CalendarStatus(authorization: initialAuthorization))
        let queue = OperationQueue()
        queue.name = "orbit.calendar.changes"
        queue.maxConcurrentOperationCount = 1
        changes = queue
        // No store, subscription, clock read or permission request at initialization.
    }

    /// What the controller last published, for the host's `state` and
    /// heartbeat replies. Never asks EventKit and never waits; `status()`
    /// refreshes it, off the caller's thread.
    public nonisolated func lastStatus() -> CalendarStatus { shared.status() }

    /// Asks EventKit for the permission's state, expires a reply past its
    /// deadline and publishes the result to `lastStatus()`.
    @discardableResult
    public func status() -> CalendarStatus {
        let permission = currentAuthorization()
        if let permit = policy.pending, reply != nil {
            if let now = clock() {
                if now < permit.started || now >= permit.deadline { invalidate(.expired) }
            } else { invalidate(.clockUnavailable) }
        }
        if permission != .fullAccess, let pending = policy.pending, pending.operation != .requestAccess, reply != nil {
            invalidate(.permissionRequired)
        }
        return publishStatus()
    }

    /// Retires the reply in flight. Kept for the daemon's `calendar_disconnect`,
    /// which it sends when a read stopped mattering; nothing is selected to retire.
    public func disconnect() { invalidate(.invalidated) }

    public nonisolated func admitCommand() -> CalendarCommandTicket {
        CalendarCommandTicket(generation: shared.ticket())
    }

    public nonisolated func permitsCommand(_ ticket: CalendarCommandTicket) -> Bool {
        ticket.generation == shared.ticket()
    }

    /// Why a write's answer cannot be delivered now, by name; nil when it can.
    /// The write is done either way: the cause is what the daemon hears with
    /// `calendar_write_outcome_unknown`.
    public func writeDeliveryRefusal(_ result: CalendarResult) async -> String? {
        let current = await host()
        if current.stopped { return "privacy_stopped" }
        if currentAuthorization() != .fullAccess { return "calendar_permission_required" }
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

    /// The awaiting transport may resume after a stop/change. Validate again
    /// right before the reply is serialized, not only in the worker return.
    public func permitsDelivery(_ result: CalendarResult) async -> Bool {
        switch result {
        case .saved, .deleted: return await writeDeliveryRefusal(result) == nil
        case .authorization, .inventory, .page: break
        }
        let current = await host()
        guard !current.stopped, currentAuthorization() == .fullAccess, let now = clock() else { return false }
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
        case .saved, .deleted:
            return false // Answered by `writeDeliveryRefusal` above.
        case .authorization:
            return false // Permission responses use a fresh permission-only status.
        }
    }

    public func invalidate(_ reason: CalendarFailure = .invalidated) {
        shared.renewTicket()
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
        invalidated(reason)
    }

    /// EventKit says the store changed. A read in flight is retired (pages of
    /// one window come from one store state); a write in flight is not — its
    /// own commit is usually what changed the store — and the daemon hears of
    /// the change either way, and reads again.
    func storeChanged() {
        if policy.pending?.operation.isWrite == true {
            invalidated(.invalidated)
        } else {
            invalidate(.invalidated)
        }
    }

    /// A calendar disconnect can precede the first turn of a queued host Task.
    /// Check its ticket inside the actual operation, before SDK admission. The
    /// host checks it again before it serializes the reply.
    public func perform(_ operation: CalendarOperation, admitted ticket: CalendarCommandTicket? = nil) async throws -> CalendarResult {
        // The host's state is the only wait before admission: from here to
        // `begin` is one turn of this actor, so no other operation can take
        // the slot between these checks and the permit.
        let current = await host()
        if let ticket, !permitsCommand(ticket) { throw CalendarFailure.invalidated }
        guard policy.pending == nil else { throw CalendarFailure.busy }
        guard let now = clock() else {
            invalidate(.clockUnavailable); throw CalendarFailure.clockUnavailable
        }
        let permission = currentAuthorization()
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
        // A store change retires a read at once, from the notification's own
        // queue; a write's own commit is what changes the store.
        shared.holdRead(operation.isWrite ? nil : readGate)
        publishStatus()
        observeChangesIfNeeded()
        return try await withCheckedThrowingContinuation { continuation in
            reply = { continuation.resume(with: $0) }
            timeout = Task { [weak self] in
                // The same deadline the permit was issued with: a permission
                // request outlives a read because it waits for a person.
                do { try await Task.sleep(nanoseconds: CalendarLimits.lifetime(for: operation)) }
                catch { return }
                await self?.expire(permit, gate: readGate)
            }
            worker = Task { await self.work(operation, permit: permit, gate: readGate) }
        }
    }

    private func expire(_ permit: CalendarReadPermit, gate readGate: CalendarReadGate) {
        guard policy.pending == permit else { return }
        policy.expire(permit)
        readGate.revoke(.expired)
        if permit.operation.isWrite {
            // EventKit has not returned: the commit may or may not be done.
            finishReply(.failure(CalendarWriteRefusal.outcomeUnknown(cause: "write_deadline")))
        } else {
            lastFailure = .expired
            finishReply(.failure(CalendarFailure.expired))
        }
    }

    private func work(_ operation: CalendarOperation, permit: CalendarReadPermit,
                      gate readGate: CalendarReadGate) async {
        let result: Result<CalendarResult, Error>
        if operation.isWrite {
            result = await write(operation, permit: permit, gate: readGate)
        } else {
            do {
                let response = try await read(operation, permit: permit, gate: readGate)
                let current = await host()
                try readGate.check()
                guard let completed = clock() else { throw CalendarFailure.clockUnavailable }
                try policy.validate(permit, now: completed, generation: current.generation,
                    stopped: current.stopped, authorization: currentAuthorization())
                result = .success(response)
            } catch {
                let failure = (error as? CalendarFailure) ?? .readFailed
                lastFailure = failure
                result = .failure(failure)
            }
        }
        policy.complete(permit); gate = nil; worker = nil
        shared.holdRead(nil)
        finishReply(result)
    }

    private func finishReply(_ result: Result<CalendarResult, Error>) {
        timeout?.cancel(); timeout = nil
        let complete = reply; reply = nil
        publishStatus()
        complete?(result)
    }

    /// EventKit's answer about the permission, kept for `lastStatus()`.
    func currentAuthorization() -> CalendarAuthorization {
        let value = authorization()
        lastAuthorization = value
        return value
    }

    @discardableResult
    private func publishStatus() -> CalendarStatus {
        let value = CalendarStatus(authorization: lastAuthorization, physicalReadPending: policy.pending != nil,
            failure: lastFailure)
        shared.publish(value)
        return value
    }

    private func observeChangesIfNeeded() {
        guard observesChanges, observer == nil else { return }
        let shared = self.shared
        observer = NotificationCenter.default.addObserver(forName: .EKEventStoreChanged, object: nil,
                                                          queue: changes) { [weak self] _ in
            // Off the main thread, on this controller's own queue. The read in
            // flight is retired at once, before its next getter or page: pages
            // of one window must come from one store state, and the daemon
            // reads a window again when it hears the store changed.
            shared.revokeHeldRead(.invalidated)
            Task { await self?.storeChanged() }
        }
    }
}

/// What the controller shares with threads that must never wait for it: the
/// admission generation, the status it last published, and the read in
/// flight a store change retires at once.
final class CalendarControllerShared: Sendable {
    private let generation = Mutex(UUID())
    private let published: Mutex<CalendarStatus>
    private let heldRead = Mutex<CalendarReadGate?>(nil)

    init(status: CalendarStatus) { published = Mutex(status) }

    func ticket() -> UUID { generation.withLock { $0 } }
    func renewTicket() { generation.withLock { $0 = UUID() } }
    func status() -> CalendarStatus { published.withLock { $0 } }
    func publish(_ value: CalendarStatus) { published.withLock { $0 = value } }
    func holdRead(_ gate: CalendarReadGate?) { heldRead.withLock { $0 = gate } }
    func revokeHeldRead(_ reason: CalendarFailure) { heldRead.withLock { $0 }?.revoke(reason) }
}
