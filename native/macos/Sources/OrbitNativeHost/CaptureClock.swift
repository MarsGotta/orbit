import CoreMedia

/// Keeps the source clock explicit. A numeric PTS alone is not host time.
package enum CaptureClock {
    package static func currentHostNanoseconds() -> UInt64? {
        nanoseconds(CMClockGetTime(CMClockGetHostTimeClock()))
    }

    static func nanoseconds(_ time: CMTime) -> UInt64? {
        guard time.isValid, time.isNumeric, time.epoch == 0,
              time.timescale > 0, time.value >= 0 else { return nil }
        let scaled = CMTimeConvertScale(time, timescale: 1_000_000_000, method: .roundTowardZero)
        guard scaled.isValid, scaled.isNumeric, scaled.value >= 0 else { return nil }
        return UInt64(scaled.value)
    }

    static func hostNanoseconds(_ time: CMTime, from clock: CMClock) -> UInt64? {
        nanoseconds(CMSyncConvertTime(time, from: clock, to: CMClockGetHostTimeClock()))
    }
}
