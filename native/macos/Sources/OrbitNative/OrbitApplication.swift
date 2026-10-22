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

final class OrbitApplication: NSObject, NSApplicationDelegate {
    private let channel: NativeChannel
    private let latch: PrivacyLatch
    private let launchOptions: NativeLaunchOptions
    private let diagnostics: DiagnosticLog
    private let audioSession = SharedAudioSession()
    private let hotkeys = GlobalHotkeys()
    private lazy var windowScreenshots = WindowScreenshots(diagnostics: diagnostics)
    /// The assistant's folders: the panel, their bookmarks, read-only reads.
    private let assistantFiles = AssistantFiles()
    private let windowText = WindowTextReading()
    private let photoJPEG = PhotoJPEG()
    private let microphoneActivity = MicrophoneActivityObserver()
    /// «Oye Deimos» on the meeting's microphone.
    private lazy var assistantTrigger = AssistantVoiceTrigger(channel: channel)
    private var calendars: CalendarController?
    private var calendarCommandInFlight = false
    private lazy var applicationIdentities = ApplicationIdentityCatalog(nativeInstanceID: windowScreenshots.nativeInstanceID)
    private lazy var microphone = MicrophoneCapture(session: audioSession, channel: channel)
    private lazy var playback = AudioPlayback(session: audioSession, channel: channel)
    private lazy var systemAudio = SystemAudioCapture(channel: channel)
    private var watchdog: Timer?
    private var channelHealth = DaemonChannelHealth(now: ProcessInfo.processInfo.systemUptime)
    /// Read-only on purpose: liveness is decided by observed traffic in
    /// `channelHealth`, never by a caller flipping a flag one way.
    private var connected: Bool { channelHealth.connected }
    private var commandRouter = NativeCommandRouter()
    private var privacyEpoch: UInt64 = 0
    private var playbackEpoch: UInt64 = 0
    private var microphonePurpose: NativeMicrophonePurpose?
    private var audioRouteEpoch: UInt64 = 0
    private var audioRouteRecoveringStreamID: String?
    private var audioRouteRecoveryInFlight: Bool { audioRouteRecoveringStreamID != nil }
    private var privacyCleanupInFlight = 0
    private let privacyDrainBarrier = AudioDrainBarrier()
    private var quitPolicy: NativeQuitPolicy
    private var quitRequested: Bool { quitPolicy.isRequested }
    private var workspaceObservers = [NSObjectProtocol]()
    private let daemonPID = getppid()
    private var parentQuitRequested = false
    private var terminatingForDaemon = false

