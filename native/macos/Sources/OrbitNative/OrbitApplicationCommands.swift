import AppKit
import Darwin
import AVFoundation
import ApplicationServices
import ScreenCaptureKit
import OrbitNativeProtocol
import OrbitNativeTransport
import OrbitNativeState
import OrbitNativeCalendar
import OrbitNativeHost

// The daemon's commands: answered at once, on their own lane, or on the
// general lane one at a time, and then performed.
extension OrbitApplication {
    func receive(_ command: NativeWireCommand) {
        // Its kind only: what the main thread was doing if it is held next.
        mainThreadWatch.commandStarted(command.command)
        defer { updateMainThreadWatch() }
        guard launchOptions.permits(command: command.command) else {
            channel.reject(command, code: "isolated_fixture_scope", message: "Operation is not allowed in the isolated fixture.")
            return
        }
        switch command {
        case .hello, .heartbeat:
            channel.respond(command, result: state())
        case .interfaceLanguage(let wire):
            MainActor.assumeIsolated { assistantFiles.locale = wire.fields.locale }
            channel.respond(command)
        case .status:
            channel.respond(command, result: state().merging(["permissions": permissions()]) { _, rhs in rhs })
        case .calendarDisconnect:
            MainActor.assumeIsolated { calendarHost().disconnect(command) }
        // `calendar_select` is gone; decoding refuses it by name.
        case .calendarSources, .calendarSnapshot:
            MainActor.assumeIsolated { calendarHost().read(command) }
        // Writes share the single EventKit slot and its admission.
        case .calendarSave, .calendarDelete:
            MainActor.assumeIsolated { calendarHost().write(command) }
        case .privacyStop:
            respondAfterStop(command, reason: "daemon_requested", latches: true,
                failureMessage: "Everything is stopped, but some previous audio could not be confirmed.")
        case .captureStop:
            respondAfterStop(command, reason: StopPolicy.daemonCutReason, latches: false,
                failureMessage: "Capture stopped; some previous audio could not be confirmed.")
        case .playbackPause:
            playback.pause()
            channel.respond(command)
        case .playbackResume:
            playback.resume()
            channel.respond(command)
        case .playbackStop:
            cancelPlayback()
            channel.respond(command)
            publishState()
        case .rearm:
            rearm(command)
        case .shutdown:
            shutDown(command)
        // The folders never take the general lane: an open panel waits for a
        // person and a search walks the disk, and neither may hold back a
        // recording that starts meanwhile. The daemon sends one at a time.
        // Nor do a window's text, which waits on another app, and a photo's
        // JPEG, which decodes a whole image: each is the user's one attachment.
        case .assistantFolderPick, .assistantFolders, .assistantFilesList, .assistantFilesSearch,
             .assistantFilesRead, .windowText, .photoJPEG:
            guard connected else {
                channel.reject(command, code: "daemon_disconnected", message: "Restart Orbit to restore the local connection.")
                return
            }
            execute(command, ownsGeneralLane: false)
        case .permissions, .audioStart, .audioPurpose, .audioMute, .audioWakePhrase, .audioStop, .playback,
             .playbackClip, .applicationSources,
             .screenSources, .screenshot, .selectedText:
            guard connected else {
                channel.reject(command, code: "daemon_disconnected", message: "Restart Orbit to restore the local connection.")
                return
            }
            routeOnGeneralLane(command)
        }
    }

    /// `rearm`: the latch comes off this Mac, only by the daemon that
    /// launched this helper and only when nothing is in flight.
    private func rearm(_ command: NativeWireCommand) {
        // Only the private pipe of the daemon that launched this helper has
        // rearm authority. Isolated fixtures do not.
        guard launchOptions.isManaged else {
            channel.reject(command, code: "local_rearm_unavailable",
                message: "Only the local Orbit daemon can rearm it through its private channel.")
            return
        }
        if let refusal = rearmReadiness().refusal {
            channel.reject(command, code: "local_rearm_unavailable", message: refusal)
            return
        }
        let generation = privacyEpoch
        privacyCleanupInFlight += 1
        Task { @MainActor in
            do {
                try await self.latch.removeStopOffMain()
                guard self.privacyEpoch == generation, !self.quitRequested else {
                    throw NativeFailure("operation_cancelled_by_stop")
                }
                self.latch.finishRearm()
                self.privacyCleanupInFlight -= 1
                self.channel.send(.makeLocalRearm())
                self.resumeMicrophoneActivity()
                self.channel.respond(command, result: self.state())
            } catch {
                self.privacyCleanupInFlight -= 1
                self.channel.reject(command, code: "privacy_rearm_failed",
                    message: "Orbit could not save the privacy state on this Mac.")
            }
            self.publishState()
        }
    }

