/// <reference types="node" />
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";
import { spanish } from "./es";
import { english } from "./en";
import { setLocale } from "./locale";
import { configureCopy } from "./index";
import { sessionWhen } from "../features/assistant/session-title";

function flatten(value: unknown, prefix = "", result: Record<string, string> = {}) {
  if (typeof value === "string") result[prefix] = value;
  else if (value && typeof value === "object") {
    for (const [key, child] of Object.entries(value)) {
      flatten(child, prefix ? `${prefix}.${key}` : key, result);
    }
  }
  return result;
}

function catalogue(language: "es" | "en") {
  setLocale(language === "es" ? "es-ES" : "en-US");
  const copy = language === "es" ? spanish() : english();
  configureCopy(language, [language === "es" ? "es-ES" : "en-US"], copy.shell.assistantName);
  const result = flatten(copy);
  // Evaluate the same formatters as the app. Named tokens remain parameters
  // for Python; punctuation and all words still belong to the catalogue.
  result["noteScreen.howLabel"] = copy.noteScreen.howLabel("{template}", "{language}", 0);
  result["notes.mutedMicrophone"] = copy.notes
    .mutedMicrophone(0, 1000)
    .replace("00:00", "{start}")
    .replace("00:01", "{end}");
  result["problem.missingPermission"] = copy.problem.missingPermission("{name}");
  result["shellInline.todayAt"] = copy.shellInline.todayAt("{time}");
  result["shellInline.yesterdayAt"] = copy.shellInline.yesterdayAt("{time}");
  // Evaluate the app's actual session formatter, including its regional
  // clock (en-US uses AM/PM). Python never reconstructs its punctuation.
  const today = new Date(2026, 0, 1, 12, 7).getTime();
  const prefix = copy.shellInline.todayAt("{time}").split("{time}")[0]!;
  for (let hour = 0; hour < 24; hour++) {
    const at = new Date(2026, 0, 1, hour, 7).getTime();
    result[`sessionClock.${hour}`] = sessionWhen(at, at)
      .slice(prefix.length)
      .replace("07", "{minute}");
  }
  const clock = result["sessionClock.12"]!.replace("{minute}", "07");
  for (let month = 0; month < 12; month++) {
    const at = new Date(2026, month, 18, 12, 7).getTime();
    result[`sessionDate.${month + 1}`] = sessionWhen(at, today)
      .replace(clock, "{time}")
      .replace("18", "{day}");
    result[`sessionDateYear.${month + 1}`] = sessionWhen(at, new Date(2027, 0, 1).getTime())
      .replace(clock, "{time}")
      .replace("18", "{day}")
      .replace("2026", "{year}");
  }
  result["notices.recording_discarded.text"] = copy.extras.noticesForAssistant().recording_discarded
    .text!({});
  result["notices.recording_silence_question.text"] = copy.extras.noticesForAssistant()
    .recording_silence_question.text!({});
  result["noteScreen.summaryLater"] = copy.noteScreen
    .summaryLater("{template}")
    .split("{template}")[0]!;
  result["noteScreen.howPrefix"] = result["noteScreen.howLabel"]!.split("{template}")[0]!;
  result["notes.mutedPrefix"] = result["notes.mutedMicrophone"]!.split("{start}")[0]!;
  result["problem.missingPermissionPrefix"] =
    result["problem.missingPermission"]!.split("{name}")[0]!;
  result["assistant.composerPrefix"] = copy.assistant.composerLabel
    .slice(0, -copy.shell.assistantName.length)
    .trimEnd();
  result["recordingCard.wakePhrase"] = copy.recordingCard.listening.match(/[«“](.*?)[»”]/)![1]!;
  return result;
}

test("the real-test catalogue export is current", () => {
  const path = resolve(process.cwd(), "../../scripts/e2e-mac/ui-copy.json");
  const generated = { es: catalogue("es"), en: catalogue("en") };
  if (process.env.ORBIT_UPDATE_TEST_CATALOGUES === "1") {
    writeFileSync(path, JSON.stringify(generated, null, 2) + "\n");
  }
  expect(
    JSON.parse(readFileSync(path, "utf8")),
    "refresh it with ORBIT_UPDATE_TEST_CATALOGUES=1 pnpm exec vitest run src/i18n/harness-export.test.ts",
  ).toEqual(generated);
  configureCopy("en", ["en-US"], "Deimos");
  setLocale("en-US");
});
