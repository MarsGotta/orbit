import EventKit
import Foundation
import XCTest
import OrbitNativeProtocol
@testable import OrbitNativeCalendar

final class EventKitCalendarReaderTests: XCTestCase {
    func testOccurrenceHelpersUseMillisecondsAndStableOmissionCodes() throws {
        XCTAssertEqual(
            try EventKitCalendarReader.milliseconds(Date(timeIntervalSince1970: 1.25)),
            1_250)
        XCTAssertEqual(CalendarOccurrenceOmission.missingTitle.rawValue, "missing_title")
        XCTAssertEqual(CalendarOccurrenceOmission.outsideQueryWindow.rawValue,
                       "outside_query_window")
    }
}
