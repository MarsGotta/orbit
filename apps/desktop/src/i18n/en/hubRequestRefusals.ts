import type { HubRequestCode } from "../es/hubRequestRefusals";
import { formatQuantity } from "../locale";
import type { Numbers } from "./refusalNumbers";

export const hubRequestRefusals: Record<HubRequestCode, (n: Numbers) => string | undefined> = {
  not_found: () => "Unknown device",
  note_not_ready: () => "The note has not been written yet",
  note_not_requested: () => "Nobody has requested this recording's note yet",
  note_template_builtin: (n) =>
    n.template_id === undefined
      ? undefined
      : `Template «${formatQuantity(n.template_id)}» belongs to Orbit and cannot be changed or deleted: save the changes as your own template with forked_from`,
  note_template_links_full: (n) =>
    n.links === undefined || n.max_links === undefined
      ? undefined
      : `There are already ${formatQuantity(n.links)} series with assigned templates and the maximum is ${formatQuantity(n.max_links)}: remove one before assigning another`,
  note_template_not_found: (n) =>
    n.template_id === undefined
      ? undefined
      : `Template not found: «${formatQuantity(n.template_id)}»`,
  note_templates_full: (n) =>
    n.max_templates === undefined || n.templates === undefined
      ? undefined
      : `There are already ${formatQuantity(n.templates)} custom templates and the maximum is ${formatQuantity(n.max_templates)}: delete one before creating another`,
  note_templates_too_large: (n) =>
    n.bytes === undefined || n.max_bytes === undefined
      ? undefined
      : `With this template the list would be ${formatQuantity(n.bytes)} bytes and the maximum is ${formatQuantity(n.max_bytes)}: shorten its instructions or delete another`,
  operation_conflict: () => "The ID already belongs to another operation",
  outcome_unknown: () => "Control already received; it will not be repeated automatically",
  permission_choice_unavailable: () => "This request offers once or deny",
  personality_too_large: (n) =>
    n.bytes === undefined || n.max_bytes === undefined
      ? undefined
      : `The personality is ${formatQuantity(n.bytes)} bytes; maximum ${formatQuantity(n.max_bytes)}`,
  rate_limited: () => "Transcription queue full; keep the audio",
  recording_deleted: () => "The recording was deleted; it accepts nothing else",
  recording_finalized: () => "Capture is closed; new chunks are not accepted",
  recording_id_ambiguous: () => "Two recordings have the same identifier on different Macs",
  recording_not_found: () => "Recording unavailable to this device",
  recording_still_open: () => "The recording is still open",
  revoked: () => "Device revoked",
  safety_capacity: () => "Too many active controls; local revocation remains available",
  safety_receipt_invalid: () => "Control receipt violates the contract",
  session_busy: () => "Wait or interrupt the current turn",
  session_disconnected: () =>
    "The session is not connected to the assistant; sending another message will attempt to resume if the previous turn is closed",
  session_message_too_large: () => "A conversation message does not fit in one page",
  session_resume_busy: () => "Hermes is still closing the previous turn (code 4009); wait",
  session_resume_closed: () => "The Orbit session closed; the message was not sent",
  session_resume_connection_lost: () => "Hermes was lost while resuming; the message was not sent",
  session_resume_history_unreadable: () =>
    "Hermes could not read stored history (code 5000); the message was not sent",
  session_resume_not_found: () =>
    "Hermes no longer retains the stored conversation (code 4007); local history is still available",
  session_resume_race: () => "Hermes closed the session while resuming (code 4007); try again",
  session_resume_storage_unavailable: () => "Hermes cannot open its session store (code 5000)",
  session_resume_too_large: () =>
    "Hermes refused to resume due to size (code 4130); no count or limit was supplied",
  session_resume_unsupported: () =>
    "This Hermes version cannot resume stored sessions (code -32601)",
  session_runtime_not_found: () => "The agent session is no longer active",
  shutting_down: () => "Server shutting down",
  sign_in_code_invalid: () =>
    "The code does not match the Claude page: copy it in full, including its «#»",
  sign_in_not_waiting_for_code: () => "This sign-in is not waiting for a code",
  skill_delete_unsupported: () =>
    "The Hermes panel cannot delete skills: disable it here or delete it on the Hermes server",
  skill_not_found: (n) =>
    n.name === undefined ? undefined : `The agent has no skill named «${formatQuantity(n.name)}»`,
  skill_too_large: (n) =>
    n.bytes === undefined || n.max_bytes === undefined || n.name === undefined
      ? undefined
      : `Skill «${formatQuantity(n.name)}» is ${formatQuantity(n.bytes)} bytes; maximum ${formatQuantity(n.max_bytes)}`,
  speech_empty: () => "The speech service delivered an empty clip",
  speech_not_configured: () => "Listening is not configured for this assistant",
  speech_too_large: () => "The speech clip exceeds the 4 MiB limit",
  speech_unavailable: () => "Hermes did not deliver the configured speech clip",
  stale_group_attempt: () => "Result belongs to a stale transcription attempt",
  stale_revision: (n) =>
    n.base_revision === undefined || n.revision === undefined
      ? undefined
      : `Private notes changed: revision ${formatQuantity(n.revision)}, the change started from ${formatQuantity(n.base_revision)}`,
  stale_turn: () => "The control does not belong to the current turn",
  storage_full: () => "The server disk is full and nothing was saved; free space on the server",
  too_large: () => "Message too large",
  too_many: () => "Too many pending captures",
  too_many_agents: (n) =>
    n.count === undefined || n.max === undefined
      ? undefined
      : `This hub already has ${formatQuantity(n.count)} assistants; maximum ${formatQuantity(n.max)}`,
  too_many_skills: (n) =>
    n.count === undefined || n.max === undefined
      ? undefined
      : `The agent already has ${formatQuantity(n.count)} skills; maximum ${formatQuantity(n.max)}. Delete one before creating another`,
  transcription_audio_empty: () => "Transcription audio is empty",
  transcription_connection_error: () => "Could not connect to the transcription service",
  transcription_connection_timeout: (n) =>
    n.timeout_ms === undefined
      ? undefined
      : `The transcription service connection took longer than ${formatQuantity(n.timeout_ms)} ms`,
  transcription_failed: () => "Meeting audio outside bounds",
  transcription_http_error: (n) =>
    n.provider_status === undefined
      ? undefined
      : `The transcription service returned HTTP ${formatQuantity(n.provider_status)}`,
  transcription_invalid_response: () => "The transcription service returned an invalid response",
  transcription_key_missing: () => "Set the OpenRouter key in Settings",
  transcription_protocol_error: () =>
    "The transcription service closed the response before completing it",
  transcription_provider_error: () =>
    "The transcription service failed before delivering a valid response",
  transcription_provider_refused: (n) =>
    n.duration_ms === undefined || n.provider_status === undefined
      ? undefined
      : `The transcription service did not accept ${formatQuantity(n.duration_ms)} ms of audio (HTTP ${formatQuantity(n.provider_status)})`,
  transcription_timeout: (n) =>
    n.timeout_ms === undefined
      ? undefined
      : `The transcription service took longer than ${formatQuantity(n.timeout_ms)} ms`,
  transcription_unavailable: () => "Transcription worker stopped; keep the audio",
  transcription_word_out_of_range: () => "Word timestamp outside its window",
  transcription_words_missing: () => "Whisper did not return word timestamps",
  turn_id_required: () => "Control requires the original turn identity",
  turn_no_longer_exists: () => "The turn is no longer in the transcript",
  unauthorized: () => "Device credential required",
  unknown_speaker: () => "The speaker is no longer in the transcript",
  unsupported_capture_type: () => "A PNG image is required",
  upstream_protocol: () => "Hermes omitted the durable identity",
  upstream_unavailable: () => "Hermes is not connected",
};
