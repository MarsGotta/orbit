import { agendaScenarios } from "./scenarios/agenda";
import { chatScenarios } from "./scenarios/chat";
import { chatMediaScenarios } from "./scenarios/chat-media";
import { designScenarios } from "./scenarios/design";
import { firstRunScenarios } from "./scenarios/first-run";
import { notesScenarios } from "./scenarios/notes";
import { panelScenarios } from "./scenarios/panel";
import { recordingScenarios } from "./scenarios/recording";
import { settingsScenarios } from "./scenarios/settings";
import { shellScenarios } from "./scenarios/shell";
import type { Scenario } from "./scenario";

/** Every scenario of the showcase, in the order the brief shows them. */
export const scenarios: Scenario[] = [
  ...shellScenarios,
  ...agendaScenarios,
  ...notesScenarios,
  ...chatScenarios,
  ...chatMediaScenarios,
  ...recordingScenarios,
  ...panelScenarios,
  ...settingsScenarios,
  ...firstRunScenarios,
  ...designScenarios,
];
