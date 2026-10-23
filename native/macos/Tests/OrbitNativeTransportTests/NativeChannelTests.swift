import Foundation
import Synchronization
import XCTest
import OrbitNativeProtocol
@testable import OrbitNativeTransport

private enum FixtureFailure: Error { case writeFailed, gateTimedOut }

private func nativeTestEvent(_ sequence: Int) -> NativeEvent {
    .makeError(code: String(sequence))
}

/// Only the injected I/O boundary is synthetic. Admission, scheduling, failure
/// and flush run through the production channel, never through a policy replica.
private final class OutputProbe: Sendable {
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    let chunks = Mutex<[Data]>([])
    let attempts = Mutex(0)
    let blocked: Bool
    let throwing: Bool

    init(blocked: Bool = false, throwing: Bool = false) {
        self.blocked = blocked
        self.throwing = throwing
    }

    func write(_ data: Data) throws {
        attempts.withLock { $0 += 1 }
        entered.signal()
        if blocked && release.wait(timeout: .now() + 3) != .success {
            throw FixtureFailure.gateTimedOut
        }
        if throwing { throw FixtureFailure.writeFailed }
        chunks.withLock { $0.append(data) }
    }
}

/// Holds only the first physical write so a fixture can fill the real pending
/// budget and then let the production writer recover without synthetic delays.
private final class RecoveringOutputProbe: Sendable {
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    let chunks = Mutex<[Data]>([])
    private let blockFirstWrite = Mutex(true)

    func write(_ data: Data) throws {
        let shouldBlock = blockFirstWrite.withLock { first in
            defer { first = false }
            return first
        }
        if shouldBlock {
            entered.signal()
            if release.wait(timeout: .now() + 3) != .success {
                throw FixtureFailure.gateTimedOut
            }
        }
        chunks.withLock { $0.append(data) }
    }
}

@MainActor
final class NativeChannelTests: XCTestCase {
    private func channel(_ probe: OutputProbe, input: Pipe = Pipe()) -> NativeChannel {
        NativeChannel(input: input.fileHandleForReading, writeOutput: { try probe.write($0) })
    }

    private func command(_ id: String, kind: String = "heartbeat") -> Data {
        Data("{\"v\":1,\"id\":\"\(id)\",\"command\":\"\(kind)\"}\n".utf8)
    }

    /// `count` commands of one kind, numbered from `first`, as one write.
    private func commands(_ kind: String, count: Int, first: Int = 0) -> Data {
        var batch = Data()
        for index in first..<(first + count) {
            batch.append(command("\(kind)-\(index)", kind: kind))
        }
        return batch
    }

