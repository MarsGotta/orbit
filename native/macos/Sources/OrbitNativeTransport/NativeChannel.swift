import Foundation
import Darwin
import Synchronization
import OrbitNativeProtocol

/// Private inherited pipes; the daemon owns network and durable work.
///
/// `send` acknowledges bounded admission, not delivery. `flush` observes all
/// earlier writer admissions, but is neither a peer ACK nor a durability receipt.
/// A blocking read/write retains its worker until real I/O returns: there is no
/// timeout, retry or cancellation that pretends the physical work has stopped.
public final class NativeChannel: Sendable {
    public typealias Receiver = @MainActor @Sendable (NativeWireCommand) -> Void
    public typealias Disconnection = @MainActor @Sendable (String) -> Void
    /// Called on the reader's queue for every command read, before the main
    /// thread runs it: the daemon answered, whatever main is doing.
    public typealias Observer = @Sendable () -> Void

    private struct RejectedCommandHeader: Decodable {
        let v: Int
        let id: String
        let command: String
    }

    private struct AudioTrack: Hashable, Sendable {
        let source: String
        let streamID: String
    }

    private struct State: Sendable {
        var started = false
        var failed = false
        var pendingBytes = 0
        var pendingAudioBytes = 0
        var pendingCommands = 0
        var pendingStops = 0
        var droppedAudioFrames = [AudioTrack: Int]()
        var gapWriteScheduled = false
        var receive: Receiver?
        var disconnected: Disconnection?
        var commandRead: Observer?
    }

    private enum Admission: Sendable {
        case accepted, discardedAudio, closed, overloaded
    }

    /// Which of the main thread's bounds a command waiting for it holds.
    private enum CommandSlot: Sendable {
        case general, stop
    }

    /// What the reader does with a command it read.
    private enum CommandAdmission: Sendable {
        case queued(CommandSlot)
        /// A beat past the bound: reading it was its answer (`commandRead`).
        case beatNotQueued
        /// Refused at once, off main, with how many commands wait for it.
        case busy(pending: Int)
        case closed
        /// A stop past its own room: the channel fails closed.
        case overloaded
    }

    private enum GapWrite: Sendable {
        case frame(Data)
        case retry
        case skip
        case finished
    }

    /// Audio the daemon has not read yet waits here, in memory and in order,
    /// and reaches it when it reads again. It protects a recording or a voice
    /// note from a daemon that stops reading for a while: on 2026-10-22 an
    /// 11 s stall of the daemon cut a meeting, and the owner's rule is that
    /// capture goes on and the audio is delivered afterwards. Five minutes of
    /// both tracks at 48 kHz, 16-bit mono PCM, base64 in JSON: 300 s * 2 *
    /// 128,000 B/s, about 77 MB, plus room for the envelopes. Audio that does
    /// not fit is counted in one `gap` (`daemon_backpressure`) and capture
    /// goes on. It limits neither a recording's length nor how long the daemon
    /// may be away: a daemon that closes the pipe ends the helper anyway.
    static let maximumBufferedAudioSeconds = 300
    static let maximumPendingAudioBytes = maximumBufferedAudioSeconds * 2
        * (48_000 * 2 * 4 / 3) + 65_536
    /// Every other event (responses, state, errors) has its own bound: the
    /// 2.5 MiB all events shared before audio got its own room (ten seconds
    /// of two-track audio). Neither kind can take the other's room.
    private static let maximumPendingBytes = 10 * 2 * (48_000 * 2 * 4 / 3) + 65_536
    /// Commands waiting for the main thread to run them. Commands still run on
    /// main, and a modal or a slow AppKit call holds it while the daemon keeps
    /// sending: this bounds that backlog's memory. Past it a command is refused
    /// at once from the reader's queue with `mainThreadBusyCode` and these
    /// numbers, and the channel stays open. It used to close at the 65th, and
    /// the helper then left, cutting a recording. 64 is the bound the channel
    /// always had. It limits neither how long main may be held nor the stops
    /// below. A beat past it is neither queued nor refused: reading it was its
    /// answer, and a refused beat every 2 s would only flood the daemon.
    static let maximumPendingCommands = 64
    /// Room past that bound for the daemon's stops (`stopCommands`): refusing
    /// one would leave capture running after the person or the daemon stopped
    /// it. The daemon keeps one whole-capture stop and one stop per track in
    /// flight, plus its `shutdown`, so 16 is ample. A stop past this room
    /// still fails the channel closed, which makes the helper stop capture.
    static let maximumPendingStops = 16
    static let mainThreadBusyCode = "native_main_thread_busy"
    private static let stopCommands: Set<String> = ["privacy_stop", "capture_stop", "audio_stop", "shutdown"]
    private let state = Mutex(State())
    private let input: FileHandle
    private let writeOutput: @Sendable (Data) throws -> Void
    private let reader = DispatchQueue(label: "orbit.native.protocol.reader")
    private let writer = DispatchQueue(label: "orbit.native.protocol.writer")

