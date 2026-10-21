import Foundation
import XCTest
@testable import OrbitNativeProtocol

/// P9-21 criterion 1, Swift side: the calendar fixtures orbit-core generates
/// reach production codecs here — typed commands and encoded reads, writes
/// and one refusal per code. Swift refusals carry exactly the detail Rust decodes.
final class CalendarWriteWireTests: XCTestCase {
    private func fixture(_ path: String) throws -> Data {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try Data(contentsOf: root.appendingPathComponent("contracts/fixtures/native/\(path)"))
    }

    private let id = "00000000-0000-4000-8000-000000000911"
    private let reference = CalendarReference(calendarID: "local-calendar", sourceID: "local-source")

    private func matches<T: Encodable>(_ value: T, _ path: String) throws {
        let produced = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as? NSDictionary)
        let expected = try XCTUnwrap(JSONSerialization.jsonObject(with: fixture(path)) as? NSDictionary)
        XCTAssertEqual(produced, expected, path)
    }

    private func occurrence(invitation: Bool = false) throws -> CalendarOccurrence {
        let organizer = try CalendarInvitee(name: invitation ? nil : "Alina",
            url: invitation ? "mailto:comite@example.test" : "mailto:alina@example.test",
            role: "chair", responseStatus: "accepted", participantType: "person", isCurrentUser: !invitation)
        let ada = try CalendarInvitee(name: "Ada", url: "mailto:ada@example.test", role: "required",
            responseStatus: "accepted", participantType: "person", isCurrentUser: false)
        let observationID = invitation ? "00000000-0000-4000-8000-000000000913" : "00000000-0000-4000-8000-000000000912"
        let itemID = invitation ? "item-913" : "item-911"
        let eventID: String? = invitation ? nil : "event-911"
        let start: Int64 = invitation ? 1_792_065_600_000 : 1_792_058_400_000
        let end: Int64 = invitation ? 1_792_067_400_000 : 1_792_062_000_000
        let eventTimezone: String? = invitation ? nil : "Europe/Madrid"
        let title = invitation ? "Comité" : "Revisión semanal"
        let status = invitation ? "tentative" : "confirmed"
        let lastModified: Int64? = invitation ? nil : 1_791_972_000_000
        let invitees: [CalendarInvitee]? = invitation ? nil : [ada]
        let readStatus = invitation ? "not_provided" : "provided"
        let eventURL: String? = invitation ? nil : "https://meet.example.test/abc"
        let location: String? = invitation ? nil : "Sala 3"
        let notes: String? = invitation ? nil : "Orden del día sintético."
        let notesLeftOut: CalendarFieldLeftOut? = invitation
            ? .init(bytes: 40_000, limit: CalendarLimits.maximumNotesBytes) : nil
        let availability: CalendarAvailability = invitation ? .tentative : .busy
        let recurrence: CalendarRecurrence
        if invitation {
            recurrence = .init(frequency: .monthly, interval: 2, daysOfWeek: [.init(day: .friday, week: -1)],
                end: .init(untilMs: 1_823_594_400_000), rulesCount: 2)
        } else {
            recurrence = .init(frequency: .weekly, interval: 1, daysOfWeek: [.init(day: .wednesday)],
                end: .init(count: 10))
        }
        let alarms = CalendarAlarmReading(minutes: invitation ? [15] : [0, 10], notEditable: invitation ? 1 : nil)
        return try CalendarOccurrence(id: observationID, calendar: reference, calendarItemID: itemID, eventID: eventID,
            originalOccurrence: start, start: start, end: end, eventTimezone: eventTimezone,
            interpretedTimezone: "Europe/Madrid", allDay: false, detached: invitation, title: title,
            status: status, lastModified: lastModified, invitees: invitees, inviteesStatus: readStatus,
            eventURL: eventURL, eventURLStatus: readStatus, location: location, notes: notes,
            notesLeftOut: notesLeftOut, organizer: organizer, availability: availability,
            recurrence: recurrence, canEdit: !invitation, alarms: alarms)
    }

    func testTheWriteCommandsDecodeTheirTypedFields() throws {
        for path in ["commands/calendar_save.json", "calendar/commands/calendar_save-change.json"] {
            let bytes = try fixture(path)
            guard case .calendarSave = try NativeWireCommand.decode(bytes) else {
                return XCTFail("\(path) is not calendar_save")
            }
        }
        let create = try JSONDecoder().decode(NativeWireCommand.self, from: fixture("commands/calendar_save.json"))
        if case .calendarSave(let envelope) = create {
            let draft = try XCTUnwrap(envelope.fields.calendar_save.create)
            XCTAssertNil(draft.calendar_id, "sin calendario: el de por defecto")
            XCTAssertEqual(draft.recurrence?.days_of_week, [CalendarRecurrenceDay(day: .wednesday)])
            XCTAssertEqual(draft.alarms_minutes, [10, 0])
        }
        let bytes = try fixture("commands/calendar_delete.json")
        guard case .calendarDelete(let envelope) = try NativeWireCommand.decode(bytes) else {
            return XCTFail("calendar_delete fixture is another command")
        }
        XCTAssertEqual(envelope.fields.calendar_delete.span, .futureEvents)
    }

    func testTheReadCarriesEveryNewField() throws {
        let events = try [occurrence(), occurrence(invitation: true)]
        let page = CalendarReadPage(id: id, nativeInstanceID: id, generation: 1,
            window: .init(start: 1_792_058_400_000, end: 1_792_144_800_000),
            started: 100, completed: 200, observedAt: 1_792_058_400_000, systemTimezone: "Europe/Madrid",
            events: events, omitted: [], afterIndex: 0, total: 2, nextAfterIndex: nil)
        try matches(page, "results/calendar_snapshot.json")
        let calendar = CalendarEntry(reference: reference, title: "Trabajo", sourceTitle: "iCloud",
            calendarType: "caldav", color: "#1BADF8", canEdit: true, isDefault: true)
        try matches(CalendarInventoryPage(id: id, nativeInstanceID: id, generation: 1,
            started: 100, completed: 200, calendars: [calendar], afterIndex: 0, total: 1,
            nextAfterIndex: nil), "results/calendar_sources.json")
        XCTAssertTrue(calendar.isValid)
    }

    func testBothWriteResultsMatchTheProductionEncoders() throws {
        let saved = try CalendarSaveResult(id: id, nativeInstanceID: id, generation: 1,
            systemTimezone: "Europe/Madrid", occurrence: occurrence())
        try matches(saved, "results/calendar_save.json")
        let deleted = CalendarDeleteResult(id: id, nativeInstanceID: id, generation: 1,
            request: .init(calendarItemID: "item-911", occurrenceStart: 1_792_058_400_000, span: .futureEvents))
        try matches(deleted, "results/calendar_delete.json")
    }

    /// The refusals Swift sends carry exactly the detail the Rust side decodes.
    func testEveryRefusalSwiftBuildsMatchesItsFixture() throws {
        let target = CalendarOccurrenceTarget(calendarItemID: "item-911", occurrenceStart: 1_792_058_400_000)
        var tooLong = CalendarInvalidChange(field: "title", problem: .tooLarge)
        tooLong.bytes = 100_000; tooLong.limit = CalendarLimits.maximumTitleBytes
        let refusals: [CalendarWriteRefusal] = [
            .eventNotFound(target),
            .notEditable(.organizedBySomeoneElse, calendarID: "local-calendar"),
            .invalidChange(tooLong),
            .permissionDenied(.writeOnly),
            .saveFailed(domain: "EKErrorDomain", code: 12),
            .outcomeUnknown(cause: "write_deadline"),
        ]
        for refusal in refusals {
            let bytes = try fixture("calendar/refusals/\(refusal.code).json")
            let expected = try XCTUnwrap(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
            let expectedError = try XCTUnwrap(expected["error"] as? [String: Any])
            XCTAssertEqual(expected["ok"] as? Bool, false)
            XCTAssertEqual(expectedError["code"] as? String, refusal.code)
            let built = NativeEvent.makeRejection(id: try XCTUnwrap(expected["id"] as? String),
                code: refusal.code, message: refusal.message, detail: refusal.detail)
            let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(built)) as? [String: Any])
            let builtError = try XCTUnwrap(encoded["error"] as? [String: Any])
            let builtDetail = try XCTUnwrap(builtError["detail"] as? NSDictionary)
            let expectedDetail = try XCTUnwrap(expectedError["detail"] as? NSDictionary)
            XCTAssertEqual(builtDetail, expectedDetail, refusal.code)
        }
    }

    func testMalformedWriteBodiesAreRefusedDuringTypedDecoding() throws {
        for body in ["{}", #"{"create":{"title":7}}"#,
            #"{"create":{"title":"x","start_ms":0,"end_ms":1,"all_day":false},"change":{"calendar_item_id":"x","occurrence_start_ms":0,"span":"this","changes":{}}}"#] {
            let bytes = Data("{\"v\":1,\"id\":\"x\",\"command\":\"calendar_save\",\"calendar_save\":\(body)}".utf8)
            XCTAssertThrowsError(try NativeWireCommand.decode(bytes), body)
        }
    }
}
