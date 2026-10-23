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
    /// Watches this main thread from a queue of its own and logs
    /// `helper_main_thread_stalled` with the command it had started.
    private let mainThreadWatch: MainThreadHangDetector
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
    /// The calendar's commands, answered off this main thread.
    private var calendars: CalendarHost?
    private lazy var applicationIdentities = ApplicationIdentityCatalog(nativeInstanceID: windowScreenshots.nativeInstanceID)
    private lazy var microphone = MicrophoneCapture(session: audioSession, channel: channel)
    private lazy var playback = AudioPlayback(session: audioSession, channel: channel)
    private lazy var systemAudio = SystemAudioCapture(channel: channel)
    private var watchdog: Timer?
    /// The daemon's beats, observed on the channel's reader queue and judged
    /// on a queue of its own: a held main thread is never its silence.
    private lazy var liveness = DaemonLivenessWatch(
        fellSilent: { [weak self, diagnostics = self.diagnostics] silence in
            // On the liveness queue: a capture in progress goes on without
            // the main thread, which is only asked when nothing is captured.
            if silence.capturing {
                diagnostics.recordOffMain("daemon_silent_capture_kept", [
                    "microphone": silence.microphone ? "true" : "false",
                    "system_audio": silence.systemAudio ? "true" : "false"
                ])
                return
            }
            Task { @MainActor in self?.daemonFellSilent() }
        },
        lateTick: { [diagnostics = self.diagnostics] gap in
            // This process was held: the next incident says whose stall it was.
            diagnostics.recordOffMain("helper_watchdog_late", ["tick_gap_ms": String(Int(gap * 1000))])
        })
    /// Read-only on purpose: liveness is decided by observed traffic in
    /// `liveness`, never by a caller flipping a flag one way.
    private var connected: Bool { liveness.connected }
    private var commandRouter = NativeCommandRouter()
    private var privacyEpoch: UInt64 = 0
    private var playbackEpoch: UInt64 = 0
    private var microphonePurpose: NativeMicrophonePurpose?
    /// The microphone feeds a meeting: it was opened for one or told it is
    /// one. A voice note dictated over it does not end the meeting.
    private var microphoneMeeting = false
    /// How the system's audio was opened, to open it again after a failure.
    private var systemAudioScope: (captureScope: String, applicationPID: Int32?)?
    /// The system audio stream already restarted once; a second failure ends it.
    private var systemAudioRestartedStreamID: String?
    /// The restart in progress, owned by whichever ends it first.
    private var systemAudioRestart: (attempt: UUID, streamID: String)?
    /// A track that failed alone and was declared stopped. Its stream no
    /// longer exists here; the daemon's `audio_stop` for it is answered as
    /// drained, since its loss was already declared with a gap.
    private var endedMicrophoneStreamID: String?
    private var endedSystemAudioStreamID: String?
    private lazy var captureActivity = CaptureActivity(
        begin: {
            ProcessInfo.processInfo.beginActivity(options: .userInitiatedAllowingIdleSystemSleep,
                                                  reason: "Recording a meeting or a voice note")
        },
        end: { ProcessInfo.processInfo.endActivity($0) })
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
        self.mainThreadWatch = MainThreadHangDetector(report: { stall in
            diagnostics.recordOffMain("helper_main_thread_stalled", stall.fields)
        })
        // Only the daemon's own child leaves when its daemon is gone.
        self.quitPolicy = NativeQuitPolicy()
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        mainThreadWatch.start()
        // Its first look at the permission runs off Main before the daemon's hello.
        _ = calendarState()
        // Every command read from the pipe is the daemon answering, not only
        // a beat, and it revives a channel the watchdog gave up on (a beat
        // used to only refresh the timestamp, so one late beat disabled
        // rearming and every permission prompt for the rest of the run). A
        // capture kept through the daemon's silence must take the
        // `audio_stop` that ends it even before the next beat. It is observed
        // where it is read, before the main thread runs it.
        channel.commandRead = { [liveness = self.liveness] in liveness.observedBeat() }
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
        microphone.failed = { [weak self] reason in self?.trackFailed(.microphone, reason: reason) }
        systemAudio.failed = { [weak self] reason in self?.trackFailed(.systemAudio, reason: reason) }
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
        liveness.start()
        let watchdog = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
            guard let self else { return }
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

    /// The daemon stopped beating while this helper was awake, and the
    /// liveness queue found nothing being captured. A recording or a voice
    /// note in progress keeps capturing, decided there without this thread:
    /// the channel keeps on this Mac what the daemon does not read and
    /// delivers it in order when it reads again (2026-10-22: a stalled daemon
    /// cut a meeting 25 minutes in). The capture is checked once more here,
    /// in case one started meanwhile.
    private func daemonFellSilent() {
        guard DaemonChannelHealth.silenceStopsCapture(microphone: microphone.slot,
                                                      systemAudio: systemAudio.slot) else {
            diagnostics.recordOffMain("daemon_silent_capture_kept", [
                "microphone": microphone.slot.isActive ? "true" : "false",
                "system_audio": systemAudio.slot.isActive ? "true" : "false"
            ])
            return
        }
        stop(reason: "daemon_heartbeat_expired")
    }

    private func daemonDisconnected(reason: String) {
        liveness.disconnected()
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
        let capturing = microphone.slot.isActive || systemAudio.slot.isActive
        mainThreadWatch.captureChanged(capturing)
        liveness.captureChanged(microphone: microphone.slot.isActive,
                                systemAudio: systemAudio.slot.isActive)
        if captureActivity.update(capturing: capturing) {
            diagnostics.recordOffMain(captureActivity.held ? "helper_activity_held" : "helper_activity_released")
        }
        let calendar = MainActor.assumeIsolated { calendarHost().status() }
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
        // Its kind only: what the main thread was doing if it is held next.
        mainThreadWatch.commandStarted(command.command)
        guard launchOptions.permits(command: command.command) else {
            channel.reject(command, code: "isolated_fixture_scope", message: "Operación no permitida en el fixture aislado.")
            return
        }
        switch command {
        case .hello, .heartbeat:
            channel.respond(command, result: state())
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
                failureMessage: "Todo está parado, pero no se pudo confirmar todo el audio previo.")
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
                    message: "Solo el daemon local de Orbit puede reactivarlo, por su canal privado.")
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
                case .calendars: try await calendarHost().requestAccess()
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
                microphoneMeeting = fields.purpose == .meeting
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
                let result = try await systemAudio.start(streamID: id, captureScope: captureScope.rawValue,
                    applicationPID: applicationPID,
                    allowStart: { [weak self] in self?.latch.stopped == false && self?.connected == true })
                systemAudioScope = (captureScope: captureScope.rawValue, applicationPID: applicationPID)
                return result
            }
        case .audioPurpose(let wire):
            guard microphone.streamID == wire.fields.stream_id
                || audioRouteRecoveringStreamID == wire.fields.stream_id else {
                throw NativeFailure("stream_not_found")
            }
            microphonePurpose = wire.fields.purpose
            if wire.fields.purpose == .meeting { microphoneMeeting = true }
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
            // The person's stop wins over a restart of the system's audio.
            let wasRestarting = systemAudioRestart?.streamID == id
            if wasRestarting { systemAudioRestart = nil }
            if microphone.streamID == id || audioRouteRecoveringStreamID == id {
                audioRouteEpoch &+= 1
                microphonePurpose = nil
                microphoneMeeting = false
                assistantTrigger.stop()
                microphone.disableImmediately()
                audioSession.stopIfIdle()
                guard await microphone.stop() else { throw NativeFailure("audio_drain_incomplete") }
            }
            else if systemAudio.streamID == id {
                guard await systemAudio.stop() else { throw NativeFailure("audio_drain_incomplete") }
            }
            // Between the old stream and the new one of a restart.
            else if wasRestarting { _ = await systemAudio.stop() }
            else if endedMicrophoneStreamID == id { endedMicrophoneStreamID = nil }
            else if endedSystemAudioStreamID == id { endedSystemAudioStreamID = nil }
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
        // A silent daemon is not a closing: a meeting kept through its silence
        // follows the new device too. A pipe that is gone has already stopped
        // the capture, and that stop's cleanup or epoch is what closes here.
        let decision = AudioRouteChangePolicy.decide(stopped: latch.stopped,
            closing: quitRequested || privacyCleanupInFlight > 0,
            microphone: microphone.slot, processing: audioSession.mode, playback: playback.playing,
            routeChanged: current.changed(from: previous), purpose: microphonePurpose,
            meetingUnderneath: microphoneMeeting || systemAudio.slot.isActive)
        let action: String
        let reason: String
        switch decision {
        case .none(let cause): action = "none"; reason = cause
        case .stop(let cause): action = "stop"; reason = cause
        case .cancelPlayback: action = "cancel_playback"; reason = "audio_device_changed"
        case .restartMicrophone: action = "restart_microphone"; reason = "audio_device_changed_meeting"
        case .restartMicrophoneCancellingVoiceNote:
            action = "restart_microphone_cancel_voice_note"; reason = "audio_device_changed_voice_note_over_meeting"
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
        case .restartMicrophone, .restartMicrophoneCancellingVoiceNote:
            // The voice note ends with the route gap the new stream opens
            // with (the daemon cancels it and says so); the meeting goes on.
            if decision == .restartMicrophoneCancellingVoiceNote { microphonePurpose = .meeting }
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
                    closing: self.quitRequested || self.privacyEpoch != epoch
                        || self.audioRouteEpoch != routeEpoch,
                    microphone: self.microphone.slot, processing: .unprocessed, playback: false,
                    failure: stopped.drained ? nil : "audio_route_drain_incomplete", recovering: true,
                    routeChanged: true, purpose: self.microphonePurpose,
                    meetingUnderneath: self.microphoneMeeting || self.systemAudio.slot.isActive)
                switch action {
                case .none:
                    return
                case .stop(let reason):
                    self.stop(reason: reason)
                case .cancelPlayback:
                    self.cancelPlayback()
                case .restartMicrophone, .restartMicrophoneCancellingVoiceNote:
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
                        // The microphone did not come back: with the system's
                        // audio still recording, only the microphone ends.
                        self.trackFailed(.microphone, streamID: streamID,
                                         reason: reportAudioRouteFailure(error, channel: self.channel))
                    }
                }
            }
        }
    }

    private func screenAuthorization() -> (generation: UInt64, stopped: Bool) {
        (privacyEpoch, latch.stopped || !connected || privacyCleanupInFlight > 0 || quitRequested)
    }

    /// The permission as the calendar last saw it; EventKit is asked off Main.
    private func calendarAuthorization() -> CalendarAuthorization {
        MainActor.assumeIsolated { calendarHost().status().authorization }
    }

    private func calendarState() -> [String: Any] {
        // AppKit delegate, native channel delivery and this host's run-loop timers
        // run on Main. Keep the existing unannotated host boundary explicit; the
        // calendar's own work runs on its actor.
        MainActor.assumeIsolated { calendarHost().stateFields() }
    }

    /// A stop, a lost daemon or quitting: the reply in flight is retired on
    /// the calendar's actor; this thread does not wait for it.
    private func invalidateCalendars(_ reason: CalendarFailure) {
        MainActor.assumeIsolated { calendars?.invalidate(reason) }
    }

    @MainActor private func calendarHost() -> CalendarHost {
        if let calendars { return calendars }
        let facts: @MainActor @Sendable () -> CalendarHostFacts = { [weak self] in
            self?.calendarFacts() ?? CalendarHostFacts(connected: false, generation: 0, stopped: true, latched: false)
        }
        let controller = CalendarHost.controller(nativeInstanceID: windowScreenshots.nativeInstanceID,
            isolatedFixture: launchOptions.isIsolatedFixture, channel: channel, facts: facts)
        let value = CalendarHost(controller: controller, channel: channel, facts: facts,
            changed: { [weak self] in self?.publishState() })
        calendars = value
        return value
    }

    private func calendarFacts() -> CalendarHostFacts {
        let current = screenAuthorization()
        return CalendarHostFacts(connected: connected, generation: current.generation,
            stopped: current.stopped, latched: latch.stopped)
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

    private func trackFailed(_ track: CaptureTrack, reason: String) {
        let current = track == .microphone ? microphone.streamID : systemAudio.streamID
        guard let streamID = current else { stop(reason: reason); return }
        trackFailed(track, streamID: streamID, reason: reason)
    }

    /// One track failed. It used to stop both (`stop(reason:)`); now only
    /// that track ends, or the system's audio is started again once, while
    /// the other keeps recording. With nothing else live it stops as before.
    private func trackFailed(_ track: CaptureTrack, streamID: String, reason: String) {
        let other = track == .microphone ? systemAudio.slot : microphone.slot
        let action = TrackFailurePolicy.decide(track: track, otherTrack: other,
            closing: latch.stopped || quitRequested,
            restartAttempted: systemAudioRestartedStreamID == streamID)
        let name: String
        switch action {
        case .stopCapture: name = "stop_capture"
        case .restartSystemAudio: name = "restart_system_audio"
        case .endTrack: name = "end_track"
        }
        diagnostics.recordOffMain("capture_track_failed", [
            "track": track.rawValue, "reason": reason, "action": name
        ])
        switch action {
        case .stopCapture:
            stop(reason: reason)
        case .endTrack:
            endTrack(track, streamID: streamID, cause: reason)
        case .restartSystemAudio:
            restartSystemAudio(streamID: streamID, cause: reason)
        }
    }

    /// Ends one track: whatever it captured is drained, then an error names
    /// the cause and a `gap` says the track stopped (`track_stopped`). The daemon
    /// keeps the recording with the other track and says which one stopped.
    private func endTrack(_ track: CaptureTrack, streamID: String, cause: String) {
        switch track {
        case .microphone:
            endedMicrophoneStreamID = streamID
            audioRouteEpoch &+= 1
            microphonePurpose = nil
            microphoneMeeting = false
            assistantTrigger.stop()
            microphone.disableImmediately()
            audioSession.stopIfIdle()
        case .systemAudio:
            endedSystemAudioStreamID = streamID
            systemAudioScope = nil
            systemAudio.disableImmediately()
        }
        Task { @MainActor in
            switch track {
            case .microphone: _ = await self.microphone.stop()
            case .systemAudio: _ = await self.systemAudio.stop()
            }
            // The cause first: the daemon's words about the stopped track,
            // said on the gap, are then what the person reads last.
            self.channel.send(.makeError(code: cause))
            self.channel.send(.makeGap(source: track.rawValue, streamID: streamID, reason: "track_stopped",
                                       frames: 0, durationUnknown: true))
            self.publishState()
        }
    }

    /// The system's audio stream stopped (the app that holds it, a display
    /// change): it is started again, once, under the same stream id, within
    /// `TrackFailurePolicy.systemAudioRestartBound`. The daemon hears the
    /// restart before the new stream's first frame, whose sequence starts
    /// over. If it does not come back, the track ends with its cause.
    private func restartSystemAudio(streamID: String, cause: String) {
        guard let scope = systemAudioScope else {
            endTrack(.systemAudio, streamID: streamID, cause: cause)
            return
        }
        let attempt = UUID()
        let epoch = privacyEpoch
        systemAudioRestartedStreamID = streamID
        systemAudioRestart = (attempt: attempt, streamID: streamID)
        systemAudio.disableImmediately()
        Task { @MainActor in
            _ = await self.systemAudio.stop()
            guard self.systemAudioRestart?.attempt == attempt else { return }
            self.channel.send(.makeGap(source: CaptureTrack.systemAudio.rawValue, streamID: streamID,
                                       reason: "system_audio_restarted", frames: 0, durationUnknown: true))
            let bound = UInt64(TrackFailurePolicy.systemAudioRestartBound * 1_000_000_000)
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: bound)
                guard self.systemAudioRestart?.attempt == attempt else { return }
                self.systemAudioRestart = nil
                self.endTrack(.systemAudio, streamID: streamID, cause: "system_audio_restart_timed_out")
            }
            do {
                _ = try await self.systemAudio.start(streamID: streamID, captureScope: scope.captureScope,
                    applicationPID: scope.applicationPID,
                    allowStart: { [weak self] in
                        self?.latch.stopped == false && self?.privacyEpoch == epoch
                            && self?.systemAudioRestart?.attempt == attempt
                    })
                guard self.systemAudioRestart?.attempt == attempt else { return }
                self.systemAudioRestart = nil
                self.systemAudioScope = scope
                self.diagnostics.recordOffMain("system_audio_restarted", ["cause": cause])
            } catch {
                guard self.systemAudioRestart?.attempt == attempt else { return }
                self.systemAudioRestart = nil
                self.diagnostics.recordOffMain("system_audio_restart_failed", [
                    "cause": cause, "code": (error as? NativeFailure)?.code ?? "system_audio_restart_failed"
                ])
                self.endTrack(.systemAudio, streamID: streamID, cause: cause)
            }
            self.publishState()
        }
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
        microphoneMeeting = false
        systemAudioRestart = nil
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
