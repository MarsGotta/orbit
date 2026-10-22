import { calendarScenarios } from "./scenarios/calendar";
import { chatScenarios } from "./scenarios/chat";
import { chatCallScenarios } from "./scenarios/chat-call";
import { chatMediaScenarios } from "./scenarios/chat-media";
import { designScenarios } from "./scenarios/design";
import { firstRunScenarios } from "./scenarios/first-run";
import { grabarScenarios } from "./scenarios/grabar";
import { hoyScenarios } from "./scenarios/hoy";
import { noteMenuScenarios } from "./scenarios/note-menus";
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
  ...noteMenuScenarios,
  ...chatScenarios,
  ...chatMediaScenarios,
  ...chatCallScenarios,
  ...grabarScenarios,
  ...panelScenarios,
  ...settingsScenarios,
  ...firstRunScenarios,
  ...paletteScenarios,
  ...designScenarios,
];
