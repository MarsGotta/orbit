import AppKit
import Darwin
import AVFoundation
import ApplicationServices
import ScreenCaptureKit
import OrbitNativeProtocol
import OrbitNativeTransport
import OrbitNativeState
import OrbitNativeCalendar
import OrbitNativeHost

// The calendar's host, made on first use, and what it is told.
extension OrbitApplication {
    /// The permission as the calendar last saw it; EventKit is asked off Main.
    func calendarAuthorization() -> CalendarAuthorization {
        MainActor.assumeIsolated { calendarHost().status().authorization }
    }

    func calendarState() -> [String: Any] {
        // AppKit delegate, native channel delivery and this host's run-loop timers
        // run on Main. Keep the existing unannotated host boundary explicit; the
        // calendar's own work runs on its actor.
        MainActor.assumeIsolated { calendarHost().stateFields() }
    }

    /// A stop, a lost daemon or quitting: the reply in flight is retired on
    /// the calendar's actor; this thread does not wait for it.
    func invalidateCalendars(_ reason: CalendarFailure) {
        MainActor.assumeIsolated { calendars?.invalidate(reason) }
    }

    @MainActor func calendarHost() -> CalendarHost {
        if let calendars { return calendars }
        let facts: @MainActor @Sendable () -> CalendarHostFacts = { [weak self] in
            self?.calendarFacts() ?? CalendarHostFacts(connected: false, generation: 0, stopped: true, latched: false)
        }
        let controller = CalendarHost.controller(nativeInstanceID: windowScreenshots.nativeInstanceID,
            isolatedFixture: launchOptions.isIsolatedFixture, channel: channel, facts: facts)
        let value = CalendarHost(controller: controller, channel: channel, facts: facts,
            changed: { [weak self] in self?.publishState() })
        calendars = value
        return value
    }

    private func calendarFacts() -> CalendarHostFacts {
        let current = screenAuthorization()
        return CalendarHostFacts(connected: connected, generation: current.generation,
            stopped: current.stopped, latched: latch.stopped)
    }
}
