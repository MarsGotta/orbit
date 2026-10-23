//! Generate synthetic native fixtures through the production Rust wire types.
use orbit_core::wire::native::{NativeCommand, NativeEvent};
use serde::Serialize;
use serde_json::{Value, json};
use std::{error::Error, fs, path::Path};

fn write<T: Serialize>(directory: &Path, name: &str, frame: &T) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(directory)?;
    fs::write(
        directory.join(format!("{name}.json")),
        format!("{}\n", serde_json::to_string_pretty(frame)?),
    )?;
    Ok(())
}

pub fn generate() -> Result<(), Box<dyn Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("fixture directory required")?;
    let root = Path::new(&root);
    let id = "00000000-0000-4000-8000-000000000911";
    let binding = json!({
        "native_instance_id":id,"privacy_generation":1,"inventory_id":id,
        "selection_token":id,"window_id":42,"application_pid":4242,
        "application_instance_id":id,"binding_quality":"observed_owner_and_window",
        "descriptor_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    });
    let application_binding = json!({
        "native_instance_id":id,"privacy_generation":1,"application_pid":4242,
        "application_instance_id":id,"identity_token":id,
        "identity":{"bundle_id":"com.example.meeting","bundle_path":"/Applications/Fixture.app",
            "designated_requirement_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
        "clock_domain":"mach_absolute_time","observed_monotonic_ns":100,
        "expires_monotonic_ns":200
    });
    let state = json!({
        "privacy_stopped":false,"connected":true,"native_instance_id":id,
        "privacy_generation":1,"selected_text_available":true,
        "microphone":false,"system_audio":false,
        "playing":false,
        "supported_audio_processing":["unprocessed","voice_processing"],
        "voice_processing_available":false,"playback_reference_available":false,
        "audio_processing":{"mode":"unprocessed","enabled":false,"bypassed":true,
            "agc_enabled":false,"playback_reference":"none","pipeline_id":"fixture",
            "engine_running":false,"render_sample_rate":48000},
        "calendar":{"authorization":"full_access","physical_read_pending":false}
    });
    for (name, fields) in [
        ("hello", json!({})),
        ("heartbeat", json!({})),
        ("status", json!({})),
        ("calendar_disconnect", json!({})),
        (
            "calendar_sources",
            json!({"calendar_page":{"after_index":0}}),
        ),
        (
            "calendar_snapshot",
            json!({"calendar_query":{"start_unix_ms":0,"end_unix_ms":1000,"after_index":0}}),
        ),
        (
            "calendar_save",
            json!({"calendar_save":{"create":create_draft()}}),
        ),
        (
            "calendar_delete",
            json!({"calendar_delete":{"calendar_item_id":"item-911",
                "occurrence_start_ms":OCCURRENCE_MS,"span":"future"}}),
        ),
        ("privacy_stop", json!({})),
        ("capture_stop", json!({})),
        ("playback_stop", json!({})),
        ("rearm", json!({})),
        ("shutdown", json!({})),
        ("permissions", json!({"request":["microphone"]})),
        (
            "audio_start",
            json!({"stream_id":"stream-1","source":"remote_app","capture_scope":"system"}),
        ),
        (
            "audio_purpose",
            json!({"stream_id":"stream-1","purpose":"voice_note"}),
        ),
        ("audio_mute", json!({"stream_id":"stream-1","muted":true})),
        (
            "audio_wake_phrase",
            json!({"stream_id":"stream-1","enabled":true,"assistant_name":"Atlas",
                "assistant_names":["Atlas","Vega"]}),
        ),
        ("audio_stop", json!({"stream_id":"stream-1"})),
        (
            "playback",
            json!({"playback_id":"play-1","sequence":0,"sample_rate":32000,"channels":1,"pcm_s16le_base64":"AQI="}),
        ),
        (
            "playback_clip",
            json!({"playback_id":"play-1","mime_type":"audio/mpeg","provider":"fixture","audio_base64":"AQI="}),
        ),
        (
            "application_sources",
            json!({"bundle_ids":["com.example.meeting"]}),
        ),
        ("screen_sources", json!({})),
        ("screenshot", json!({"window_binding":binding})),
        ("selected_text", json!({"window_binding":binding})),
        ("assistant_folder_pick", json!({})),
        (
            "assistant_folders",
            json!({"folders":[{"id":FOLDER_ID,"bookmark_base64":"Ym9va21hcms="}],
                "path":"/Users/ejemplo/Proyectos/plan.md"}),
        ),
        (
            "assistant_files_list",
            json!({"folder":{"id":FOLDER_ID,"bookmark_base64":"Ym9va21hcms="},
                "path":"/Users/ejemplo/Proyectos","after_index":0,"max_entries":200}),
        ),
        (
            "assistant_files_search",
            json!({"roots":[{"folder":{"id":FOLDER_ID,"bookmark_base64":"Ym9va21hcms="},
                "path":"/Users/ejemplo/Proyectos"}],"query":"presupuesto","content":true,
                "limits":{"max_results":50,"max_visited":20000,"max_files_read":1000,
                    "max_bytes_read":33554432,"max_file_bytes":1048576,
                    "max_snippet_chars":160,"deadline_ms":10000}}),
        ),
        (
            "assistant_files_read",
            json!({"folder":{"id":FOLDER_ID,"bookmark_base64":"Ym9va21hcms="},
                "path":"/Users/ejemplo/Proyectos/plan.md",
                "read":{"as":"text","after_index":0,"max_bytes":48000}}),
        ),
        (
            "window_text",
            json!({"application_pid":4242,"window_id":42,
                "limits":{"max_bytes":131072,"max_nodes":2000,"deadline_ms":3000}}),
        ),
        (
            "photo_jpeg",
            json!({"source_path":"/Users/ejemplo/Fotos/IMG_0001.HEIC",
                "destination_path":"/Users/ejemplo/Library/Application Support/Orbit/attachments/convert-1/IMG_0001.jpg"}),
        ),
    ] {
        write(
            &root.join("commands"),
            name,
            &NativeCommand::from_parts(id.into(), name, fields)?,
        )?;
    }
    for (name, fields) in [
        (
            "ready",
            json!({"protocol":1,"platform":"macos","selected_text_available":true,
            "supported_audio_processing":["unprocessed","voice_processing"],
            "voice_processing_available":false,"playback_reference_available":false,
            "launch_mode":"managed_stdio"}),
        ),
        ("state", state.clone()),
        ("response", json!({"id":id,"ok":true,"result":{}})),
        (
            "error",
            json!({"code":"audio_route_start_failed",
            "os_domain":"NSOSStatusErrorDomain","os_status":-50}),
        ),
        (
            "hotkey",
            json!({"action":"meeting_toggle","phase":"pressed",
            "applications":[{"application_pid":4242,"bundle_id":"com.example.meeting","name":"Fixture"}]}),
        ),
        (
            "privacy_stop",
            json!({"reason":"local_hotkey","privacy_stopped":true,"drain_complete":true}),
        ),
        (
            "capture_stopped",
            json!({"reason":"session_inactive","privacy_stopped":false,"drain_complete":true}),
        ),
        ("local_rearm", json!({"privacy_stopped":false})),
        (
            "calendar_invalidated",
            json!({"code":"calendar_invalidated"}),
        ),
        (
            "gap",
            json!({"source":"mic_local","stream_id":"stream-1","reason":"audio_device_changed","frames":0,
                "duration_unknown":false,"duration_ns":2_000_000_000_u64}),
        ),
        (
            "audio",
            json!({"source":"mic_local","stream_id":"stream-1","sequence":0,
            "sample_rate":32000,"channels":1,"frames":1,"monotonic_ns":100,
            "clock_domain":"mach_absolute_time","pcm_s16le_base64":"AQI=",
            "processing":"unprocessed","processing_enabled":false,
            "processing_bypassed":true,"agc_enabled":false,
            "playback_reference":"none","pipeline_id":"fixture",
            "playback_active":true,"playback_id":"play-1"}),
        ),
        (
            "playback_progress",
            json!({"playback_id":"play-1","state":"started",
            "playback_active":true,"sequence":0}),
        ),
        (
            "microphone_activity",
            json!({"applications":[{"application_pid":4242,"bundle_id":"us.zoom.xos","name":"zoom.us"}]}),
        ),
        (
            "assistant_question",
            json!({"stream_id":"stream-1","text":"¿Qué dijo Lucía del presupuesto?",
            "started_ms":1_800_000_000_000_i64,"ended_ms":1_800_000_004_200_i64,
            "assistant_name":"Vega"}),
        ),
        ("voice_trigger", json!({"state":"downloading","percent":42})),
        (
            "notice",
            json!({"code":"wake_phrase_abandoned", "abandonment": {
                "cause":"phrase_revised", "elapsed_ms":1800, "silent_ms":0
            }}),
        ),
        ("helper_disconnected", json!({})),
    ] {
        let mut object = fields.as_object().ok_or("event fields not object")?.clone();
        if name != "helper_disconnected" {
            object.insert("v".into(), json!(1));
        }
        object.insert("event".into(), json!(name));
        let event: NativeEvent = serde_json::from_value(Value::Object(object))?;
        write(&root.join("events"), name, &event)?;
    }
    for (name, result) in [
        ("hello", state.clone()),
        ("heartbeat", state.clone()),
        ("status", {
            let mut value = state.clone();
            value["permissions"] = json!({"microphone":"authorized","screen":true,
                "accessibility":true,"calendars":"full_access"});
            value
        }),
        (
            "calendar_disconnect",
            json!({"authorization":"full_access","physical_read_pending":false}),
        ),
        (
            "calendar_sources",
            json!({"inventory_id":id,"native_instance_id":id,"privacy_generation":1,
            "clock_domain":"mach_absolute_time","observed_monotonic_ns":100,"completed_monotonic_ns":200,
            "calendars":[{"reference":{"calendar_id":"local-calendar","source_id":"local-source"},
                "title":"Trabajo","source_title":"iCloud","calendar_type":"caldav",
                "color":"#1BADF8","can_edit":true,"is_default":true}],
            "after_index":0,"total":1}),
        ),
        (
            "calendar_snapshot",
            json!({"read_id":id,"native_instance_id":id,"privacy_generation":1,
            "window":{"start_unix_ms":OCCURRENCE_MS,"end_unix_ms":OCCURRENCE_MS + 86_400_000},
            "clock_domain":"mach_absolute_time",
            "capture_started_monotonic_ns":100,"capture_completed_monotonic_ns":200,
            "observed_at_unix_ms":OCCURRENCE_MS,"system_timezone":"Europe/Madrid",
            "provider_sync_status":"unknown","enumeration":"bounded_local_read",
            "events":[full_occurrence(), invitation_occurrence()],"omitted":[],"after_index":0,"total":2}),
        ),
        (
            "calendar_save",
            json!({"write_id":id,"native_instance_id":id,"privacy_generation":1,
            "system_timezone":"Europe/Madrid","occurrence":full_occurrence()}),
        ),
        (
            "calendar_delete",
            json!({"write_id":id,"native_instance_id":id,"privacy_generation":1,
            "calendar_item_id":"item-911","occurrence_start_ms":OCCURRENCE_MS,"span":"future"}),
        ),
        ("privacy_stop", {
            let mut value = state.clone();
            value["privacy_stopped"] = json!(true);
            value["drain_complete"] = json!(true);
            value["drain"] = json!("complete");
            value
        }),
        ("capture_stop", {
            let mut value = state.clone();
            value["drain_complete"] = json!(true);
            value["drain"] = json!("complete");
            value
        }),
        ("rearm", state.clone()),
        (
            "permissions",
            json!({"microphone":"authorized","screen":true,"accessibility":true,"calendars":"full_access"}),
        ),
        (
            "audio_start",
            json!({"stream_id":"stream-1","source":"remote_app",
            "sample_rate":48000,"channels":1,"capture_scope":"system"}),
        ),
        (
            "audio_start-mic",
            json!({"stream_id":"stream-1","source":"mic_local","sample_rate":48000,
            "channels":1,"input_channels":9,"echo_control":"apple_voice_processing_unvalidated",
            "processing":"voice_processing",
            "processing_enabled":true,"processing_bypassed":false,"agc_enabled":false,
            "playback_reference":"shared_engine","pipeline_id":"fixture"}),
        ),
        ("audio_mute", json!({"stream_id":"stream-1","muted":true})),
        (
            "audio_stop",
            json!({"stream_id":"stream-1","drain_complete":true}),
        ),
        (
            "selected_text",
            json!({"binding":binding,"clock_domain":"mach_absolute_time",
                "capture_started_monotonic_ns":100,"capture_completed_monotonic_ns":200,
                "mime_type":"text/plain; charset=utf-8","source":"native_selected_text",
                "text":"texto sintético","range":{"location":0,"length":15,"unit":"utf16_code_units"},
                "ax_binding_quality":"unique_owner_title_bounds",
                "selection_scope":"application_focused_element","sensitivity":"not_certified"}),
        ),
        (
            "application_sources",
            json!({"native_instance_id":id,"privacy_generation":1,
            "clock_domain":"mach_absolute_time","observed_monotonic_ns":100,
            "completed_monotonic_ns":200,"applications":[{"application_pid":4242,
            "application_name":"Fixture","bundle_id":"com.example.meeting",
            "binding":application_binding,"unavailable_reason":null}],"omitted_count":0}),
        ),
        (
            "assistant_folder_pick",
            json!({"state":"chosen","path":"/Users/ejemplo/Proyectos",
                "bookmark_base64":"Ym9va21hcms="}),
        ),
        (
            "assistant_folders",
            json!({"folders":[{"state":"accessible","id":FOLDER_ID,
                    "path":"/Users/ejemplo/Proyectos"},
                {"state":"unavailable","id":"00000000-0000-4000-8000-000000000914",
                    "cause":{"kind":"bookmark_unresolvable","domain":"NSCocoaErrorDomain","code":4}}],
                "requested":{"state":"found","path":"/Users/ejemplo/Proyectos/plan.md",
                    "kind":"file","size_bytes":2048,"modified_ms":OCCURRENCE_MS}}),
        ),
        (
            "assistant_files_list",
            json!({"entries":[{"name":"informes","kind":"folder","modified_ms":OCCURRENCE_MS},
                {"name":"plan.md","kind":"file","size_bytes":2048,"modified_ms":OCCURRENCE_MS},
                {"name":"atajo","kind":"link"}],
                "total":3,"after_index":0}),
        ),
        (
            "assistant_files_search",
            json!({"results":[{"path":"/Users/ejemplo/Proyectos/plan.md","kind":"file",
                    "size_bytes":2048,"line":3,"snippet":"El presupuesto del trimestre."}],
                "visited":12,"files_read":4,"bytes_read":8192,"partial_files":0}),
        ),
        (
            "assistant_files_read",
            json!({"content":"text","text":"# Plan\nEl presupuesto del trimestre.\n",
                "after_index":0,"size_bytes":37}),
        ),
        (
            "window_text",
            json!({"text":"Lista de la compra\nPan\nAgua","stopped":"deadline"}),
        ),
    ] {
        write(&root.join("results"), name, &result)?;
    }
    calendar_variants(&root.join("calendar"), id)
}

/// The folder every assistant-folder fixture is about.
const FOLDER_ID: &str = "00000000-0000-4000-8000-000000000913";

/// Wednesday 14 October 2026, 10:00 UTC: the occurrence every calendar
/// fixture is about.
const OCCURRENCE_MS: i64 = 1_792_058_400_000;

fn create_draft() -> Value {
    json!({"title":"Revisión semanal","start_ms":OCCURRENCE_MS,"end_ms":OCCURRENCE_MS + 3_600_000,
        "all_day":false,"time_zone":"Europe/Madrid","location":"Sala 3",
        "notes":"Orden del día sintético.","url":"https://meet.example.test/abc",
        "availability":"busy",
        "recurrence":{"frequency":"weekly","interval":1,"days_of_week":[{"day":"wednesday"}],
            "end":{"count":10}},
        "alarms_minutes":[10,0]})
}

/// One event with every field a read carries, organized by the user.
fn full_occurrence() -> Value {
    json!({"observation_id":"00000000-0000-4000-8000-000000000912",
        "calendar":{"calendar_id":"local-calendar","source_id":"local-source"},
        "calendar_item_id":"item-911","event_id":"event-911",
        "original_occurrence_unix_ms":OCCURRENCE_MS,
        "start_unix_ms":OCCURRENCE_MS,"end_unix_ms":OCCURRENCE_MS + 3_600_000,
        "event_timezone":"Europe/Madrid","interpreted_timezone":"Europe/Madrid",
        "is_floating":false,"is_all_day":false,"is_detached":false,
        "title":"Revisión semanal","status":"confirmed","last_modified_unix_ms":OCCURRENCE_MS - 86_400_000,
        "invitees":[{"name":"Ada","participant_url":"mailto:ada@example.test",
            "email":"ada@example.test","role":"required","response_status":"accepted",
            "participant_type":"person","is_current_user":false}],
        "invitees_status":"provided",
        "event_url":"https://meet.example.test/abc","event_url_status":"provided",
        "location":"Sala 3","notes":"Orden del día sintético.",
        "organizer":{"name":"Alina","participant_url":"mailto:alina@example.test",
            "email":"alina@example.test","role":"chair","response_status":"accepted",
            "participant_type":"person","is_current_user":true},
        "availability":"busy",
        "recurrence":{"frequency":"weekly","interval":1,"days_of_week":[{"day":"wednesday"}],
            "end":{"count":10}},
        "can_edit":true,"alarms_minutes":[0,10]})
}

/// An invitation someone else organized, whose notes are too long to travel
/// and whose series holds two rules: the fields that say what did not arrive.
fn invitation_occurrence() -> Value {
    json!({"observation_id":"00000000-0000-4000-8000-000000000913",
        "calendar":{"calendar_id":"local-calendar","source_id":"local-source"},
        "calendar_item_id":"item-913",
        "original_occurrence_unix_ms":OCCURRENCE_MS + 7_200_000,
        "start_unix_ms":OCCURRENCE_MS + 7_200_000,"end_unix_ms":OCCURRENCE_MS + 9_000_000,
        "interpreted_timezone":"Europe/Madrid","is_floating":true,"is_all_day":false,
        "is_detached":true,"title":"Comité","status":"tentative",
        "invitees_status":"not_provided","event_url_status":"not_provided",
        "notes_left_out":{"bytes":40_000,"limit":32_768},
        "organizer":{"participant_url":"mailto:comite@example.test",
            "email":"comite@example.test","role":"chair","response_status":"accepted",
            "participant_type":"person","is_current_user":false},
        "availability":"tentative",
        "recurrence":{"frequency":"monthly","interval":2,
            "days_of_week":[{"day":"friday","week":-1}],
            "end":{"until_ms":OCCURRENCE_MS + 31_536_000_000},"rules_count":2},
        "can_edit":false,"alarms_minutes":[15],"alarms_not_editable":1})
}

/// Every write variant besides the ones in `commands/` and `results/`: the
/// change of an occurrence, and one refusal per code, as the helper's
/// `response` event with its detail.
fn calendar_variants(root: &Path, id: &str) -> Result<(), Box<dyn Error>> {
    write(
        &root.join("commands"),
        "calendar_save-change",
        &NativeCommand::from_parts(
            id.into(),
            "calendar_save",
            json!({"calendar_save":{"change":{"calendar_item_id":"item-911",
                "occurrence_start_ms":OCCURRENCE_MS,"span":"this",
                "changes":{"start_ms":OCCURRENCE_MS + 3_600_000,
                    "end_ms":OCCURRENCE_MS + 7_200_000,"location":""}}}}),
        )?,
    )?;
    for (code, message, detail) in [
        (
            "calendar_event_not_found",
            "No hay ninguna ocurrencia de ese evento en esa fecha.",
            json!({"calendar_item_id":"item-911","occurrence_start_ms":OCCURRENCE_MS}),
        ),
        (
            "calendar_not_editable",
            "Este evento lo organizó otra persona.",
            json!({"reason":"organized_by_someone_else","calendar_id":"local-calendar"}),
        ),
        (
            "calendar_invalid_change",
            "El título es demasiado largo.",
            json!({"field":"title","problem":"too_large","bytes":100_000,"limit":98_304}),
        ),
        (
            "calendar_permission_denied",
            "macOS no da acceso completo a Calendarios.",
            json!({"authorization":"write_only"}),
        ),
        (
            "calendar_save_failed",
            "EventKit rechazó el cambio.",
            json!({"error_domain":"EKErrorDomain","error_code":12}),
        ),
        (
            "calendar_write_outcome_unknown",
            "No se sabe si el cambio llegó al calendario.",
            json!({"cause":"write_deadline"}),
        ),
    ] {
        let event: NativeEvent = serde_json::from_value(json!({"v":1,"event":"response",
            "id":id,"ok":false,"error":{"code":code,"message":message,"detail":detail}}))?;
        write(&root.join("refusals"), code, &event)?;
    }
    Ok(())
}
