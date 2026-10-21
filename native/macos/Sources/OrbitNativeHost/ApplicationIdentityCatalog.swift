import AppKit
import OrbitNativeProtocol

/// Main-thread ownership of short-lived observations and retained process
/// instances. The physical SDK-read slot survives privacy revocation and timeout
/// until the worker actually returns. No automatic discovery runs at launch.
package final class ApplicationIdentityCatalog {
    private final class ProcessInstance {
        let application: NSRunningApplication
        let metadata: ApplicationProcessMetadata
        let identity: StableApplicationIdentity
        let id: String
        init(application: NSRunningApplication, metadata: ApplicationProcessMetadata,
             identity: StableApplicationIdentity) {
            self.application = application
            self.metadata = metadata
            self.identity = identity
            id = UUID().uuidString.lowercased()
        }
    }

    private struct Observation {
        let binding: ApplicationIdentityBinding
        let process: ProcessInstance
    }

    private let nativeInstanceID: String
    private var inFlight = false
    private var processes = [Int32: ProcessInstance]()
    private var observations = [String: Observation]()

    package init(nativeInstanceID: String) { self.nativeInstanceID = nativeInstanceID }

    package func invalidate() {
        observations.removeAll()
        processes.removeAll()
        // Do not release inFlight: Security may still be reading on its worker.
    }

    @MainActor package func inventory(bundleIDs: [String], privacyGeneration: UInt64,
                              authorization: () -> (generation: UInt64, stopped: Bool)) async throws -> [String: Any] {
        guard !inFlight else { throw NativeFailure("application_identity_busy") }
        guard !bundleIDs.isEmpty, bundleIDs.count <= ApplicationIdentityPolicy.maximumBundleIDs,
              Set(bundleIDs).count == bundleIDs.count,
              bundleIDs.allSatisfy(ApplicationIdentityPolicy.validBundleID) else {
            throw NativeFailure("invalid_application_scope")
        }
        inFlight = true
        defer { inFlight = false }
        try authorize(privacyGeneration, authorization)
        let started = try clock()
        let applications = NSWorkspace.shared.runningApplications
        guard applications.count <= 512 else { throw NativeFailure("application_inventory_limit") }
        let selected = applications.filter { application in
            application.bundleIdentifier.map { bundleIDs.contains($0) } == true && !application.isTerminated
        }
        guard selected.count <= ApplicationIdentityPolicy.maximumApplications else {
            throw NativeFailure("application_inventory_limit")
        }
        var rows = [[String: Any]]()
        for application in selected {
            try authorize(privacyGeneration, authorization)
            let now = try clock()
            guard now >= started, now - started < ApplicationIdentityPolicy.lifetimeNanoseconds else {
                throw NativeFailure("application_identity_expired")
            }
            var row: [String: Any] = ["application_pid": application.processIdentifier,
                "application_name": WindowPresentation.bounded(application.localizedName ?? "", maximumBytes: 128),
                "bundle_id": application.bundleIdentifier ?? ""]
            do {
                let binding = try await observe(application.processIdentifier, generation: privacyGeneration, authorization: authorization)
                row["binding"] = binding.fields
                row["unavailable_reason"] = NSNull()
            } catch let error as NativeFailure where error.code == "application_identity_unavailable" {
                row["binding"] = NSNull()
                row["unavailable_reason"] = "application_identity_unavailable"
            }
            rows.append(row)
        }
        try authorize(privacyGeneration, authorization)
        let completed = try clock()
        guard completed >= started, completed - started < ApplicationIdentityPolicy.lifetimeNanoseconds else {
            throw NativeFailure("application_identity_expired")
        }
        let result: [String: Any] = ["native_instance_id": nativeInstanceID,
            "privacy_generation": privacyGeneration, "clock_domain": "mach_absolute_time",
            "observed_monotonic_ns": started, "completed_monotonic_ns": completed,
            "applications": rows, "omitted_count": 0]
        guard try JSONSerialization.data(withJSONObject: result).count <= ApplicationIdentityPolicy.maximumResponseBytes else {
            throw NativeFailure("application_inventory_limit")
        }
        return result
    }

    @MainActor private func observe(_ pid: Int32, generation: UInt64,
                                     authorization: () -> (generation: UInt64, stopped: Bool)) async throws -> ApplicationIdentityBinding {
        try authorize(generation, authorization)
        let started = try clock()
        prune(now: started)
        guard observations.count < ApplicationIdentityPolicy.maximumObservations,
              processes[pid] != nil || processes.count < ApplicationIdentityPolicy.maximumApplications else {
            throw NativeFailure("application_identity_limit")
        }
        guard pid > 0, let application = NSRunningApplication(processIdentifier: pid),
              let before = metadata(application) else { throw NativeFailure("application_identity_unavailable") }
        let identity = await ApplicationCodeIdentity.read(before)
        try authorize(generation, authorization)
        let completed = try clock()
        guard completed >= started, completed - started < ApplicationIdentityPolicy.lifetimeNanoseconds else {
            throw NativeFailure("application_identity_expired")
        }
        guard let identity, !application.isTerminated,
              let current = NSRunningApplication(processIdentifier: pid), metadata(current) == before else {
            throw NativeFailure("application_identity_unavailable")
        }
        let process: ProcessInstance
        if let previous = processes[pid], !previous.application.isTerminated,
           previous.metadata == before, previous.identity == identity {
            process = previous
        } else {
            process = ProcessInstance(application: application, metadata: before, identity: identity)
            processes[pid] = process
        }
        let (expires, overflow) = started.addingReportingOverflow(ApplicationIdentityPolicy.lifetimeNanoseconds)
        guard !overflow else { throw NativeFailure("capture_clock_unavailable") }
        let binding = ApplicationIdentityBinding(nativeInstanceID: nativeInstanceID, privacyGeneration: generation,
            applicationPID: pid, applicationInstanceID: process.id, identityToken: UUID().uuidString.lowercased(),
            identity: identity, observedNanoseconds: started, expiresNanoseconds: expires)
        guard binding.isWellFormed else { throw NativeFailure("application_identity_unavailable") }
        observations[binding.identity_token] = Observation(binding: binding, process: process)
        return binding
    }

    @MainActor private func metadata(_ application: NSRunningApplication) -> ApplicationProcessMetadata? {
        guard !application.isTerminated, application.processIdentifier > 0,
              let date = application.launchDate, date.timeIntervalSinceReferenceDate.isFinite,
              let bundleID = application.bundleIdentifier, ApplicationIdentityPolicy.validBundleID(bundleID),
              let url = application.bundleURL, url.isFileURL, url.path.utf8.count <= 4096 else { return nil }
        return ApplicationProcessMetadata(pid: application.processIdentifier,
            launchDateBits: date.timeIntervalSinceReferenceDate.bitPattern,
            bundleID: bundleID, bundlePath: url.path)
    }

    @MainActor private func prune(now: UInt64) {
        observations = observations.filter { $0.value.binding.expires_monotonic_ns > now }
        processes = processes.filter { _, process in
            !process.application.isTerminated && metadata(process.application) == process.metadata
        }
    }

    private func authorize(_ generation: UInt64, _ authorization: () -> (generation: UInt64, stopped: Bool)) throws {
        let current = authorization()
        guard !current.stopped, current.generation == generation else { throw NativeFailure("operation_cancelled_by_stop") }
    }

    private func clock() throws -> UInt64 {
        guard let now = CaptureClock.currentHostNanoseconds() else { throw NativeFailure("capture_clock_unavailable") }
        return now
    }
}
