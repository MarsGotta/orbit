import Carbon
import OrbitNativeProtocol

/// Register only explicit shortcuts. No global keyboard event monitor is installed.
package final class GlobalHotkeys {
    private var handler: EventHandlerRef?
    private var hotkeys = [EventHotKeyRef]()
    private var pressed = Set<UInt32>()
    package var onEvent: ((String, String) -> Void)?

    private let registerKey: ((UInt32, UInt32) -> OSStatus)?

    /// Inject only the OS registration boundary; tests never own global keys.
    package init(registerKey: ((UInt32, UInt32) -> OSStatus)? = nil) {
        self.registerKey = registerKey
    }

    package func register() -> [NativeShortcutFailure] {
        var types = [
            EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed)),
            EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyReleased))
        ]
        let context = Unmanaged.passUnretained(self).toOpaque()
        let result = InstallEventHandler(GetApplicationEventTarget(), { _, event, context in
            guard let event, let context else { return OSStatus(eventNotHandledErr) }
            let owner = Unmanaged<GlobalHotkeys>.fromOpaque(context).takeUnretainedValue()
            return owner.handle(event)
        }, types.count, &types, context, &handler)
        let entries: [(UInt32, UInt32, String)] = [
            (1, UInt32(kVK_Space), "⌃⌥␣"), (2, UInt32(kVK_ANSI_D), "⌃⌥D"),
            (3, UInt32(kVK_ANSI_M), "⌃⌥M"), (4, UInt32(kVK_Escape), "⌃⌥Esc")
        ]
        guard result == noErr else {
            return entries.map { NativeShortcutFailure(combination: $0.2, osStatus: result) }
        }
        return entries.compactMap { id, keyCode, combination in
            let status = registerKey?(id, keyCode) ?? registerCarbonKey(id: id, keyCode: keyCode)
            return status == noErr ? nil : NativeShortcutFailure(combination: combination, osStatus: status)
        }
    }

    private func registerCarbonKey(id: UInt32, keyCode: UInt32) -> OSStatus {
        var reference: EventHotKeyRef?
        let status = RegisterEventHotKey(keyCode, UInt32(controlKey | optionKey),
            EventHotKeyID(signature: 0x4F524254, id: id), GetApplicationEventTarget(), 0, &reference)
        if status == noErr, let reference { hotkeys.append(reference) }
        return status
    }

    private func handle(_ event: EventRef) -> OSStatus {
        var id = EventHotKeyID()
        let status = GetEventParameter(event, EventParamName(kEventParamDirectObject),
            EventParamType(typeEventHotKeyID), nil, MemoryLayout<EventHotKeyID>.size, nil, &id)
        guard status == noErr else { return status }
        let actions: [UInt32: String] = [1: "ptt", 2: "call_toggle", 3: "meeting_toggle", 4: "privacy_stop"]
        guard let action = actions[id.id] else { return OSStatus(eventNotHandledErr) }
        if GetEventKind(event) == UInt32(kEventHotKeyPressed) {
            if pressed.insert(id.id).inserted { onEvent?(action, "pressed") }
        } else if pressed.remove(id.id) != nil {
            onEvent?(action, "released")
        }
        return noErr
    }

    package func unregister() {
        hotkeys.forEach { UnregisterEventHotKey($0) }
        hotkeys.removeAll()
        pressed.removeAll()
        if let handler { RemoveEventHandler(handler) }
        handler = nil
    }

    deinit { unregister() }
}
