import Foundation
import XCTest
@testable import OrbitNativeState

/// The rearm path is the documented way out of a privacy stop. These pin the two
/// failures that made it unreachable: a channel flag that only ever went down,
/// and a refusal that said nothing at all.
final class RearmTests: XCTestCase {
    private func ready(connected: Bool = true, quitRequested: Bool = false,
                       commandInFlight: Bool = false, cleanupInFlight: Int = 0,
                       microphoneSlot: CaptureSlot = .idle,
                       systemAudioSlot: CaptureSlot = .idle) -> RearmReadiness {
        RearmReadiness(connected: connected, quitRequested: quitRequested,
                       commandInFlight: commandInFlight, cleanupInFlight: cleanupInFlight,
                       microphoneSlot: microphoneSlot,
                       systemAudioSlot: systemAudioSlot)
    }

    func testALaterBeatBringsBackAChannelTheWatchdogGaveUpOn() {
        // The flag was set false by the watchdog and by the transport, and set
        // true by nobody, so one late beat during sleep disabled rearming and
        // every permission prompt until the app was restarted.
        var health = DaemonChannelHealth(now: 100)
        XCTAssertTrue(health.tick(now: 100 + DaemonChannelHealth.beatTimeout + 1))
        XCTAssertFalse(health.connected)

        health.observedBeat(at: 200)

        XCTAssertTrue(health.connected)
        XCTAssertFalse(health.tick(now: 201))
    }

    func testSilenceTakesTheChannelDownExactlyOnce() {
        // The caller stops capture on the transition, so repeating it on every
        // one-second tick would restop a Mac that is already stopped.
        var health = DaemonChannelHealth(now: 0)
        XCTAssertFalse(health.tick(now: DaemonChannelHealth.beatTimeout))
        XCTAssertTrue(health.tick(now: DaemonChannelHealth.beatTimeout + 1))
        XCTAssertFalse(health.tick(now: DaemonChannelHealth.beatTimeout + 2))
    }

    func testATransportDisconnectSurvivesUntilRealTrafficReturns() {
        var health = DaemonChannelHealth(now: 0)
        health.disconnected()
        XCTAssertFalse(health.connected)
        XCTAssertFalse(health.tick(now: 1))
        health.observedBeat(at: 2)
        XCTAssertTrue(health.connected)
    }

    func testEveryRefusalToRearmNamesItselfInsteadOfStayingSilent() {
        // A bare `guard ... else { return }` behind the only rearm control Orbit
        // offers is indistinguishable from a broken app.
        let blocked: [RearmReadiness] = [
            ready(quitRequested: true),
            ready(connected: false),
            ready(commandInFlight: true),
            ready(cleanupInFlight: 1),
            ready(microphoneSlot: .active),
            ready(systemAudioSlot: .draining)
        ]
        for condition in blocked {
            XCTAssertFalse(condition.allowed)
            let refusal = condition.refusal ?? ""
            XCTAssertFalse(refusal.isEmpty, "a blocked rearm must say why")
            XCTAssertGreaterThan(refusal.count, 20, "the reason must be readable prose")
        }
    }

    func testAnIdleConnectedMacIsAllowedToRearm() {
        XCTAssertTrue(ready().allowed)
        XCTAssertNil(ready().refusal)
    }

    func testDisconnectedServiceRefusalTakesPriorityOverQuitRequested() {
        let refusal = ready(connected: false, quitRequested: true).refusal

        XCTAssertEqual(refusal, ready(connected: false).refusal)
        XCTAssertTrue(refusal?.contains("servicio") == true)
        XCTAssertFalse(refusal?.contains("cerrando") == true)
    }

    func testAnUnconfirmedDrainKeepsItsRecordAndAllowsRearm() {
        // Rearming neither clears the prior audio queue nor erases uncertainty;
        // the slot keeps that record without trapping rearm behind a restart.
        let slot = CaptureSlot.unconfirmed(reason: "audio_drain_incomplete")
        let readiness = ready(microphoneSlot: slot)

        XCTAssertEqual(readiness.microphoneSlot, slot)
        XCTAssertTrue(readiness.allowed)
        XCTAssertNil(readiness.refusal)
    }
}
