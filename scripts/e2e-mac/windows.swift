// For the real test on the Mac: the windows of one process as WindowServer
// lists them, one JSON object per line: id, layer, whether it is on screen,
// width and height.
//
// CuaDriver's `list_windows` lists layer-0 windows only, so Orbit's floating
// panel («Orbit · llamada», always on top) never appears there; this lists
// every layer. Titles need Screen Recording, which an SSH shell does not
// have: the panel is told by its layer and its accessibility tree.
import CoreGraphics
import Foundation

guard CommandLine.arguments.count == 2, let pid = Int32(CommandLine.arguments[1]) else {
    FileHandle.standardError.write("usage: swift windows.swift PID\n".data(using: .utf8)!)
    exit(2)
}
let windows = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] ?? []
for window in windows where (window[kCGWindowOwnerPID as String] as? Int32) == pid {
    let bounds = window[kCGWindowBounds as String] as? [String: Any] ?? [:]
    let row: [String: Any] = [
        "window_id": window[kCGWindowNumber as String] as? Int ?? -1,
        "layer": window[kCGWindowLayer as String] as? Int ?? -1,
        "is_on_screen": window[kCGWindowIsOnscreen as String] as? Bool ?? false,
        "width": bounds["Width"] as? Double ?? -1,
        "height": bounds["Height"] as? Double ?? -1,
    ]
    let line = try JSONSerialization.data(withJSONObject: row, options: [.sortedKeys])
    print(String(decoding: line, as: UTF8.self))
}