    /// `shutdown`: everything stops and drains before the answer, and the
    /// helper leaves once that answer has been flushed.
    private func shutDown(_ command: NativeWireCommand) {
        stop(reason: "shutdown") { [weak self] drained in
            guard let self else { return }
            if drained {
                self.channel.respond(command, result: ["drain_complete": true])
                self.finishQuitAfterDrain()
            } else {
                self.channel.reject(command, code: "audio_drain_incomplete", message: "Shutdown cannot be confirmed until previous audio has drained.")
            }
        }
    }

    /// The general lane runs one command at a time: the others wait behind
    /// it or pass beside it, and are refused when the queue is full or the
    /// one ahead outlived its deadline.
    private func routeOnGeneralLane(_ command: NativeWireCommand) {
        switch commandRouter.route(command, at: ProcessInfo.processInfo.systemUptime) {
        case .start:
            execute(command, ownsGeneralLane: true)
        case .queue:
            break
        case .bypass:
            execute(command, ownsGeneralLane: false)
        case .reject(let code):
            let message = code == "native_backpressure"
                ? "The native queue is full."
                : "The previous native operation exceeded its deadline; try again."
            channel.reject(command, code: code, message: message)
        }
    }

    private func respondAfterStop(_ command: NativeWireCommand, reason: String,
                                  latches: Bool, failureMessage: String) {
        let drain = stopWork(latches: latches).drain
        stop(reason: reason) { [weak self] drained in
            guard let self else { return }
            if drained {
                self.channel.respond(command, result: self.state().merging([
                    "drain_complete": true, "drain": drain
                ]) { _, rhs in rhs })
            } else {
                self.channel.reject(command, code: "audio_drain_incomplete", message: failureMessage)
            }
        }
    }

    private func execute(_ command: NativeWireCommand, ownsGeneralLane: Bool) {
        nativeOperations += 1
        if command.command != "permissions" && latch.stopped {
            channel.reject(command, code: "privacy_stopped", message: "Orbit is stopped on this Mac.")
            finish(command, ownsGeneralLane: ownsGeneralLane)
            return
        }
        let epoch = privacyEpoch
        let expectedPlaybackEpoch = playbackEpoch
        Task { @MainActor in
            defer { self.finish(command, ownsGeneralLane: ownsGeneralLane) }
            if ["playback", "playback_clip"].contains(command.command)
                && expectedPlaybackEpoch != self.playbackEpoch {
                self.channel.respond(command, result: ["state": "cancelled"])
                return
            }
            do {
                guard self.connected, epoch == self.privacyEpoch else {
                    throw NativeFailure("operation_cancelled_by_stop")
                }
                let result = try await self.perform(command)
                if epoch != self.privacyEpoch && command.command != "permissions" {
                    _ = await self.microphone.stop()
                    _ = await self.systemAudio.stop()
                    throw NativeFailure("operation_cancelled_by_stop")
                }
                self.channel.respond(command, result: result)
            } catch let failure as AssistantFilesFailure {
                // A folder refusal carries its cause, never a path or content.
                if !self.channel.send(.makeRejection(id: command.id, code: failure.code,
                        message: "Orbit cannot read that in the assistant folders.",
                        detail: failure.detail)) {
                    FileHandle.standardError.write(Data("native_rejection_not_admitted event=response\n".utf8))
                }
            } catch {
                let typed = (error as? WindowSelectionFailure)?.rawValue
                    ?? (error as? SelectedTextFailure)?.rawValue
                    ?? (error as? CalendarFailure)?.rawValue
                    ?? (error as? NativeFailure)?.code
                // An error the helper did not foresee, or one of its own over
                // a macOS error, says that error's domain and number: without
                // them «native_operation_failed» left a call that did not
                // start without a cause (2026-10-17).
                let detail = osErrorDetail((error as? NativeFailure)?.underlying
                    ?? (typed == nil ? error as NSError : nil))
                let message = "The native operation could not be completed."
                if let detail {
                    if !self.channel.send(.makeRejection(id: command.id, code: typed ?? "native_operation_failed",
                                                         message: message, detail: detail)) {
                        FileHandle.standardError.write(Data("native_rejection_not_admitted event=response\n".utf8))
                    }
                } else {
                    self.channel.reject(command, code: typed ?? "native_operation_failed", message: message)
                }
            }
        }
    }

