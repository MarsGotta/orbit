// For the real test on the Mac: moves the system's pointer over one window
// (`over`: its centre) or just beside it (`away`: 40 points to its left, or
// to its right when the screen ends there), and says where the pointer is
// afterwards, one JSON object.
//
// CGWarpMouseCursorPosition moves the pointer without a mouse event. Orbit
// reads the pointer's position itself, a few times a second
// (apps/desktop/src-tauri/src/panel_hover.rs, NSEvent.mouseLocation through
// Tauri), so the warp is what it sees; no hover event of the webview is
// involved. The window's bounds, the warp and the pointer's location are all
// global display points, top-left origin; the scale of the display under
// the pointer (2 on Retina) is said beside them.
import CoreGraphics
import Foundation

func fail(_ text: String, _ code: Int32) -> Never {
    FileHandle.standardError.write(Data((text + "\n").utf8))
    exit(code)
}

let arguments = CommandLine.arguments
guard arguments.count == 3, let id = UInt32(arguments[1]), ["over", "away"].contains(arguments[2]) else {
    fail("usage: swift pointer.swift WINDOW_ID over|away", 2)
}
let listed = CGWindowListCopyWindowInfo([.optionIncludingWindow], CGWindowID(id)) as? [[String: Any]] ?? []
guard let raw = listed.first?[kCGWindowBounds as String] as? NSDictionary,
      let bounds = CGRect(dictionaryRepresentation: raw as CFDictionary) else {
    fail("WindowServer lists no window \(id)", 1)
}
let before = CGEvent(source: nil)?.location ?? CGPoint(x: -1, y: -1)
let target = arguments[2] == "over"
    ? CGPoint(x: bounds.midX, y: bounds.midY)
    : CGPoint(x: bounds.minX >= 40 ? bounds.minX - 40 : bounds.maxX + 40, y: bounds.midY)
let warped = CGWarpMouseCursorPosition(target)
// The location is read back until it is the target, a second at most: a
// warp WindowServer refused leaves it where it was, and the caller fails.
var after = CGEvent(source: nil)?.location ?? before
for _ in 0..<20 where abs(after.x - target.x) > 1 || abs(after.y - target.y) > 1 {
    usleep(50_000)
    after = CGEvent(source: nil)?.location ?? after
}
var display: CGDirectDisplayID = 0
var displays: UInt32 = 0
CGGetDisplaysWithPoint(after, 1, &display, &displays)
var scale = -1.0
if displays > 0, let mode = CGDisplayCopyDisplayMode(display), mode.width > 0 {
    scale = Double(mode.pixelWidth) / Double(mode.width)
}
let row: [String: Any] = [
    "window_id": Int(id),
    "bounds": [
        "x": Double(bounds.minX), "y": Double(bounds.minY),
        "width": Double(bounds.width), "height": Double(bounds.height),
    ],
    "before": ["x": Double(before.x), "y": Double(before.y)],
    "after": ["x": Double(after.x), "y": Double(after.y)],
    "warped": warped == .success,
    "inside": bounds.contains(after),
    "scale": scale,
]
let line = try JSONSerialization.data(withJSONObject: row, options: [.sortedKeys])
print(String(decoding: line, as: UTF8.self))
