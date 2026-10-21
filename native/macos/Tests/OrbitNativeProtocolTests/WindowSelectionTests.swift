import XCTest
@testable import OrbitNativeProtocol

final class WindowSelectionTests: XCTestCase {
    private let nativeID = "11111111-1111-4111-8111-111111111111"
    private let inventoryID = "22222222-2222-4222-8222-222222222222"
    private let token = "33333333-3333-4333-8333-333333333333"
    private let applicationID = "44444444-4444-4444-8444-444444444444"
    private let otherID = "55555555-5555-4555-8555-555555555555"
    private let descriptorDigest = String(repeating: "a", count: 64)

    private func binding(token: String? = nil, pid: Int32 = 123, window: UInt32 = 42,
                         generation: UInt64 = 7, application: String? = nil,
                         hash: String? = nil, inventory: String? = nil, native: String? = nil) -> WindowBinding {
        WindowBinding(nativeInstanceID: native ?? nativeID, privacyGeneration: generation,
            inventoryID: inventory ?? inventoryID, selectionToken: token ?? self.token,
            windowID: window, applicationPID: pid, applicationInstanceID: application ?? applicationID,
            descriptorHash: hash ?? descriptorDigest)
    }

    private func issued(_ bindings: [WindowBinding]? = nil) throws -> WindowSelectionPolicy {
        var policy = WindowSelectionPolicy(nativeInstanceID: nativeID)
        try policy.issue(inventoryID: inventoryID, observedNanoseconds: 100,
            expiresNanoseconds: 30_000_000_100, privacyGeneration: 7, bindings: bindings ?? [binding()])
        return policy
    }

    private func assertFailure<T>(_ expected: WindowSelectionFailure,
                                  _ operation: @autoclosure () throws -> T,
                                  file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertThrowsError(try operation(), file: file, line: line) { error in
            XCTAssertEqual(error as? WindowSelectionFailure, expected, file: file, line: line)
        }
    }

