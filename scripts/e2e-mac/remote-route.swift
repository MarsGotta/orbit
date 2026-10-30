// Read-only CoreAudio preflight for the two silent fixture sinks. No inventories saved.
import AppKit
import CoreAudio

struct RouteFailure: Error, CustomStringConvertible {
    let description: String
    init(_ text: String) { description = text }
}

func address(_ selector: AudioObjectPropertySelector,
             _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
}

func numbers(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector,
             _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) throws -> [UInt32] {
    var key = address(selector, scope)
    var size: UInt32 = 0
    let status = AudioObjectGetPropertyDataSize(object, &key, 0, nil, &size)
    guard status == noErr else { throw RouteFailure("CoreAudio size read failed: \(status)") }
    guard size > 0 else { return [] }
    var values = [UInt32](repeating: 0, count: Int(size) / MemoryLayout<UInt32>.size)
    let read = AudioObjectGetPropertyData(object, &key, 0, nil, &size, &values)
    guard read == noErr else { throw RouteFailure("CoreAudio read failed: \(read)") }
    return Array(values.prefix(Int(size) / MemoryLayout<UInt32>.size))
}

func number(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) throws -> UInt32 {
    let values = try numbers(object, selector)
    guard values.count == 1 else { throw RouteFailure("CoreAudio scalar missing") }
    return values[0]
}

func string(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) throws -> String {
    var key = address(selector)
    var value: CFString = "" as CFString
    var size = UInt32(MemoryLayout<CFString>.size)
    let status = AudioObjectGetPropertyData(object, &key, 0, nil, &size, &value)
    guard status == noErr else { throw RouteFailure("CoreAudio string read failed: \(status)") }
    return value as String
}

func silentRoute(_ allowed: Set<UInt32>) throws -> [String: Any] {
    let system = AudioObjectID(kAudioObjectSystemObject)
    let devices = try numbers(system, kAudioHardwarePropertyDevices)
    let sinks = ["BlackHole 2ch": "BlackHole2ch_UID", "BlackHole 16ch": "BlackHole16ch_UID"]
    var rows = [[String: Any]]()
    var sinkIDs = Set<UInt32>()
    for device in devices {
        let name = try string(device, kAudioObjectPropertyName)
        guard let uid = sinks[name] else { continue }
        guard try string(device, kAudioDevicePropertyDeviceUID) == uid else {
            throw RouteFailure("\(name) has an unexpected device UID")
        }
        guard try number(device, kAudioDevicePropertyTransportType) == kAudioDeviceTransportTypeVirtual else {
            throw RouteFailure("\(name) is not a direct virtual device")
        }
        guard try number(device, kAudioDevicePropertyDeviceIsAlive) == 1 else {
            throw RouteFailure("\(name) is not alive")
        }
        let inputs = try numbers(device, kAudioDevicePropertyStreams, kAudioObjectPropertyScopeInput)
        guard !inputs.isEmpty else {
            throw RouteFailure("\(name) has no input stream")
        }
        let outputs = try numbers(device, kAudioDevicePropertyStreams, kAudioObjectPropertyScopeOutput)
        guard !outputs.isEmpty else {
            throw RouteFailure("\(name) has no output stream")
        }
        sinkIDs.insert(device)
        rows.append(["name": name, "uid": uid, "virtual": true, "alive": true])
    }
    for name in sinks.keys where !rows.contains(where: { $0["name"] as? String == name }) {
        throw RouteFailure("missing \(name) input/output; no driver is installed by check 25")
    }
    // Reject even inactive aggregate/multi-output membership: it can become an
    // audible monitoring branch. Never rename or dismantle the owner's devices.
    for device in devices {
        guard try number(device, kAudioDevicePropertyTransportType) == kAudioDeviceTransportTypeAggregate else { continue }
        let children = try numbers(device, kAudioAggregateDevicePropertyActiveSubDeviceList)
        if !sinkIDs.isDisjoint(with: children) {
            throw RouteFailure("aggregate/multi-output device contains a fixture sink")
        }
    }
    for object in try numbers(system, kAudioHardwarePropertyProcessObjectList) {
        let pid = try number(object, kAudioProcessPropertyPID)
        if !allowed.contains(pid), try number(object, kAudioProcessPropertyIsRunningInput) != 0 {
            // A reader of another device (Siri on the built-in microphone, say)
            // cannot hear the fixture sinks; one reading them could forward
            // them. An unreadable device list fails closed. Save only a cause.
            let reading = try numbers(object, kAudioProcessPropertyDevices, kAudioObjectPropertyScopeInput)
            if !sinkIDs.isDisjoint(with: reading) {
                throw RouteFailure("another input client reads a fixture device; close it before speaking")
            }
        }
    }
    return ["devices": rows, "unsafe_readers": 0, "unsafe_aggregates": 0]
}
