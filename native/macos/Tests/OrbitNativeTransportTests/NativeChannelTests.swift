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

    private func command(_ id: String) -> Data {
        Data("{\"v\":1,\"id\":\"\(id)\",\"command\":\"heartbeat\"}\n".utf8)
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

        let framesPerEvent = 48_000
        let pcm = Data(repeating: 0, count: framesPerEvent * 2).base64EncodedString()
        let submittedCount = 40
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

    func testCommandOverloadDiscardsTheAlreadyAdmittedMainQueueBacklog() async throws {
        let input = Pipe()
        defer { try? input.fileHandleForWriting.close() }
        let channel = channel(OutputProbe(), input: input)
        let failure = expectation(description: "command budget")
        let reasons = Mutex<[String]>([])
        let deliveries = Mutex(0)
        channel.receive = { _ in deliveries.withLock { $0 += 1 } }
        channel.disconnected = { reason in reasons.withLock { $0.append(reason) }; failure.fulfill() }
        channel.start()
        let terminalObserved = DispatchSemaphore(value: 0)
        let observing = Task.detached {
            let deadline = ContinuousClock.now.advanced(by: .seconds(2))
            while ContinuousClock.now < deadline {
                if !(await channel.flush()) { terminalObserved.signal(); return }
                await Task.yield()
            }
        }
        let batch = (0..<65).reduce(into: Data()) { $0.append(command(String($1))) }
        // Deliberately hold the fixture's MainActor briefly so the real reader
        // fills its 64 pending slots. The production callback never blocks main.
        // Model a synchronous main-thread stall explicitly, outside an async
        // context. This is a bounded fixture, not a production wait strategy.
        let observed = try await MainActor.run {
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
