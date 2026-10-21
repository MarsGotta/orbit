import XCTest
@testable import OrbitNativeProtocol

private actor DrainGate {
    private var continuation: CheckedContinuation<Void, Never>?
    private(set) var entered = false
    private var open = false

    func wait() async {
        entered = true
        if open { return }
        await withCheckedContinuation { continuation = $0 }
    }
    func release() { open = true; continuation?.resume(); continuation = nil }
}

private actor DrainTrace {
    private(set) var entries = [String]()
    func record(_ value: String) { entries.append(value) }
}

final class AudioDrainBarrierTests: XCTestCase {
    func testConcurrentStopsCannotAcknowledgeBeforeBothProducerTails() async {
        let barrier = AudioDrainBarrier()
        let microphone = DrainGate()
        let remote = DrainGate()
        let trace = DrainTrace()
        let first = Task {
            let completed = await barrier.complete {
                await trace.record("drain-start")
                await microphone.wait()
                await trace.record("microphone-tail")
                await remote.wait()
                await trace.record("remote-tail")
                return true
            }
            if completed { await trace.record("ack-first") }
            return completed
        }
        await eventually { await microphone.entered }
        let second = Task {
            let completed = await barrier.complete {
                await trace.record("incorrect-second-drain")
                return false
            }
            if completed { await trace.record("ack-second") }
            return completed
        }
        await eventually { await barrier.waiterCount == 2 }
        let beforeMicrophone = await trace.entries
        XCTAssertEqual(beforeMicrophone, ["drain-start"])
        await microphone.release()
        await eventually { await remote.entered }
        let beforeRemote = await trace.entries
        XCTAssertEqual(beforeRemote, ["drain-start", "microphone-tail"])
        await remote.release()
        let firstResult = await first.value
        let secondResult = await second.value
        XCTAssertTrue(firstResult)
        XCTAssertTrue(secondResult)
        let completed = await trace.entries
        XCTAssertEqual(Array(completed.prefix(3)), ["drain-start", "microphone-tail", "remote-tail"])
        XCTAssertEqual(Set(completed.suffix(2)), ["ack-first", "ack-second"])
        XCTAssertEqual(completed.count, 5)
    }

    func testIncompleteDrainIsNotSuccessAndLaterAttemptIsIndependent() async {
        let barrier = AudioDrainBarrier()
        let failed = await barrier.complete { false }
        XCTAssertFalse(failed)
        let recovered = await barrier.complete { true }
        XCTAssertTrue(recovered)
    }

    private func eventually(_ condition: @escaping () async -> Bool) async {
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: .seconds(2))
        while !(await condition()) {
            if clock.now >= deadline { XCTFail("Fixture did not reach the expected drain phase"); return }
            await Task.yield()
        }
    }
}