    init(channel: NativeChannel, latch: PrivacyLatch,
         launchOptions: NativeLaunchOptions, diagnostics: DiagnosticLog) {
        self.channel = channel
        self.latch = latch
        self.launchOptions = launchOptions
        self.diagnostics = diagnostics
        // Only the daemon's own child leaves when its daemon is gone.
        self.quitPolicy = NativeQuitPolicy()
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        channel.receive = { [weak self] command in self?.receive(command) }
        channel.disconnected = { [weak self] reason in
            self?.daemonDisconnected(reason: reason)
        }
        audioSession.configurationChanged = { [weak self] previous, current in
            self?.audioRouteChanged(previous: previous, current: current)
        }
        playback.becameIdle = { [weak self] in
            self?.audioSession.stopIfIdle()
        }
        microphone.failed = { [weak self] reason in self?.stop(reason: reason) }
        systemAudio.failed = { [weak self] reason in self?.stop(reason: reason) }
        if !launchOptions.isIsolatedFixture {
            hotkeys.onEvent = { [weak self] action, phase in self?.hotkey(action: action, phase: phase) }
            do { try hotkeys.register() }
            catch { channel.send(.makeError(code: "hotkey_registration_failed")) }
            for name in [NSWorkspace.willSleepNotification, NSWorkspace.sessionDidResignActiveNotification] {
                let observer = NSWorkspace.shared.notificationCenter.addObserver(
                    forName: name, object: nil, queue: .main
                ) { [weak self] _ in self?.stop(reason: "session_inactive") }
                workspaceObservers.append(observer)
            }
        }
        let watchdog = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
            guard let self else { return }
            if self.channelHealth.tick(now: ProcessInfo.processInfo.systemUptime) {
                self.stop(reason: "daemon_heartbeat_expired")
            }
            for rejection in self.commandRouter.watchdog(at: ProcessInfo.processInfo.systemUptime) {
                self.channel.reject(rejection.command, code: rejection.code,
                    message: "La operación nativa anterior superó su plazo; vuelve a intentarlo.")
            }
            if !self.latch.stopped && self.microphone.slot.isActive && !self.audioSession.referenceIsHealthy() {
                self.stop(reason: "voice_processing_state_lost")
            }
        }
        RunLoop.main.add(watchdog, forMode: .common)
        self.watchdog = watchdog
        channel.start()
        channel.send(.makeReady(selectedTextAvailable: AXIsProcessTrusted(),
            launchMode: launchOptions.launchModeName,
            isolatedFixture: launchOptions.isIsolatedFixture))
        if !launchOptions.isIsolatedFixture {
            microphoneActivity.changed = { [weak self] applications in
                guard let self, !self.latch.stopped else { return }
                self.channel.send(.makeMicrophoneActivity(applications.map {
                    (pid: $0.applicationPID, bundleID: $0.bundleID, name: $0.name)
                }))
            }
            resumeMicrophoneActivity()
            assistantTrigger.announce()
        }
        diagnostics.recordOffMain("host_ready", [
            "launch_mode": launchOptions.launchModeName,
            "privacy_stopped": latch.stopped ? "true" : "false"
        ])
        publishState()
    }

    private func daemonDisconnected(reason: String) {
        channelHealth.disconnected()
        switch quitPolicy.disconnected(reason: reason) {
        case .stopThenTerminate:
            leaveWithoutDaemon(reason: reason)
        case .terminate:
            terminateAfterDaemon()
        case .none:
            stop(reason: reason)
        }
    }

    /// The daemon that owned this helper closed the pipe — an
    /// orderly shutdown or a crash — and nobody can command it again. It stops
    /// capture, without writing a latch nobody pressed (`StopPolicy`), then leaves;
    /// if that stop does not finish within `orphanExitGrace`, it leaves anyway.
    /// Leaving releases the global shortcuts, so the next helper can register
    /// them.
    private func leaveWithoutDaemon(reason: String) {
        diagnostics.recordOffMain("leaving_without_daemon", ["reason": reason])
        let grace = UInt64(NativeQuitPolicy.orphanExitGrace * 1_000_000_000)
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: grace)
            self.terminateAfterDaemon()
        }
        stop(reason: reason) { _ in
            self.terminateAfterDaemon()
        }
    }

    private func state() -> [String: Any] {
        ["privacy_stopped": latch.stopped, "connected": connected,
         "native_instance_id": windowScreenshots.nativeInstanceID, "privacy_generation": privacyEpoch,
         "selected_text_available": !launchOptions.isIsolatedFixture && AXIsProcessTrusted(),
         "microphone": microphone.slot.isActive, "system_audio": systemAudio.slot.isActive,
         "playing": playback.playing,
         "supported_audio_processing": ["unprocessed", "voice_processing"],
         "voice_processing_available": audioSession.voiceProcessingAvailable,
         "playback_reference_available": audioSession.playbackReferenceAvailable,
         "audio_processing": audioSession.status, "calendar": calendarState()]
    }

    private func publishState() {
        let calendar = MainActor.assumeIsolated {
            calendars?.status() ?? CalendarStatus(authorization: calendarAuthorization())
        }
        channel.send(.makeState(stopped: latch.stopped, connected: connected,
            instanceID: windowScreenshots.nativeInstanceID, generation: privacyEpoch,
            selectedTextAvailable: !launchOptions.isIsolatedFixture && AXIsProcessTrusted(),
            microphone: microphone.slot.isActive, systemAudio: systemAudio.slot.isActive,
            playing: playback.playing, voiceProcessingAvailable: audioSession.voiceProcessingAvailable,
            playbackReferenceAvailable: audioSession.playbackReferenceAvailable,
            audioProcessing: audioSession.statusFields,
            calendar: NativeCalendarStatusResultFields(authorization: calendar.authorization.rawValue,
                physicalReadPending: calendar.physical_read_pending,
                failure: calendar.failure?.rawValue)))
    }

    private func permissions() -> [String: Any] {
        let mic: String
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: mic = "authorized"
        case .denied: mic = "denied"
        case .restricted: mic = "restricted"
        default: mic = "not_determined"
        }
        return ["microphone": mic, "screen": CGPreflightScreenCaptureAccess(),
                "accessibility": AXIsProcessTrusted(), "calendars": calendarAuthorization().rawValue]
    }

    private func receive(_ command: NativeWireCommand) {
        guard launchOptions.permits(command: command.command) else {
            channel.reject(command, code: "isolated_fixture_scope", message: "Operación no permitida en el fixture aislado.")
            return
        }
        switch command {
        case .hello, .heartbeat:
            // A beat also revives a channel the watchdog gave up on. It used to
            // only refresh the timestamp, so one late beat disabled rearming and
            // every permission prompt for the rest of the run.
            channelHealth.observedBeat(at: ProcessInfo.processInfo.systemUptime)
            channel.respond(command, result: state())
        case .status:
            channel.respond(command, result: state().merging(["permissions": permissions()]) { _, rhs in rhs })
        case .calendarDisconnect:
            invalidateCalendars(.invalidated)
            channel.respond(command, result: calendarState())
        // `calendar_select` is gone; decoding refuses it by name.
        case .calendarSources, .calendarSnapshot:
            receiveCalendar(command)
        // Writes share the single EventKit slot and its admission.
        case .calendarSave, .calendarDelete:
            receiveCalendar(command, writes: true)
        case .privacyStop:
            respondAfterStop(command, reason: "daemon_requested", latches: true,
                failureMessage: "Privacidad detenida; no se pudo confirmar todo el audio previo.")
        case .captureStop:
            respondAfterStop(command, reason: StopPolicy.daemonCutReason, latches: false,
                failureMessage: "Captura cortada; no se pudo confirmar todo el audio previo.")
        case .playbackStop:
            cancelPlayback()
            channel.respond(command)
            publishState()
        case .rearm:
            // Only the private pipe of the daemon that launched this helper has
            // rearm authority. Isolated fixtures do not.
            guard launchOptions.isManaged else {
                channel.reject(command, code: "local_rearm_unavailable",
                    message: "El rearme por pipe solo está disponible para el daemon local de Orbit.")
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
                        message: "Orbit no pudo guardar el estado de privacidad en este Mac.")
                }
                self.publishState()
            }
        case .shutdown:
            stop(reason: "shutdown") { [weak self] drained in
                guard let self else { return }
                if drained {
                    self.channel.respond(command, result: ["drain_complete": true])
                    self.finishQuitAfterDrain()
                } else {
                    self.channel.reject(command, code: "audio_drain_incomplete", message: "No se confirma cierre hasta drenar el audio previo.")
                }
            }
        // The folders never take the general lane: an open panel waits for a
        // person and a search walks the disk, and neither may hold back a
        // recording that starts meanwhile. The daemon sends one at a time.
        // Nor do a window's text, which waits on another app, and a photo's
        // JPEG, which decodes a whole image: each is the user's one attachment.
        case .assistantFolderPick, .assistantFolders, .assistantFilesList, .assistantFilesSearch,
             .assistantFilesRead, .windowText, .photoJPEG:
            guard connected else {
                channel.reject(command, code: "daemon_disconnected", message: "Reinicia Orbit para recuperar la conexión local.")
                return
            }
            execute(command, ownsGeneralLane: false)
        case .permissions, .audioStart, .audioPurpose, .audioWakePhrase, .audioStop, .playback,
             .playbackClip, .applicationSources,
             .screenSources, .screenshot, .selectedText:
            guard connected else {
                channel.reject(command, code: "daemon_disconnected", message: "Reinicia Orbit para recuperar la conexión local.")
                return
            }
            switch commandRouter.route(command, at: ProcessInfo.processInfo.systemUptime) {
            case .start:
                execute(command, ownsGeneralLane: true)
            case .queue:
                break
            case .bypass:
                execute(command, ownsGeneralLane: false)
            case .reject(let code):
                let message = code == "native_backpressure"
                    ? "La cola nativa está llena."
                    : "La operación nativa anterior superó su plazo; vuelve a intentarlo."
                channel.reject(command, code: code, message: message)
            }
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
        if command.command != "permissions" && latch.stopped {
            channel.reject(command, code: "privacy_stopped", message: "Orbit está detenido en este Mac.")
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
                        message: "Orbit no puede leer eso en las carpetas del asistente.",
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
                let message = "No se pudo completar la operación nativa."
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
        let next = ownsGeneralLane ? commandRouter.complete(commandID: command.id) : nil
        publishState()
        if let next { receive(next) }
    }

    @MainActor private func perform(_ command: NativeWireCommand) async throws -> [String: Any] {
        switch command {
        case .permissions(let wire):
            // An already-stopped latch permits explicit permission requests;
            // a new stop or disconnection revokes the remaining prompts.
            let permissionEpoch = privacyEpoch
            // Orbit runs as an accessory, so it never becomes the active app on
            // its own and a system prompt it raises can open behind whatever the
            // person is looking at, usually the terminal they just typed in. The
            // dialog then waits unseen and the permission stays undetermined.
            // Only an explicit permission request brings the app forward.
            if launchOptions.isManaged, !(wire.fields.request ?? []).isEmpty {
                NSApplication.shared.activate()
            }
            for permission in wire.fields.request ?? [] {
                guard connected, !quitRequested, privacyEpoch == permissionEpoch else {
                    throw NativeFailure("operation_cancelled_by_stop")
                }
                switch permission {
                case .microphone: _ = await AVCaptureDevice.requestAccess(for: .audio)
                case .screen: _ = CGRequestScreenCaptureAccess()
                case .accessibility:
                    let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true]
                    _ = AXIsProcessTrustedWithOptions(options as CFDictionary)
                case .calendars: _ = try await calendarController().perform(.requestAccess)
                }
            }
            return permissions()
        case .audioStart(let wire):
            let fields = wire.fields
            let id = fields.stream_id
            guard !id.isEmpty, id.utf8.count <= 128 else {
                throw NativeFailure("invalid_stream_id")
            }
            switch fields.source {
            case .micLocal:
                let requested = fields.processing ?? .unprocessed
                guard AudioConfigurationPolicy.permitsCapture(requested: requested, current: audioSession.mode,
                    configured: audioSession.configured, microphoneActive: microphone.slot.isActive,
                    remoteActive: systemAudio.slot.isActive || privacyCleanupInFlight > 0,
                    playbackActive: playback.playing) else { throw NativeFailure("audio_session_busy") }
                let result = try microphone.start(streamID: id, processing: requested)
                microphonePurpose = fields.purpose
                return result
            case .remoteApp:
                guard fields.processing == nil || fields.processing == .unprocessed else {
                    throw NativeFailure("processing_not_supported_for_source")
                }
                guard audioSession.mode != .voiceProcessing || !audioSession.engine.isRunning else {
                    throw NativeFailure("audio_session_busy")
                }
                let captureScope = fields.capture_scope ?? .system
                let applicationPID: Int32?
                switch captureScope {
                case .system:
                    applicationPID = nil
                case .application:
                    guard let pid = fields.application_pid, pid > 0 else {
                        throw NativeFailure("application_pid_required")
                    }
                    applicationPID = pid
                }
                // The application's audio is captured by its PID, the
                // one `microphone_activity` reported; no identity receipt
                // travels with the start (the meeting rules that sent one were
                // retired).
                return try await systemAudio.start(streamID: id, captureScope: captureScope.rawValue,
                    applicationPID: applicationPID,
                    allowStart: { [weak self] in self?.latch.stopped == false && self?.connected == true })
            }
        case .audioPurpose(let wire):
            guard microphone.streamID == wire.fields.stream_id
                || audioRouteRecoveringStreamID == wire.fields.stream_id else {
                throw NativeFailure("stream_not_found")
            }
            microphonePurpose = wire.fields.purpose
            return ["stream_id": wire.fields.stream_id, "purpose": wire.fields.purpose.rawValue]
        case .audioWakePhrase(let wire):
            // «Oye Deimos» goes with the meeting recording, on the
            // live microphone stream whoever opened it: the daemon turns it
            // on when the meeting starts or resumes and off when it pauses
            // or ends, also while a voice note keeps the stream open.
            let id = wire.fields.stream_id
            guard !id.isEmpty, id.utf8.count <= 128 else { throw NativeFailure("invalid_stream_id") }
            if wire.fields.enabled {
                guard microphone.activeStreamID == id else { throw NativeFailure("stream_not_found") }
                let sink = assistantTrigger.start(streamID: id, assistantName: wire.fields.assistant_name ?? "Deimos",
                                                  assistantNames: wire.fields.assistant_names, allow: { [weak self] stream in
                    guard let self else { return false }
                    return !self.latch.stopped && self.microphone.activeStreamID == stream
                })
                do { try microphone.setFrameSink(sink, streamID: id) }
                catch { assistantTrigger.stop(); throw error }
            } else {
                assistantTrigger.stop()
                if microphone.streamID == id { try? microphone.setFrameSink(nil, streamID: id) }
            }
            return ["stream_id": id, "enabled": wire.fields.enabled]
        case .audioStop(let wire):
            let id = wire.fields.stream_id
            if microphone.streamID == id || audioRouteRecoveringStreamID == id {
                audioRouteEpoch &+= 1
                microphonePurpose = nil
                assistantTrigger.stop()
                microphone.disableImmediately()
                audioSession.stopIfIdle()
                guard await microphone.stop() else { throw NativeFailure("audio_drain_incomplete") }
            }
            else if systemAudio.streamID == id {
                guard await systemAudio.stop() else { throw NativeFailure("audio_drain_incomplete") }
            }
            else { throw NativeFailure("stream_not_found") }
            return ["stream_id": id, "drain_complete": true]
        case .playback(let wire):
            guard !audioRouteRecoveryInFlight else { throw NativeFailure("audio_route_recovery_in_progress") }
            return try playback.enqueue(wire.fields)
        case .playbackClip(let wire):
            guard !audioRouteRecoveryInFlight else { throw NativeFailure("audio_route_recovery_in_progress") }
            return try playback.enqueueClip(wire.fields)
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
        case .hello, .heartbeat, .status, .calendarDisconnect,
             .calendarSources, .calendarSnapshot, .calendarSave, .calendarDelete,
             .privacyStop, .captureStop,
             .playbackStop, .rearm, .shutdown:
            throw NativeFailure("unsupported_command")
        }
    }

    private func audioRouteChanged(previous: AudioRouteDevices?, current: AudioRouteDevices) {
        let decision = AudioRouteChangePolicy.decide(stopped: latch.stopped,
            closing: !connected || quitRequested || privacyCleanupInFlight > 0,
            microphone: microphone.slot, processing: audioSession.mode, playback: playback.playing,
            routeChanged: current.changed(from: previous), purpose: microphonePurpose)
        let action: String
        let reason: String
        switch decision {
        case .none(let cause): action = "none"; reason = cause
        case .stop(let cause): action = "stop"; reason = cause
        case .cancelPlayback: action = "cancel_playback"; reason = "audio_device_changed"
        case .restartMicrophone: action = "restart_microphone"; reason = "audio_device_changed_meeting"
        }
        diagnostics.recordOffMain("audio_configuration_changed", [
            "previous_input": previous.flatMap { $0.input }.map { String($0) } ?? "unknown",
            "previous_output": previous.flatMap { $0.output }.map { String($0) } ?? "unknown",
            "current_input": current.input.map { String($0) } ?? "unknown",
            "current_output": current.output.map { String($0) } ?? "unknown",
            "action": action, "reason": reason,
            "purpose": microphonePurpose?.rawValue ?? "unknown"
        ])
        switch decision {
        case .none:
            return
        case .stop(let reason):
            stop(reason: reason)
        case .cancelPlayback:
            channel.send(.makeError(code: "playback_cancelled_audio_device_changed"))
            cancelPlayback()
            audioSession.stopIfIdle()
            publishState()
        case .restartMicrophone:
            guard let streamID = microphone.streamID, let sampleRate = microphone.sampleRate else {
                stop(reason: "audio_route_stream_identity_missing")
                return
            }
            let epoch = privacyEpoch
            let routeEpoch = audioRouteEpoch
            let wasListening = assistantTrigger.isListening
            // Reuse the existing cleanup admission: a privacy stop during the
            // drain still runs in full and invalidates this epoch before restart.
            audioRouteRecoveringStreamID = streamID
            privacyCleanupInFlight += 1
            assistantTrigger.stop()
            microphone.disableImmediately()
            if playback.playing { channel.send(.makeError(code: "playback_cancelled_audio_device_changed")) }
            cancelPlayback()
            audioSession.stop()
            Task { @MainActor in
                defer {
                    self.audioRouteRecoveringStreamID = nil
                    self.privacyCleanupInFlight -= 1
                    self.publishState()
                }
                let stopped = await self.microphone.stopForRouteChange()
                let action = AudioRouteChangePolicy.decide(stopped: self.latch.stopped,
                    closing: !self.connected || self.quitRequested || self.privacyEpoch != epoch
                        || self.audioRouteEpoch != routeEpoch,
                    microphone: self.microphone.slot, processing: .unprocessed, playback: false,
                    failure: stopped.drained ? nil : "audio_route_drain_incomplete", recovering: true,
                    routeChanged: true, purpose: self.microphonePurpose)
                switch action {
                case .none:
                    return
                case .stop(let reason):
                    self.stop(reason: reason)
                case .cancelPlayback:
                    self.cancelPlayback()
                case .restartMicrophone:
                    do {
                        _ = try self.microphone.start(streamID: streamID, processing: .unprocessed,
                            outputRate: sampleRate, routeResume: stopped.resume)
                        if wasListening {
                            let sink = self.assistantTrigger.start(streamID: streamID, allow: { [weak self] stream in
                                guard let self else { return false }
                                return !self.latch.stopped && self.microphone.activeStreamID == stream
                            })
                            try self.microphone.setFrameSink(sink, streamID: streamID)
                        }
                    } catch {
                        self.stop(reason: reportAudioRouteFailure(error, channel: self.channel))
                    }
                }
            }
        }
    }

    private func screenAuthorization() -> (generation: UInt64, stopped: Bool) {
        (privacyEpoch, latch.stopped || !connected || privacyCleanupInFlight > 0 || quitRequested)
    }

    private func calendarAuthorization() -> CalendarAuthorization {
        launchOptions.isIsolatedFixture ? .unavailable : EventKitCalendarReader.authorization()
    }

    private func calendarState() -> [String: Any] {
        // AppKit delegate, native channel delivery and this host's run-loop timers
        // run on Main. Keep the existing unannotated host boundary explicit without
        // moving any calendar work onto Main or changing unrelated callback types.
        MainActor.assumeIsolated {
            let value = calendars?.status() ?? CalendarStatus(authorization: calendarAuthorization())
            return (try? CalendarBridge.fields(value)) ?? ["authorization": "unknown",
                "physical_read_pending": calendars != nil, "failure": "calendar_read_failed"]
        }
    }

    private func invalidateCalendars(_ reason: CalendarFailure) {
        MainActor.assumeIsolated { calendars?.invalidate(reason) }
    }

    @MainActor private func calendarController() -> CalendarController {
        if let calendars { return calendars }
        let value = CalendarController(nativeInstanceID: windowScreenshots.nativeInstanceID,
            backend: EventKitCalendarReader(), clock: { CaptureClock.currentHostNanoseconds() },
            unixClock: { CalendarBridge.unixMilliseconds() },
            authorization: { EventKitCalendarReader.authorization() },
            host: { [weak self] in self?.screenAuthorization() ?? (generation: 0, stopped: true) })
        value.invalidated = { [weak self] reason in
            self?.channel.send(.makeCalendarInvalidated(code: reason.rawValue))
        }
        calendars = value
        return value
    }

    private func receiveCalendar(_ command: NativeWireCommand, writes: Bool = false) {
        guard connected, !screenAuthorization().stopped else {
            // The same code covers the user's latch and a stop still cleaning up;
            // the message says which, and only the user's latch needs a rearm.
            let message = latch.stopped
                ? "Orbit está detenido por tu parada de privacidad; rearma para usar los calendarios."
                : "Orbit está terminando de parar la captura o no está conectado; vuelve a intentarlo en unos segundos."
            channel.reject(command, code: "privacy_stopped", message: message)
            return
        }
        guard !calendarCommandInFlight,
              MainActor.assumeIsolated({ calendars?.status().physical_read_pending != true }) else {
            channel.reject(command, code: "calendar_busy", message: "La operación anterior de EventKit aún no ha terminado.")
            return
        }
        // Capture the calendar generation synchronously, before scheduling work.
        // A calendar-only disconnect does not increment the audio/privacy epoch.
        let controller = MainActor.assumeIsolated { calendarController() }
        let ticket = MainActor.assumeIsolated { controller.admitCommand() }
        calendarCommandInFlight = true
        let generation = privacyEpoch
        // Its single physical slot is separate from audio's command queue. Status,
        // disconnect and physical privacy stop never wait for EventKit to return.
        Task { @MainActor in
            defer { self.calendarCommandInFlight = false }
            if writes {
                await self.performCalendarWrite(command, controller: controller, ticket: ticket,
                                                generation: generation)
                self.publishState()
                return
            }
            do {
                guard self.connected, self.privacyEpoch == generation, !self.screenAuthorization().stopped else {
                    throw CalendarFailure.invalidated
                }
                let result = try await controller.perform(CalendarBridge.operation(command), admitted: ticket)
                guard self.connected, self.privacyEpoch == generation,
                      controller.permitsCommand(ticket),
                      controller.permitsDelivery(result) else { throw CalendarFailure.invalidated }
                self.channel.respond(command, result: try CalendarBridge.fields(result))
            } catch {
                let code = (error as? CalendarFailure)?.rawValue ?? "calendar_read_failed"
                self.channel.reject(command, code: code, message: "No se pudo leer el calendario; no se ha devuelto una agenda vacía.")
            }
            self.publishState()
        }
    }

    /// One calendar write, answered with what EventKit did. A refusal travels
    /// with its numbers in `error.detail`. Before the controller takes the
    /// slot nothing was written, so a stop says `privacy_stopped`; once it
    /// may have been, a withheld answer says the outcome is unknown.
    @MainActor private func performCalendarWrite(_ command: NativeWireCommand, controller: CalendarController,
                                                 ticket: CalendarCommandTicket, generation: UInt64) async {
        let operation: CalendarOperation
        do { operation = try CalendarBridge.operation(command) }
        catch {
            channel.reject(command, code: CalendarFailure.invalidRequest.rawValue,
                message: "La orden de escritura del calendario no tiene la forma del protocolo; no se cambió nada.")
            return
        }
        guard connected, privacyEpoch == generation, !screenAuthorization().stopped else {
            channel.reject(command, code: "privacy_stopped",
                message: "Orbit se detuvo antes de escribir; no se cambió nada.")
            return
        }
        let result: CalendarResult
        do {
            result = try await controller.perform(operation, admitted: ticket)
        } catch let refusal as CalendarWriteRefusal {
            rejectCalendarWrite(command, refusal)
            return
        } catch {
            // Refused before the slot was taken or any commit asked for.
            let code = (error as? CalendarFailure)?.rawValue ?? CalendarFailure.invalidRequest.rawValue
            channel.reject(command, code: code, message: "No se escribió en el calendario; no se cambió nada.")
            return
        }
        // Each withheld answer names its own cause; the write is done.
        if !connected {
            rejectCalendarWrite(command, .outcomeUnknown(cause: "daemon_disconnected"))
            return
        }
        if privacyEpoch != generation {
            rejectCalendarWrite(command, .outcomeUnknown(cause: "privacy_generation_changed"))
            return
        }
        if let cause = controller.writeDeliveryRefusal(result) {
            rejectCalendarWrite(command, .outcomeUnknown(cause: cause))
            return
        }
        guard let fields = try? CalendarBridge.fields(result) else {
            rejectCalendarWrite(command, .outcomeUnknown(cause: "result_not_encodable"))
            return
        }
        channel.respond(command, result: fields)
    }

    private func rejectCalendarWrite(_ command: NativeWireCommand, _ refusal: CalendarWriteRefusal) {
        if !channel.send(.makeRejection(id: command.id, code: refusal.code, message: refusal.message,
                                        detail: refusal.detail)) {
            FileHandle.standardError.write(Data("native_rejection_not_admitted event=response\n".utf8))
        }
    }

    private func hotkey(action: String, phase: String) {
        if action == "privacy_stop" && phase == "pressed" { stop(reason: "local_hotkey") }
        if action == "ptt" && phase == "pressed" { cancelPlayback() }
        var applications: [(pid: Int32, bundleID: String, name: String)]?
        if action == "meeting_toggle" && phase == "pressed" {
            // A meeting started from the keyboard used to carry no application,
            // so it recorded the microphone and nothing else: the other voices
            // were simply absent from the transcript, discovered afterwards.
            // The list travels with the press; which one to record is decided in
            // orbit-core, so the rule lives in one language only.
            applications = MeetingApplications.reportable(
                NSWorkspace.shared.runningApplications.map {
                    (pid: $0.processIdentifier,
                     bundleID: $0.bundleIdentifier,
                     name: $0.localizedName,
                     regular: $0.activationPolicy == .regular,
                     terminated: $0.isTerminated)
                }
            ).map { (pid: $0.applicationPID, bundleID: $0.bundleID, name: $0.name) }
        }
        channel.send(.makeHotkey(action: action, phase: phase, applications: applications))
    }

    private func stopWork(latches: Bool) -> PrivacyStopWork {
        PrivacyStopWork(
            latchAlreadyStopped: latch.stopped,
            microphoneSlot: microphone.slot,
            systemAudioSlot: systemAudio.slot,
            playbackLive: playback.playing,
            cleanupInFlight: privacyCleanupInFlight,
            latches: latches)
    }

    /// Stops capture and playback for `reason`. Only the user's own brake writes
    /// the durable latch (`StopPolicy`): every other reason leaves it
    /// as it was, so a sleeping Mac, a changed audio route, a silent service
    /// or a closing never leaves Orbit stopped after a restart. The event says
    /// which of the two happened: `privacy_stop` or `capture_stopped`.
    private func stop(reason: String, completion: ((Bool) -> Void)? = nil) {
        let latches = StopPolicy.latches(reason: reason)
        // The reason is the single most useful line in the log: it separates a
        // deliberate stop from a dead daemon, a sleeping Mac or a lost device.
        diagnostics.recordOffMain(latches ? "privacy_stopped" : "capture_stopped", ["reason": reason])
        // «Oye Deimos» stops first, with every stop — the user's brake
        // included — even when nothing else is live (a listener can still be
        // preparing its model).
        assistantTrigger.stop()
        let work = stopWork(latches: latches)
        if work.isRedundant {
            // Nothing is live and nothing is left to persist. Running the full
            // stop would bump privacyEpoch, and a pending permission request
            // compares that epoch before handing macOS's answer back, so this
            // silently cancelled the TCC dialog the person was reading. It also
            // reported an incomplete drain, which the daemon treats as a failure
            // and answers with another stop: that is how it became a loop.
            channel.send(.makeStop(latches: latches, reason: reason, stopped: latch.stopped,
                drainComplete: true))
            completion?(work.reportsDrained)
            publishState()
            return
        }
        privacyEpoch &+= 1
        microphonePurpose = nil
        if latches { latch.beginStop() }
        invalidateCalendars(latches ? .privacyStopped : .invalidated)
        systemAudio.disableImmediately()
        microphone.disableImmediately()
        cancelPlayback()
        audioSession.stop()
        windowScreenshots.invalidate()
        applicationIdentities.invalidate()
        // The user's brake silences the microphone observer too; any other stop
        // (sleep, a changed device) leaves it watching for the next call.
        if latches { microphoneActivity.stop() }
        channel.send(.makeStop(latches: latches, reason: reason, stopped: latch.stopped,
            drainComplete: false))
        privacyCleanupInFlight += 1
        Task { @MainActor in
            async let latchSaved = self.persistLatchIfUserBrake(latches)
            let drained = await self.privacyDrainBarrier.complete { @MainActor in
                async let microphoneDrained = self.microphone.stop()
                async let systemDrained = self.systemAudio.stop()
                let results = await (microphoneDrained, systemDrained)
                return results.0 && results.1
            }
            let persisted = await latchSaved
            self.privacyCleanupInFlight -= 1
            self.publishState()
            completion?(drained && persisted)
        }
        publishState()
    }

    /// Starts watching which applications hold the microphone, unless the
    /// user's brake is on. The first answer goes out at once, so the daemon never
    /// reads the silence before it as "no call".
    private func resumeMicrophoneActivity() {
        guard !launchOptions.isIsolatedFixture, !latch.stopped else { return }
        microphoneActivity.start()
    }

    /// Writes the durable latch only for the user's brake; any other stop has
    /// nothing to persist.
    private func persistLatchIfUserBrake(_ latches: Bool) async -> Bool {
        guard latches else { return true }
        return await persistPrivacyStop()
    }

    private func persistPrivacyStop() async -> Bool {
        do {
            try await latch.persistStopOffMain()
            return true
        } catch {
            channel.send(.makeError(code: "privacy_stop_persistence_failed"))
            return false
        }
    }

    private func rearmReadiness() -> RearmReadiness {
        RearmReadiness(
            connected: connected, quitRequested: quitRequested,
            commandInFlight: commandRouter.hasCommandInFlight, cleanupInFlight: privacyCleanupInFlight,
            microphoneSlot: microphone.slot,
            systemAudioSlot: systemAudio.slot)
    }

    private func cancelPlayback() {
        playbackEpoch &+= 1
        let cancelled = commandRouter.removeQueued {
            $0.command == "playback" || $0.command == "playback_clip"
        }
        for command in cancelled { channel.respond(command, result: ["state": "cancelled"]) }
        playback.stop()
    }
    private func finishQuitAfterDrain() {
        let requestID = UUID()
        guard quitPolicy.beginFlush(id: requestID) else { return }
        Task { @MainActor in
            let flushed = await self.channel.flush()
            switch self.quitPolicy.completeFlush(id: requestID, succeeded: flushed) {
            case .terminate, .stopThenTerminate:
                self.terminateAfterDaemon()
            case .none:
                break
            }
        }
    }

    // AppKit registers this process in the containing app's identity too. A
    // quit Apple Event addressed to Orbit may therefore land here (2026-10-18).
    // Keep the helper alive for the daemon's drain; SIGTERM already enters its
    // ordered closure, and that closure sends us `shutdown` over the pipe.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard !terminatingForDaemon else { return .terminateNow }
        guard !parentQuitRequested else { return .terminateLater }
        guard daemonPID > 1, getppid() == daemonPID else {
            daemonDisconnected(reason: "daemon_parent_gone")
            return .terminateCancel
        }
        if kill(daemonPID, SIGTERM) == 0 {
            parentQuitRequested = true
            diagnostics.recordOffMain("quit_forwarded_to_daemon", ["parent_pid": String(daemonPID)])
            return .terminateLater
        } else {
            let code = errno
            diagnostics.recordOffMain("quit_parent_signal_failed", [
                "parent_pid": String(daemonPID), "errno": String(code)
            ])
        }
        return .terminateCancel
    }

    private func terminateAfterDaemon() {
        terminatingForDaemon = true
        if parentQuitRequested {
            NSApplication.shared.reply(toApplicationShouldTerminate: true)
        } else {
            NSApplication.shared.terminate(nil)
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        diagnostics.recordAndWaitOffMain("host_terminating", ["connected": connected ? "true" : "false"])
        watchdog?.invalidate()
        invalidateCalendars(.invalidated)
        systemAudio.disableImmediately()
        microphone.disableImmediately()
        playback.stop()
        audioSession.stop()
        // The shortcuts go explicitly, not only with the process, so the next
        // helper can register them.
        hotkeys.unregister()
    }
}