    /// Set callbacks before `start`. Their bodies only execute on MainActor;
    /// configuring them does not replay previously delivered terminal events.
    public var receive: Receiver? {
        get { state.withLock { $0.receive } }
        set { state.withLock { $0.receive = newValue } }
    }

    public var disconnected: Disconnection? {
        get { state.withLock { $0.disconnected } }
        set { state.withLock { $0.disconnected = newValue } }
    }

    /// Set before `start`. Runs on the reader's queue, never on main.
    public var commandRead: Observer? {
        get { state.withLock { $0.commandRead } }
        set { state.withLock { $0.commandRead = newValue } }
    }

    public convenience init(input: FileHandle, output: FileHandle) {
        self.init(input: input, writeOutput: { data in
            try output.write(contentsOf: data)
        })
    }

    /// Testable I/O boundary, not a different queue/admission implementation.
    /// A synthetic writer returning successfully does not prove pipe delivery.
    init(input: FileHandle, writeOutput: @escaping @Sendable (Data) throws -> Void) {
        self.input = input
        self.writeOutput = writeOutput
    }

    @MainActor public func start() {
        state.withLock { current in
            guard !current.started, !current.failed else { return }
            current.started = true
            reader.async { self.readCommands() }
        }
    }

    @discardableResult
    public func send(_ event: NativeEvent) -> Bool {
        let body: Data
        do { body = try JSONEncoder().encode(event) }
        catch {
            FileHandle.standardError.write(Data("native_event_encode_failed event=\(event.name)\n".utf8))
            return false
        }
        guard body.count < NativeLimits.maximumEventLineBytes else {
            fail("native_event_invalid_or_too_large_\(event.name)")
            return false
        }
        var line = body
        line.append(10)
        let frame = line
        let audio = Self.audioTrackAndFrames(event)
        let admission = state.withLock { current -> Admission in
            guard !current.failed else { return .closed }
            guard let (track, frames) = audio else {
                guard frame.count <= Self.maximumPendingBytes - current.pendingBytes else {
                    return .overloaded
                }
                current.pendingBytes += frame.count
                writer.async { self.writeFrame(frame, audio: false) }
                return .accepted
            }
            let available = Self.maximumPendingAudioBytes - current.pendingAudioBytes
            let gapBytes = Self.pendingGapBytes(current.droppedAudioFrames)
            guard gapBytes <= available, frame.count <= available - gapBytes else {
                let previous = current.droppedAudioFrames[track, default: 0]
                current.droppedAudioFrames[track] = previous > Int.max - frames
                    ? Int.max : previous + frames
                if !current.gapWriteScheduled {
                    current.gapWriteScheduled = true
                    writer.async { self.writePendingAudioGaps() }
                }
                return .discardedAudio
            }
            current.pendingAudioBytes += frame.count
            // Admission AND enqueue are one ordered transition with flush.
            // The closure takes the mutex only after this short section exits.
            writer.async { self.writeFrame(frame, audio: true) }
            return .accepted
        }
        switch admission {
        case .accepted, .discardedAudio:
            return true
        case .closed, .overloaded:
            return false
        }
    }

    private static func audioTrackAndFrames(_ event: NativeEvent) -> (AudioTrack, Int)? {
        guard case .audio(let envelope) = event else { return nil }
        let source = envelope.fields.source
        let streamID = envelope.fields.stream_id
        let frames = Int(envelope.fields.frames)
        guard frames > 0 else { return nil }
        return (AudioTrack(source: source, streamID: streamID), frames)
    }

