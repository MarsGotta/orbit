import AppKit
import ApplicationServices
import ScreenCaptureKit
import OrbitNativeProtocol
import OrbitNativeState

/// Only check names, booleans and numeric identities reach the durable log.
/// The shared adapter never receives a window title, text or descriptor hash.
func recordWindowReadCheck(_ passed: Bool, _ name: String, diagnostics: DiagnosticLog,
                           fields: [String: String] = [:]) -> Bool {
    diagnostics.recordOffMain("window_read_check", fields.merging(
        ["check": name, "passed": passed ? "true" : "false"]) { _, value in value })
    return passed
}

/// Bitmap conversion and base64 are CPU work and never run on AppKit's thread.
private final class ScreenshotEncoder: @unchecked Sendable {
    private let queue = DispatchQueue(label: "orbit.native.screenshot-encoding", qos: .userInitiated)

    func encode(_ image: CGImage) async throws -> String {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                let bitmap = NSBitmapImageRep(cgImage: image)
                guard let data = bitmap.representation(using: .png, properties: [:]),
                      !data.isEmpty, data.count <= NativeLimits.maximumScreenshotPNGBytes else {
                    continuation.resume(throwing: NativeFailure("screenshot_too_large"))
                    return
                }
                continuation.resume(returning: data.base64EncodedString())
            }
        }
    }
}

