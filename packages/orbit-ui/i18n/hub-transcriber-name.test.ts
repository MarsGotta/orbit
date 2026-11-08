import { expect, it } from "vitest";
import { hubAdmissionRefusals as esAdmission } from "./es/hubAdmissionRefusals";
import { hubAdmissionRefusals as enAdmission } from "./en/hubAdmissionRefusals";
import { hubRequestRefusals as esRequest } from "./es/hubRequestRefusals";
import { hubRequestRefusals as enRequest } from "./en/hubRequestRefusals";
import { settingsInline as esInline } from "./es/settingsInline";
import { settingsInline as enInline } from "./en/settingsInline";
import { transcription as esTranscription } from "./es/transcription";
import { transcription as enTranscription } from "./en/transcription";

// The hub's own transcriber is whatever its operator configured (Parakeet
// since 2026-11-05); `whisper` stays only as the wire identifier.
it("names the hub's transcriber without calling it Whisper, in both languages", () => {
  const read = [
    esTranscription.whisper,
    enTranscription.whisper,
    esInline.hubModelsHelp,
    enInline.hubModelsHelp,
    esRequest.transcription_words_missing(),
    enRequest.transcription_words_missing({}),
    esAdmission.final_window_too_large(),
    enAdmission.final_window_too_large({}),
    esAdmission.invalid_transcription_setting(),
    enAdmission.invalid_transcription_setting({}),
  ];
  for (const text of read) expect(text).not.toMatch(/whisper/i);
  expect(esTranscription.whisper).toBe("El modelo de tu hub");
  expect(enTranscription.whisper).toBe("Your hub's model");
});