    private static func pendingGapBytes(_ dropped: [AudioTrack: Int]) -> Int {
        var total = 0
        for (track, frames) in dropped {
            guard let frame = gapFrame(track: track, frames: frames) else { continue }
            guard frame.count <= maximumPendingAudioBytes - total else { return maximumPendingAudioBytes }
            total += frame.count
        }
        return total
    }

    private static func gapFrame(track: AudioTrack, frames: Int) -> Data? {
        guard let count = UInt64(exactly: frames) else { return nil }
        let event = NativeEvent.makeGap(source: track.source, streamID: track.streamID,
            reason: "daemon_backpressure", frames: count)
        guard var line = try? JSONEncoder().encode(event),
              line.count < NativeLimits.maximumEventLineBytes else { return nil }
        line.append(10)
        return line
    }

    /// A zero-byte marker is queued at the first drop. It sits behind every
    /// frame admitted before saturation and ahead of later recovery traffic.
    private func writePendingAudioGaps() {
        while true {
            let action = state.withLock { current -> GapWrite in
                guard !current.failed else {
                    current.droppedAudioFrames.removeAll()
                    current.gapWriteScheduled = false
                    return .finished
                }
                guard let (track, frames) = current.droppedAudioFrames.first else {
                    current.gapWriteScheduled = false
                    return .finished
                }
                guard let frame = Self.gapFrame(track: track, frames: frames) else {
                    current.droppedAudioFrames.removeValue(forKey: track)
                    return .skip
                }
                guard frame.count <= Self.maximumPendingAudioBytes - current.pendingAudioBytes else {
                    return .retry
                }
                current.droppedAudioFrames.removeValue(forKey: track)
                current.pendingAudioBytes += frame.count
                return .frame(frame)
            }
            switch action {
            case .frame(let frame):
                writeFrame(frame, audio: true)
            case .retry:
                writer.async { self.writePendingAudioGaps() }
                return
            case .skip:
                continue
            case .finished:
                return
            }
        }
    }

    /// The marker is enqueued under the same mutex as every frame admission.
    /// An in-flight write keeps this pending until its actual return, even when
    /// the caller cancels its task. A prior channel failure yields false.
    public func flush() async -> Bool {
        await withCheckedContinuation { continuation in
            state.withLock { _ in
                writer.async {
                    let complete = self.state.withLock { !$0.failed }
                    continuation.resume(returning: complete)
                }
            }
        }
    }

    public func respond(_ command: NativeWireCommand, result: [String: Any] = [:]) {
        do {
            let event = try NativeEvent.makeResponse(id: command.id, result: result)
            if !send(event) {
                FileHandle.standardError.write(Data("native_response_not_admitted command=\(command.command)\n".utf8))
            }
        } catch let error as NativeEventFieldError {
            reject(command, code: error.description,
                message: "La respuesta nativa \(command.command) contiene un tipo inválido en \(error.field).")
        } catch {
            reject(command, code: "native_response_invalid",
                message: "La respuesta nativa \(command.command) no pudo codificarse.")
        }
    }

    public func reject(_ command: NativeWireCommand, code: String, message: String) {
        reject(id: command.id, code: code, message: message)
    }

    private func reject(id: String, code: String, message: String) {
        if !send(.makeRejection(id: id, code: code, message: message)) {
            FileHandle.standardError.write(Data("native_rejection_not_admitted event=response\n".utf8))
        }
    }

    private func readCommands() {
        var decoder = LineDecoder()
        var buffer = [UInt8](repeating: 0, count: 65_536)
        do {
            while !state.withLock({ $0.failed }) {
                // FileHandle.read(upToCount:) may fill the requested length on
                // macOS before returning. One POSIX read returns the available
                // pipe prefix, so a short command never needs EOF or 64 KiB of
                // following commands to reach the decoder. The channel retains
                // its FileHandle for the entire blocking read.
                let count = buffer.withUnsafeMutableBytes {
                    Darwin.read(input.fileDescriptor, $0.baseAddress, $0.count)
                }
                if count < 0 {
                    if errno == EINTR { continue }
                    fail("invalid_protocol_or_disconnected")
                    return
                }
                guard count > 0 else {
                    fail("daemon_disconnected")
                    return
                }
                guard !state.withLock({ $0.failed }) else { return }
                let data = Data(buffer.prefix(count))
                for line in try decoder.append(data) {
                    do {
                        let command = try NativeWireCommand.decode(line)
                        guard admit(command) else { return }
                    } catch {
                        // A malformed body can still name its request. Only this
                        // error path reads a header again; valid commands decode once.
                        let header = try JSONDecoder().decode(RejectedCommandHeader.self, from: line)
                        guard header.v == NativeProtocol.version,
                              !header.id.isEmpty, header.id.utf8.count <= 128 else { throw error }
                        reject(id: header.id, code: "invalid_native_command",
                            message: "La orden nativa \(header.command) no tiene la forma del protocolo.")
                    }
                }
            }
        } catch {
            fail("invalid_protocol_or_disconnected")
        }
    }

