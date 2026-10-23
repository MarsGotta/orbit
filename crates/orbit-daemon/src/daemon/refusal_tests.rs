use super::*;
use orbit_core::refusal::Permission;

#[test]
fn typed_cause_survives_anyhow_context_and_unknown_names_its_internal_cause() {
    use orbit_core::refusal::Refusal as Core;
    for permission in [Permission::Microphone, Permission::ScreenRecording] {
        for (cause, expected) in [
            (
                Core::PermissionDenied { permission },
                Refusal::PermissionDenied { permission },
            ),
            (
                Core::PermissionNotGranted { permission },
                Refusal::PermissionNotGranted { permission },
            ),
            (
                Core::PermissionUnknown { permission },
                Refusal::PermissionUnknown { permission },
            ),
        ] {
            assert_eq!(refusal_from_error(&cause.into()), expected);
        }
    }

    let error = anyhow::Error::new(orbit_core::refusal::Refusal::PermissionUnknown {
        permission: Permission::Microphone,
    })
    .context("orden rechazada");
    assert_eq!(
        refusal_from_error(&error),
        Refusal::PermissionUnknown {
            permission: orbit_core::refusal::Permission::Microphone
        }
    );
    let upstream = anyhow::Error::new(crate::retry::Refused(403)).context("send");
    assert_eq!(
        refusal_from_error(&upstream),
        Refusal::UpstreamFailed {
            service: Service::Hub,
            status: Some(403)
        }
    );
    let missing = anyhow::Error::new(orbit_store::Error::NotFound).context("lectura");
    assert!(matches!(
        refusal_from_error(&missing),
        Refusal::StorageFailed {
            operation: orbit_core::refusal::Operation::Storage,
            ..
        }
    ));
    let unknown = anyhow::anyhow!("synthetic cause");
    assert_eq!(
        refusal_from_error(&unknown),
        Refusal::Internal {
            cause: orbit_core::refusal::InternalCause::DaemonOperation {
                error: "synthetic cause".into()
            }
        }
    );
    use orbit_core::refusal::{InternalCause, Operation};
    let hub = Refusal::HubUnreachable { since_ms: None };
    assert_eq!(refusal_from_error(&anyhow::Error::new(hub.clone())), hub);
    for error in [
        orbit_store::Error::NotFound,
        orbit_store::Error::Invalid("synthetic"),
    ] {
        let cause = error.to_string();
        assert_eq!(
            refusal_from_error(&error.into()),
            Refusal::StorageFailed {
                operation: Operation::Storage,
                cause: Some(cause.into())
            }
        );
    }
    let error = anyhow::anyhow!("synthetic error");
    assert_eq!(
        refusal_from_error(&error),
        Refusal::Internal {
            cause: InternalCause::DaemonOperation {
                error: error.to_string()
            }
        }
    );
}
