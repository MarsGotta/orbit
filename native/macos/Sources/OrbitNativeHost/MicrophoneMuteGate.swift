import Foundation

/// «Silenciar» on one microphone stream: which drained blocks leave as
/// silence. Muted, every block does, with its frames and timing kept; after
/// an unmute, a block captured before it (still in the ring) does too, so
/// nothing said while muted ever leaves the helper.
struct MicrophoneMuteGate {
    private(set) var muted = false
    /// The host time of the last unmute, in nanoseconds.
    private var unmutedAtNS: UInt64 = 0

    /// Mutes, or unmutes at host time `nowNS`.
    mutating func set(muted value: Bool, nowNS: UInt64) {
        if muted && !value { unmutedAtNS = nowNS }
        muted = value
    }

    /// What leaves for a block captured at `capturedAtNS`.
    func outgoing(_ pcm: Data, capturedAtNS: UInt64) -> Data {
        guard muted || capturedAtNS < unmutedAtNS else { return pcm }
        return Data(count: pcm.count)
    }
}
