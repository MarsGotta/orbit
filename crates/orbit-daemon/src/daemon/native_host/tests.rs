use super::*;
use serde_json::json;

const INSTANCE: &str = "4c1ac2d0-cb80-4ab0-ab31-5cc1d1d4420e";

fn state(generation: u64, stopped: bool) -> Value {
    json!({"event":"state", "native_instance_id":INSTANCE,
        "privacy_generation":generation, "privacy_stopped":stopped})
}

fn available() -> NativeHost {
    let mut host = NativeHost::default();
    host.observe(&json!({"event":"ready", "capabilities":["application_identity"]}));
    host.observe(&state(4, false));
    host
}

#[test]
fn startup_and_state_do_not_invent_backend_support() {
    let mut host = NativeHost::default();
    assert!(host.identity().is_none());
    assert_eq!(host.observe(&state(4, false)), HostUpdate::default());
    assert_eq!(
        host.identity().map(|value| value.id.as_str()),
        Some(INSTANCE)
    );
}

#[test]
fn every_ready_clears_identity_and_requires_fresh_readback() {
    let mut host = available();
    for capabilities in [json!(["application_identity"]), json!([]), json!(null)] {
        let update = host.observe(&json!({"event":"ready", "capabilities":capabilities,
            "native_instance_id":INSTANCE, "privacy_generation":4, "privacy_stopped":false}));
        assert!(update.authority_changed);
        assert!(!update.privacy_stopped);
        assert!(host.identity().is_none());
        assert_eq!(host.observe(&state(4, false)), HostUpdate::default());
    }
}

#[test]
fn successful_readback_shares_exact_epoch_and_stop_facts() {
    let mut host = available();
    let update = host.observe(&json!({"event":"response", "ok":true,
        "result":state(u64::MAX, true)}));
    assert_eq!(
        update,
        HostUpdate {
            authority_changed: true,
            capture_cut_rebound: false,
            privacy_stopped: true,
            capture_cut: false,
        }
    );
    assert_eq!(
        host.identity(),
        Some(&HostIdentity {
            id: INSTANCE.into(),
            generation: u64::MAX,
            stopped: true,
        })
    );
    assert_eq!(
        host.observe(&state(u64::MAX, true)),
        HostUpdate {
            authority_changed: false,
            capture_cut_rebound: false,
            privacy_stopped: true,
            capture_cut: false,
        }
    );
}

#[test]
fn changed_process_or_generation_invalidates_old_consumers() {
    let mut host = available();
    assert!(host.observe(&state(5, false)).authority_changed);
    let mut replacement = state(5, false);
    replacement["native_instance_id"] = json!("7a41d970-0a46-4e98-8b8f-2b36ff615a72");
    assert!(host.observe(&replacement).authority_changed);
    assert_eq!(host.observe(&replacement), HostUpdate::default());
    // A clock/epoch rollback is also a change, never authority renewal.
    replacement["privacy_generation"] = json!(0);
    assert!(host.observe(&replacement).authority_changed);
}

#[test]
fn malformed_facts_or_unsuccessful_responses_never_replace_observations() {
    let mut host = available();
    let before = host.identity().cloned();
    for invalid in [
        json!({"native_instance_id":INSTANCE,"privacy_generation":5}),
        json!({"native_instance_id":INSTANCE,"privacy_generation":-1,"privacy_stopped":false}),
        json!({"native_instance_id":INSTANCE,"privacy_generation":"5","privacy_stopped":false}),
        json!({"native_instance_id":INSTANCE,"privacy_generation":5,"privacy_stopped":"false"}),
        json!({"native_instance_id":"4c1ac2d0cb804ab0ab315cc1d1d4420e","privacy_generation":5,"privacy_stopped":false}),
        json!({"native_instance_id":"not-a-native-process","privacy_generation":5,"privacy_stopped":false}),
    ] {
        assert_eq!(
            host.observe(&json!({"event":"response", "ok":true,"result":invalid})),
            HostUpdate::default()
        );
        assert_eq!(host.identity(), before.as_ref());
    }
    for success in [json!(false), json!("true"), json!(null)] {
        assert_eq!(
            host.observe(&json!({"event":"response","ok":success,"result":state(5,true)})),
            HostUpdate::default()
        );
        assert_eq!(host.identity(), before.as_ref());
    }
}

#[test]
fn stop_and_disconnect_signal_revocation_even_before_identity_exists() {
    for event in ["privacy_stop", "helper_disconnected"] {
        for mut host in [NativeHost::default(), available()] {
            let update = host.observe(&json!({"event":event}));
            assert!(update.privacy_stopped);
            assert!(host.identity().is_none_or(|identity| identity.stopped));
            // Rearm is handled elsewhere; this event cannot manufacture an
            // observed native epoch or reactivate a consumer's old grant.
            assert_eq!(
                host.observe(&json!({"event":"local_rearm","privacy_stopped":false})),
                HostUpdate::default()
            );
            assert!(host.identity().is_none_or(|identity| identity.stopped));
        }
    }
}

/// A capture cut retires the old epoch like the user's stop does, and is
/// reported as the cut it is: consumers never call it a privacy stop.
#[test]
fn a_capture_cut_revokes_the_epoch_without_being_called_a_privacy_stop() {
    for mut host in [NativeHost::default(), available()] {
        let update = host.observe(&json!({"event":"capture_stopped",
            "reason":"session_inactive","privacy_stopped":false}));
        assert!(update.capture_cut && update.stopped());
        assert!(!update.privacy_stopped);
        assert!(host.identity().is_none_or(|identity| identity.stopped));
    }
}

#[test]
fn only_the_same_helper_next_epoch_rebinds_a_capture_cut() {
    let mut host = available();
    host.observe(&json!({"event":"capture_stopped","reason":"session_inactive"}));
    let next = host.observe(&state(5, false));
    assert!(next.authority_changed && next.capture_cut_rebound);
    assert!(!host.observe(&state(5, false)).capture_cut_rebound);

    let mut replacement = available();
    replacement.observe(&json!({"event":"capture_stopped","reason":"session_inactive"}));
    let mut changed = state(5, false);
    changed["native_instance_id"] = json!("7a41d970-0a46-4e98-8b8f-2b36ff615a72");
    assert!(!replacement.observe(&changed).capture_cut_rebound);

    let mut skipped = available();
    skipped.observe(&json!({"event":"capture_stopped","reason":"session_inactive"}));
    assert!(!skipped.observe(&state(6, false)).capture_cut_rebound);
}

#[test]
fn unrelated_native_payloads_are_not_host_readbacks() {
    let mut host = available();
    let before = host.identity().cloned();
    for name in ["audio", "hotkey", "command", "application_identity"] {
        let mut event = state(5, true);
        event["event"] = json!(name);
        assert_eq!(host.observe(&event), HostUpdate::default());
        assert_eq!(host.identity(), before.as_ref());
    }
}

#[test]
fn reducing_facts_does_not_write_state_or_emit_effects() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = crate::daemon::audio::tests::fixture(directory.path())?;
    let before = runtime.state.clone();
    runtime.fixture_store.save_snapshot(&before)?;
    runtime.observe_native_host(&json!({"event":"ready","capabilities":["application_identity"]}));
    runtime.observe_native_host(&state(4, false));
    assert!(runtime.observe_native_host(&state(5, true)).privacy_stopped);
    assert_eq!(runtime.state, before);
    assert_eq!(
        runtime.fixture_store.load_snapshot::<orbit_core::State>()?,
        Some(before)
    );
    assert!(native.try_recv().is_err());
    Ok(())
}
