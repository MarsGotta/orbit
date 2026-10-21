import Foundation
import XCTest
@testable import OrbitNativeProtocol

final class CalendarEventTests: XCTestCase {
    private let id = "00000000-0000-4000-8000-000000000001"
    private func event(title: String = "Revisión\n🪐", floating: Bool = false, start: Int64 = 2000,
                       observation: String? = nil) throws -> CalendarOccurrence {
        try CalendarOccurrence(id: observation ?? id, calendar: CalendarReference(calendarID: "local", sourceID: "account"),
            calendarItemID: "item", eventID: "event", originalOccurrence: 1000, start: start, end: start + 90_000_000,
            eventTimezone: floating ? nil : "Europe/Madrid", interpretedTimezone: "Europe/Madrid",
            allDay: true, detached: true, title: title, status: "canceled", lastModified: nil,
            invitees: nil, inviteesStatus: "not_provided", eventURL: nil, eventURLStatus: "not_provided")
    }

    /// REF-10 A10 and decision 4: the field bounds are named once, mirror
    /// orbit-core's, let a 5 KB title through and name the field, its bytes and
    /// the bound one byte past it — so the reader can leave that one event out.
    func testFieldBoundsAreNamedAndOneBytePastThemNamesTheField() throws {
        // Shared values are checked by contracts/constants.json; these are
        // the local relationships that leave room inside a protocol line.
        XCTAssertLessThan(CalendarLimits.pageEventBytes, CalendarLimits.maximumPageBytes)
        XCTAssertLessThan(CalendarLimits.maximumPageBytes, NativeProtocol.maximumLineBytes)
        XCTAssertEqual(CalendarLimits.maximumEventBytes, CalendarLimits.maximumPageBytes - 65_536)
        let long = try event(title: String(repeating: "x", count: 5 * 1024))
        XCTAssertEqual(long.title.utf8.count, 5 * 1024)
        let longest = try event(title: String(repeating: "x", count: CalendarLimits.maximumTitleBytes))
        XCTAssertEqual(longest.title.utf8.count, CalendarLimits.maximumTitleBytes)
        XCTAssertThrowsError(try event(title: String(repeating: "x", count: CalendarLimits.maximumTitleBytes + 1))) {
            XCTAssertEqual($0 as? CalendarOversizedField, CalendarOversizedField(field: "title",
                bytes: CalendarLimits.maximumTitleBytes + 1, limit: CalendarLimits.maximumTitleBytes))
        }
        XCTAssertTrue(CalendarLimits.identifier(String(repeating: "i", count: CalendarLimits.maximumLabelBytes)))
        XCTAssertFalse(CalendarLimits.identifier(String(repeating: "i", count: CalendarLimits.maximumLabelBytes + 1)))
        let url = "urn:" + String(repeating: "u", count: CalendarLimits.maximumURLBytes)
        XCTAssertThrowsError(try CalendarInvitee(name: nil, url: url, role: "required",
            responseStatus: "accepted", participantType: "person", isCurrentUser: false)) {
            XCTAssertEqual($0 as? CalendarOversizedField, CalendarOversizedField(field: "invitee_url",
                bytes: url.utf8.count, limit: CalendarLimits.maximumURLBytes))
        }
    }