    private func decodedChunks(_ probe: OutputProbe) throws -> [[String: Any]] {
        var messages = [[String: Any]]()
        for data in probe.chunks.withLock({ $0 }) {
            messages.append(try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any]))
        }
        return messages
    }

    private func audioEvent(sequence: Int, streamID: String, frames: Int,
                            pcm: String) throws -> NativeEvent {
        .makeAudio(source: "mic_local", streamID: streamID, sequence: UInt64(sequence),
            sampleRate: 48_000, frames: UInt32(frames),
            monotonicNS: UInt64(sequence) * 1_000_000_000, pcmBase64: pcm,
            processing: nil)
    }

    private func decodedChunks(_ probe: RecoveringOutputProbe) throws -> [[String: Any]] {
        try probe.chunks.withLock { $0 }.map { data in
            try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        }
    }

    func testMalformedTypedBodyIsRefusedAndTheNextCommandStillArrives() async throws {
        let input = Pipe()
        let probe = OutputProbe()
        let channel = channel(probe, input: input)
        defer {
            channel.receive = nil
            channel.disconnected = nil
            try? input.fileHandleForWriting.close()
        }
        let next = expectation(description: "heartbeat after malformed body")
        let received = Mutex<[String]>([])
        let disconnected = Mutex<[String]>([])
        channel.receive = { command in
            received.withLock { $0.append(command.id) }
            next.fulfill()
        }
        channel.disconnected = { reason in disconnected.withLock { $0.append(reason) } }
        channel.start()
        var lines = Data(#"{"v":1,"id":"malformed","command":"calendar_save","calendar_save":{}}"#.utf8)
        lines.append(10)
        lines.append(command("after"))
        try input.fileHandleForWriting.write(contentsOf: lines)
        await fulfillment(of: [next], timeout: 2)
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        XCTAssertEqual(received.withLock { $0 }, ["after"])
        XCTAssertEqual(disconnected.withLock { $0 }, [])
        let messages = try probe.chunks.withLock { $0 }.map {
            try XCTUnwrap(JSONSerialization.jsonObject(with: $0) as? [String: Any])
        }
        XCTAssertEqual(messages.count, 1)
        let refusal = try XCTUnwrap(messages.first)
        XCTAssertEqual(refusal["id"] as? String, "malformed")
        XCTAssertEqual(refusal["ok"] as? Bool, false)
        XCTAssertEqual((refusal["error"] as? [String: Any])?["code"] as? String, "invalid_native_command")
    }

    func testSaturatedAudioQueueReportsOneCountedGapAndStaysAlive() async throws {
        let probe = RecoveringOutputProbe()
        defer { probe.release.signal() }
        let channel = NativeChannel(
            input: Pipe().fileHandleForReading,
            writeOutput: { try probe.write($0) }
        )
        let disconnections = Mutex<[String]>([])
        channel.disconnected = { reason in disconnections.withLock { $0.append(reason) } }

        // Six seconds per event keeps each line under the 1 MiB event bound;
        // twenty events beyond the audio budget's two tracks saturate it.
        let secondsPerEvent = 6
        let framesPerEvent = secondsPerEvent * 48_000
        let pcm = Data(repeating: 0, count: framesPerEvent * 2).base64EncodedString()
        let submittedCount = NativeChannel.maximumBufferedAudioSeconds * 2 / secondsPerEvent + 20
        var admissions = [channel.send(try audioEvent(
            sequence: 0, streamID: "saturated-stream", frames: framesPerEvent, pcm: pcm
        ))]
        XCTAssertEqual(probe.entered.wait(timeout: .now() + 2), .success)
        for sequence in 1..<submittedCount {
            admissions.append(channel.send(try audioEvent(
                sequence: sequence, streamID: "saturated-stream",
                frames: framesPerEvent, pcm: pcm
            )))
        }

        probe.release.signal()
        let backlogFlushed = await channel.flush()
        let recoverySequence = submittedCount
        let recoveryAccepted = channel.send(try audioEvent(
            sequence: recoverySequence, streamID: "saturated-stream",
            frames: framesPerEvent, pcm: pcm
        ))
        let controlAccepted = channel.send(nativeTestEvent(9_999))
        let recoveryFlushed = await channel.flush()

        let messages = try decodedChunks(probe)
        let audio = messages.filter { $0["event"] as? String == "audio" }
        let deliveredBeforeRecovery = Set(audio.compactMap { $0["sequence"] as? Int }
            .filter { $0 < submittedCount })
        let droppedFrames = (submittedCount - deliveredBeforeRecovery.count) * framesPerEvent
        let gaps = messages.filter { $0["event"] as? String == "gap" }

        XCTAssertTrue(admissions.allSatisfy { $0 }, "discarding audio is admission, not disconnection")
        XCTAssertTrue(backlogFlushed)
        XCTAssertTrue(recoveryAccepted)
        XCTAssertTrue(controlAccepted)
        XCTAssertTrue(recoveryFlushed)
        XCTAssertGreaterThan(droppedFrames, 0, "the fixture must actually saturate the queue")
        XCTAssertEqual(gaps.count, 1)
        let gap = try XCTUnwrap(gaps.first)
        XCTAssertEqual(gap["reason"] as? String, "daemon_backpressure")
        XCTAssertEqual(gap["frames"] as? Int, droppedFrames)
        XCTAssertTrue(audio.contains { $0["sequence"] as? Int == recoverySequence })
        XCTAssertTrue(messages.contains { $0["code"] as? String == "9999" })
        XCTAssertEqual(disconnections.withLock { $0 }, [])
    }

    func testOutputBudgetAdmitsTenSecondsOfTwoTrackAudio() async throws {
        let probe = RecoveringOutputProbe()
        defer { probe.release.signal() }
        let channel = NativeChannel(
            input: Pipe().fileHandleForReading,
            writeOutput: { try probe.write($0) }
        )
        let disconnections = Mutex<[String]>([])
        channel.disconnected = { reason in disconnections.withLock { $0.append(reason) } }

        let framesPerSecond = 48_000
        let pcm = Data(repeating: 0, count: framesPerSecond * 2).base64EncodedString()
        var admissions = [channel.send(try audioEvent(
            sequence: 0, streamID: "track-0", frames: framesPerSecond, pcm: pcm
        ))]
        XCTAssertEqual(probe.entered.wait(timeout: .now() + 2), .success)
        for second in 0..<10 {
            for track in 0..<2 where !(second == 0 && track == 0) {
                admissions.append(channel.send(try audioEvent(
                    sequence: second, streamID: "track-\(track)",
                    frames: framesPerSecond, pcm: pcm
                )))
            }
        }

        probe.release.signal()
        let flushed = await channel.flush()
        let messages = try decodedChunks(probe)

        XCTAssertTrue(admissions.allSatisfy { $0 })
        XCTAssertTrue(flushed)
        XCTAssertEqual(messages.filter { $0["event"] as? String == "audio" }.count, 20)
        XCTAssertFalse(messages.contains { $0["event"] as? String == "gap" })
        XCTAssertEqual(disconnections.withLock { $0 }, [])
    }

    func testOutputBudgetKeepsFiveMinutesOfTwoTrackAudioForAStalledDaemon() async throws {
        // The daemon stops reading (the first write never returns until
        // released): five minutes of both tracks wait in order, without a gap,
        // and reach it when it reads again.
        XCTAssertEqual(NativeChannel.maximumBufferedAudioSeconds, 300)
        let probe = RecoveringOutputProbe()
        defer { probe.release.signal() }
        let channel = NativeChannel(
            input: Pipe().fileHandleForReading,
            writeOutput: { try probe.write($0) }
        )
        let disconnections = Mutex<[String]>([])
        channel.disconnected = { reason in disconnections.withLock { $0.append(reason) } }

        let secondsPerEvent = 6
        let framesPerEvent = secondsPerEvent * 48_000
        let pcm = Data(repeating: 0, count: framesPerEvent * 2).base64EncodedString()
        let eventsPerTrack = NativeChannel.maximumBufferedAudioSeconds / secondsPerEvent
        var admissions = [channel.send(try audioEvent(
            sequence: 0, streamID: "track-0", frames: framesPerEvent, pcm: pcm
        ))]
        XCTAssertEqual(probe.entered.wait(timeout: .now() + 2), .success)
        for sequence in 0..<eventsPerTrack {
            for track in 0..<2 where !(sequence == 0 && track == 0) {
                admissions.append(channel.send(try audioEvent(
                    sequence: sequence, streamID: "track-\(track)",
                    frames: framesPerEvent, pcm: pcm
                )))
            }
        }

        probe.release.signal()
        let flushed = await channel.flush()
        let messages = try decodedChunks(probe)
        let audio = messages.filter { $0["event"] as? String == "audio" }

        XCTAssertTrue(admissions.allSatisfy { $0 })
        XCTAssertTrue(flushed)
        XCTAssertEqual(audio.count, eventsPerTrack * 2)
        XCTAssertFalse(messages.contains { $0["event"] as? String == "gap" })
        for track in 0..<2 {
            let sequences = audio.filter { $0["stream_id"] as? String == "track-\(track)" }
                .compactMap { $0["sequence"] as? Int }
            XCTAssertEqual(sequences, Array(0..<eventsPerTrack), "track-\(track) arrives in order")
        }
        XCTAssertEqual(disconnections.withLock { $0 }, [])
    }

    func testRealPipeReceivesVersionedCompleteLineBeforeFlushReturns() async throws {
        let input = Pipe()
        let output = Pipe()
        let channel = NativeChannel(input: input.fileHandleForReading, output: output.fileHandleForWriting)
        XCTAssertTrue(channel.send(nativeTestEvent(7)))
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        try output.fileHandleForWriting.close()
        let bytes = try XCTUnwrap(output.fileHandleForReading.read(upToCount: 4096))
        XCTAssertEqual(bytes.last, 10)
        let decoded = try XCTUnwrap(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
        XCTAssertEqual(decoded["v"] as? Int, NativeProtocol.version)
        XCTAssertEqual(decoded["code"] as? String, "7")
        XCTAssertEqual(decoded["event"] as? String, "error")
    }

    func testConcurrentProducersKeepEveryFrameWholeAndFlushCoversAllAcceptedFrames() async throws {
        let probe = OutputProbe()
        let channel = channel(probe)
        let rejected = Mutex(0)
        DispatchQueue.concurrentPerform(iterations: 128) { sequence in
            if !channel.send(nativeTestEvent(sequence)) {
                rejected.withLock { $0 += 1 }
            }
        }
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        XCTAssertEqual(rejected.withLock { $0 }, 0)
        let chunks = probe.chunks.withLock { $0 }
        XCTAssertEqual(chunks.count, 128)
        let sequences = try chunks.map { data -> Int in
            XCTAssertEqual(data.last, 10)
            let value = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            return try XCTUnwrap(Int(try XCTUnwrap(value["code"] as? String)))
        }
        XCTAssertEqual(Set(sequences), Set(0..<128))
    }

    func testFlushDoesNotConfirmAWriteWhileItsActualIOIsStillBlocked() async {
        let probe = OutputProbe(blocked: true)
        defer { probe.release.signal() }
        let channel = channel(probe)
        XCTAssertTrue(channel.send(nativeTestEvent(1)))
        XCTAssertEqual(probe.entered.wait(timeout: .now() + 2), .success)
        let flushStarted = expectation(description: "flush task entered")
        let result = Mutex<Bool?>(nil)
        let flush = Task.detached {
            flushStarted.fulfill()
            let complete = await channel.flush()
            result.withLock { $0 = complete }
            return complete
        }
        await fulfillment(of: [flushStarted], timeout: 2)
        XCTAssertNil(result.withLock { $0 })
        XCTAssertEqual(probe.chunks.withLock { $0.count }, 0)
        probe.release.signal()
        let flushed = await flush.value
        XCTAssertTrue(flushed)
        XCTAssertEqual(probe.chunks.withLock { $0.count }, 1)
    }

    func testFailedPhysicalWriteLatchesFailureAndCannotProduceASuccessfulFlush() async {
        let probe = OutputProbe(throwing: true)
        let channel = channel(probe)
        let failure = expectation(description: "one write failure")
        let reasons = Mutex<[String]>([])
        channel.disconnected = { reason in reasons.withLock { $0.append(reason) }; failure.fulfill() }
        XCTAssertTrue(channel.send(nativeTestEvent(1)))
        let flushed = await channel.flush()
        XCTAssertFalse(flushed)
        XCTAssertFalse(channel.send(nativeTestEvent(2)))
        let secondFlush = await channel.flush()
        XCTAssertFalse(secondFlush)
        await fulfillment(of: [failure], timeout: 2)
        XCTAssertEqual(reasons.withLock { $0 }, ["daemon_output_disconnected"])
        XCTAssertEqual(probe.attempts.withLock { $0 }, 1)
    }

    func testOversizeTypedEventFailsBeforeWritingAndNeverRepeatsNotification() async {
        let payload = NativeEvent.makeError(code: String(repeating: "x", count: NativeLimits.maximumEventLineBytes))
        let probe = OutputProbe()
        let channel = channel(probe)
        let failure = expectation(description: "validation failed once")
        let reasons = Mutex<[String]>([])
        channel.disconnected = { reason in reasons.withLock { $0.append(reason) }; failure.fulfill() }
        XCTAssertFalse(channel.send(payload))
        XCTAssertFalse(channel.send(payload))
        let flushed = await channel.flush()
        XCTAssertFalse(flushed)
        await fulfillment(of: [failure], timeout: 2)
        XCTAssertEqual(reasons.withLock { $0 }, ["native_event_invalid_or_too_large_error"])
        XCTAssertEqual(probe.attempts.withLock { $0 }, 0)
    }

    func testNonFiniteResponseIsRefusedWithoutClosingTheChannel() async throws {
        let probe = OutputProbe()
        let channel = channel(probe)
        let request = try NativeWireCommand.decode(command("nan"))
        channel.respond(request, result: ["levels": ["rms_dbfs": Double.nan]])
        XCTAssertTrue(channel.send(nativeTestEvent(2)))
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        let chunks = probe.chunks.withLock { $0 }
        XCTAssertEqual(chunks.count, 2)
        let refusal = try XCTUnwrap(JSONSerialization.jsonObject(with: chunks[0]) as? [String: Any])
        let error = try XCTUnwrap(refusal["error"] as? [String: Any])
        XCTAssertTrue((error["code"] as? String)?.contains("result.levels.rms_dbfs") == true)
        XCTAssertEqual(refusal["ok"] as? Bool, false)
    }

    func testEventLineBoundIncludesTheFinalNewline() async throws {
        let limit = NativeLimits.maximumEventLineBytes
        let empty = try JSONEncoder().encode(NativeEvent.makeError(code: "")).count
        let probe = OutputProbe()
        let channel = channel(probe)
        XCTAssertTrue(channel.send(.makeError(code: String(repeating: "x", count: limit - empty - 1))))
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        let written = try XCTUnwrap(probe.chunks.withLock { $0.first })
        XCTAssertEqual(written.count, limit)
        XCTAssertFalse(channel.send(.makeError(code: String(repeating: "x", count: limit - empty))))
        XCTAssertEqual(probe.attempts.withLock { $0 }, 1)
    }

    func testOutputBudgetIncludesBlockedWriteAndPreservesQueuedWritesAfterBackpressure() async {
        let probe = RecoveringOutputProbe()
        defer { probe.release.signal() }
        let channel = NativeChannel(
            input: Pipe().fileHandleForReading,
            writeOutput: { try probe.write($0) }
        )
        let reasons = Mutex<[String]>([])
        channel.disconnected = { reason in reasons.withLock { $0.append(reason) } }
        let payload = NativeEvent.makeError(code: String(repeating: "x", count: 700_000))
        XCTAssertTrue(channel.send(payload))
        XCTAssertEqual(probe.entered.wait(timeout: .now() + 2), .success)
        XCTAssertTrue(channel.send(payload))
        XCTAssertTrue(channel.send(payload))
        XCTAssertFalse(channel.send(payload))
        probe.release.signal()
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        XCTAssertEqual(reasons.withLock { $0 }, [])
        // The blocked write consumes budget, and every admitted write survives
        // the rejected frame once the physical writer can make progress again.
        XCTAssertEqual(probe.chunks.withLock { $0.count }, 3)
    }

    func testOverloadedResponseOrRejectionDoesNotStopCapture() async throws {
        for rejection in [false, true] {
            let probe = RecoveringOutputProbe()
            let channel = NativeChannel(
                input: Pipe().fileHandleForReading,
                writeOutput: { try probe.write($0) }
            )
            let reasons = Mutex<[String]>([])
            channel.disconnected = { reason in reasons.withLock { $0.append(reason) } }
            let payload = NativeEvent.makeError(code: String(repeating: "x", count: 700_000))
            XCTAssertTrue(channel.send(payload))
            XCTAssertEqual(probe.entered.wait(timeout: .now() + 2), .success)
            XCTAssertTrue(channel.send(payload))
            XCTAssertTrue(channel.send(payload))
            let request = try NativeWireCommand.decode(command("overloaded-reply"))
            if rejection {
                channel.reject(request, code: String(repeating: "r", count: 600_000),
                    message: "Respuesta saturada")
            } else {
                channel.respond(request, result: ["payload": String(repeating: "r", count: 600_000)])
            }
            probe.release.signal()
            let backlogFlushed = await channel.flush()
            XCTAssertTrue(backlogFlushed)
            XCTAssertEqual(probe.chunks.withLock { $0.count }, 3)
            XCTAssertTrue(reasons.withLock { $0.isEmpty })
            XCTAssertTrue(channel.send(nativeTestEvent(1)))
            let recoveryFlushed = await channel.flush()
            XCTAssertTrue(recoveryFlushed)
            XCTAssertEqual(probe.chunks.withLock { $0.count }, 4)
        }
    }

    func testReaderHandlesFragmentedAndCoalescedCommandsAndStartsOnlyOnce() async throws {
        let input = Pipe()
        defer { try? input.fileHandleForWriting.close() }
        let channel = channel(OutputProbe(), input: input)
        let received = expectation(description: "two commands")
        received.expectedFulfillmentCount = 2
        let ids = Mutex<[String]>([])
        channel.receive = { value in ids.withLock { $0.append(value.id) }; received.fulfill() }
        channel.start()
        channel.start()
        let first = command("first")
        try input.fileHandleForWriting.write(contentsOf: first.prefix(11))
        try input.fileHandleForWriting.write(contentsOf: first.dropFirst(11) + command("second"))
        await fulfillment(of: [received], timeout: 2)
        XCTAssertEqual(ids.withLock { $0 }, ["first", "second"])
    }

    /// The daemon's beat is observed on the reader's queue while the main
    /// thread is held: before main has run any of the commands.
    func testACommandIsObservedOffMainBeforeTheMainThreadRunsIt() async throws {
        let input = Pipe()
        defer { try? input.fileHandleForWriting.close() }
        let channel = channel(OutputProbe(), input: input)
        let observed = DispatchSemaphore(value: 0)
        let observedOnMain = Mutex<Bool?>(nil)
        let deliveries = Mutex(0)
        channel.commandRead = {
            observedOnMain.withLock { $0 = Thread.isMainThread }
            observed.signal()
        }
        channel.receive = { _ in deliveries.withLock { $0 += 1 } }
        channel.start()
        let seen = try await MainActor.run {
            // Main is held from the write until the beat is observed.
            try input.fileHandleForWriting.write(contentsOf: command("beat"))
            let seen = observed.wait(timeout: .now() + 2)
            XCTAssertEqual(deliveries.withLock { $0 }, 0)
            return seen
        }
        XCTAssertEqual(seen, .success)
        XCTAssertEqual(observedOnMain.withLock { $0 }, false)
    }

    func testFailureBeforeReaderAdmissionPreventsReceivingFurtherCommands() async throws {
        let input = Pipe()
        let channel = channel(OutputProbe(), input: input)
        let failure = expectation(description: "one failure")
        let deliveries = Mutex(0)
        channel.receive = { _ in deliveries.withLock { $0 += 1 } }
        channel.disconnected = { _ in failure.fulfill() }
        channel.start()
        // Keep main occupied until failure has been latched. No later callback
        // can be admitted merely because the reader decoded it successfully.
        XCTAssertFalse(channel.send(.makeError(code: String(repeating: "x", count: NativeLimits.maximumEventLineBytes))))
        try input.fileHandleForWriting.write(contentsOf: command("late"))
        try input.fileHandleForWriting.close()
        await fulfillment(of: [failure], timeout: 2)
        XCTAssertEqual(deliveries.withLock { $0 }, 0)
        let flushed = await channel.flush()
        XCTAssertFalse(flushed)
    }

    func testEOFIsAFailureNotASuccessfulFlush() async throws {
        let input = Pipe()
        let channel = channel(OutputProbe(), input: input)
        let failure = expectation(description: "EOF")
        let reasons = Mutex<[String]>([])
        channel.disconnected = { reason in reasons.withLock { $0.append(reason) }; failure.fulfill() }
        channel.start()
        try input.fileHandleForWriting.close()
        await fulfillment(of: [failure], timeout: 2)
        XCTAssertEqual(reasons.withLock { $0 }, ["daemon_disconnected"])
        let flushed = await channel.flush()
        XCTAssertFalse(flushed)
    }

    func testMalformedInputCannotBecomeACommand() async throws {
        let input = Pipe()
        defer { try? input.fileHandleForWriting.close() }
        let channel = channel(OutputProbe(), input: input)
        let failure = expectation(description: "decode rejected")
        let reasons = Mutex<[String]>([])
        let deliveries = Mutex(0)
        channel.receive = { _ in deliveries.withLock { $0 += 1 } }
        channel.disconnected = { reason in reasons.withLock { $0.append(reason) }; failure.fulfill() }
        channel.start()
        try input.fileHandleForWriting.write(contentsOf: Data("{invalid}\n".utf8))
        await fulfillment(of: [failure], timeout: 2)
        XCTAssertEqual(reasons.withLock { $0 }, ["invalid_protocol_or_disconnected"])
        XCTAssertEqual(deliveries.withLock { $0 }, 0)
    }

    /// A main thread held past the backlog's bound refuses what does not fit,
    /// from the reader's queue and while main is still held, with its code and
    /// its numbers. The channel stays open: once main returns, everything
    /// admitted runs in order and the next command still arrives.
    func testCommandsPastTheMainThreadBacklogAreRefusedBusyAndTheChannelStaysOpen() async throws {
        let input = Pipe()
        defer { try? input.fileHandleForWriting.close() }
        let probe = OutputProbe()
        let channel = channel(probe, input: input)
        let reasons = Mutex<[String]>([])
        let delivered = Mutex<[String]>([])
        let limit = NativeChannel.maximumPendingCommands
        let extra = 3
        let backlog = expectation(description: "the admitted backlog once main returns")
        backlog.expectedFulfillmentCount = limit
        let after = expectation(description: "a command after main returns")
        channel.disconnected = { (reason: String) in reasons.withLock { $0.append(reason) } }
        channel.receive = { (command: NativeWireCommand) in
            delivered.withLock { $0.append(command.id) }
            if command.id == "after" { after.fulfill() } else { backlog.fulfill() }
        }
        channel.start()
        let batch = commands("status", count: limit + extra)
        // Main is held from the write until every refusal has been written,
        // off main: none of the admitted commands can run meanwhile.
        let refused = try await MainActor.run { () throws -> Int in
            try input.fileHandleForWriting.write(contentsOf: batch)
            var written = 0
            while written < extra, probe.entered.wait(timeout: .now() + 2) == .success {
                written += 1
            }
            XCTAssertEqual(delivered.withLock { $0.count }, 0)
            return written
        }
        XCTAssertEqual(refused, extra)
        await fulfillment(of: [backlog], timeout: 2)
        try input.fileHandleForWriting.write(contentsOf: command("after", kind: "status"))
        await fulfillment(of: [after], timeout: 2)
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        XCTAssertEqual(reasons.withLock { $0 }, [])
        var expected = [String]()
        for index in 0..<limit { expected.append("status-\(index)") }
        expected.append("after")
        XCTAssertEqual(delivered.withLock { $0 }, expected)
        let messages = try decodedChunks(probe)
        var refusedIDs = [String]()
        for message in messages { refusedIDs.append(message["id"] as? String ?? "") }
        XCTAssertEqual(refusedIDs, ["status-\(limit)", "status-\(limit + 1)", "status-\(limit + 2)"])
        for message in messages {
            XCTAssertEqual(message["ok"] as? Bool, false)
            let error = try XCTUnwrap(message["error"] as? [String: Any])
            XCTAssertEqual(error["code"] as? String, NativeChannel.mainThreadBusyCode)
            let detail = try XCTUnwrap(error["detail"] as? [String: Any])
            XCTAssertEqual(detail["pending_commands"] as? Int, limit)
            XCTAssertEqual(detail["max_pending_commands"] as? Int, limit)
        }
    }

    /// A beat is a probe, and reading it is its answer (`commandRead`): past
    /// the backlog's bound beats are neither queued nor refused, so a long
    /// hold never floods the daemon with refusals or closes the channel.
    func testHeartbeatsPastTheMainThreadBacklogAreNeverRefused() async throws {
        let input = Pipe()
        defer { try? input.fileHandleForWriting.close() }
        let probe = OutputProbe()
        let channel = channel(probe, input: input)
        let beats = NativeChannel.maximumPendingCommands * 3
        let reads = Mutex(0)
        let allRead = DispatchSemaphore(value: 0)
        let delivered = Mutex<[String]>([])
        let limit = NativeChannel.maximumPendingCommands
        let backlog = expectation(description: "the admitted beats once main returns")
        backlog.expectedFulfillmentCount = limit
        let after = expectation(description: "a beat after main returns")
        let reasons = Mutex<[String]>([])
        channel.commandRead = { () -> Void in
            let count = reads.withLock { value -> Int in
                value += 1
                return value
            }
            if count == beats { allRead.signal() }
        }
        channel.disconnected = { (reason: String) in reasons.withLock { $0.append(reason) } }
        channel.receive = { (command: NativeWireCommand) in
            delivered.withLock { $0.append(command.id) }
            if command.id == "after" { after.fulfill() } else { backlog.fulfill() }
        }
        channel.start()
        let batch = commands("heartbeat", count: beats)
        let seen = try await MainActor.run { () throws -> DispatchTimeoutResult in
            try input.fileHandleForWriting.write(contentsOf: batch)
            return allRead.wait(timeout: .now() + 2)
        }
        XCTAssertEqual(seen, .success)
        // Only once the admitted beats ran is there room for the next one.
        await fulfillment(of: [backlog], timeout: 2)
        try input.fileHandleForWriting.write(contentsOf: command("after"))
        await fulfillment(of: [after], timeout: 2)
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        var expected = [String]()
        for index in 0..<limit { expected.append("heartbeat-\(index)") }
        expected.append("after")
        XCTAssertEqual(delivered.withLock { $0 }, expected)
        XCTAssertEqual(probe.attempts.withLock { $0 }, 0, "no beat is refused")
        XCTAssertEqual(reasons.withLock { $0 }, [])
    }

    /// The daemon's stops never get a busy refusal: past the backlog they
    /// wait for main in a room of their own, and run once main returns.
    func testStopsPastTheBacklogWaitForMainInTheirOwnRoom() async throws {
        let input = Pipe()
        defer { try? input.fileHandleForWriting.close() }
        let probe = OutputProbe()
        let channel = channel(probe, input: input)
        let limit = NativeChannel.maximumPendingCommands
        var batch = commands("status", count: limit)
        let stops = ["privacy_stop", "capture_stop", "audio_stop", "shutdown"]
        for stop in stops {
            // `audio_stop` names the stream it stops.
            let fields = stop == "audio_stop" ? ",\"stream_id\":\"stream-1\"" : ""
            batch.append(Data("{\"v\":1,\"id\":\"\(stop)\",\"command\":\"\(stop)\"\(fields)}\n".utf8))
        }
        let total = limit + stops.count
        let reads = Mutex(0)
        let allRead = DispatchSemaphore(value: 0)
        let delivered = Mutex<[String]>([])
        let all = expectation(description: "every command after main returns")
        all.expectedFulfillmentCount = total
        let reasons = Mutex<[String]>([])
        channel.commandRead = { () -> Void in
            let count = reads.withLock { value -> Int in
                value += 1
                return value
            }
            if count == total { allRead.signal() }
        }
        channel.disconnected = { (reason: String) in reasons.withLock { $0.append(reason) } }
        channel.receive = { (command: NativeWireCommand) in
            delivered.withLock { $0.append(command.id) }
            all.fulfill()
        }
        channel.start()
        let seen = try await MainActor.run { () throws -> DispatchTimeoutResult in
            try input.fileHandleForWriting.write(contentsOf: batch)
            return allRead.wait(timeout: .now() + 2)
        }
        XCTAssertEqual(seen, .success)
        await fulfillment(of: [all], timeout: 2)
        let flushed = await channel.flush()
        XCTAssertTrue(flushed)
        XCTAssertEqual(Array(delivered.withLock { $0 }.suffix(stops.count)), stops)
        XCTAssertEqual(probe.attempts.withLock { $0 }, 0, "no stop is refused")
        XCTAssertEqual(reasons.withLock { $0 }, [])
    }

    /// Past the stops' own room the channel still fails closed, as every
    /// command past the backlog once did: the helper then stops capture and
    /// leaves, and the backlog main had not run is discarded.
    func testAStopPastItsOwnRoomStillFailsTheChannelClosed() async throws {
        let input = Pipe()
        defer { try? input.fileHandleForWriting.close() }
        let channel = channel(OutputProbe(), input: input)
        let failure = expectation(description: "stop room")
        let reasons = Mutex<[String]>([])
        let deliveries = Mutex(0)
        channel.receive = { (_: NativeWireCommand) in deliveries.withLock { $0 += 1 } }
        channel.disconnected = { (reason: String) in
            reasons.withLock { $0.append(reason) }
            failure.fulfill()
        }
        channel.start()
        let terminalObserved = DispatchSemaphore(value: 0)
        let observing = Task.detached { () async -> Void in
            let deadline = ContinuousClock.now.advanced(by: .seconds(2))
            while ContinuousClock.now < deadline {
                if !(await channel.flush()) { terminalObserved.signal(); return }
                await Task.yield()
            }
        }
        var batch = commands("status", count: NativeChannel.maximumPendingCommands)
        batch.append(commands("privacy_stop", count: NativeChannel.maximumPendingStops + 1))
        // Deliberately hold the fixture's MainActor briefly so the real reader
        // fills its pending slots and the stops' room. The production callback
        // never blocks main. This is a bounded fixture, not a production wait.
        let observed = try await MainActor.run { () throws -> DispatchTimeoutResult in
            // No suspension between writing the batch and holding main: even
            // an executor hop into this fixture cannot drain its backlog early.
            try input.fileHandleForWriting.write(contentsOf: batch)
            return terminalObserved.wait(timeout: .now() + 2)
        }
        observing.cancel()
        XCTAssertEqual(observed, .success)
        await fulfillment(of: [failure], timeout: 2)
        XCTAssertEqual(reasons.withLock { $0 }, ["native_command_backpressure"])
        XCTAssertEqual(deliveries.withLock { $0 }, 0)
    }

    func testCancellingFlushCannotFreeItsPhysicalWriteOrConfirmEarly() async {
        let probe = OutputProbe(blocked: true)
        defer { probe.release.signal() }
        let channel = channel(probe)
        XCTAssertTrue(channel.send(nativeTestEvent(1)))
        XCTAssertEqual(probe.entered.wait(timeout: .now() + 2), .success)
        let entered = expectation(description: "flush before cancellation")
        let result = Mutex<Bool?>(nil)
        let flush = Task.detached {
            entered.fulfill()
            let complete = await channel.flush()
            result.withLock { $0 = complete }
            return complete
        }
        await fulfillment(of: [entered], timeout: 2)
        flush.cancel()
        XCTAssertNil(result.withLock { $0 })
        XCTAssertEqual(probe.chunks.withLock { $0.count }, 0)
        probe.release.signal()
        let complete = await flush.value
        XCTAssertTrue(complete)
        XCTAssertEqual(probe.chunks.withLock { $0.count }, 1)
    }
}
