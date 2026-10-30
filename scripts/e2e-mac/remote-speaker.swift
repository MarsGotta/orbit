// A separate, visible audio owner for ScreenCaptureKit application capture.
// AVPlayer's explicit CoreAudio UID keeps the owner's default output untouched.
import AppKit
import AVFoundation

let arguments = CommandLine.arguments
let allowed = Set((arguments.count > 2 ? arguments[2] : "").split(separator: ",").compactMap { UInt32($0) })
if arguments.count > 1, arguments[1] == "--probe" {
    do {
        let data = try JSONSerialization.data(withJSONObject: silentRoute(allowed))
        print(String(decoding: data, as: UTF8.self))
    } catch { fputs("\(error)\n", stderr); exit(1) }
    exit(0)
}
guard arguments.count == 4, let playbackBudget = Double(arguments[3]), playbackBudget > 0 else { exit(2) }
let directory = URL(fileURLWithPath: arguments[1], isDirectory: true)
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 320, height: 100),
                      styleMask: [.titled], backing: .buffered, defer: false)
window.title = "Orbit Pruebas Remote"
window.orderFrontRegardless()

func receipt(_ name: String, _ value: [String: Any]) {
    do {
        let data = try JSONSerialization.data(withJSONObject: value)
        try data.write(to: directory.appendingPathComponent(name + ".json"), options: .atomic)
    } catch { exit(1) }
}

var player: AVPlayer?
var observer: NSObjectProtocol?
var phase: String?
var started = Date()
var idle = Date()
// The local process is finite even when SSH/the orchestrator disappears. The
// caller supplies its playback budget; two idle minutes bound an abandoned app.
let timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { _ in
    do {
        _ = try silentRoute(allowed)
        if phase != nil {
            if player?.error != nil || Date().timeIntervalSince(started) > playbackBudget {
                throw RouteFailure("remote playback failed or exceeded \(playbackBudget) seconds")
            }
            guard player?.audioOutputDeviceUniqueID == "BlackHole16ch_UID" else {
                throw RouteFailure("remote playback output binding changed")
            }
            return
        }
        if Date().timeIntervalSince(idle) > 120 { app.terminate(nil) }
        let request = directory.appendingPathComponent("request.json")
        guard FileManager.default.fileExists(atPath: request.path) else { return }
        let data = try Data(contentsOf: request)
        try FileManager.default.removeItem(at: request)
        let body = try JSONSerialization.jsonObject(with: data) as? [String: String]
        guard let next = body?["phase"], ["remote", "remote-muted"].contains(next) else {
            throw RouteFailure("invalid remote playback phase")
        }
        phase = next
        started = Date()
        let item = AVPlayerItem(url: directory.appendingPathComponent(next + ".aiff"))
        let output = AVPlayer(playerItem: item)
        output.audioOutputDeviceUniqueID = "BlackHole16ch_UID"
        guard output.audioOutputDeviceUniqueID == "BlackHole16ch_UID" else {
            throw RouteFailure("remote playback output binding refused")
        }
        if let old = observer { NotificationCenter.default.removeObserver(old) }
        observer = NotificationCenter.default.addObserver(
            forName: .AVPlayerItemDidPlayToEndTime, object: item, queue: .main) { _ in
                output.pause()
                receipt(next + "-done", ["played": true, "output_uid": "BlackHole16ch_UID"])
                player = nil
                phase = nil
                idle = Date()
            }
        player = output
        output.play()
    } catch {
        player?.pause()
        receipt((phase ?? "remote") + "-done", ["error": String(describing: error)])
        app.terminate(nil)
    }
}
do {
    _ = try silentRoute(allowed)
    receipt("ready", ["pid": ProcessInfo.processInfo.processIdentifier])
    app.run()
} catch {
    receipt("ready", ["error": String(describing: error)])
    exit(1)
}
