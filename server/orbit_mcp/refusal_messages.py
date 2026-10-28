"""The sentences a refusal is said with: one per code, per permission, per
engine cause, and the words for the numbers that follow them."""

# Every message names a control Orbit actually has today: a message that sends
# the user to a switch or a step that no longer exists is worse than a generic one.
from .notes_refusals import MESSAGES as NOTES_MESSAGES

ERROR_MESSAGES = {
    **NOTES_MESSAGES,
    "encode_failed": "Orbit could not encode the response to that read.",
    "helper_reply_invalid": "The helper response violates the contract; the field and expected shape are provided. Its contents remain unknown.",
    "read_interrupted": "The read was interrupted for the stated cause; its contents remain unknown.",
    "helper_refused": "The helper refused the read for the stated cause.",
    "internal": "Orbit could not complete the operation because of an identified internal failure.",
    "protocol_version": "The received protocol version does not match the required version.",
    "hub_reply_invalid": "The hub response violates the contract; it is not treated as an empty list.",
    "daemon_reply_invalid": "The Orbit service response violates the contract; it is not treated as an empty list.",
    "attachment": "A message attachment could not be used; the error message names the file, cause and numbers.",
    "calendar_read_limit": "The helper could not deliver a calendar read within the stated limits; nothing was trimmed.",
    "capture_clock_unavailable": "The helper could not verify the capture clock. No read was delivered; request it again.",
    "invalid_screenshot_dimensions": "The screenshot has invalid dimensions and was not delivered. Request it again if still needed.",
    "invalid_window_dimensions": "The window has invalid capture dimensions. List the windows again and choose another.",
    "recovering": "Orbit is recovering its state on that Mac. Wait for it to finish, then request the read again.",
    "screen_inventory_busy": "The helper is still listing windows for another request. Wait for it to finish before requesting them again.",
    "screen_inventory_too_large": "The window inventory exceeds the size the helper can deliver. No partial list was delivered.",
    "screen_permission_required": "macOS has not granted Orbit Screen Recording on that Mac. Grant it in System Settings › Privacy & Security › Screen Recording.",
    "screenshot_sdk_failed": "ScreenCaptureKit could not obtain that window's image. Request the screenshot again if still needed.",
    "screenshot_too_large": "The helper could not encode the PNG within the supported screenshot size; the image was not delivered.",
    "window_identity_changed": "The window changed between being listed and read. Request the window or screenshot again.",
    "invalid_screen_inventory": "The window inventory violates the helper contract. List the windows again.",
    "invalid_window_binding": "The requested window identity is invalid. List the windows again and choose one.",
    "screen_selection_unavailable": "The Mac cannot select that window now. List the windows again.",
    "screen_selection_expired": "The window selection expired. List the windows again before capturing.",
    "screenshot_busy": "The helper is finishing another screenshot. Wait for it to finish before requesting another.",
    "screenshot_expired": "The screenshot did not finish within the helper's deadline. Request it again if still needed.",
    "selected_text_changed": "The text selection changed while being read. Request the selected text again.",
    "selected_text_classification_incomplete": "The Mac could not verify that the selected field is safe. Its text was not read.",
    "selected_text_expired": "The selected-text read expired. Request it again if still needed.",
    "selected_text_identity_changed": "The window or its selection changed during the read. Request the selected text again.",
    "selected_text_invalid_range": "The selected-text range is invalid. Request the selection again.",
    "selected_text_invalid_text": "Accessibility returned invalid text; it was not used. Request it again.",
    "selected_text_limit": "The selection exceeds the helper's read limit. Request a shorter selection.",
    "selected_text_secure_field": "The selection is in a protected field, which Orbit does not read. Request another field.",
    "selected_text_empty": "Accessibility confirms an empty selection range: no text is selected in that field.",
    "selected_text_not_exposed": "The application does not expose the Accessibility attributes needed to read the selection; whether text is selected is unknown.",
    "selected_text_unavailable": "Accessibility could not deliver that window's selection; whether text is selected is unknown.",
    "selected_text_window_ambiguous": "Accessibility could not distinguish the requested window from others. List the windows again and select an unambiguous one.",
    "calendar_invalidated": "The calendar changed during the read. Request the agenda again; its contents remain unknown.",
    "calendar_selection_unavailable": "The requested calendar is unavailable for this read. Its contents remain unknown.",
    "invalid_calendar_request": "The helper refused the calendar-read parameters. Correct them before requesting it again.",
    "expired": "The requested resource expired on the Mac.",
    "helper_not_running": "The Orbit helper is not running on that Mac.",
    "hub_unreachable": "That Mac cannot reach the Orbit hub.",
    "hub_sessions_unpaged": "The Orbit hub delivers only some of that Mac's conversations; update the hub to read the rest.",
    "hub_session_invalid_row": "A conversation in the hub list has an invalid row; update the hub to correct it.",
    "hub_sessions_cursor_mismatch": "The conversation cursor does not match the last row the hub delivered; update the hub.",
    "invalid_parameter": "The request contains an invalid parameter; check the expected value.",
    "legacy": "The Mac's cause uses a code from before the refusal registry.",
    "mac_disconnected": "That Mac disconnected from the Orbit hub.",
    "not_found": "The requested resource does not exist on that Mac.",
    "not_paired": "That Mac is not paired with the hub.",
    "outcome_unknown": "The action may have been applied; check its result before repeating it.",
    "permission_denied": "macOS denies the stated permission for this request.",
    "permission_unknown": "The Mac does not yet know the state of the stated permission.",
    "permission_not_granted": "macOS has not granted Orbit the stated permission on that Mac. It does not say whether it was denied or has not been decided, but Orbit cannot do this without it. Grant it in System Settings › Privacy & Security.",
    "revision_changed": "The note changed on the Mac while being read or corrected; request it again from its current revision.",
    "storage_failed": "Storage failed during the stated operation.",
    "timeout": "The operation reached its deadline on the Mac; check its result.",
    "work_in_flight": "Another local operation is in flight; wait for it to finish or close the view. Check the result before repeating a change.",
    "physical_work_finishing": "The previous, already closed operation is still finishing on disk; wait for it to finish. Check the result before repeating a change.",
    "work_expired": "The result expired when the Mac's authority changed; request the operation again if still needed.",
    "update": "The Orbit update on that Mac did not finish; the cause names the failed step and its numbers.",
    "too_large": "The resource exceeds the size the link supports.",
    "too_many": "The request contains more items than allowed.",
    "upstream_failed": "The service this operation depends on failed.",
    "transcription_pending": "That meeting's transcript is not ready yet. There is no text to deliver yet.",
    "transcription_failed": "That meeting's transcription failed. It will not retry automatically; the original audio is retained. Open the note in Orbit and click Retry when the service works.",
    "native_command_invalid": "Orbit could not construct a native command before sending it to the Mac; the helper did not refuse anything.",
    "cancelled": "The Mac cancelled the request before completing it; it delivered no result.",
    "capture_missing": "The screenshot sent by the Mac was unavailable when the hub prepared the response.",
    "connection_lost": "The connection to the Mac was lost during the action; the outcome is uncertain. Do not repeat the input without checking its effect.",
    "device_reply_invalid": "The Mac replied outside the requested capability's contract. Check the field, expected value and Mac version before requesting the read again; for an action, check its effect and do not repeat it.",
    "device_reply_timeout": "The Mac did not confirm the action within the stated deadline. The outcome is uncertain: check its effect and do not repeat it.",
    "hub_response_without_code": "The hub returned an error without a code; check the hub version and state.",
    "internal_error": "Orbit encountered an internal failure while preparing the response.",
    "device_unavailable": "The Mac is not connected to Orbit. Check that Orbit is open and connected.",
    # What lets Deimos see and act on a Mac is the macOS permission the user
    # grants, and nothing else. Each code names the one that is missing.
    "accessibility_permission_required": "macOS has not granted Orbit Accessibility on that Mac, so it cannot use the computer. Grant it in System Settings › Privacy & Security › Accessibility.",
    "screen_recording_permission_required": "macOS has not granted Orbit Screen Recording on that Mac, so it cannot see the screen. Grant it in System Settings › Privacy & Security › Screen Recording.",
    "computer_permissions_required": "macOS has granted Orbit neither Accessibility nor Screen Recording on that Mac, so it cannot see or use the computer. Grant them in System Settings › Privacy & Security.",
    "engine_unavailable": "The control engine is unavailable on that Mac. Check its state before retrying.",
    "no_answer": "The Mac did not answer in time. Check its connection before retrying.",
    "catalogue_changed": "The engine catalogue changed during the read. Request the tool list again.",
    "unknown_capability": "The Mac does not recognize that capability. Update Orbit before retrying.",
    # These three codes arrived from the Mac's own Refusals but had no entry
    # here, so Deimos read a message without a cause. Each one now says what
    # stands between the request and the Mac.
    "privacy_stopped": "Everything is stopped on that Mac through its own controls: Orbit cannot see or act on it until it is reactivated there, on the Mac itself. This is not a Mac or connection error; ask again once reactivated.",
    "busy": "That Mac is already handling another request. This is not a refusal: wait for it to finish, then request it again.",
    "pending_recording": "That Mac already has a pending recording. Finish it or wait for it to close before starting another capture or call.",
    "hub_unavailable": "The Orbit hub is unavailable. Check the service before retrying.",
    "invalid_response": "Orbit returned an invalid response. Check the hub and Mac versions.",
    "invalid_parameters": "The parameters are invalid. Correct their types and limits before retrying.",
    "operation_not_found": "That operation was not found on the selected Mac. Check the identifier and device.",
    "meetings_not_configured": "The private meetings key is missing from the Orbit MCP configuration.",
    # Settings › Agents: what each agent may use. Not macOS's permission and
    # not the Mac's refusal; the user changes it in Orbit's settings.
    "agent_access_disabled": "This agent has the access group this tool belongs to disabled in Settings › Agents, so it cannot use it. This is neither a macOS permission nor a Mac failure.",
    "agent_not_found": "The hub has no agent matching the one this translator was started for (ORBIT_MCP_AGENT). Check that agent's configuration.",
    # orbit_skill: the agent's own skills, which the hub keeps.
    "skill_not_enabled": "This agent has no skill with that name enabled in Orbit. Use a name from your skills list.",
    "skills_not_in_hub": "Orbit does not store this agent's skills: they belong to its Hermes profile and are loaded with its own tools.",
    # A recording and its note, read from the hub. Each state of a note
    # that is not there has its own code; none of them means an empty meeting.
    "recording_not_found": "That recording does not exist in the hub: neither its audio nor its closure has arrived. Check the identifier with orbit_meetings.",
    "note_not_requested": "Nobody has requested that recording's note from the Mac yet, so it does not exist. This does not mean the meeting has no content: read its transcript with orbit_transcript.",
    "note_not_configured": "The hub has no model configured to write notes, so that recording's note has not been written. This is not a meeting failure: read its transcript with orbit_transcript.",
    "note_waiting_transcript": "That recording's note is waiting for its transcription to finish; there is no note to read yet. Read what is transcribed so far with orbit_transcript.",
    "note_generating": "The model is writing that recording's note now; there is no note to read yet. Request it again in a couple of minutes.",
    "note_queued": "That recording's note failed for a transient cause and the hub will retry automatically; there is no note to read yet.",
    "note_failed": "That recording's note could not be written and the hub will not retry automatically. Request it again from the note in Orbit once the cause is resolved.",
    "attachment_not_found": "That attachment is not in the hub for that Mac, or does not belong to this agent's conversation. Use the id from the message that attached it.",
    "after_index_out_of_range": "after_index is past the end of the requested text; start again without after_index or with a smaller value.",
    # Orbit no longer keeps a copy of the agenda anywhere, so every code
    # that described one is gone with it. What is left are the conditions that
    # really stand between Deimos and the user's agenda — the macOS permission, the
    # Mac being reachable, its reader — and each one says which it is. None of
    # them may read as “nothing that day”: that is an ordinary answer.
    "calendar_permission_required": "Orbit does not have full read access to your Calendars in macOS, so it cannot read the agenda. This does NOT mean the agenda is empty. Grant it in System Settings › Privacy & Security › Calendars.",
    # Unknown is its own state. The Mac has not said yet whether it holds
    # the permission — it has just started, or its reader is reconnecting — and
    # saying “does not have access” or pretending it does would both be untrue.
    "calendar_permission_unknown": "Orbit does not yet know whether it has permission to read that Mac's Calendars: the Mac has not reported it yet, because it just started or its reader is reconnecting. This is not a denial and does not mean the agenda is empty. Ask again in a few seconds.",
    "calendar_backend_unavailable": "EventKit is unavailable on that Mac right now, so Orbit cannot read its agenda. This does NOT mean the agenda is empty. Check Calendars in macOS and ask again when EventKit is available.",
    "calendar_helper_stopped": "The Orbit helper is stopped on that Mac; the agenda remains unknown. This does NOT mean it is empty.",
    "calendar_read_restarted": "The Mac lost the agenda read midway because its reader restarted, and Orbit started it over without finishing. Ask again in a few seconds.",
    "capture_stopped": "The Mac stopped capture while reading the agenda (for example, because the lid closed or the session became inactive). This is not a privacy stop and the read can be retried.",
    "calendar_changed": "The agenda changed several times while Orbit read it, so there is no consistent read to deliver. Ask again in a few seconds.",
    # What EventKit itself could not do. The Mac's own sentence follows each
    # one with its numbers; none of them is an empty agenda.
    "calendar_event_too_large": "An agenda page exceeds the largest message its reader can deliver even after explicitly leaving out events that do not fit, because the list of those events is too long. Nothing was trimmed. This does NOT mean the agenda is empty; request a shorter range.",
    "calendar_limit_exceeded": "An event or calendar has a field longer than Orbit reads, and the read stopped instead of trimming it. This does NOT mean the agenda is empty.",
    "calendar_read_expired": "EventKit did not finish reading the agenda within the read deadline, even for the shortest range Orbit requests. Ask again in a few seconds.",
    "calendar_generation_changed": "Orbit's generation changed during the agenda read, so the previous response was not used. This does NOT mean the agenda is empty.",
    "calendar_reply_replaced": "The response belonged to another agenda read and Orbit did not use it. This does NOT mean the agenda is empty.",
    "calendar_clock_unavailable": "Orbit could not verify the clock for that agenda read. This does NOT mean the agenda is empty.",
    "generation_changed": "Orbit's generation changed during the source read; that previous response was not used.",
    "conversation_changed": "The conversation changed during the source read; that previous response was not used.",
    "helper_restarted": "The helper restarted during the source read; the previous request has no valid response.",
    "source_reply_replaced": "The response belongs to another source read and was not used.",
    "native_refused": "The helper refused the source read; there is no usable result.",
    "calendar_read_failed": "EventKit returned an error reading the agenda on the Mac. This does NOT mean the agenda is empty.",
    "calendar_busy": "EventKit was still finishing a previous agenda read on that Mac and Orbit does not interrupt it. Ask again in a few seconds.",
    "unknown_device": "That Mac was not found in Orbit. Check its name with orbit_devices.",
    "config_invalid": "The Orbit MCP configuration changed and is no longer valid. Correct mcp.toml.",
    # The assistant's folders: each says its path and folders in
    # `files.refusal_message`; these are the sentences without them.
    "no_folders": "The Orbit user has not given the assistant any folders on that Mac.",
    "outside_folders": "That path is outside the folders the assistant can read.",
    "folder_unavailable": "That assistant folder cannot be opened now.",
    "file_unavailable": "That file in the assistant's folders cannot be read as requested.",
    "file_too_large": "That file is larger than Orbit reads for its type.",
    # A folder request that ended without an answer: nothing to check after
    # it, and no «source» in it.
    "files_timeout": "The Mac did not finish reading the assistant's folders in time and delivered nothing. Request it again.",
    "files_helper_restarted": "The Orbit helper restarted while reading the assistant's folders and delivered nothing. Request it again.",
}

