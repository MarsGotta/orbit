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
        var pendingCommands = 0
        var droppedAudioFrames = [AudioTrack: Int]()
        var gapWriteScheduled = false
        var receive: Receiver?
        var disconnected: Disconnection?
    }

    private enum Admission: Sendable {
        case accepted, discardedAudio, closed, overloaded
    }

    private enum GapWrite: Sendable {
        case frame(Data)
        case retry
        case skip
        case finished
    }

    private static let maximumBufferedAudioSeconds = 10
    // Ten seconds * two tracks * 48 kHz * 16-bit PCM * base64 expansion,
    // plus a bounded allowance for the versioned JSON envelopes.
    private static let maximumPendingBytes = maximumBufferedAudioSeconds * 2
        * (48_000 * 2 * 4 / 3) + 65_536
    private static let maximumPendingCommands = 64
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
            let available = Self.maximumPendingBytes - current.pendingBytes
            let gapBytes = Self.pendingGapBytes(current.droppedAudioFrames)
            guard gapBytes <= available, frame.count <= available - gapBytes else {
                guard let (track, frames) = audio else { return .overloaded }
                let previous = current.droppedAudioFrames[track, default: 0]
                current.droppedAudioFrames[track] = previous > Int.max - frames
                    ? Int.max : previous + frames
                if !current.gapWriteScheduled {
                    current.gapWriteScheduled = true
                    writer.async { self.writePendingAudioGaps() }
                }
                return .discardedAudio
            }
            current.pendingBytes += frame.count
            // Admission AND enqueue are one ordered transition with flush.
            // The closure takes the mutex only after this short section exits.
            writer.async { self.writeFrame(frame) }
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
            guard frame.count <= maximumPendingBytes - total else { return maximumPendingBytes }
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
                guard frame.count <= Self.maximumPendingBytes - current.pendingBytes else {
                    return .retry
                }
                current.droppedAudioFrames.removeValue(forKey: track)
                current.pendingBytes += frame.count
                return .frame(frame)
            }
            switch action {
            case .frame(let frame):
                writeFrame(frame)
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
        let admission = state.withLock { current -> Admission in
            guard !current.failed else { return .closed }
            guard current.pendingCommands < Self.maximumPendingCommands else {
                current.failed = true
                return .overloaded
            }
            current.pendingCommands += 1
            DispatchQueue.main.async { self.deliver(command) }
            return .accepted
        }
        return finishAdmission(admission, reason: "native_command_backpressure")
    }

    @MainActor private func deliver(_ command: NativeWireCommand) {
        defer { state.withLock { $0.pendingCommands -= 1 } }
        let callback = state.withLock { current -> Receiver? in
            guard !current.failed else { return nil }
            return current.receive
        }
        // This last check admits delivery, not cancellation of a callback that
        // has already begun. No user callback or its effects run under the lock.
        callback?(command)
    }

    private func writeFrame(_ frame: Data) {
        defer { state.withLock { $0.pendingBytes -= frame.count } }
        guard !state.withLock({ $0.failed }) else { return }
        do {
            try writeOutput(frame)
        } catch {
            fail("daemon_output_disconnected")
        }
        // A concurrent failure cannot undo bytes already written. The budget
        // remains reserved until this real write returns, including on error.
    }

    private func finishAdmission(_ admission: Admission, reason: String) -> Bool {
        switch admission {
        case .accepted, .discardedAudio:
            return true
        case .closed:
            return false
        case .overloaded:
            // The failure was latched in the admission transaction. Schedule
            // its one notification only after releasing that non-recursive lock.
            notifyFailure(reason)
            return false
        }
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
