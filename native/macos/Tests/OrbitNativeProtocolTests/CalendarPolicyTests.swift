import Foundation
import XCTest
@testable import OrbitNativeProtocol

final class CalendarPolicyTests: XCTestCase {
    private let host = "00000000-0000-4000-8000-000000000001"
    private let request = "00000000-0000-4000-8000-000000000002"
    private let other = "00000000-0000-4000-8000-000000000003"
    private let now: UInt64 = 100
    private let day: Int64 = 86_400_000

    func testStartsIdleAndHoldsNothingButTheSlot() throws {
        var value = CalendarPolicy(nativeInstanceID: host)
        XCTAssertNil(value.pending)
        let permit = try value.begin(.sources(afterIndex: 0), id: request, now: now, generation: 2,
            stopped: false, authorization: .fullAccess)
        XCTAssertEqual(value.pending, permit)
        value.complete(permit)
        XCTAssertNil(value.pending)
        XCTAssertNil(CalendarPolicy(nativeInstanceID: host).pending)
    }

    func testOnlyFullAccessCanReadAndAPermissionRequestIsNotARead() throws {
        for permission in [CalendarAuthorization.notDetermined, .denied, .restricted, .writeOnly, .unknown, .unavailable] {
            var value = CalendarPolicy(nativeInstanceID: host)
            for operation in [CalendarOperation.sources(afterIndex: 0),
                              .read(CalendarReadQuery(start: 0, end: day))] {
                XCTAssertThrowsError(try value.begin(operation, id: request, now: now, generation: 2,
                    stopped: false, authorization: permission)) {
                    XCTAssertEqual($0 as? CalendarFailure, .permissionRequired)
                }
            }
            let permit = try value.begin(.requestAccess, id: request, now: now, generation: 2, stopped: true, authorization: permission)
            value.complete(permit)
        }
    }

    func testPrivacyStopRefusesEveryRead() {
        var value = CalendarPolicy(nativeInstanceID: host)
        XCTAssertThrowsError(try value.begin(.read(CalendarReadQuery(start: 0, end: day)), id: request, now: now,
            generation: 2, stopped: true, authorization: .fullAccess)) {
            XCTAssertEqual($0 as? CalendarFailure, .privacyStopped)
        }
    }

    /// A window as wide as EventKit reads without shortening it is a
    /// valid query, and one day wider is not — EventKit would drop the rest.
    func testQueryWindowIsBoundedByEventKitNotByOrbit() {
        XCTAssertTrue(CalendarReadQuery(start: 0, end: CalendarLimits.maximumQueryMilliseconds).isValid)
        XCTAssertTrue(CalendarReadQuery(start: 0, end: 31 * day, afterIndex: 4_000).isValid)
        for query in [CalendarReadQuery(start: 0, end: 0),
                      CalendarReadQuery(start: 1, end: 0),
                      CalendarReadQuery(start: Int64.min, end: Int64.max),
                      CalendarReadQuery(start: 0, end: CalendarLimits.maximumQueryMilliseconds + 1)] {
            XCTAssertFalse(query.isValid)
            var value = CalendarPolicy(nativeInstanceID: host)
            XCTAssertThrowsError(try value.begin(.read(query), id: request, now: now, generation: 2,
                stopped: false, authorization: .fullAccess)) {
                XCTAssertEqual($0 as? CalendarFailure, .invalidRequest)
            }
        }
    }

    func testRevokeAndExpiryKeepPhysicalSlotUntilExactCompletion() throws {
        for revoked in [false, true] {
            var value = CalendarPolicy(nativeInstanceID: host)
            let permit = try value.begin(.sources(afterIndex: 0), id: request, now: now, generation: 2,
                stopped: false, authorization: .fullAccess)
            if revoked { value.invalidate() } else { value.expire(permit) }
            XCTAssertThrowsError(try value.validate(permit, now: now + 1, generation: 2, stopped: false, authorization: .fullAccess))
            XCTAssertThrowsError(try value.begin(.sources(afterIndex: 0), id: other, now: now + 1, generation: 2,
                stopped: false, authorization: .fullAccess)) {
                XCTAssertEqual($0 as? CalendarFailure, .busy)
            }
            let stale = CalendarReadPermit(id: other, operation: .sources(afterIndex: 0), generation: 2,
                started: now, deadline: permit.deadline)
            value.complete(stale); XCTAssertEqual(value.pending, permit)
            value.complete(permit); XCTAssertNil(value.pending)
            let fresh = try value.begin(.sources(afterIndex: 0), id: other, now: now + 2, generation: 2,
                stopped: false, authorization: .fullAccess)
            value.complete(permit); XCTAssertEqual(value.pending, fresh)
        }
    }

    func testPrivacyNewGenerationAndDeadlineCannotDeliver() throws {
        var value = CalendarPolicy(nativeInstanceID: host)
        let permit = try value.begin(.read(CalendarReadQuery(start: 0, end: day)), id: request, now: now,
            generation: 2, stopped: false, authorization: .fullAccess)
        for (time, generation, stopped) in [(now + 1, UInt64(3), false), (now + 1, UInt64(2), true),
                                          (permit.deadline, UInt64(2), false), (now - 1, UInt64(2), false)] {
            XCTAssertThrowsError(try value.validate(permit, now: time, generation: generation, stopped: stopped, authorization: .fullAccess))
        }
        XCTAssertThrowsError(try value.validate(permit, now: now + 1, generation: 2, stopped: false, authorization: .denied))
    }

    /// The wire has no selection any more. A read names its window and
    /// its position; a listing names only its position.
    func testWireCommandsCarryAWindowOrAPageAndNoSelection() throws {
        let read = Data("{\"v\":1,\"id\":\"read\",\"command\":\"calendar_snapshot\",\"calendar_query\":{\"start_unix_ms\":0,\"end_unix_ms\":\(day),\"after_index\":7}}".utf8)
        guard case .calendarSnapshot(let decoded) = try NativeWireCommand.decode(read) else {
            return XCTFail("calendar_snapshot variant")
        }
        XCTAssertEqual(decoded.fields.calendar_query, CalendarReadQuery(start: 0, end: day, afterIndex: 7))
        let list = Data("{\"v\":1,\"id\":\"list\",\"command\":\"calendar_sources\",\"calendar_page\":{\"after_index\":300}}".utf8)
        guard case .calendarSources(let decodedList) = try NativeWireCommand.decode(list) else {
            return XCTFail("calendar_sources variant")
        }
        XCTAssertEqual(decodedList.fields.calendar_page, CalendarPageRequest(afterIndex: 300))
        let missing = String(decoding: read, as: UTF8.self).replacingOccurrences(of: ",\"after_index\":7", with: "")
        XCTAssertThrowsError(try NativeWireCommand.decode(Data(missing.utf8)))
    }
}