TYPED_PERMISSION_MESSAGES = {
    ("permission_denied", "calendar"): ERROR_MESSAGES["calendar_permission_required"],
    ("permission_unknown", "calendar"): ERROR_MESSAGES["calendar_permission_unknown"],
    ("permission_denied", "screen_recording"): ERROR_MESSAGES[
        "screen_recording_permission_required"
    ],
    ("permission_denied", "accessibility"): ERROR_MESSAGES["accessibility_permission_required"],
    ("permission_denied", "screen_and_accessibility"): ERROR_MESSAGES[
        "computer_permissions_required"
    ],
    # A `false` from the helper is a known fact — not granted — but macOS does
    # not say whether the user denied it or never decided.
    ("permission_not_granted", "calendar"): ERROR_MESSAGES["calendar_permission_required"],
    ("permission_not_granted", "screen_recording"): ERROR_MESSAGES[
        "screen_recording_permission_required"
    ],
    ("permission_not_granted", "accessibility"): ERROR_MESSAGES[
        "accessibility_permission_required"
    ],
    ("permission_not_granted", "screen_and_accessibility"): ERROR_MESSAGES[
        "computer_permissions_required"
    ],
    (
        "permission_unknown",
        "screen_recording",
    ): "Orbit does not yet know whether macOS grants it Screen Recording on that Mac. This is not a denial; ask again when the helper reports its state.",
    (
        "permission_unknown",
        "accessibility",
    ): "Orbit does not yet know whether macOS grants it Accessibility on that Mac. This is not a denial; ask again when the helper reports its state.",
    (
        "permission_unknown",
        "screen_and_accessibility",
    ): "Orbit does not yet know whether macOS grants it Screen Recording and Accessibility on that Mac. This is not a denial; ask again when the helper reports its state.",
}

