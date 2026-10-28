import type { HubAdmissionCode } from "../es/hubAdmissionRefusals";
import { formatQuantity } from "../locale";
import type { Numbers } from "./refusalNumbers";

export const hubAdmissionRefusals: Record<HubAdmissionCode, (n: Numbers) => string | undefined> = {
  agent_add_unsupported: () => "This hub cannot add assistants",
  agent_name_taken: () => "You already have another assistant with that name",
  agent_protocol: () => "Codex sent an invalid JSON message",
  agent_stopped: () => "The connection closed while the agent was starting",
  ai_capability_missing: () => "The model lacks the capability this function needs",
  ai_function_invalid: () => "The AI function does not exist",
  ai_key_missing: () => "Enter the OpenRouter key",
  ai_models_invalid: () => "Correct only the models returned by the provider",
  ai_provider_invalid: () => "Invalid provider identifier",
  ai_provider_missing: () => "The transcription provider no longer exists",
  ai_providers_full: (n) =>
    n.count === undefined || n.max_count === undefined
      ? undefined
      : `There are ${formatQuantity(n.count)} providers; the maximum is ${formatQuantity(n.max_count)}`,
  ai_registry_full: (n) =>
    n.bytes === undefined || n.max_bytes === undefined
      ? undefined
      : `The configuration is ${formatQuantity(n.bytes)} bytes; the maximum is ${formatQuantity(n.max_bytes)}`,
  ai_settings_busy: () => "An AI check is already in progress; wait for it to finish",
  ai_settings_invalid: () => "Check the provider name, URL, models and key",
  assistant_failed: () =>
    "The assistant failed before finishing the response; it will not be repeated",
  assistant_tools_unavailable: (n) =>
    n.timeout_ms === undefined
      ? undefined
      : `orbit-mcp did not start within ${formatQuantity(n.timeout_ms)} ms; the assistant has no Orbit tools`,
  assistant_turn_too_long: (n) =>
    n.max_requests === undefined
      ? undefined
      : `The assistant made ${formatQuantity(n.max_requests)} model requests in this turn without finishing; it was stopped`,
  attachment_not_found: () => "That attachment is not on the hub",
  attachment_refused: (n) =>
    n.upstream_code === undefined
      ? undefined
      : `Hermes did not accept a message image (code ${formatQuantity(n.upstream_code)})`,
  attachment_type_unsupported: (n) =>
    n.mime === undefined || n.name === undefined
      ? undefined
      : `«${formatQuantity(n.name)}» is ${formatQuantity(n.mime)}: no agent can read it. You can attach PNG, JPEG, GIF or WebP images, PDFs and text files`,
  attachment_unreadable: () =>
    "Could not prepare message attachments for the agent; the message was not sent",
  attachments_too_many: (n) =>
    n.files === undefined || n.max_files === undefined
      ? undefined
      : `This message has ${formatQuantity(n.files)} attachments; the ceiling is ${formatQuantity(n.max_files)} per message`,
  capture_duplicate: () => "The capture is already stored",
  capture_missing: () =>
    "The capture is no longer on the hub (it expires after five minutes or was already used)",
  chunk_conflict: () => "Capture metadata differs for the same chunk",
  clarification_question_invalid: () => "That question is not pending",
  configuration: () => "Hermes credential missing or invalid",
  diarization_invalid_response: () => "Invalid separator response",
  diarization_not_configured: () => "Local separation configuration missing",
  diarization_output_too_large: () => "Separator response too large",
  diarization_process_failed: () => "The separation process failed",
  diarization_start_failed: () => "Could not start the separator",
  dictionaries_full: (n) =>
    n.dictionaries === undefined || n.max_dictionaries === undefined
      ? undefined
      : `There are ${formatQuantity(n.dictionaries)} dictionaries; the maximum is ${formatQuantity(n.max_dictionaries)}`,
  dictionaries_too_large: (n) =>
    n.bytes === undefined || n.max_bytes === undefined
      ? undefined
      : `The list is ${formatQuantity(n.bytes)} bytes; the maximum is ${formatQuantity(n.max_bytes)}`,
  dictionary_forms_full: (n) =>
    n.forms === undefined || n.max_forms === undefined
      ? undefined
      : `There are ${formatQuantity(n.forms)} incorrect forms; the maximum is ${formatQuantity(n.max_forms)}`,
  dictionary_full: (n) =>
    n.max_terms === undefined || n.terms === undefined
      ? undefined
      : `There are ${formatQuantity(n.terms)} terms; the maximum is ${formatQuantity(n.max_terms)}`,
  dictionary_id_ambiguous: () => "Two dictionaries have the same identifier",
  dictionary_not_found: () => "The dictionary does not exist",
  dictionary_series_full: (n) =>
    n.max_series === undefined || n.series === undefined
      ? undefined
      : `There are ${formatQuantity(n.series)} series; the maximum is ${formatQuantity(n.max_series)}`,
  dictionary_series_missing: () => "The meeting has no calendar series",
  dictionary_text_invalid: (n) =>
    n.field === undefined
      ? undefined
      : `Field ${formatQuantity(n.field)} is empty or contains control characters`,
  dictionary_text_too_long: (n) =>
    n.characters === undefined || n.field === undefined || n.max_characters === undefined
      ? undefined
      : `Field ${formatQuantity(n.field)} has ${formatQuantity(n.characters)} characters; the maximum is ${formatQuantity(n.max_characters)}`,
  dictionary_too_large: (n) =>
    n.bytes === undefined || n.max_bytes === undefined
      ? undefined
      : `The dictionary is ${formatQuantity(n.bytes)} bytes; the maximum is ${formatQuantity(n.max_bytes)}`,
  final_audio_conversion_failed: () => "Could not convert the audio",
  final_audio_empty: () => "The recording has no readable audio",
  final_audio_time_out_of_range: () => "The chunk starts outside the recording",
  final_audio_timeout: () => "Audio conversion exceeded its deadline",
  final_window_too_large: () => "The Opus window exceeds the Whisper limit",
  group_not_found: () => "Group unavailable",
  handover_no_question: () =>
    "This conversation has no unanswered question to hand over; write the question",
  hash_mismatch: () => "The hash does not match received bytes",
  interaction_not_found: () => "This assistant does not request authorizations",
  invalid_attachment: (n) =>
    n.max_characters === undefined
      ? undefined
      : `The attachment name is invalid: 1 to ${formatQuantity(n.max_characters)} characters, without control characters or «/»`,
  invalid_audio: () => "Valid 16-bit PCM WAV required",
  invalid_capture: () => "The capture digest does not match",
  invalid_chunk: () => "Clock/format metadata is incomplete or differs from the WAV",
  invalid_dictionary_id: () => "Invalid dictionary identifier",
  invalid_group_audio: () => "Audio source count violates the contract",
  invalid_manifest: () => "Invalid closure manifest",
  invalid_note_request: () => "Invalid recording identifier",
  invalid_note_template: (n) =>
    n.max === undefined
      ? undefined
      : `Invalid template identifier: lowercase letters, digits, «-» or «_», up to ${formatQuantity(n.max)} characters`,
  invalid_pairing: () => "Invalid, used or expired code",
  invalid_parameters: (n) =>
    n.names !== undefined
      ? `A turn allows only one name (names: ${formatQuantity(n.names)}, max: ${formatQuantity(n.max)})`
      : "Invalid reminder key: expected the title SHA-256 in hexadecimal",
  invalid_payload: () => "Empty payload",
  invalid_recording: () => "Invalid recording ID",
  invalid_request: () => "Invalid pairing request",
  invalid_transcription_setting: () => "Choose Whisper or MAI for transcription",
  invalid_working_folder: () => "This assistant does not use a hub working folder",
  manifest_conflict: () => "The recording already has a different closure manifest",
  manifest_mismatch: () => "Manifest tracks do not match durable chunks",
  meeting_audio_unreadable: (n) =>
    n.groups === undefined || n.total === undefined
      ? undefined
      : `The hub cannot read the audio for ${formatQuantity(n.groups)} of ${formatQuantity(n.total)} groups`,
  meeting_not_found: () => "The meeting has not reached the hub yet",
  meeting_series_missing: () =>
    "The meeting has no calendar series: neither template nor dictionary was saved",
  method_forbidden: () => "Hermes method violates the contract",
};
