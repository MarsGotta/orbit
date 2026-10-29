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
    let channel: NativeChannel
    let latch: PrivacyLatch
    let launchOptions: NativeLaunchOptions
    let diagnostics: DiagnosticLog
    /// Watches this main thread from a queue of its own and logs
    /// `helper_main_thread_stalled` with the command it had started.
    let mainThreadWatch: MainThreadHangDetector
    let audioSession = SharedAudioSession()
    private let hotkeys = GlobalHotkeys()
    private var shortcutFailures = [NativeShortcutFailure]()
    lazy var windowScreenshots = WindowScreenshots(diagnostics: diagnostics)
    /// The assistant's folders: the panel, their bookmarks, read-only reads.
    let assistantFiles = AssistantFiles()
    let windowText = WindowTextReading()
    let photoJPEG = PhotoJPEG()
    let microphoneActivity = MicrophoneActivityObserver()
    /// «Oye Deimos» on the meeting's microphone.
    lazy var assistantTrigger = AssistantVoiceTrigger(channel: channel)
    /// The calendar's commands, answered off this main thread.
    var calendars: CalendarHost?
    lazy var applicationIdentities = ApplicationIdentityCatalog(nativeInstanceID: windowScreenshots.nativeInstanceID)
    lazy var microphone = MicrophoneCapture(session: audioSession, channel: channel)
    lazy var playback = AudioPlayback(session: audioSession, channel: channel)
    lazy var systemAudio = SystemAudioCapture(channel: channel)
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
    var connected: Bool { liveness.connected }
    var commandRouter = NativeCommandRouter()
    var nativeOperations = 0
    var privacyEpoch: UInt64 = 0
    var playbackEpoch: UInt64 = 0
    var microphonePurpose: NativeMicrophonePurpose?
    /// The microphone feeds a meeting, as the daemon last said who holds it
    /// (`audio_start`, `audio_purpose`). A voice note dictated over a meeting
    /// does not end it; a note that outlived its meeting is a note alone.
    var microphoneMeeting = false
    /// How the system's audio was opened, to open it again after a failure.
    var systemAudioScope: (captureScope: String, applicationPID: Int32?)?
    /// The system audio stream already restarted once; a second failure ends it.
    var systemAudioRestartedStreamID: String?
    /// The restart in progress, owned by whichever ends it first.
    var systemAudioRestart: (attempt: UUID, streamID: String)?
    /// A track that failed alone and was declared stopped. Its stream no
    /// longer exists here; the daemon's `audio_stop` for it is answered as
    /// drained, since its loss was already declared with a gap.
    var endedMicrophoneStreamID: String?
    var endedSystemAudioStreamID: String?
    private lazy var captureActivity = CaptureActivity(
        begin: {
            ProcessInfo.processInfo.beginActivity(options: .userInitiatedAllowingIdleSystemSleep,
                                                  reason: "Recording a meeting or a voice note")
        },
        end: { ProcessInfo.processInfo.endActivity($0) })
    var audioRouteEpoch: UInt64 = 0
    var audioRouteRecoveringStreamID: String?
    var audioRouteRecoveryInFlight: Bool { audioRouteRecoveringStreamID != nil }
    var privacyCleanupInFlight = 0
    let privacyDrainBarrier = AudioDrainBarrier()
    private var quitPolicy: NativeQuitPolicy
    var quitRequested: Bool { quitPolicy.isRequested }
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
        channel.commandPending = { [watch = mainThreadWatch] id, queued in
            watch.commandPending(id: id, queued: queued)
        }
        channel.receive = { [weak self] command in self?.receive(command) }
        channel.disconnected = { [weak self] reason in
            self?.daemonDisconnected(reason: reason)
        }
        audioSession.configurationChanged = { [weak self] previous, current in
            self?.audioRouteChanged(previous: previous, current: current)
        }
        playback.becameIdle = { [weak self] in
            self?.audioSession.stopIfIdle()
            self?.updateMainThreadWatch()
        }
        microphone.failed = { [weak self] reason in self?.trackFailed(.microphone, reason: reason) }
        systemAudio.failed = { [weak self] reason in self?.trackFailed(.systemAudio, reason: reason) }
        if !launchOptions.isIsolatedFixture {
            hotkeys.onEvent = { [weak self] action, phase in self?.hotkey(action: action, phase: phase) }
            shortcutFailures = hotkeys.register()
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
                    message: "The previous native operation exceeded its deadline; try again.")
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
            isolatedFixture: launchOptions.isIsolatedFixture, preferredLanguages: Locale.preferredLanguages))
        if !launchOptions.isIsolatedFixture {
            microphoneActivity.changed = { [weak self] observation in
                guard let self, !self.latch.stopped else { return }
                switch observation {
                case .success(let applications):
                    self.channel.send(.makeMicrophoneActivity(applications.map {
                        (pid: $0.applicationPID, bundleID: $0.bundleID, name: $0.name)
                    }))
                case .failure(let failure):
                    self.channel.send(.makeMicrophoneActivityFailure(osStatus: failure.osStatus))
                }
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

    func state() -> [String: Any] {
        ["preferred_languages": Locale.preferredLanguages,
         "privacy_stopped": latch.stopped, "connected": connected,
         "native_instance_id": windowScreenshots.nativeInstanceID, "privacy_generation": privacyEpoch,
         "selected_text_available": !launchOptions.isIsolatedFixture && AXIsProcessTrusted(),
         "microphone": microphone.slot.isActive, "system_audio": systemAudio.slot.isActive,
         "playing": playback.playing, "shortcut_failures": shortcutFailures.map {
             ["combination": $0.combination, "os_status": $0.os_status] as [String: Any]
         },
         "supported_audio_processing": ["unprocessed", "voice_processing"],
         "voice_processing_available": audioSession.voiceProcessingAvailable,
         "playback_reference_available": audioSession.playbackReferenceAvailable,
         "audio_processing": audioSession.status, "calendar": calendarState()]
    }

    func publishState() {
        let capturing = microphone.slot.isActive || systemAudio.slot.isActive
        updateMainThreadWatch()
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
            playing: playback.playing, shortcutFailures: shortcutFailures, voiceProcessingAvailable: audioSession.voiceProcessingAvailable,
            playbackReferenceAvailable: audioSession.playbackReferenceAvailable,
            audioProcessing: audioSession.statusFields,
            preferredLanguages: Locale.preferredLanguages,
            calendar: NativeCalendarStatusResultFields(authorization: calendar.authorization.rawValue,
                physicalReadPending: calendar.physical_read_pending,
                failure: calendar.failure?.rawValue)))
    }

    func updateMainThreadWatch() {
        // assumeIsolated traps off the main thread: a helper crash would end
        // the capture, so a caller elsewhere only loses the calendar's share.
        let calendarPending = Thread.isMainThread
            ? MainActor.assumeIsolated { calendars?.operationPending ?? false } : false
        mainThreadWatch.activityChanged(pending: nativeOperations > 0 || commandRouter.hasCommandInFlight
            || privacyCleanupInFlight > 0 || audioRouteRecoveryInFlight || calendarPending,
            capturing: microphone.slot.isActive || systemAudio.slot.isActive,
            playing: playback.playing)
    }

    func permissions() -> [String: Any] {
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

    func screenAuthorization() -> (generation: UInt64, stopped: Bool) {
        (privacyEpoch, latch.stopped || !connected || privacyCleanupInFlight > 0 || quitRequested)
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

    /// Starts watching which applications hold the microphone, unless the
    /// user's brake is on. The first answer goes out at once, so the daemon never
    /// reads the silence before it as "no call".
    func resumeMicrophoneActivity() {
        guard !launchOptions.isIsolatedFixture, !latch.stopped else { return }
        microphoneActivity.start()
    }

    func finishQuitAfterDrain() {
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
