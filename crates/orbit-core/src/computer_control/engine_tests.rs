//! REF-11: the engine's lifecycle as a pure decision, walked over every
//! combination of the facts it reads: 5 × 5 permission states × privacy ×
//! helper × running × retry = 400.
use super::*;

const STATES: [PermissionState; 5] = [
    PermissionState::Granted,
    PermissionState::Denied,
    PermissionState::NotGranted,
    PermissionState::NotDetermined,
    PermissionState::Unknown,
];

/// A Mac whose helper is connected and reports these permissions.
fn reported(accessibility: PermissionState, screen_recording: PermissionState) -> Observed {
    Observed {
        privacy_stopped: false,
        helper_connected: true,
        accessibility,
        screen_recording,
    }
}

fn every_fact() -> Vec<Facts> {
    let mut all = Vec::new();
    for accessibility in STATES {
        for screen_recording in STATES {
            for privacy_stopped in [false, true] {
                for helper_connected in [false, true] {
                    for running in [false, true] {
                        for retry_due in [false, true] {
                            all.push(Facts {
                                observed: Observed {
                                    privacy_stopped,
                                    helper_connected,
                                    accessibility,
                                    screen_recording,
                                },
                                running,
                                retry_due,
                                closing: false,
                            });
                        }
                    }
                }
            }
        }
    }
    all
}

fn known_no(state: PermissionState) -> bool {
    matches!(
        state,
        PermissionState::Denied | PermissionState::NotGranted | PermissionState::NotDetermined
    )
}

/// A2: with both permissions and privacy armed, the engine starts, and the
/// facts the decision reads name no switch, no pairing and no hub.
#[test]
fn ref11_a2_both_permissions_start_the_engine_and_nothing_else_is_asked() {
    let facts = Facts {
        observed: reported(PermissionState::Granted, PermissionState::Granted),
        running: false,
        retry_due: true,
        closing: false,
    };
    assert_eq!(decide(facts), Decision::Start);
    // Already running: nothing to do, and nothing restarts it.
    assert_eq!(
        decide(Facts {
            running: true,
            ..facts
        }),
        Decision::Keep
    );
    // The pause after a failure is the only thing that holds a start back.
    assert_eq!(
        decide(Facts {
            retry_due: false,
            ..facts
        }),
        Decision::Keep
    );
}

/// REF-12: these are the facts the decision currently sees on the first
/// maintenance turn after close_for_exit. They must not permit a restart.
#[test]
fn ref12_closing_does_not_restart_a_granted_engine() {
    let after_close = Facts {
        observed: reported(PermissionState::Granted, PermissionState::Granted),
        running: false,
        retry_due: true,
        closing: true,
    };
    assert_eq!(decide(after_close), Decision::Keep);
    assert_eq!(
        decide(Facts {
            running: true,
            ..after_close
        }),
        Decision::StopForExit
    );
}

/// A3: without a permission it does not start, and the refusal says which one
/// is missing; an unknown permission is its own refusal, never a «no».
#[test]
fn ref11_a3_a_missing_permission_is_named_and_an_unknown_one_is_not_a_no() {
    let refused = |accessibility, screen_recording| {
        admits(
            SystemPermissions::ENGINE,
            reported(accessibility, screen_recording),
        )
    };
    let only_accessibility = SystemPermissions {
        accessibility: true,
        screen_recording: false,
    };
    let only_screen = SystemPermissions {
        accessibility: false,
        screen_recording: true,
    };
    assert_eq!(
        refused(PermissionState::Denied, PermissionState::Granted),
        Err(Refusal::PermissionDenied {
            permission: crate::refusal::missing_permission_name(only_accessibility)
        })
    );
    assert_eq!(
        refused(PermissionState::Granted, PermissionState::Denied),
        Err(Refusal::PermissionDenied {
            permission: crate::refusal::missing_permission_name(only_screen)
        })
    );
    assert_eq!(
        refused(PermissionState::NotDetermined, PermissionState::Denied),
        Err(Refusal::PermissionDenied {
            permission: crate::refusal::missing_permission_name(SystemPermissions::ENGINE)
        })
    );
    // A known «no» wins over an unknown: it is already certain.
    assert_eq!(
        refused(PermissionState::Denied, PermissionState::Unknown),
        Err(Refusal::PermissionDenied {
            permission: crate::refusal::missing_permission_name(only_accessibility)
        })
    );
    assert_eq!(
        refused(PermissionState::Unknown, PermissionState::Granted),
        Err(Refusal::PermissionUnknown {
            permission: crate::refusal::missing_permission_name(only_accessibility)
        })
    );
    assert_eq!(
        refused(PermissionState::Unknown, PermissionState::Unknown),
        Err(Refusal::PermissionUnknown {
            permission: crate::refusal::missing_permission_name(SystemPermissions::ENGINE)
        })
    );
    assert_eq!(
        refused(PermissionState::Granted, PermissionState::Granted),
        Ok(())
    );

    assert_eq!(
        Refusal::PermissionDenied {
            permission: crate::refusal::missing_permission_name(only_accessibility)
        }
        .code()
        .name(),
        "permission_denied"
    );
    assert_eq!(
        Refusal::PermissionDenied {
            permission: crate::refusal::missing_permission_name(only_screen)
        }
        .code()
        .name(),
        "permission_denied"
    );
    assert_eq!(
        Refusal::PermissionDenied {
            permission: crate::refusal::missing_permission_name(SystemPermissions::ENGINE)
        }
        .code()
        .name(),
        "permission_denied"
    );
    assert_eq!(
        Refusal::PermissionUnknown {
            permission: crate::refusal::missing_permission_name(only_screen)
        }
        .code()
        .name(),
        "permission_unknown"
    );
}

