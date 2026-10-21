import AppKit
import CoreAudio
import Darwin
import OrbitNativeState

/// Tells the daemon which applications hold the microphone, whenever that changes.
///
/// CoreAudio keeps one object per process that uses audio (macOS 14+). This
/// observer listens to the list of those objects and to each one's
/// "running input" flag, and reports the applications behind them. Reading
/// these properties needs no permission: it says *that* an app uses the
/// microphone, never what it hears.
public final class MicrophoneActivityObserver: @unchecked Sendable {
    /// Called on the main queue with the new list, only when it changed.
    public var changed: (([MeetingApplication]) -> Void)?

    private let queue = DispatchQueue(label: "app.orbit.microphone-activity")
    private let ownPID = getpid()
    private let ownBundlePrefix: String
    private var listening = false
    private var systemListener: AudioObjectPropertyListenerBlock?
    private var processListeners: [AudioObjectID: AudioObjectPropertyListenerBlock] = [:]
    private var last: [MeetingApplication]?

    public init(ownBundlePrefix: String = "app.orbit") {
        self.ownBundlePrefix = ownBundlePrefix
    }

    public func start() {
        queue.async { [self] in
            guard !listening else { return }
            listening = true
            let listener: AudioObjectPropertyListenerBlock = { [weak self] _, _ in
                self?.refreshProcesses()
            }
            var address = Self.address(kAudioHardwarePropertyProcessObjectList)
            if AudioObjectAddPropertyListenerBlock(
                AudioObjectID(kAudioObjectSystemObject), &address, queue, listener) == noErr {
                systemListener = listener
            }
            refreshProcesses()
        }
    }

    /// Stops listening and forgets the last answer, so a later start reports
    /// the list afresh even if it did not change in between.
    public func stop() {
        queue.sync { [self] in
            guard listening else { return }
            listening = false
            if let listener = systemListener {
                var address = Self.address(kAudioHardwarePropertyProcessObjectList)
                AudioObjectRemovePropertyListenerBlock(
                    AudioObjectID(kAudioObjectSystemObject), &address, queue, listener)
            }
            systemListener = nil
            for (object, listener) in processListeners {
                var address = Self.address(kAudioProcessPropertyIsRunningInput)
                AudioObjectRemovePropertyListenerBlock(object, &address, queue, listener)
            }
            processListeners = [:]
            last = nil
        }
    }

    // MARK: - On `queue`

    private func refreshProcesses() {
        guard listening else { return }
        let objects = Set(Self.processObjects())
        for gone in Set(processListeners.keys).subtracting(objects) {
            if let listener = processListeners.removeValue(forKey: gone) {
                var address = Self.address(kAudioProcessPropertyIsRunningInput)
                AudioObjectRemovePropertyListenerBlock(gone, &address, queue, listener)
            }
        }
        for object in objects.subtracting(processListeners.keys) {
            let listener: AudioObjectPropertyListenerBlock = { [weak self] _, _ in self?.evaluate() }
            var address = Self.address(kAudioProcessPropertyIsRunningInput)
            if AudioObjectAddPropertyListenerBlock(object, &address, queue, listener) == noErr {
                processListeners[object] = listener
            }
        }
        evaluate()
    }

    private func evaluate() {
        guard listening else { return }
        let processes = Self.processObjects().compactMap { object -> MicrophoneProcess? in
            guard let pid: pid_t = Self.read(object, kAudioProcessPropertyPID) else { return nil }
            let input: UInt32 = Self.read(object, kAudioProcessPropertyIsRunningInput) ?? 0
            return MicrophoneProcess(pid: pid, runningInput: input != 0)
        }
        let applications = MicrophoneUsers.reportable(
            processes, ownPID: ownPID, ownBundlePrefix: ownBundlePrefix,
            resolve: Self.responsibleApplication)
        guard applications != last else { return }
        last = applications
        DispatchQueue.main.async { [weak self] in self?.changed?(applications) }
    }

    // MARK: - CoreAudio

    private static func address(_ selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress(
            mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain)
    }

    private static func processObjects() -> [AudioObjectID] {
        var address = address(kAudioHardwarePropertyProcessObjectList)
        let system = AudioObjectID(kAudioObjectSystemObject)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(system, &address, 0, nil, &size) == noErr, size > 0 else {
            return []
        }
        var objects = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
        guard AudioObjectGetPropertyData(system, &address, 0, nil, &size, &objects) == noErr else {
            return []
        }
        return Array(objects.prefix(Int(size) / MemoryLayout<AudioObjectID>.size))
    }

    private static func read<T: FixedWidthInteger>(
        _ object: AudioObjectID, _ selector: AudioObjectPropertySelector
    ) -> T? {
        var address = address(selector)
        var value: T = 0
        var size = UInt32(MemoryLayout<T>.size)
        guard AudioObjectGetPropertyData(object, &address, 0, nil, &size, &value) == noErr else {
            return nil
        }
        return value
    }

    // MARK: - Which application a process belongs to

    private typealias Responsible = @convention(c) (pid_t) -> pid_t

    /// The kernel's own answer to "who is this process working for": a
    /// browser's audio helper or WebKit's GPU process answers with the browser.
    /// Looked up at run time so a future macOS without it falls back to the
    /// parent chain instead of failing to launch.
    private static let responsible: Responsible? = {
        guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2),
            "responsibility_get_pid_responsible_for_pid") else { return nil }
        return unsafeBitCast(symbol, to: Responsible.self)
    }()

    private static func parent(of pid: pid_t) -> pid_t? {
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.stride
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, pid]
        guard sysctl(&mib, UInt32(mib.count), &info, &size, nil, 0) == 0, size > 0 else { return nil }
        let parent = info.kp_eproc.e_ppid
        return parent > 1 ? parent : nil
    }

    static func responsibleApplication(_ pid: Int32) -> ResponsibleApplication? {
        var candidates: [pid_t] = []
        if let responsible {
            let owner = responsible(pid)
            if owner > 0 { candidates.append(owner) }
        }
        candidates.append(pid)
        var current = pid
        // Helpers sit at most a few levels under their application.
        for _ in 0..<8 {
            guard let parent = parent(of: current) else { break }
            candidates.append(parent)
            current = parent
        }
        for candidate in candidates {
            guard let application = NSRunningApplication(processIdentifier: candidate),
                  application.activationPolicy == .regular, !application.isTerminated,
                  let bundleID = application.bundleIdentifier,
                  let name = application.localizedName else { continue }
            return ResponsibleApplication(pid: candidate, bundleID: bundleID, name: name)
        }
        return nil
    }
}
