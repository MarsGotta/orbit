// Consented acceptance input, scoped to the exact Orbit Pruebas process.
// Built as «Orbit Pruebas Teclas.app» (build-keys-app.sh) and launched with
// `open`, so the Accessibility permission the owner grants is this app's own,
// never the SSH session's. Its answer goes to the file after `--result`.
import AppKit
import ApplicationServices
import Foundation

var args = CommandLine.arguments
var resultPath: String?
if let flag = args.firstIndex(of: "--result"), flag + 1 < args.count {
    resultPath = args[flag + 1]
    args.removeSubrange(flag...(flag + 1))
}
func answer(_ text: String, code: Int32) -> Never {
    if let resultPath {
        try? Data((text + "\n").utf8).write(to: URL(fileURLWithPath: resultPath), options: .atomic)
    } else if code == 0 {
        print(text)
    } else {
        FileHandle.standardError.write(Data((text + "\n").utf8))
    }
    exit(code)
}
func fail(_ text: String) -> Never {
    answer("{\"failure\":\(String(reflecting: text))}", code: 1)
}
// «trust» asks macOS once, with its own prompt, and says whether it is granted.
if args.count == 2, args[1] == "trust" {
    let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
    answer("{\"trusted\":\(AXIsProcessTrustedWithOptions(options))}", code: 0)
}
guard args.count >= 3, let pid = Int32(args[1]),
      let app = NSRunningApplication(processIdentifier: pid),
      app.bundleIdentifier == "app.orbit.companion.pruebas" else {
    fail("expected Orbit Pruebas PID and action")
}
let action = args[2]
if action == "dock" {
    guard let dock = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.dock").first else {
        fail("Dock is not running")
    }
    func children(_ element: AXUIElement) -> [AXUIElement] {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &value) == .success else { return [] }
        return value as? [AXUIElement] ?? []
    }
    let root = AXUIElementCreateApplication(dock.processIdentifier)
    let tiles = children(root).flatMap { children($0) }
    let matches = tiles.filter { tile in
        var value: CFTypeRef?
        AXUIElementCopyAttributeValue(tile, "AXURL" as CFString, &value)
        return (value as? URL)?.standardizedFileURL == app.bundleURL?.standardizedFileURL
    }
    guard matches.count == 1,
          AXUIElementPerformAction(matches[0], kAXPressAction as CFString) == .success else {
        fail("Dock must contain exactly one Orbit Pruebas tile; could not press it")
    }
} else {
    let deadline = Date().addingTimeInterval(2)
    while NSWorkspace.shared.frontmostApplication?.processIdentifier != pid && Date() < deadline {
        RunLoop.current.run(until: Date().addingTimeInterval(0.02))
    }
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else {
        fail("Orbit Pruebas is not frontmost; no key posted")
    }
    guard CGPreflightPostEventAccess() else { fail("posting keys requires the system Accessibility permission") }
    let source = CGEventSource(stateID: .hidSystemState)
    let keys: [String: (CGKeyCode, CGEventFlags)] = [
        "privacy": (53, [.maskControl, .maskAlternate]),
        "close": (13, .maskCommand), "quit": (12, .maskCommand),
        "undo": (6, .maskCommand), "redo": (6, [.maskCommand, .maskShift]),
        "copy": (8, .maskCommand), "paste": (9, .maskCommand),
        "select-all": (0, .maskCommand), "end": (124, .maskCommand),
    ]
    let key: CGKeyCode
    let flags: CGEventFlags
    if action == "text" { key = 0; flags = [] }
    else if let binding = keys[action] { (key, flags) = binding }
    else { fail("unsupported key action") }
    guard let down = CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: true),
          let up = CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: false) else {
        fail("could not create keyboard events")
    }
    down.flags = flags
    up.flags = flags
    if action == "text" {
        guard args.count == 4 else { fail("text action requires text") }
        let units = Array(args[3].utf16)
        down.keyboardSetUnicodeString(stringLength: units.count, unicodeString: units)
        up.keyboardSetUnicodeString(stringLength: units.count, unicodeString: units)
    }
    down.post(tap: .cghidEventTap)
    up.post(tap: .cghidEventTap)
}
answer("{\"posted\":true}", code: 0)
