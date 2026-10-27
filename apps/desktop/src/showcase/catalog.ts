import { vaultScenarios } from "./scenarios/vault";
import { assistantsScenarios } from "./scenarios/assistants";
import { assistantsGearScenarios } from "./scenarios/assistants-gear";
import { assistantsLimitScenarios } from "./scenarios/assistants-limit";
import { assistantsSignInScenarios } from "./scenarios/assistants-sign-in";
import { calendarScenarios } from "./scenarios/calendar";
import { chatScenarios } from "./scenarios/chat";
import { chatAnswerScenarios } from "./scenarios/chat-answers";
import { chatCallScenarios } from "./scenarios/chat-call";
import { chatMediaScenarios } from "./scenarios/chat-media";
import { designScenarios } from "./scenarios/design";
import { designAnnexScenarios } from "./scenarios/design-annex";
import { editorScenarios } from "./scenarios/editor";
import { firstRunScenarios } from "./scenarios/first-run";
import { grabarScenarios } from "./scenarios/grabar";
import { hoyScenarios } from "./scenarios/hoy";
import { noteMenuScenarios } from "./scenarios/note-menus";
import { noteRedoScenarios } from "./scenarios/note-redo";
import { noteWordScenarios } from "./scenarios/note-words";
import { notesScenarios } from "./scenarios/notes";
import { panelScenarios } from "./scenarios/panel";
import { paletteScenarios } from "./scenarios/palette";
import { settingsScenarios } from "./scenarios/settings";
import { shellScenarios } from "./scenarios/shell";
import { windowScenarios } from "./scenarios/window";
import type { Scenario } from "./scenario";

/** Every scenario of the showcase, in the order the brief shows them. */
export const scenarios: Scenario[] = [
  ...shellScenarios,
  ...windowScenarios,
  ...hoyScenarios,
  ...calendarScenarios,
  ...notesScenarios,
  ...vaultScenarios,
  ...noteMenuScenarios,
  ...editorScenarios,
  ...noteWordScenarios,
  ...noteRedoScenarios,
  ...chatScenarios,
  ...chatAnswerScenarios,
  ...chatMediaScenarios,
  ...chatCallScenarios,
  ...assistantsScenarios,
  ...assistantsSignInScenarios,
  ...assistantsLimitScenarios,
  ...assistantsGearScenarios,
  ...grabarScenarios,
  ...panelScenarios,
  ...settingsScenarios,
  ...firstRunScenarios,
  ...paletteScenarios,
  ...designScenarios,
  ...designAnnexScenarios,
];