ENGINE_CAUSE_MESSAGES = {
    "not_installed": "The control engine is not installed on that Mac.",
    "not_running": "The control engine is not running on that Mac.",
    "unhealthy": "The control engine stopped responding on that Mac; check its state.",
    "unsupported": "This Orbit version does not support that engine or function on the Mac; update Orbit.",
    "voice_processing_unsupported": "The call needs VoiceProcessing; this Mac does not support the requested voice processing.",
    "initialization_failed": "The control engine could not start on that Mac; check its state.",
}

# The refusal's fields that are said, with the words they are said with, in
# the order they are said.
FIELD_LABELS = {
    "max_page_size": "maximum entries per page",
    "max_page": "maximum page",
    "entries": "entries",
    "max_entries": "maximum entries",
    "max_depth": "maximum depth",
    "upstream_code": "Fast Note Sync code",
    "upstream_status": "Fast Note Sync HTTP status",
    "timeout_seconds": "deadline in seconds",
    "resource": "resource",
    "bytes": "size in bytes",
    "chars": "characters",
    "max_chars": "maximum characters",
    "start_ms": "start in ms",
    "end_ms": "end in ms",
    "max_bytes": "maximum bytes",
    "count": "items",
    "delivered": "conversations delivered",
    "total": "total conversations",
    "position": "position",
    "field": "field",
    "rows": "rows on this page",
    "max": "maximum",
    "in_flight": "in flight",
    "started_generation": "initial generation",
    "current_generation": "current generation",
    "waited_ms": "waited ms",
    "deadline_ms": "deadline ms",
    "retry_after_ms": "suggested wait in ms",
    "since_ms": "since ms",
    "last_seen_ms": "last connection in ms",
    "expired_at_ms": "expired at ms",
    "status": "HTTP status",
    "permission": "permission",
    "operation": "operation",
    "service": "service",
    "cause": "cause",
    "parameter": "parameter",
    "expected": "expected value",
    "received_keys": "received keys",
    "received": "received",
    "required": "required",
    "received_bytes": "received bytes",
    "helper_code": "helper cause",
    "issue": "response issue",
    "interruption": "interruption reason",
    "step": "recovery step",
    "elapsed_ms": "elapsed ms",
    "received_type": "received type",
    "device_version": "Mac version",
    "operation_id": "operation identifier",
    "retry_in_ms": "retry in ms",
    "detail": "detail",
    "expected_revision": "requested revision",
    "actual_revision": "current revision",
    "expected_state_sha256": "requested state",
    "actual_state_sha256": "current state",
    "id": "recording identifier",
    "recording_id": "recording identifier",
    "title": "title",
    "phase": "phase",
    "sealed_chunks": "stored chunks",
    "gaps": "gaps",
    "expires_at_ms": "expires at ms",
    "agent_id": "agent id",
    "group": "access group",
    "agent_ids": "hub agents",
}
