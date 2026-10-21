import Foundation
import Darwin
import OrbitNativeProtocol

/// Read-only gate for an explicitly supplied, fresh fixture directory. This is
/// not a sandbox against an adversary changing its trusted parent concurrently.
public enum NativeFixtureDirectory {
    public static func validate(_ path: String) throws -> URL {
        guard NativeLaunchOptions.isCanonicalAbsolutePath(path) else {
            throw NativeStateFailure.invalidFixtureDirectory
        }
        // Foundation may remove /private even from a physical macOS path.
        // The external runner uses POSIX resolution; do not normalize that
        // result back to a system alias, or compare Unicode-equivalent names.
        var buffer = [CChar](repeating: 0, count: Int(PATH_MAX))
        let canonical: String? = buffer.withUnsafeMutableBufferPointer { storage in
            guard let address = storage.baseAddress,
                  realpath(path, address) != nil else { return nil }
            return String(validatingUTF8: address)
        }
        guard let canonical, path.utf8.elementsEqual(canonical.utf8) else {
            throw NativeStateFailure.invalidFixtureDirectory
        }
        let descriptor = open(path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
        guard descriptor >= 0 else { throw NativeStateFailure.invalidFixtureDirectory }
        var metadata = stat()
        guard fstat(descriptor, &metadata) == 0,
              metadata.st_mode & 0o170000 == 0o040000,
              metadata.st_uid == geteuid(), metadata.st_mode & 0o7777 == 0o700 else {
            close(descriptor)
            throw NativeStateFailure.invalidFixtureDirectory
        }
        guard let directory = fdopendir(descriptor) else {
            close(descriptor)
            throw NativeStateFailure.invalidFixtureDirectory
        }
        defer { closedir(directory) } // fdopendir owns the descriptor now.
        while true {
            errno = 0
            guard let entry = readdir(directory) else {
                guard errno == 0 else { throw NativeStateFailure.invalidFixtureDirectory }
                break
            }
            var name = entry.pointee.d_name
            let capacity = MemoryLayout.size(ofValue: name)
            let value = withUnsafePointer(to: &name) {
                $0.withMemoryRebound(to: CChar.self, capacity: capacity) {
                    String(cString: $0)
                }
            }
            guard value == "." || value == ".." else {
                throw NativeStateFailure.invalidFixtureDirectory
            }
        }
        return URL(fileURLWithPath: path, isDirectory: true)
    }
}
