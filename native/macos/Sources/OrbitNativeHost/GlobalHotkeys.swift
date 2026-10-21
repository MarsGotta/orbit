import Carbon

/// Register only explicit shortcuts. No global keyboard event monitor is installed.
package final class GlobalHotkeys {
    private var handler: EventHandlerRef?
    private var hotkeys = [EventHotKeyRef]()
    private var pressed = Set<UInt32>()
    package var onEvent: ((String, String) -> Void)?

    package init() {}

    package func register() throws {
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
        guard result == noErr else { throw NativeFailure("hotkey_handler_unavailable") }
        let entries: [(UInt32, UInt32)] = [
            (1, UInt32(kVK_Space)), (2, UInt32(kVK_ANSI_D)),
            (3, UInt32(kVK_ANSI_M)), (4, UInt32(kVK_Escape))
        ]
        for (id, keyCode) in entries {
            var reference: EventHotKeyRef?
            let status = RegisterEventHotKey(keyCode, UInt32(controlKey | optionKey),
                EventHotKeyID(signature: 0x4F524254, id: id), GetApplicationEventTarget(), 0, &reference)
            guard status == noErr, let reference else {
                unregister()
                throw NativeFailure("hotkey_conflict")
            }
            hotkeys.append(reference)
        }
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
        if let handler { RemoveEventHandler(handler) }
        handler = nil
    }

    deinit { unregister() }
}
