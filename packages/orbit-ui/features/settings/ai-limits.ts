// Bound provider labels, destinations and write-only keys in the settings
// document. These are not model, audio or inference limits. The shared copies
// in the hub are guarded by contracts/constants.json.
export const MAX_AI_PROVIDER_NAME_CHARS = 80;
export const MAX_AI_PROVIDER_URL_CHARS = 2048;
export const MAX_AI_PROVIDER_KEY_CHARS = 4096;