/// Review (b): no helper is not «not said yet». It has its own code, and its
/// sentence neither blames macOS nor promises an answer in a few seconds —
/// with no helper nothing will answer, however often somebody asks. Whatever
/// the permissions last reported, and whether or not they were granted.
#[test]
fn ref11_review_b_no_helper_is_its_own_cause_and_promises_no_answer() {
    for accessibility in STATES {
        for screen_recording in STATES {
            let observed = Observed {
                helper_connected: false,
                ..reported(accessibility, screen_recording)
            };
            assert_eq!(
                admits(SystemPermissions::ENGINE, observed),
                Err(Refusal::HelperNotRunning),
                "{observed:?}"
            );
            assert_eq!(
                admits(SystemPermissions::SCREEN, observed),
                Err(Refusal::HelperNotRunning),
                "{observed:?}"
            );
        }
    }
    let refusal = Refusal::HelperNotRunning;
    assert_eq!(refusal.code().name(), "helper_not_running");
    assert_ne!(
        refusal.code().name(),
        Refusal::PermissionUnknown {
            permission: crate::refusal::missing_permission_name(SystemPermissions::ENGINE)
        }
        .code()
        .name()
    );

    assert_eq!(
        Status::refused(refusal),
        Some(Status::HelperNotConnected),
        "la app recibe la misma causa"
    );
    // The privacy stop still comes first: it is Mars's, and it is known.
    assert_eq!(
        admits(
            SystemPermissions::ENGINE,
            Observed {
                privacy_stopped: true,
                helper_connected: false,
                ..reported(PermissionState::Granted, PermissionState::Granted)
            }
        ),
        Err(Refusal::PrivacyStopped { since_ms: None })
    );
}

/// A3: what is not known — a report not in yet, or no helper to make one —
/// neither starts the engine nor stops it, over all 400 combinations of the
/// facts rather than a hand-picked few.
#[test]
fn ref11_a3_unknown_neither_starts_nor_stops_over_every_combination() {
    let all = every_fact();
    assert_eq!(all.len(), 400);
    for facts in all {
        let decision = decide(facts);
        let observed = facts.observed;
        let both_granted = observed.accessibility == PermissionState::Granted
            && observed.screen_recording == PermissionState::Granted;
        let known_refusal = observed.privacy_stopped
            || observed.helper_connected
                && (known_no(observed.accessibility) || known_no(observed.screen_recording));
        let expected = if facts.running {
            if known_refusal { "stop" } else { "keep" }
        } else if both_granted
            && observed.helper_connected
            && !observed.privacy_stopped
            && facts.retry_due
        {
            "start"
        } else {
            "keep"
        };
        let actual = match decision {
            Decision::Start => "start",
            Decision::Keep => "keep",
            Decision::Stop(_) => "stop",
            Decision::StopForExit => "stop",
        };
        assert_eq!(actual, expected, "{facts:?} → {decision:?}");
    }
}

/// A4: withdrawing a permission stops a running engine, and the stop says
/// which one was withdrawn.
#[test]
fn ref11_a4_withdrawing_a_permission_stops_the_engine_with_its_name() {
    let running = Facts {
        observed: reported(PermissionState::Denied, PermissionState::Granted),
        running: true,
        retry_due: true,
        closing: false,
    };
    let Decision::Stop(refusal) = decide(running) else {
        panic!("una retirada conocida no paró el motor");
    };
    assert_eq!(refusal.code().name(), "permission_denied");

    assert_eq!(
        Status::refused(refusal),
        Some(Status::PermissionMissing {
            permissions: SystemPermissions {
                accessibility: true,
                screen_recording: false,
            }
        })
    );
}

/// A7: the privacy stop still stops it, whatever the permissions say.
#[test]
fn ref11_a7_the_privacy_stop_stops_the_engine_whatever_the_permissions_say() {
    for accessibility in STATES {
        for screen_recording in STATES {
            for helper_connected in [false, true] {
                let facts = Facts {
                    observed: Observed {
                        privacy_stopped: true,
                        helper_connected,
                        accessibility,
                        screen_recording,
                    },
                    running: true,
                    retry_due: true,
                    closing: false,
                };
                assert_eq!(
                    decide(facts),
                    Decision::Stop(Refusal::PrivacyStopped { since_ms: None }),
                    "{facts:?}"
                );
                assert_eq!(
                    decide(Facts {
                        running: false,
                        ..facts
                    }),
                    Decision::Keep,
                    "con la parada echada no arranca: {facts:?}"
                );
            }
        }
    }
}

/// A screenshot needs the screen and nothing else: a missing Accessibility
/// does not refuse it, and an unknown one does not either.
#[test]
fn a_screen_only_request_reads_only_screen_recording() {
    for accessibility in STATES {
        assert_eq!(
            admits(
                SystemPermissions::SCREEN,
                reported(accessibility, PermissionState::Granted)
            ),
            Ok(())
        );
    }
    assert_eq!(
        admits(
            SystemPermissions::SCREEN,
            reported(PermissionState::Granted, PermissionState::Denied)
        ),
        Err(Refusal::PermissionDenied {
            permission: crate::refusal::missing_permission_name(SystemPermissions::SCREEN)
        })
    );
}
