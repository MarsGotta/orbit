use orbit_core::features::FeatureId;
use serde_json::{Value, json};
pub fn cases() -> Vec<(Value, FeatureId)> {
    vec![
        (
            json!({"command":"calendar","request":{"action":"read","window":{"start_unix_ms":1792058400000_i64,"end_unix_ms":1792060200000_i64}}}),
            FeatureId::Agenda,
        ),
        (
            json!({"command":"error_reports","enabled":true}),
            FeatureId::Status,
        ),
        (json!({"command":"calendar_show_today"}), FeatureId::Agenda),
        (json!({"command":"privacy_stop"}), FeatureId::Status),
        (json!({"command":"rearm"}), FeatureId::Status),
        (
            json!({"command":"pair","hub_url":"https://example.invalid","code":"1234","device_name":"Mac"}),
            FeatureId::Status,
        ),
        (
            json!({"command":"open_url","url":"https://example.invalid"}),
            FeatureId::Status,
        ),
        (json!({"command":"quit"}), FeatureId::Status),
        (json!({"command":"restart"}), FeatureId::Status),
        (
            json!({"command":"permissions","request":[orbit_ipc::permissions::MICROPHONE]}),
            FeatureId::Permissions,
        ),
        (
            json!({"command":"open_system_privacy","pane":"Microphone"}),
            FeatureId::Permissions,
        ),
        (
            json!({"command":"calendar","request":{"action":"request_permission"}}),
            FeatureId::Agenda,
        ),
        (
            json!({"command":"calendar_set_range","view":"week","anchor_ms":1790000000000_i64,"days":7,"week_start":"monday"}),
            FeatureId::Agenda,
        ),
        (json!({"command":"calendar_refresh"}), FeatureId::Agenda),
        (
            json!({"command":"calendar","request":{"action":"save","request_id":"create-1",
                    "save":{"create":{"title":"Café","start_ms":1792058400000_i64,
                        "end_ms":1792060200000_i64,"all_day":false}}}}),
            FeatureId::Agenda,
        ),
        (
            json!({"command":"calendar","request":{"action":"save","request_id":"move-1","save":{"change":{
                    "calendar_item_id":"item-1","occurrence_start_ms":1792058400000_i64,
                    "span":"future","changes":{"start_ms":1792062000000_i64}}}}}),
            FeatureId::Agenda,
        ),
        (
            json!({"command":"calendar","request":{"action":"delete","request_id":"delete-1","delete":{
                    "calendar_item_id":"item-1","occurrence_start_ms":1792058400000_i64,
                    "span":"this"}}}),
            FeatureId::Agenda,
        ),
        (json!({"command":"notes_refresh"}), FeatureId::Notes),
        (json!({"command":"notes_more"}), FeatureId::Notes),
        (
            json!({"command":"note_library","request":{"action":"dictionary","request":{"action":"save","dictionary_id":"00000000-0000-4000-8000-000000000001","name":"Proyecto"}}}),
            FeatureId::Notes,
        ),
        (
            json!({"command":"note_library","request":{"action":"save_template","template_id":"0b1c2d3e-4f50-4617-8293-a4b5c6d7e8f9",
                    "draft":{"name":"Comité","context":"Inversiones",
                    "sections":[{"name":"Propuestas","instructions":"Con importe"}],
                    "forked_from":"project"}}}),
            FeatureId::Notes,
        ),
        (
            json!({"command":"note_library","request":{"action":"delete_template","template_id":"mia"}}),
            FeatureId::Notes,
        ),
        (
            json!({"command":"note_library","request":{"action":"restore_template","request_id":"deshacer-1","template_id":"mia",
                    "draft":{"name":"Comité","context":"Inversiones",
                    "sections":[{"name":"Propuestas","instructions":"Con importe"}],
                    "forked_from":"project"},"series":["serie-1"]}}),
            FeatureId::Notes,
        ),
        (
            json!({"command":"note_library","request":{"action":"assign","calendar_item_id":"serie-1","template_id":null}}),
            FeatureId::Notes,
        ),
        (
            json!({"command":"note_library","request":{"action":"use_for_series","recording_id":"recording-1",
                    "template_id":"project","dictionary_id":null}}),
            FeatureId::Notes,
        ),
        (
            json!({"command":"notes_open","recording_id":"recording-1"}),
            FeatureId::Note,
        ),
        (
            json!({"command":"speaker_rename","speaker_key":"mic_local","name":"Ada","scope":{"kind":"voice"}}),
            FeatureId::Note,
        ),
        (
            json!({"command":"speaker_rename","speaker_key":"remote_app","name":null,"scope":{"kind":"voice"}}),
            FeatureId::Note,
        ),
        (
            json!({"command":"transcript_retry","recording_id":"recording-1"}),
            FeatureId::Note,
        ),
        (
            json!({"command":"transcript_retranscribe","recording_id":"recording-1"}),
            FeatureId::Note,
        ),
        (
            json!({"command":"note_private_save","recording_id":"recording-1","markdown":"- a"}),
            FeatureId::Note,
        ),
        (
            json!({"command":"note_edit","recording_id":"recording-1",
                    "sections":[{"name":"Resumen","markdown":"- b"}]}),
            FeatureId::Note,
        ),
        (
            json!({"command":"note_action_toggle","recording_id":"recording-1",
                    "action_id":"a0123456789ab","done":true}),
            FeatureId::Note,
        ),
        (
            json!({"command":"note_regenerate","recording_id":"recording-1","template_id":"general"}),
            FeatureId::Note,
        ),
        (
            json!({"command":"note_restore_superseded","recording_id":"recording-1"}),
            FeatureId::Note,
        ),
        (
            json!({"command":"ask_orbit_about","recording_id":"recording-1"}),
            FeatureId::Note,
        ),
        (
            json!({"command":"meeting_language","recording_id":"recording-1","language":"en"}),
            FeatureId::Note,
        ),
        (
            json!({"command":"meeting_rename","recording_id":"recording-1","title":"Presupuesto"}),
            FeatureId::Note,
        ),
        (
            json!({"command":"meeting_delete","recording_id":"recording-1"}),
            FeatureId::Note,
        ),
        (
            json!({"command":"meeting_start","title":"Daily","audio":null}),
            FeatureId::Recording,
        ),
        (
            json!({"command":"meeting_ask","question":"¿Qué dijo?","minutes":5}),
            FeatureId::Recording,
        ),
        (
            json!({"command":"meeting_question_close","id":"turn-1"}),
            FeatureId::Recording,
        ),
        (
            json!({"command":"meeting_question_hold","id":"turn-1","held":true}),
            FeatureId::Recording,
        ),
        (
            json!({"command":"reminder_read","series_key":"revisión semanal"}),
            FeatureId::Notes,
        ),
        (
            json!({"command":"reminder_save","series_key":"revisión semanal",
                "text":"Reservar la sala","edited_at_ms":7}),
            FeatureId::Notes,
        ),
        (
            json!({"command":"reminder_write","key": "ab".repeat(32),
                "text":"Reservar la sala","edited_at_ms":7}),
            FeatureId::Notes,
        ),
        (json!({"command":"meeting_pause"}), FeatureId::Recording),
        (json!({"command":"meeting_resume"}), FeatureId::Recording),
        (json!({"command":"meeting_finish"}), FeatureId::Recording),
        (
            json!({"command":"meeting_discard","recording_id":"recording-1"}),
            FeatureId::Recording,
        ),
        (
            json!({"command":"microphone_mute","muted":true}),
            FeatureId::Recording,
        ),
        (
            json!({"command":"call_record","call_id":"call-1"}),
            FeatureId::Recording,
        ),
        (
            json!({"command":"call_dismiss","call_id":"call-1"}),
            FeatureId::Recording,
        ),
        (
            json!({"command":"recording_keep","recording_id":"recording-1"}),
            FeatureId::Recording,
        ),
        (
            json!({"command":"call_preference","preference":"nothing"}),
            FeatureId::Recording,
        ),
        (
            json!({"command":"audio_sources_refresh"}),
            FeatureId::Recording,
        ),
        (
            json!({"command":"assistant_folder_add"}),
            FeatureId::Control,
        ),
        (
            json!({"command":"assistant_folder_remove","id":"00000000-0000-4000-8000-000000000913"}),
            FeatureId::Control,
        ),
        (
            json!({"command":"chat","text":"Hola"}),
            FeatureId::Assistant,
        ),
        (json!({"command":"new_session"}), FeatureId::Assistant),
        (
            json!({"command":"handover","session_id":"30000000-0000-4000-8000-000000000005",
                   "agent_id":"deimos","text":"¿Y mañana?"}),
            FeatureId::Assistant,
        ),
        (json!({"command":"interrupt"}), FeatureId::Assistant),
        (
            json!({"command":"approval","approved":true}),
            FeatureId::Assistant,
        ),
        (
            json!({"command":"clarify","answer":"Sí"}),
            FeatureId::Assistant,
        ),
        (json!({"command":"voice_start"}), FeatureId::Assistant),
        (json!({"command":"voice_end"}), FeatureId::Assistant),
        (json!({"command":"voice_cancel"}), FeatureId::Assistant),
        (
            json!({"command":"voice_retry","message_id":"message-1"}),
            FeatureId::Assistant,
        ),
        (
            json!({"command":"voice_discard","message_id":"message-1"}),
            FeatureId::Assistant,
        ),
        (
            json!({"command":"speak","message_id":"answer-1","text":"Texto"}),
            FeatureId::Assistant,
        ),
        (json!({"command":"speak_stop"}), FeatureId::Assistant),
        (
            json!({"command":"voice_note_audio","message_id":"message-1"}),
            FeatureId::Assistant,
        ),
        (
            json!({"command":"call_start","duration_minutes":10}),
            FeatureId::Assistant,
        ),
        (json!({"command":"call_stop"}), FeatureId::Assistant),
        (
            json!({"command":"session_open","id":"session-1"}),
            FeatureId::Assistant,
        ),
        (json!({"command":"login_item","on":true}), FeatureId::Local),
        (json!({"command":"update_check"}), FeatureId::Local),
        (json!({"command":"update_install"}), FeatureId::Local),
        (json!({"command":"onboarding_done"}), FeatureId::Local),
        (
            json!({"command":"profile_set","name":"Alina"}),
            FeatureId::Local,
        ),
    ]
}