    /// Decision 4: a page names the events it left out, and says nothing when
    /// there is none, so the page before this change is byte for byte the same.
    func testAPageNamesTheEventsItLeftOutAndOnlyThen() throws {
        let left = CalendarOversizedEvent(calendar: CalendarReference(calendarID: "local", sourceID: "account"),
            calendarItemID: "item-500", start: 100, end: 200, field: "title", bytes: 120_000,
            limit: CalendarLimits.maximumTitleBytes)
        let named = CalendarReadPage(id: id, nativeInstanceID: id, generation: 2,
            window: CalendarWindow(start: 0, end: 100_000_000), started: 1, completed: 2, observedAt: 100,
            systemTimezone: "Europe/Madrid", events: [], omitted: [], oversized: [left],
            afterIndex: 0, total: 0, nextAfterIndex: nil)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(named)) as? [String: Any])
        let list = try XCTUnwrap(payload["oversized"] as? [[String: Any]])
        XCTAssertEqual(list.first?["calendar_item_id"] as? String, "item-500")
        XCTAssertEqual(list.first?["field"] as? String, "title")
        XCTAssertEqual(list.first?["bytes"] as? Int, 120_000)
        XCTAssertEqual(list.first?["limit"] as? Int, 98_304)
        let plain = CalendarReadPage(id: id, nativeInstanceID: id, generation: 2,
            window: CalendarWindow(start: 0, end: 100_000_000), started: 1, completed: 2, observedAt: 100,
            systemTimezone: "Europe/Madrid", events: [], omitted: [], afterIndex: 0, total: 0, nextAfterIndex: nil)
        let bare = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(plain)) as? [String: Any])
        XCTAssertNil(bare["oversized"])
    }

    func testMovedOccurrenceAndTwentyFiveHourDayRemainDistinctFromUTCAndSeriesIdentity() throws {
        let value = try event(floating: true)
        XCTAssertEqual(value.end_unix_ms - value.start_unix_ms, 90_000_000)
        XCTAssertEqual(value.original_occurrence_unix_ms, 1000)
        XCTAssertNotEqual(value.original_occurrence_unix_ms, value.start_unix_ms)
        XCTAssertTrue(value.is_floating); XCTAssertTrue(value.is_all_day); XCTAssertTrue(value.is_detached)
        XCTAssertNil(value.event_timezone); XCTAssertEqual(value.interpreted_timezone, "Europe/Madrid")
        XCTAssertEqual(value.status, "canceled")
    }

    func testNamesAndURLsArePartialCandidatesNotAttendanceOrInventedEmail() throws {
        let participant = try CalendarInvitee(name: nil, url: "urn:uuid:room-1", role: "required",
            responseStatus: "accepted", participantType: "room", isCurrentUser: false)
        XCTAssertNil(participant.name); XCTAssertNil(participant.email)
        XCTAssertEqual(participant.participant_url, "urn:uuid:room-1")
        XCTAssertEqual(CalendarInvitee.mailbox("mailto:Ana%2Bwork@Example.test"), "Ana+work@Example.test")
        for uri in ["https://example.test/user@host.test", "urn:person:ana", "mailto:a@b.test,c@d.test",
                    "mailto:a@b.test?subject=hidden", "mailto:a%0A@b.test", "mailto:a@@b.test", "mailto:a%FF@b.test",
                    "mailto:a%ZZ@b.test", "mailto:a%@b.test", "\nmailto:a@b.test"] {
            XCTAssertNil(CalendarInvitee.mailbox(uri), uri)
        }
    }

    func testExactTextPreservedWithoutNormalizationOrSilentTruncation() throws {
        let title = "  e\u{301}\t\n🪐 \u{202e}  "
        let value = try event(title: title)
        XCTAssertEqual(Array(value.title.utf8), Array(title.utf8))
        // The bound comes from the constant, never a number written here: a
        // four-byte character that straddles it is one byte too many, and the
        // same characters exactly at it pass whole (REF-10, decision 4).
        let atTheBound = String(repeating: "🪐", count: CalendarLimits.maximumTitleBytes / 4)
        XCTAssertEqual(try event(title: atTheBound).title.utf8.count, CalendarLimits.maximumTitleBytes)
        let straddling = String(repeating: "x", count: CalendarLimits.maximumTitleBytes - 3) + "🪐"
        XCTAssertEqual(straddling.utf8.count, CalendarLimits.maximumTitleBytes + 1)
        XCTAssertThrowsError(try event(title: straddling)) {
            XCTAssertEqual($0 as? CalendarOversizedField, CalendarOversizedField(field: "title",
                bytes: CalendarLimits.maximumTitleBytes + 1, limit: CalendarLimits.maximumTitleBytes))
        }
        XCTAssertThrowsError(try CalendarInvitee(name: nil, url: "mailto:a\u{0}@b.test", role: "unknown",
            responseStatus: "unknown", participantType: "unknown", isCurrentUser: false))
    }

    /// REF-10 A3: a page takes whole items while they fit, always at least one,
    /// and says where the next page starts; walking the pages returns every
    /// item once. An index past the end belongs to another enumeration.
    func testPagingTakesWholeItemsAndNeverStalls() throws {
        let events = try (0..<500).map {
            try event(title: String(repeating: "x", count: 2048), start: Int64($0),
                      observation: String(format: "00000000-0000-4000-8000-%012ld", $0))
        }
        let ordered = CalendarPaging.ordered(Array(events.reversed()))
        XCTAssertEqual(ordered.map(\.start_unix_ms), (0..<500).map { Int64($0) })
        var collected = [CalendarOccurrence]()
        var after = 0
        var pages = 0
        while true {
            let page = try CalendarPaging.page(ordered, after: after, budget: CalendarLimits.pageEventBytes)
            XCTAssertFalse(page.items.isEmpty)
            collected += page.items; pages += 1
            guard let next = page.next else { break }
            after = next
        }
        XCTAssertGreaterThan(pages, 1)
        XCTAssertEqual(collected, ordered)
        XCTAssertThrowsError(try CalendarPaging.page(ordered, after: 501, budget: CalendarLimits.pageEventBytes)) {
            XCTAssertEqual($0 as? CalendarFailure, .invalidated)
        }
        let alone = try CalendarPaging.page(ordered, after: 0, budget: 1)
        XCTAssertEqual(alone.items.count, 1, "an item larger than the budget still travels alone")
        XCTAssertEqual(alone.next, 1)
    }

    func testAPageEncodesTheNextIndexWhenMoreEventsFollow() throws {
        let page = CalendarReadPage(id: id, nativeInstanceID: id, generation: 2,
            window: CalendarWindow(start: 0, end: 100_000_000), started: 1, completed: 2, observedAt: 100,
            systemTimezone: "Europe/Madrid", events: [try event()], omitted: [], afterIndex: 0, total: 2,
            nextAfterIndex: 1)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(page)) as? [String: Any])
        XCTAssertEqual(payload["next_after_index"] as? Int, 1)
    }

    /// P9-22: the reminders Orbit edits are the relative ones at or before
    /// the start, in whole minutes; the rest are counted, never dropped, and a
    /// list past `maximumAlarms` does not travel at all.
    func testRemindersOrbitEditsAreReadAndTheOthersCounted() throws {
        let relative = { (minutes: Double) in
            CalendarAlarmFacts(relativeOffset: -minutes * 60, hasAbsoluteDate: false, hasLocation: false)
        }
        let reading = CalendarAlarmReading([
            relative(10), relative(0),
            CalendarAlarmFacts(relativeOffset: 0, hasAbsoluteDate: true, hasLocation: false),
            CalendarAlarmFacts(relativeOffset: 0, hasAbsoluteDate: false, hasLocation: true),
            CalendarAlarmFacts(relativeOffset: 300, hasAbsoluteDate: false, hasLocation: false),
            relative(Double(CalendarLimits.maximumAlarmMinutes + 1)),
        ])
        XCTAssertEqual(reading, CalendarAlarmReading(minutes: [0, 10], notEditable: 4))
        let many = CalendarAlarmReading((0...CalendarLimits.maximumAlarms).map { relative(Double($0)) })
        XCTAssertEqual(many, CalendarAlarmReading(minutes: nil, notEditable: CalendarLimits.maximumAlarms + 1))
        XCTAssertEqual(CalendarAlarmReading([]), CalendarAlarmReading(minutes: [], notEditable: nil))
        let read = try CalendarOccurrence(id: id, calendar: CalendarReference(calendarID: "local", sourceID: "account"),
            calendarItemID: "item", eventID: nil, originalOccurrence: nil, start: 2000, end: 3000,
            eventTimezone: nil, interpretedTimezone: "Europe/Madrid", allDay: false, detached: false,
            title: "Recordatorios", status: "confirmed", lastModified: nil, invitees: nil,
            inviteesStatus: "not_provided", eventURL: nil, eventURLStatus: "not_provided", alarms: reading)
        let json = try XCTUnwrap(String(data: JSONEncoder().encode(read), encoding: .utf8))
        XCTAssertTrue(json.contains("\"alarms_minutes\":[0,10]"), json)
        XCTAssertTrue(json.contains("\"alarms_not_editable\":4"), json)
        XCTAssertThrowsError(try CalendarOccurrence(id: id,
            calendar: CalendarReference(calendarID: "local", sourceID: "account"), calendarItemID: "item",
            eventID: nil, originalOccurrence: nil, start: 2000, end: 3000, eventTimezone: nil,
            interpretedTimezone: "Europe/Madrid", allDay: false, detached: false, title: "Cero",
            status: "confirmed", lastModified: nil, invitees: nil, inviteesStatus: "not_provided",
            eventURL: nil, eventURLStatus: "not_provided",
            alarms: CalendarAlarmReading(minutes: [10], notEditable: 0)))
    }
}
