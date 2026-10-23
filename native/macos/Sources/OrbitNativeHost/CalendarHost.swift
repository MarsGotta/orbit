import Foundation
import OrbitNativeProtocol
import OrbitNativeTransport
import OrbitNativeCalendar

/// What calendar work needs of the helper's state, read where that state is
/// kept: on the main thread.
package struct CalendarHostFacts: Sendable {
    package let connected: Bool
    package let generation: UInt64
    /// Anything that stops calendar work: the user's latch, a stop still
    /// cleaning up, a quit, a daemon that is not connected.
    package let stopped: Bool
    /// The user's own latch, the only stop a rearm lifts.
    package let latched: Bool

    package init(connected: Bool, generation: UInt64, stopped: Bool, latched: Bool) {
        self.connected = connected; self.generation = generation
        self.stopped = stopped; self.latched = latched
    }
}

/// The helper's calendar commands, answered off the main thread.
///
/// The main thread reads the daemon's heartbeat and starts and stops audio.
/// Here it only admits a command — a few comparisons and the ticket — and
/// reads `CalendarController.lastStatus()` for the state it reports. The
/// EventKit call, the paging, the checks and the encoding of a page (up to
/// `CalendarLimits.maximumPageBytes`) run on the controller's actor and on
/// the cooperative pool, never on the main thread; the flows ask the main
/// thread only for `facts`, and the main thread never waits for them.
@MainActor package final class CalendarHost {
    private let controller: CalendarController
    private let channel: NativeChannel
    private let facts: @MainActor @Sendable () -> CalendarHostFacts
    private let changed: @MainActor @Sendable () -> Void
    /// The single EventKit slot is taken from admission until the command is
    /// answered; the controller's own `physical_read_pending` covers the rest.
    private var commandInFlight = false
    private var refreshing = false
    /// The controller was created on first use before it reported the status
    /// too: until a calendar command or permission request, a stop has no
    /// reply to retire and tells the daemon nothing.
    private var used = false

    /// `changed` publishes the helper's state again: a command was answered,
    /// or EventKit reported a different permission.
    package init(controller: CalendarController, channel: NativeChannel,
                 facts: @escaping @MainActor @Sendable () -> CalendarHostFacts,
                 changed: @escaping @MainActor @Sendable () -> Void) {
        self.controller = controller; self.channel = channel
        self.facts = facts; self.changed = changed
    }

    /// The production controller: EventKit, off the main thread. An isolated
    /// fixture never asks EventKit anything and reports the calendar unavailable.
    package nonisolated static func controller(nativeInstanceID: String, isolatedFixture: Bool,
                                               channel: NativeChannel,
                                               facts: @escaping @MainActor @Sendable () -> CalendarHostFacts)
        -> CalendarController {
        // Each closure typed on its own: as one expression (and as a ternary
        // between two closures) the type checker gave up on it.
        let authorization: @Sendable () -> CalendarAuthorization
        if isolatedFixture {
            authorization = { CalendarAuthorization.unavailable }
        } else {
            authorization = { EventKitCalendarReader.authorization() }
        }
        let clock: @Sendable () -> UInt64? = { CaptureClock.currentHostNanoseconds() }
        let unixClock: @Sendable () -> Int64? = { CalendarBridge.unixMilliseconds() }
        let host: @Sendable () async -> CalendarHostState = {
            let current: CalendarHostFacts = await facts()
            return (generation: current.generation, stopped: current.stopped)
        }
        let invalidated: @Sendable (CalendarFailure) -> Void = { reason in
            let event: NativeEvent = .makeCalendarInvalidated(code: reason.rawValue)
            _ = channel.send(event)
        }
        let initial: CalendarAuthorization = isolatedFixture ? .unavailable : .unknown
        return CalendarController(nativeInstanceID: nativeInstanceID, backend: EventKitCalendarReader(),
                                  clock: clock, unixClock: unixClock, authorization: authorization,
                                  host: host, invalidated: invalidated, initialAuthorization: initial)
    }

    /// The calendar's status for `state`, heartbeats and `permissions`, as
    /// the controller last published it. EventKit is asked again off the main
    /// thread; a different permission publishes the state again.
    package func status() -> CalendarStatus {
        refresh()
        return controller.lastStatus()
    }

    package func stateFields() -> [String: Any] { Self.fields(status()) }

    /// Asks for the macOS permission; the dialog is EventKit's, the wait off Main.
    package func requestAccess() async throws {
        used = true
        _ = try await controller.perform(.requestAccess)
    }

    /// A stop, a lost daemon or quitting: the reply in flight is retired on
    /// the controller's actor. The main thread does not wait for it; a reply
    /// already on its way is held back by the helper's state the flow checks.
    package func invalidate(_ reason: CalendarFailure) {
        guard used else { return }
        let controller = self.controller
        Task.detached { await controller.invalidate(reason) }
    }

    /// `calendar_disconnect`: the reply in flight is retired, then answered.
    package func disconnect(_ command: NativeWireCommand) {
        let controller = self.controller, channel = self.channel, used = self.used
        Task.detached {
            if used { await controller.disconnect() }
            channel.respond(command, result: CalendarHost.fields(controller.lastStatus()))
        }
    }

    /// `calendar_sources` and `calendar_snapshot`.
    package func read(_ command: NativeWireCommand) {
        guard let flow = admit(command) else { return }
        Task.detached { [weak self] in
            await flow.read()
            await self?.finished()
        }
    }

    /// `calendar_save` and `calendar_delete`, which share the single EventKit
    /// slot and its admission.
    package func write(_ command: NativeWireCommand) {
        guard let flow = admit(command) else { return }
        Task.detached { [weak self] in
            await flow.write()
            await self?.finished()
        }
    }

    private func admit(_ command: NativeWireCommand) -> CalendarCommandFlow? {
        let current = facts()
        guard current.connected, !current.stopped else {
            // The same code covers the user's latch and a stop still cleaning up;
            // the message says which, and only the user's latch needs a rearm.
            let message = current.latched
                ? "Todo está parado: reactiva Orbit para usar los calendarios."
                : "Orbit está terminando de parar la captura o no está conectado; vuelve a intentarlo en unos segundos."
            channel.reject(command, code: "privacy_stopped", message: message)
            return nil
        }
        guard !commandInFlight, !controller.lastStatus().physical_read_pending else {
            channel.reject(command, code: "calendar_busy", message: "La operación anterior de EventKit aún no ha terminado.")
            return nil
        }
        // Capture the calendar generation now, before scheduling work. A
        // calendar-only disconnect does not increment the audio/privacy epoch.
        commandInFlight = true; used = true
        return CalendarCommandFlow(command: command, ticket: controller.admitCommand(),
            generation: current.generation, controller: controller, channel: channel, facts: facts)
    }

    private func finished() {
        commandInFlight = false
        changed()
    }

    private func refresh() {
        guard !refreshing else { return }
        refreshing = true
        let controller = self.controller
        let before = controller.lastStatus().authorization
        Task.detached { [weak self] in
            let after = await controller.status().authorization
            await self?.refreshed(changed: after != before)
        }
    }

    private func refreshed(changed permission: Bool) {
        refreshing = false
        if permission { changed() }
    }

    package nonisolated static func fields(_ value: CalendarStatus) -> [String: Any] {
        (try? CalendarBridge.fields(value)) ?? ["authorization": "unknown",
            "physical_read_pending": true, "failure": "calendar_read_failed"]
    }
}

