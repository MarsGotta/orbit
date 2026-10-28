export const transcription = {
  title: "Transcript",
  label: "Transcription model",
  whisper: "Whisper on your hub",
  mai: "MAI-Transcribe-2 (OpenRouter)",
  privacy:
    "Audio from meetings, voice notes and calls is sent to Microsoft through OpenRouter. Zero data retention (ZDR) is requested.",
  scope: "This choice is saved on the hub and applies to all its Macs.",
  key: "OpenRouter key",
  keyStored: "A key is already saved on the hub. Leave this empty to keep it.",
  keyHelp:
    "Validated before saving and stored encrypted on the hub. The key never returns to the Mac.",
  save: "Save transcription settings",
  saving: "Validating and saving…",
  saved: "Saved to the hub",
  loading: "Reading transcription settings from the hub…",
  unavailable: "Connect to the hub to change transcription settings.",
  invalid: "The hub didn’t return transcription settings.",
  compare: "Transcribe again with…",
} as const;

export const transcriptionRefusals: Record<string, string> = {
  transcription_key_missing: "Enter your OpenRouter key in Settings › Meetings.",
  transcription_key_invalid: "The OpenRouter key is invalid.",
  transcription_key_forbidden: "The OpenRouter key can’t access MAI.",
  transcription_quota_exhausted: "OpenRouter credit has run out.",
  transcription_rate_limited: "OpenRouter rate-limited requests. Try again.",
  transcription_connection_error: "Couldn’t connect to the transcription service.",
  transcription_timeout: "The transcription service took too long.",
};