    private func admit(_ command: NativeWireCommand) -> Bool {
        // Observed before the main thread can run it: a command waiting
        // behind a held main thread is the daemon answering, not silence.
        let observer = state.withLock { $0.failed ? nil : $0.commandRead }
        observer?()
        let name = command.command
        let admission = state.withLock { current -> CommandAdmission in
            let decided = Self.admission(of: name, in: &current)
            if case .queued(let slot) = decided {
                DispatchQueue.main.async { self.deliver(command, slot: slot) }
            }
            return decided
        }
        switch admission {
        case .queued, .beatNotQueued:
            return true
        case .busy(let pending):
            refuseBusy(command, pending: pending)
            return true
        case .closed:
            return false
        case .overloaded:
            // Latched in the admission transaction; notified after the lock.
            notifyFailure("native_command_backpressure")
            return false
        }
    }

    /// The main thread's bounds, decided under the channel's lock.
    private static func admission(of name: String, in current: inout State) -> CommandAdmission {
        guard !current.failed else { return .closed }
        if current.pendingCommands < maximumPendingCommands {
            current.pendingCommands += 1
            return .queued(.general)
        }
        if name == "heartbeat" {
            return .beatNotQueued
        }
        guard stopCommands.contains(name) else {
            return .busy(pending: current.pendingCommands)
        }
        guard current.pendingStops < maximumPendingStops else {
            current.failed = true
            return .overloaded
        }
        current.pendingStops += 1
        return .queued(.stop)
    }

    private static func release(_ slot: CommandSlot, in current: inout State) {
        switch slot {
        case .general:
            current.pendingCommands -= 1
        case .stop:
            current.pendingStops -= 1
        }
    }

    /// Answered from the reader's queue, never waiting for main: the daemon
    /// hears at once that this order did not run, and why.
    private func refuseBusy(_ command: NativeWireCommand, pending: Int) {
        let detail: [String: Any] = [
            "pending_commands": pending,
            "max_pending_commands": Self.maximumPendingCommands
        ]
        let message = "El hilo principal de Orbit en este Mac tiene \(pending) órdenes pendientes; "
            + "\(command.command) no se ejecutó."
        let event = NativeEvent.makeRejection(id: command.id, code: Self.mainThreadBusyCode,
            message: message, detail: detail)
        if !send(event) {
            FileHandle.standardError.write(Data("native_rejection_not_admitted event=response\n".utf8))
        }
    }

    @MainActor private func deliver(_ command: NativeWireCommand, slot: CommandSlot) {
        defer { state.withLock { current in Self.release(slot, in: &current) } }
        let callback = state.withLock { current -> Receiver? in
            guard !current.failed else { return nil }
            return current.receive
        }
        // This last check admits delivery, not cancellation of a callback that
        // has already begun. No user callback or its effects run under the lock.
        callback?(command)
    }

    /// `audio` names the budget the frame was admitted under: audio frames
    /// and their gaps, or every other event.
    private func writeFrame(_ frame: Data, audio: Bool) {
        defer {
            state.withLock {
                if audio { $0.pendingAudioBytes -= frame.count } else { $0.pendingBytes -= frame.count }
            }
        }
        guard !state.withLock({ $0.failed }) else { return }
        do {
            try writeOutput(frame)
        } catch {
            fail("daemon_output_disconnected")
        }
        // A concurrent failure cannot undo bytes already written. The budget
        // remains reserved until this real write returns, including on error.
    }

    private func fail(_ reason: String) {
        let first = state.withLock { current in
            guard !current.failed else { return false }
            current.failed = true
            return true
        }
        if first { notifyFailure(reason) }
    }

    private func notifyFailure(_ reason: String) {
        DispatchQueue.main.async {
            let callback = self.state.withLock { $0.disconnected }
            callback?(reason)
        }
    }
}