/// One admitted calendar command, from the controller to its reply, off the
/// main thread.
///
/// The helper's state is checked again after EventKit returned and before the
/// reply is serialized, as it was when this ran on the main thread; the reply
/// is then sent from here. A stop between that check and the send cannot
/// carry agenda past it: the channel writes in order, so a reply sent after
/// the stop's state reaches the daemon after the stop, and the daemon drops a
/// read answered under a stop or for another privacy generation.
private struct CalendarCommandFlow: Sendable {
    let command: NativeWireCommand
    let ticket: CalendarCommandTicket
    let generation: UInt64
    let controller: CalendarController
    let channel: NativeChannel
    let facts: @MainActor @Sendable () -> CalendarHostFacts

    func read() async {
        do {
            let before = await facts()
            guard before.connected, before.generation == generation, !before.stopped else {
                throw CalendarFailure.invalidated
            }
            let result = try await controller.perform(CalendarBridge.operation(command), admitted: ticket)
            let after = await facts()
            guard after.connected, after.generation == generation,
                  controller.permitsCommand(ticket),
                  await controller.permitsDelivery(result) else { throw CalendarFailure.invalidated }
            channel.respond(command, result: try CalendarBridge.fields(result))
        } catch {
            let code = (error as? CalendarFailure)?.rawValue ?? "calendar_read_failed"
            channel.reject(command, code: code, message: "No se pudo leer el calendario; no se ha devuelto una agenda vacía.")
        }
    }

    /// One calendar write, answered with what EventKit did. A refusal travels
    /// with its numbers in `error.detail`. Before the controller takes the
    /// slot nothing was written, so a stop says `privacy_stopped`; once it
    /// may have been, a withheld answer says the outcome is unknown.
    func write() async {
        let operation: CalendarOperation
        do { operation = try CalendarBridge.operation(command) }
        catch {
            channel.reject(command, code: CalendarFailure.invalidRequest.rawValue,
                message: "La orden de escritura del calendario no tiene la forma del protocolo; no se cambió nada.")
            return
        }
        let before = await facts()
        guard before.connected, before.generation == generation, !before.stopped else {
            channel.reject(command, code: "privacy_stopped",
                message: "Orbit se detuvo antes de escribir; no se cambió nada.")
            return
        }
        let result: CalendarResult
        do {
            result = try await controller.perform(operation, admitted: ticket)
        } catch let refusal as CalendarWriteRefusal {
            reject(refusal)
            return
        } catch {
            // Refused before the slot was taken or any commit asked for.
            let code = (error as? CalendarFailure)?.rawValue ?? CalendarFailure.invalidRequest.rawValue
            channel.reject(command, code: code, message: "No se escribió en el calendario; no se cambió nada.")
            return
        }
        // Each withheld answer names its own cause; the write is done.
        let after = await facts()
        if !after.connected { return reject(.outcomeUnknown(cause: "daemon_disconnected")) }
        if after.generation != generation { return reject(.outcomeUnknown(cause: "privacy_generation_changed")) }
        if let cause = await controller.writeDeliveryRefusal(result) {
            return reject(.outcomeUnknown(cause: cause))
        }
        guard let fields = try? CalendarBridge.fields(result) else {
            return reject(.outcomeUnknown(cause: "result_not_encodable"))
        }
        channel.respond(command, result: fields)
    }

    private func reject(_ refusal: CalendarWriteRefusal) {
        if !channel.send(.makeRejection(id: command.id, code: refusal.code, message: refusal.message,
                                        detail: refusal.detail)) {
            FileHandle.standardError.write(Data("native_rejection_not_admitted event=response\n".utf8))
        }
    }
}
