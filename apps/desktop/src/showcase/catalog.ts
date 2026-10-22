import { calendarScenarios } from "./scenarios/calendar";
import { chatScenarios } from "./scenarios/chat";
import { chatMediaScenarios } from "./scenarios/chat-media";
import { designScenarios } from "./scenarios/design";
import { firstRunScenarios } from "./scenarios/first-run";
import { grabarScenarios } from "./scenarios/grabar";
import { hoyScenarios } from "./scenarios/hoy";
import { notesScenarios } from "./scenarios/notes";
import { panelScenarios } from "./scenarios/panel";
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
  ...chatScenarios,
  ...chatMediaScenarios,
  ...grabarScenarios,
  ...panelScenarios,
  ...settingsScenarios,
  ...firstRunScenarios,
  ...designScenarios,
];
