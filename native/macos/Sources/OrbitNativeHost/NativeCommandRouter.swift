import Foundation
import OrbitNativeProtocol

package enum NativeCommandRoute: Equatable {
    case start
    case queue
    case bypass
    case reject(code: String)
}

package struct NativeCommandRejection {
    package let command: NativeWireCommand
    package let code: String
    package var commandID: String { command.id }
}

/// Pure admission state for native commands. Urgent audio commands never enter
/// the general lane, whose bounded queue remains owned by its in-flight command.
package struct NativeCommandRouter {
    private struct InFlight {
        let commandID: String
        let startedAt: TimeInterval
        let deadline: TimeInterval
    }

    private static let maximumQueuedCommands = 32
    private static let defaultDeadline: TimeInterval = 30
    private static let permissionDeadline: TimeInterval = 120

    private var inFlight: InFlight?
    private var queued = [NativeWireCommand]()

    package init() {}

    package var hasCommandInFlight: Bool { inFlight != nil }

    package mutating func route(_ command: NativeWireCommand, at now: TimeInterval) -> NativeCommandRoute {
        if Self.usesUrgentAudioLane(command) { return .bypass }
        if let inFlight {
            if Self.isStalled(inFlight, at: now) {
                return .reject(code: "native_command_stalled")
            }
            guard queued.count < Self.maximumQueuedCommands else {
                return .reject(code: "native_backpressure")
            }
            queued.append(command)
            return .queue
        }
        inFlight = InFlight(commandID: command.id, startedAt: now,
                            deadline: Self.deadline(for: command))
        return .start
    }

    /// Completes only the command that owns the general lane. The returned
    /// command must be routed again so current connectivity/privacy still apply.
    package mutating func complete(commandID: String) -> NativeWireCommand? {
        guard inFlight?.commandID == commandID else { return nil }
        inFlight = nil
        guard !queued.isEmpty else { return nil }
        return queued.removeFirst()
    }

    package mutating func watchdog(at now: TimeInterval) -> [NativeCommandRejection] {
        guard let inFlight, Self.isStalled(inFlight, at: now), !queued.isEmpty else {
            return []
        }
        let stalled = queued
        queued.removeAll(keepingCapacity: true)
        return stalled.map { NativeCommandRejection(command: $0, code: "native_command_stalled") }
    }

    package mutating func removeQueued(where shouldRemove: (NativeWireCommand) -> Bool) -> [NativeWireCommand] {
        var removed = [NativeWireCommand]()
        queued.removeAll { command in
            if shouldRemove(command) {
                removed.append(command)
                return true
            }
            return false
        }
        return removed
    }

    private static func usesUrgentAudioLane(_ command: NativeWireCommand) -> Bool {
        command.command == "playback" || command.command == "playback_clip"
            || command.command == "audio_stop" || command.command == "audio_purpose"
    }

    private static func deadline(for command: NativeWireCommand) -> TimeInterval {
        command.command == "permissions" ? permissionDeadline : defaultDeadline
    }

    private static func isStalled(_ command: InFlight, at now: TimeInterval) -> Bool {
        now >= command.startedAt && now - command.startedAt > command.deadline
    }
}