    func testCommandRequiresTheCompleteBindingNotAnOldBareWindowID() throws {
        let data = try JSONSerialization.data(withJSONObject: ["v": 1, "id": "capture-1",
            "command": "screenshot", "window_binding": binding().fields])
        guard case .screenshot(let command) = try NativeWireCommand.decode(data) else {
            return XCTFail("screenshot variant")
        }
        XCTAssertEqual(command.fields.window_binding, binding())
        XCTAssertThrowsError(try NativeWireCommand.decode(Data(#"{"v":1,"id":"old","command":"screenshot","window_id":42}"#.utf8)))
        var policy = try issued()
        assertFailure(.invalidBinding, try policy.consume(nil, now: 200,
            privacyGeneration: 7, privacyStopped: false))
        XCTAssertFalse(policy.capturePending)
    }

    func testOnlyAnExactIssuedBindingCanConsumeItsToken() throws {
        var policy = try issued()
        let mismatches = [binding(token: otherID), binding(pid: 124), binding(window: 43),
            binding(generation: 8), binding(application: otherID), binding(hash: String(repeating: "b", count: 64)),
            binding(inventory: otherID), binding(native: otherID)]
        for changed in mismatches {
            assertFailure(.selectionUnavailable, try policy.consume(changed, now: 200,
                privacyGeneration: 7, privacyStopped: false))
            XCTAssertFalse(policy.capturePending)
        }
        let permit = try policy.consume(binding(), now: 200, privacyGeneration: 7, privacyStopped: false)
        XCTAssertEqual(permit.binding, binding())
    }

    func testSingleUseSurvivesCaptureFailure() throws {
        var policy = try issued()
        let permit = try policy.consume(binding(), now: 200, privacyGeneration: 7, privacyStopped: false)
        policy.complete(permit) // A failed SDK operation must not restore the capability.
        assertFailure(.selectionUnavailable, try policy.consume(binding(), now: 201,
            privacyGeneration: 7, privacyStopped: false))
    }

    func testInventoryExpiresAtTheBoundaryAndRejectsBackwardsClock() throws {
        for now in [UInt64(99), 30_000_000_100, UInt64.max] {
            var policy = try issued()
            assertFailure(.selectionExpired, try policy.consume(binding(), now: now,
                privacyGeneration: 7, privacyStopped: false))
        }
        var policy = try issued()
        let permit = try policy.consume(binding(), now: 30_000_000_099, privacyGeneration: 7, privacyStopped: false)
        XCTAssertEqual(permit.deadlineNanoseconds, 30_000_000_100)
        assertFailure(.captureExpired, try policy.validate(permit, now: 30_000_000_100,
            privacyGeneration: 7, privacyStopped: false, descriptorHash: descriptorDigest))
    }

    func testNewInventoryInvalidatesThePreviousSelection() throws {
        var policy = try issued()
        try policy.issue(inventoryID: otherID, observedNanoseconds: 200, expiresNanoseconds: 300,
                         privacyGeneration: 7, bindings: [])
        assertFailure(.selectionUnavailable, try policy.consume(binding(), now: 201,
            privacyGeneration: 7, privacyStopped: false))
    }

    func testRearmCannotResurrectARevokedInventoryOrLatePixels() throws {
        var policy = try issued()
        let permit = try policy.consume(binding(), now: 200, privacyGeneration: 7, privacyStopped: false)
        policy.invalidate()
        XCTAssertTrue(policy.capturePending, "privacy revocation does not cancel the physical SDK call")
        assertFailure(.privacyStopped, try policy.validate(permit, now: 201,
            privacyGeneration: 8, privacyStopped: false, descriptorHash: descriptorDigest))
        assertFailure(.captureExpired, try policy.validate(permit, now: 201,
            privacyGeneration: 7, privacyStopped: false, descriptorHash: descriptorDigest))
        policy.complete(permit)
        assertFailure(.selectionUnavailable, try policy.consume(binding(), now: 201,
            privacyGeneration: 7, privacyStopped: false))
    }

    func testPrivacyStopPreventsEvenTheFirstCapture() throws {
        var policy = try issued()
        assertFailure(.privacyStopped, try policy.consume(binding(), now: 200,
            privacyGeneration: 7, privacyStopped: true))
        XCTAssertFalse(policy.capturePending)
    }

    func testTimeoutKeepsPhysicalSlotOccupiedUntilTheOriginalCallbackCompletes() throws {
        let second = binding(token: otherID)
        var policy = try issued([binding(), second])
        let first = try policy.consume(binding(), now: 200, privacyGeneration: 7, privacyStopped: false)
        policy.expire(first)
        XCTAssertTrue(policy.capturePending)
        assertFailure(.captureExpired, try policy.validate(first, now: 201,
            privacyGeneration: 7, privacyStopped: false, descriptorHash: descriptorDigest))
        assertFailure(.captureBusy, try policy.consume(second, now: 201,
            privacyGeneration: 7, privacyStopped: false))
        policy.complete(first)
        let next = try policy.consume(second, now: 202, privacyGeneration: 7, privacyStopped: false)
        policy.complete(first) // A duplicate old callback cannot free another request's slot.
        XCTAssertTrue(policy.capturePending)
        try policy.validate(next, now: 203, privacyGeneration: 7, privacyStopped: false, descriptorHash: descriptorDigest)
    }

    func testPreAndPostObservationsRejectMissingOrChangedIdentity() throws {
        var policy = try issued()
        let permit = try policy.consume(binding(), now: 200, privacyGeneration: 7, privacyStopped: false)
        try policy.validate(permit, now: 201, privacyGeneration: 7, privacyStopped: false, descriptorHash: descriptorDigest)
        for observed in [nil, String(repeating: "b", count: 64)] {
            assertFailure(.identityChanged, try policy.validate(permit, now: 202,
                privacyGeneration: 7, privacyStopped: false, descriptorHash: observed))
        }
        assertFailure(.captureExpired, try policy.validate(permit, now: 199,
            privacyGeneration: 7, privacyStopped: false, descriptorHash: descriptorDigest))
    }

    func testInventoryRejectsExcessiveLifetimeDuplicateTokensAndCounts() throws {
        var policy = WindowSelectionPolicy(nativeInstanceID: nativeID)
        for expires in [UInt64(100), 99, 30_000_000_101] {
            assertFailure(.invalidInventory, try policy.issue(inventoryID: inventoryID,
                observedNanoseconds: 100, expiresNanoseconds: expires, privacyGeneration: 7, bindings: []))
        }
        assertFailure(.invalidInventory, try policy.issue(inventoryID: inventoryID,
            observedNanoseconds: 100, expiresNanoseconds: 200, privacyGeneration: 7, bindings: [binding(), binding()]))
        assertFailure(.invalidInventory, try policy.issue(inventoryID: inventoryID,
            observedNanoseconds: 100, expiresNanoseconds: 200, privacyGeneration: 7,
            bindings: Array(repeating: binding(), count: 201)))
        try policy.issue(inventoryID: inventoryID, observedNanoseconds: UInt64.max - 1,
                         expiresNanoseconds: UInt64.max, privacyGeneration: 7, bindings: [binding()])
        let permit = try policy.consume(binding(), now: UInt64.max - 1, privacyGeneration: 7, privacyStopped: false)
        XCTAssertEqual(permit.deadlineNanoseconds, UInt64.max)
    }

    func testMalformedIdentityAndHashCannotEnterInventory() {
        XCTAssertFalse(binding(token: "not-a-uuid").isWellFormed)
        XCTAssertFalse(binding(pid: 0).isWellFormed)
        XCTAssertFalse(binding(window: 0).isWellFormed)
        XCTAssertFalse(binding(hash: String(repeating: "A", count: 64)).isWellFormed)
        XCTAssertFalse(binding(hash: String(repeating: "a", count: 63)).isWellFormed)
        XCTAssertTrue(binding().isWellFormed)
    }

    func testPresentationAndPixelBoundsDoNotSplitUTF8OrConvertNonfiniteDimensions() {
        let text = String(repeating: "é🙂", count: 200)
        let title = WindowPresentation.bounded(text, maximumBytes: 256)
        XCTAssertLessThanOrEqual(title.utf8.count, 256)
        XCTAssertTrue(text.hasPrefix(title))
        XCTAssertEqual(WindowPresentation.bounded("🙂", maximumBytes: 3), "")
        for invalid in [Double.nan, Double.infinity, -Double.infinity, 0, -1] {
            XCTAssertNil(WindowPresentation.captureSize(width: invalid, height: 100))
            XCTAssertNil(WindowPresentation.captureSize(width: 100, height: invalid))
        }
        let size = WindowPresentation.captureSize(width: 2560, height: 1600)
        XCTAssertEqual(size?.width, 1280)
        XCTAssertEqual(size?.height, 800)
        XCTAssertNil(WindowPresentation.captureSize(width: Double.greatestFiniteMagnitude, height: 1))
    }

    func testTextAndScreenshotConsumeTheSameTokenWithoutFallback() throws {
        for kind in [WindowCaptureKind.screenshot, .selectedText] {
            var policy = try issued()
            let first = try policy.consume(binding(), now: 200, privacyGeneration: 7, privacyStopped: false, kind: kind)
            policy.complete(first)
            let fallback: WindowCaptureKind = kind == .screenshot ? .selectedText : .screenshot
            assertFailure(.selectionUnavailable, try policy.consume(binding(), now: 201,
                privacyGeneration: 7, privacyStopped: false, kind: fallback))
        }
    }

    func testTextDeadlineIsFiveSecondsAndKeepsBothKindsBusyAfterTimeoutOrStop() throws {
        let second = binding(token: otherID)
        var policy = try issued([binding(), second])
        let first = try policy.consume(binding(), now: 200, privacyGeneration: 7,
                                       privacyStopped: false, kind: .selectedText)
        XCTAssertEqual(first.deadlineNanoseconds, 5_000_000_200)
        assertFailure(.captureExpired, try policy.validate(first, now: first.deadlineNanoseconds,
            privacyGeneration: 7, privacyStopped: false, descriptorHash: descriptorDigest))
        policy.expire(first)
        policy.invalidate()
        for kind in [WindowCaptureKind.screenshot, .selectedText] {
            assertFailure(.captureBusy, try policy.consume(second, now: 202,
                privacyGeneration: 7, privacyStopped: false, kind: kind))
        }
        policy.complete(first)
        XCTAssertFalse(policy.capturePending)
        assertFailure(.selectionUnavailable, try policy.consume(second, now: 203,
            privacyGeneration: 7, privacyStopped: false, kind: .selectedText))
    }

    func testSelectedTextCommandHasTheSameExactBindingEnvelope() throws {
        let encoded = try JSONSerialization.data(withJSONObject: ["v": 1, "id": "text-1",
            "command": "selected_text", "window_binding": binding().fields])
        guard case .selectedText(let command) = try NativeWireCommand.decode(encoded) else {
            return XCTFail("selected_text variant")
        }
        XCTAssertEqual(command.fields.window_binding, binding())
    }

    func testSelectedWindowMayBeJoinedWithAnUntitledAuxiliaryWindowForTheSamePID() {
        XCTAssertTrue(WindowIdentityJoin.containsSelectedWindow(42,
            screenCaptureWindowIDs: [42], coreGraphicsWindowIDs: [42, 99]))
        XCTAssertFalse(WindowIdentityJoin.containsSelectedWindow(42,
            screenCaptureWindowIDs: [42], coreGraphicsWindowIDs: [99]))
        XCTAssertFalse(WindowIdentityJoin.containsSelectedWindow(42,
            screenCaptureWindowIDs: [99], coreGraphicsWindowIDs: [42]))
    }

    func testInventoryRefreshRevokesFutureWorkerReadsWithoutFreeingThePhysicalSlot() throws {
        var policy = try issued()
        let first = try policy.consume(binding(), now: 200, privacyGeneration: 7,
                                       privacyStopped: false, kind: .selectedText)
        let worker = WindowReadRevocation()
        var budget = SelectedTextReadBudget(started: first.startedNanoseconds, deadline: first.deadlineNanoseconds)
        try budget.admit(now: 201, revoked: worker.revoked)
        WindowReadRevocation.invalidate(&policy, worker: worker) // Same production refresh helper.
        XCTAssertTrue(worker.revoked)
        XCTAssertTrue(policy.capturePending)
        XCTAssertThrowsError(try budget.admit(now: 202, revoked: worker.revoked))
        XCTAssertEqual(budget.calls, 1, "refresh must prevent every subsequent content getter")
        let nextBinding = binding(token: otherID, inventory: otherID)
        try policy.issue(inventoryID: otherID, observedNanoseconds: 203, expiresNanoseconds: 1000,
                         privacyGeneration: 7, bindings: [nextBinding])
        assertFailure(.captureBusy, try policy.consume(nextBinding, now: 204,
            privacyGeneration: 7, privacyStopped: false, kind: .screenshot))
        policy.complete(first) // Only physical completion frees the slot.
        _ = try policy.consume(nextBinding, now: 205, privacyGeneration: 7, privacyStopped: false)
    }
}
