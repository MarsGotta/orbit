import { formatQuantity } from "../locale";
export const aiCopy = {
  title: "AI",
  lead: "Your providers and the model for each job. Keys are stored encrypted on the hub.",
  functions: {
    meeting_transcription: "Meeting transcription",
    listening: "Voice notes and calls: listening",
    speech: "Assistant voices: speaking",
    meeting_notes: "Meeting summary and note",
    vault: "Notes distillation and review",
    assistants: "OpenAI-compatible assistants",
  } as const,
  capabilities: {
    text: "Text",
    tools: "Tools",
    transcription: "Transcript",
    word_timestamps: "Word timestamps",
    speech: "Voice",
  } as const,
  unavailable: "Couldn’t read AI settings. Check the connection to your hub.",
  saved: "Saved to the hub",
  providerName: "Provider name",
  baseUrl: "Base URL",
  key: "Key (optional)",
  keyHelp: "Leave empty to keep the key saved for the same URL. It never returns from the hub.",
  add: "Add a provider",
  save: "Check and save",
  saving: "Checking on the hub…",
  local: "Local AI",
  cloud: "Cloud, with a key",
  operator: "Operator configuration",
};

const aiRefusals: Record<string, string> = {
  ai_key_invalid: "The provider rejected the key",
  ai_key_missing: "Enter your OpenRouter key",
  ai_provider_unreachable: "Couldn’t connect to the provider; check its URL",
  ai_invalid_catalog: "The provider returned an invalid model catalogue",
  ai_settings_invalid:
    "Check the provider’s name, URL, models and key. The URL requires HTTPS except on loopback, and must not include credentials, a query or a fragment",
  ai_settings_busy: "An AI check is already running; wait until it finishes",
  ai_provider_invalid: "Check the provider ID and URL; the OpenRouter URL can’t be changed",
  ai_provider_missing: "The selected provider isn’t configured",
  ai_capability_missing: "The model lacks the capability this function needs",
  ai_function_invalid: "This AI function doesn’t exist",
  ai_models_invalid: "Only change models returned by the provider",
};

export function aiRefusalText(
  refusal: import("../../bridge/generated/AgentRefusal").AgentRefusal,
): string | undefined {
  const n = refusal.numbers;
  switch (refusal.code) {
    case "ai_provider_timeout":
      return `The provider didn’t finish checking within ${formatQuantity(n.timeout_ms)} ms.`;
    case "ai_provider_http_error":
      return `The provider returned HTTP ${String(n.provider_status)}. Check its credit, quota or availability.`;
    case "ai_catalog_size":
      return `The catalogue has ${formatQuantity(n.count)} models; it must have between 1 and ${formatQuantity(n.max_count)}.`;
    case "ai_providers_full":
      return `There are ${formatQuantity(n.count)} providers; the maximum is ${formatQuantity(n.max_count)}.`;
    case "ai_response_too_large":
    case "ai_registry_full":
      return `The settings or response take ${formatQuantity(n.bytes)} bytes; the maximum is ${formatQuantity(n.max_bytes)}.`;
    default:
      return Object.hasOwn(aiRefusals, refusal.code)
        ? `${aiRefusals[refusal.code]}${n.provider_status === undefined ? "" : ` (HTTP ${String(n.provider_status)})`}.`
        : undefined;
  }
}