/// Shared source coordinator. Main-thread state owns BOTH source kinds' token,
/// physical slot and delivery; blocking Accessibility reads have a serial worker.
/// It never activates a window, asks for TCC, or falls back to another source kind.
package final class WindowScreenshots {
    package let nativeInstanceID = UUID().uuidString.lowercased()
    private lazy var policy = WindowSelectionPolicy(nativeInstanceID: nativeInstanceID)
    private var targets = [String: Target]()
    private var inventoryInFlight = false
    private var physicalCapture: WindowCapturePermit?
    private var reply: CheckedContinuation<[String: Any], Error>?
    private var deadlineTask: Task<Void, Never>?
    private var textRevocation: WindowReadRevocation?
    private let diagnostics: DiagnosticLog
    private let textCapture: AXSelectedTextCapture
    private let screenshotEncoder = ScreenshotEncoder()
    private typealias Target = ObservedWindowTarget

    /// Revocation cancels delivery, not the uninterruptible SDK call or its slot.
    package init(diagnostics: DiagnosticLog) {
        self.diagnostics = diagnostics
        textCapture = AXSelectedTextCapture(diagnostics: diagnostics)
    }

    private func check(_ passed: Bool, _ name: String, _ fields: [String: String] = [:]) -> Bool {
        recordWindowReadCheck(passed, name, diagnostics: diagnostics, fields: fields)
    }

    package func invalidate() {
        WindowReadRevocation.invalidate(&policy, worker: textRevocation)
        targets.removeAll()
        finishReply(.failure(WindowSelectionFailure.privacyStopped))
    }

    @MainActor package func inventory(privacyGeneration: UInt64,
                             authorization: () -> (generation: UInt64, stopped: Bool)) async throws -> [String: Any] {
        guard !inventoryInFlight else { throw NativeFailure("screen_inventory_busy") }
        inventoryInFlight = true
        defer { inventoryInFlight = false }
        // A refresh revokes the previous inventory even when a direct caller
        // bypasses the application's command serialization. Stop further AX
        // getters as well as delivery, without freeing an in-flight SDK slot.
        WindowReadRevocation.invalidate(&policy, worker: textRevocation)
        finishReply(.failure(WindowSelectionFailure.selectionUnavailable))
        targets.removeAll()
        try authorize(generation: privacyGeneration, authorization: authorization)
        guard check(CGPreflightScreenCaptureAccess(), "screen_permission") else { throw NativeFailure("screen_permission_required") }
        let observed = try clock()
        let (expires, overflow) = observed.addingReportingOverflow(WindowSelectionPolicy.inventoryLifetimeNanoseconds)
        guard !overflow else { throw NativeFailure("capture_clock_unavailable") }
        let inventoryID = UUID().uuidString.lowercased()
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
        try authorize(generation: privacyGeneration, authorization: authorization)
        var rows = [[String: Any]]()
        var bindings = [WindowBinding]()
        var issued = [String: Target]()
        var applicationInstances = [Int32: (launchDate: Date, bundleID: String, id: String)]()
        var rowBytes = 0
        for window in content.windows.prefix(WindowSelectionPolicy.maximumWindows) {
            let owner = window.owningApplication
            let pid = owner?.processID ?? 0
            let fullTitle = window.title ?? ""
            let title = WindowPresentation.bounded(fullTitle, maximumBytes: 256)
            let name = WindowPresentation.bounded(owner?.applicationName ?? "", maximumBytes: 128)
            let application = pid > 0 ? NSRunningApplication(processIdentifier: pid) : nil
            let bundleID = application?.bundleIdentifier ?? ""
            var binding: WindowBinding?
            var target: Target?
            if let application, !application.isTerminated, let launchDate = application.launchDate,
               !bundleID.isEmpty, bundleID.utf8.count <= 256,
               owner?.bundleIdentifier == bundleID {
                let instanceID: String
                if let previous = applicationInstances[pid], previous.launchDate == launchDate,
                   previous.bundleID == bundleID {
                    instanceID = previous.id
                } else {
                    instanceID = UUID().uuidString.lowercased()
                    applicationInstances[pid] = (launchDate, bundleID, instanceID)
                }
                if let descriptor = ObservedWindowDescriptor.observe(window: window, applicationInstanceID: instanceID,
                                               launchDate: launchDate, bundleID: bundleID),
                   let hash = descriptor.hash {
                    let token = UUID().uuidString.lowercased()
                    binding = WindowBinding(nativeInstanceID: nativeInstanceID,
                        privacyGeneration: privacyGeneration, inventoryID: inventoryID,
                        selectionToken: token, windowID: window.windowID, applicationPID: pid,
                        applicationInstanceID: instanceID, descriptorHash: hash)
                    target = Target(window: window, filter: SCContentFilter(desktopIndependentWindow: window),
                        application: application, launchDate: launchDate, bundleID: bundleID,
                        applicationInstanceID: instanceID, descriptor: descriptor)
                }
            }
            let row: [String: Any] = ["binding": binding?.fields as Any? ?? NSNull(),
                "window_id": window.windowID, "application_pid": pid, "application_name": name,
                "bundle_id": bundleID.utf8.count <= 256 ? bundleID : "", "title": title,
                "title_truncated": title != fullTitle,
                "unavailable_reason": binding == nil ? "window_identity_unavailable" as Any : NSNull()]
            let bytes = try JSONSerialization.data(withJSONObject: row).count
            // Reserve an envelope budget and count JSON commas/brackets conservatively.
            guard rowBytes + bytes + 2 <= WindowSelectionPolicy.maximumInventoryBytes - 2048 else { break }
            rowBytes += bytes + 2
            rows.append(row)
            if let binding, let target {
                bindings.append(binding)
                issued[binding.selection_token] = target
            }
        }
        let now = try clock()
        try authorize(generation: privacyGeneration, authorization: authorization)
        guard now >= observed, now < expires else { throw WindowSelectionFailure.selectionExpired }
        let result: [String: Any] = ["native_instance_id": nativeInstanceID,
            "privacy_generation": privacyGeneration, "inventory_id": inventoryID,
            "clock_domain": "mach_absolute_time", "observed_monotonic_ns": observed,
            "expires_monotonic_ns": expires, "windows": rows,
            "omitted_count": content.windows.count - rows.count, "selected_text_available": AXIsProcessTrusted()]
        guard try JSONSerialization.data(withJSONObject: result).count <= WindowSelectionPolicy.maximumInventoryBytes else {
            throw NativeFailure("screen_inventory_too_large")
        }
        try policy.issue(inventoryID: inventoryID, observedNanoseconds: observed, expiresNanoseconds: expires,
                         privacyGeneration: privacyGeneration, bindings: bindings)
        targets = issued
        for binding in bindings {
            _ = check(true, "inventory_binding", ["window_id": String(binding.window_id),
                      "pid": String(binding.application_pid), "inventory_id": binding.inventory_id])
        }
        return result
    }

    @MainActor package func capture(_ binding: WindowBinding?,
                           kind: WindowCaptureKind = .screenshot,
                           authorization: @escaping () -> (generation: UInt64, stopped: Bool)) async throws -> [String: Any] {
        let auth = authorization()
        let permit: WindowCapturePermit
        do {
            permit = try policy.consume(binding, now: clock(), privacyGeneration: auth.generation,
                                        privacyStopped: auth.stopped, kind: kind)
        } catch {
            diagnostics.recordOffMain("window_read_refused", ["check": "selection_permit",
                "code": (error as? WindowSelectionFailure)?.rawValue ?? "capture_clock_unavailable"])
            throw error
        }
        guard let target = targets.removeValue(forKey: permit.binding.selection_token) else {
            policy.complete(permit)
            throw WindowSelectionFailure.selectionUnavailable
        }
        _ = check(true, "capture_binding", ["window_id": String(permit.binding.window_id),
                  "pid": String(permit.binding.application_pid), "inventory_id": permit.binding.inventory_id])
        // Includes preparation: even a slow preflight query must not accumulate.
        physicalCapture = permit
        if kind == .selectedText { textRevocation = WindowReadRevocation() }
        return try await withCheckedThrowingContinuation { continuation in
            reply = continuation
            deadlineTask = Task { @MainActor [weak self] in
                guard let self else { return }
                let now = (try? self.clock()) ?? permit.deadlineNanoseconds
                let remaining = permit.deadlineNanoseconds > now ? permit.deadlineNanoseconds - now : 0
                do { try await Task.sleep(nanoseconds: remaining) } catch { return }
                guard self.physicalCapture == permit else { return }
                self.textRevocation?.revoke()
                self.policy.expire(permit)
                self.finishReply(.failure(permit.kind == .selectedText
                    ? SelectedTextFailure.expired as Error : WindowSelectionFailure.captureExpired))
            }
            Task { @MainActor [weak self] in
                await self?.prepare(permit, target: target, authorization: authorization)
            }
        }
    }

    @MainActor private func prepare(_ permit: WindowCapturePermit, target: Target,
                                    authorization: @escaping () -> (generation: UInt64, stopped: Bool)) async {
        do {
            try validate(permit, descriptorHash: target.descriptor.hash, authorization: authorization)
            if permit.kind == .selectedText {
                try await prepareText(permit, target: target, authorization: authorization)
                releaseSlot(permit)
                return
            }
            let before = try await currentDescriptor(target)
            try validate(permit, descriptorHash: before, authorization: authorization)
            guard let size = WindowPresentation.captureSize(width: target.descriptor.width, height: target.descriptor.height) else {
                throw NativeFailure("invalid_window_dimensions")
            }
            let configuration = SCStreamConfiguration()
            configuration.width = size.width
            configuration.height = size.height
            configuration.showsCursor = false
            // No suspension between the final authorization and SDK submission.
            try validate(permit, descriptorHash: before, authorization: authorization)
            SCScreenshotManager.captureImage(contentFilter: target.filter, configuration: configuration) {
                [weak self] image, error in
                Task { @MainActor in
                    await self?.completed(permit, target: target, image: image, error: error,
                                          authorization: authorization)
                }
            }
        } catch {
            finishReply(.failure(error))
            releaseSlot(permit)
        }
    }

    @MainActor private func completed(_ permit: WindowCapturePermit, target: Target, image: CGImage?,
                                      error: Error?, authorization: () -> (generation: UInt64, stopped: Bool)) async {
        guard physicalCapture == permit else { return }
        defer { releaseSlot(permit) }
        // Revoked/expired requests release their slot here, but never process or deliver pixels.
        guard reply != nil else { return }
        do {
            try validate(permit, descriptorHash: target.descriptor.hash, authorization: authorization)
            guard error == nil, let image else { throw NativeFailure("screenshot_sdk_failed") }
            guard image.width > 0, image.height > 0, image.width <= 1280, image.height <= 1024 else {
                throw NativeFailure("invalid_screenshot_dimensions")
            }
            let after = try await currentDescriptor(target)
            try validate(permit, descriptorHash: after, authorization: authorization)
            guard check(CGPreflightScreenCaptureAccess(), "screen_permission") else { throw NativeFailure("screen_permission_required") }
            let encoded = try await screenshotEncoder.encode(image)
            let completed = try clock()
            let auth = authorization()
            try policy.validate(permit, now: completed, privacyGeneration: auth.generation,
                                privacyStopped: auth.stopped, descriptorHash: after)
            finishReply(.success(["binding": permit.binding.fields, "clock_domain": "mach_absolute_time",
                "capture_started_monotonic_ns": permit.startedNanoseconds,
                "capture_completed_monotonic_ns": completed, "mime_type": "image/png",
                "width": image.width, "height": image.height, "data_base64": encoded,
                "source": "native_window"]))
        } catch { finishReply(.failure(error)) }
    }

    private func finishReply(_ result: Result<[String: Any], Error>) {
        let continuation = reply
        reply = nil
        if case .failure(let error) = result, continuation != nil {
            diagnostics.recordOffMain("window_read_refused", [
                "code": (error as? SelectedTextFailure)?.rawValue
                    ?? (error as? WindowSelectionFailure)?.rawValue
                    ?? (error as? NativeFailure)?.code ?? "source_capture_failed",
                "window_id": String(physicalCapture?.binding.window_id ?? 0),
                "pid": String(physicalCapture?.binding.application_pid ?? 0)
            ])
        }
        continuation?.resume(with: result)
    }

    private func releaseSlot(_ permit: WindowCapturePermit) {
        guard physicalCapture == permit else { return }
        deadlineTask?.cancel()
        deadlineTask = nil
        physicalCapture = nil
        textRevocation = nil
        policy.complete(permit)
    }

    @MainActor private func currentDescriptor(_ target: Target) async throws -> String? {
        guard check(!target.application.isTerminated, "screenshot_retained_process") else { return nil }
        guard check(CGPreflightScreenCaptureAccess(), "screenshot_screen_permission") else {
            throw NativeFailure("screen_permission_required")
        }
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
        guard check(!target.application.isTerminated, "screenshot_retained_process_after_inventory") else { return nil }
        guard let application = NSRunningApplication(processIdentifier: target.descriptor.applicationPID) else {
            _ = check(false, "screenshot_process_lookup")
            return nil
        }
        guard check(!application.isTerminated, "screenshot_process_running"),
              check(application.launchDate == target.launchDate, "screenshot_process_launch"),
              check(application.bundleIdentifier == target.bundleID, "screenshot_process_bundle") else { return nil }
        guard let window = content.windows.first(where: { $0.windowID == target.window.windowID }) else {
            _ = check(false, "screenshot_window_lookup", ["window_id": String(target.window.windowID),
                      "windows": String(content.windows.count)])
            return nil
        }
        guard check(window.owningApplication?.processID == target.descriptor.applicationPID,
                    "screenshot_window_process", ["expected_pid": String(target.descriptor.applicationPID),
                    "observed_pid": String(window.owningApplication?.processID ?? 0)]),
              check(window.owningApplication?.bundleIdentifier == target.bundleID, "screenshot_window_bundle") else { return nil }
        let descriptor = ObservedWindowDescriptor.observe(window: window, applicationInstanceID: target.applicationInstanceID,
                          launchDate: target.launchDate, bundleID: target.bundleID)
        _ = check(descriptor != nil, "screenshot_descriptor_available")
        if let descriptor { recordDescriptor(descriptor, target: target, stage: "screenshot") }
        return descriptor?.hash
    }

    private func recordDescriptor(_ observed: ObservedWindowDescriptor, target: Target, stage: String) {
        let expected = target.descriptor
        _ = check(observed.hash == expected.hash, "\(stage)_descriptor", [
            "window_id": String(observed.windowID), "pid": String(observed.applicationPID),
            "title_matches": observed.title.utf16.elementsEqual(expected.title.utf16) ? "true" : "false",
            "x_matches": observed.x == expected.x ? "true" : "false",
            "y_matches": observed.y == expected.y ? "true" : "false",
            "width_matches": observed.width == expected.width ? "true" : "false",
            "height_matches": observed.height == expected.height ? "true" : "false",
            "level_matches": observed.level == expected.level ? "true" : "false"
        ])
    }

    @MainActor private func prepareText(_ permit: WindowCapturePermit, target: Target,
                                       authorization: @escaping () -> (generation: UInt64, stopped: Bool)) async throws {
        guard check(AXIsProcessTrusted(), "text_accessibility_permission"), let revocation = textRevocation else {
            throw SelectedTextFailure.permissionRequired
        }
        let before = try await textScope(target)
        try validate(permit, descriptorHash: before.candidates.first(where: { $0.windowID == before.windowID })?.descriptorHash,
                     authorization: authorization)
        let selection = try await textCapture.read(scope: before, permit: permit, revocation: revocation)
        // Even when delivery expired, the outer task retains this physical slot
        // until the uninterruptible AX read returned. Do not reread after stop.
        try validate(permit, descriptorHash: target.descriptor.hash, authorization: authorization)
        let after = try await textScope(target)
        guard check(AXIsProcessTrusted(), "text_accessibility_permission_after_read") else {
            throw SelectedTextFailure.permissionRequired
        }
        guard check(before == after, "text_scope_after_read") else { throw SelectedTextFailure.identityChanged }
        let completed = try clock()
        let auth = authorization()
        try policy.validate(permit, now: completed, privacyGeneration: auth.generation,
                            privacyStopped: auth.stopped, descriptorHash: target.descriptor.hash)
        finishReply(.success(["binding": permit.binding.fields, "clock_domain": "mach_absolute_time",
            "capture_started_monotonic_ns": permit.startedNanoseconds,
            "capture_completed_monotonic_ns": completed, "mime_type": "text/plain; charset=utf-8",
            "source": "native_selected_text", "text": selection.text, "range": selection.range.fields,
            "ax_binding_quality": "unique_owner_title_bounds", "selection_scope": "application_focused_element",
            "sensitivity": "not_certified"]))
    }

    @MainActor private func textScope(_ target: Target) async throws -> TextWindowScope {
        guard check(CGPreflightScreenCaptureAccess(), "text_screen_permission") else {
            throw NativeFailure("screen_permission_required")
        }
        guard check(!target.application.isTerminated, "text_retained_process") else {
            throw SelectedTextFailure.identityChanged
        }
        // Include hidden/minimized peers: a visible-only join could confuse the
        // selected window with a hidden AXWindow sharing its title and bounds.
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: false)
        _ = check(content.windows.count > 0 && content.windows.count <= WindowSelectionPolicy.maximumWindows,
                  "text_inventory_count", ["count": String(content.windows.count), "max": String(WindowSelectionPolicy.maximumWindows)])
        try SelectedTextReader.validateNativeCount(content.windows.count)
        guard check(!target.application.isTerminated, "text_retained_process_after_inventory") else {
            throw SelectedTextFailure.identityChanged
        }
        guard let app = NSRunningApplication(processIdentifier: target.descriptor.applicationPID) else {
            _ = check(false, "text_process_lookup")
            throw SelectedTextFailure.identityChanged
        }
        guard check(!app.isTerminated, "text_process_running"),
              check(app.launchDate == target.launchDate, "text_process_launch"),
              check(app.bundleIdentifier == target.bundleID, "text_process_bundle") else {
            throw SelectedTextFailure.identityChanged
        }
        let windows = content.windows.filter { $0.owningApplication?.processID == target.descriptor.applicationPID }
        guard check(!windows.isEmpty, "text_process_windows", ["pid": String(target.descriptor.applicationPID)]) else {
            throw SelectedTextFailure.limit
        }
        guard check(windows.count <= SelectedTextReader.maximumWindows, "text_window_count",
                    ["count": String(windows.count), "max": String(SelectedTextReader.maximumWindows)]) else { throw SelectedTextFailure.limit }
        let candidates = try windows.map { window -> TextWindowCandidate in
            guard check(window.owningApplication?.bundleIdentifier == target.bundleID, "text_window_bundle") else {
                throw SelectedTextFailure.identityChanged
            }
            guard let descriptor = ObservedWindowDescriptor.observe(window: window,
                    applicationInstanceID: target.applicationInstanceID, launchDate: target.launchDate, bundleID: target.bundleID),
                  let hash = descriptor.hash else {
                _ = check(false, "text_descriptor_available", ["window_id": String(window.windowID)])
                throw SelectedTextFailure.identityChanged
            }
            if window.windowID == target.window.windowID { recordDescriptor(descriptor, target: target, stage: "text") }
            return TextWindowCandidate(windowID: window.windowID, title: descriptor.title, descriptorHash: hash)
        }.sorted { $0.windowID < $1.windowID }
        guard check(Set(candidates.map(\.windowID)).count == candidates.count, "text_unique_window_ids"),
              check(candidates.contains(where: { $0.windowID == target.window.windowID }), "text_window_lookup",
                    ["window_id": String(target.window.windowID), "windows": String(candidates.count)]),
              check(candidates.first(where: { $0.windowID == target.window.windowID })?.descriptorHash == target.descriptor.hash,
                    "text_selected_descriptor") else {
            throw SelectedTextFailure.identityChanged
        }
        return TextWindowScope(windowID: target.window.windowID,
                               applicationPID: target.descriptor.applicationPID, candidates: candidates)
    }

    private func validate(_ permit: WindowCapturePermit, descriptorHash: String?,
                          authorization: () -> (generation: UInt64, stopped: Bool)) throws {
        let auth = authorization()
        _ = check(descriptorHash == permit.binding.descriptor_hash, "permit_descriptor",
                  ["window_id": String(permit.binding.window_id), "pid": String(permit.binding.application_pid)])
        do {
            try policy.validate(permit, now: clock(), privacyGeneration: auth.generation,
                                privacyStopped: auth.stopped, descriptorHash: descriptorHash)
        } catch {
            diagnostics.recordOffMain("window_read_refused", ["check": "capture_permit",
                "code": (error as? WindowSelectionFailure)?.rawValue ?? "capture_clock_unavailable"])
            throw error
        }
    }

    private func authorize(generation: UInt64, authorization: () -> (generation: UInt64, stopped: Bool)) throws {
        let auth = authorization()
        guard !auth.stopped, auth.generation == generation else { throw WindowSelectionFailure.privacyStopped }
    }

    private func clock() throws -> UInt64 {
        guard let now = CaptureClock.currentHostNanoseconds() else {
            _ = check(false, "capture_clock")
            throw NativeFailure("capture_clock_unavailable")
        }
        return now
    }
}