    private func finish(_ command: NativeWireCommand, ownsGeneralLane: Bool) {
        nativeOperations -= 1
        let next = ownsGeneralLane ? commandRouter.complete(commandID: command.id) : nil
        publishState()
        if let next { receive(next) }
    }

    @MainActor private func perform(_ command: NativeWireCommand) async throws -> [String: Any] {
        switch command {
        case .permissions(let wire):
            return try await requestPermissions(wire.fields.request ?? [])
        case .audioStart(let wire):
            return try await startAudio(wire.fields)
        case .audioPurpose(let wire):
            return try changePurpose(wire.fields)
        case .audioMute(let wire):
            return try changeMute(wire.fields)
        case .audioWakePhrase(let wire):
            return try changeWakePhrase(wire.fields)
        case .audioStop(let wire):
            return try await stopAudio(wire.fields)
        case .playback(let wire):
            guard !audioRouteRecoveryInFlight else { throw NativeFailure("audio_route_recovery_in_progress") }
            return try playback.enqueue(wire.fields)
        case .playbackClip(let wire):
            guard !audioRouteRecoveryInFlight else { throw NativeFailure("audio_route_recovery_in_progress") }
            return try await playback.enqueueClip(wire.fields)
        case .applicationSources(let wire):
            let bundleIDs = wire.fields.bundle_ids
            return try await applicationIdentities.inventory(bundleIDs: bundleIDs, privacyGeneration: privacyEpoch) { [weak self] in
                self?.screenAuthorization() ?? (generation: 0, stopped: true)
            }
        case .screenSources:
            return try await windowScreenshots.inventory(privacyGeneration: privacyEpoch) { [weak self] in
                self?.screenAuthorization() ?? (generation: 0, stopped: true)
            }
        case .screenshot(let wire):
            return try await windowScreenshots.capture(wire.fields.window_binding) { [weak self] in
                self?.screenAuthorization() ?? (generation: 0, stopped: true)
            }
        case .selectedText(let wire):
            return try await windowScreenshots.capture(wire.fields.window_binding, kind: .selectedText) { [weak self] in
                self?.screenAuthorization() ?? (generation: 0, stopped: true)
            }
        case .assistantFolderPick:
            return try await assistantFiles.pick()
        case .assistantFolders(let wire):
            return try await assistantFiles.resolve(wire.fields)
        case .assistantFilesList(let wire):
            return try await assistantFiles.list(wire.fields)
        case .assistantFilesSearch(let wire):
            return try await assistantFiles.search(wire.fields)
        case .assistantFilesRead(let wire):
            return try await assistantFiles.read(wire.fields)
        case .windowText(let wire):
            return try await windowText.read(wire.fields)
        case .photoJPEG(let wire):
            return try await photoJPEG.convert(wire.fields)
        case .hello, .heartbeat, .status, .interfaceLanguage, .calendarDisconnect,
             .calendarSources, .calendarSnapshot, .calendarSave, .calendarDelete,
             .privacyStop, .captureStop,
             .playbackStop, .playbackPause, .playbackResume, .rearm, .shutdown:
            throw NativeFailure("unsupported_command")
        }
    }

    /// `permissions`: asks macOS for each one requested, in order, and
    /// answers with what is granted now.
    @MainActor private func requestPermissions(_ requested: [NativeWirePermission]) async throws -> [String: Any] {
        // An already-stopped latch permits explicit permission requests;
        // a new stop or disconnection revokes the remaining prompts.
        let permissionEpoch = privacyEpoch
        // Orbit runs as an accessory, so it never becomes the active app on
        // its own and a system prompt it raises can open behind whatever the
        // person is looking at, usually the terminal they just typed in. The
        // dialog then waits unseen and the permission stays undetermined.
        // Only an explicit permission request brings the app forward.
        if launchOptions.isManaged, !requested.isEmpty {
            NSApplication.shared.activate()
        }
        for permission in requested {
            guard connected, !quitRequested, privacyEpoch == permissionEpoch else {
                throw NativeFailure("operation_cancelled_by_stop")
            }
            switch permission {
            case .microphone: _ = await AVCaptureDevice.requestAccess(for: .audio)
            case .screen: _ = CGRequestScreenCaptureAccess()
            case .accessibility:
                let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true]
                _ = AXIsProcessTrustedWithOptions(options as CFDictionary)
            case .calendars: try await calendarHost().requestAccess()
            }
        }
        return permissions()
    }
}
